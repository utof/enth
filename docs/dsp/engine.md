# Engine (`enth-engine`, `enth-render`)

- Lanes: P1a S2a, the walking skeleton (freeze point F2), then S2b: the voice manager, smoothing, the modulation matrix and the plan swap. After F2, changes to the types below are additive only.
- Code: `crates/enth-engine/src/` (module contract, plan compiler, segment loop, voice manager, smoothers and modulation, plan swap, stub modules), `crates/enth-render/src/lib.rs` (offline render), `crates/enth-dsp/src/util/{math,rng}.rs` (the `libm` door and seeded randomness), `tests/golden/` (cases and the patch source).
- Decisions behind it: [design.md §2, §3 and §5](../design.md#3-engine-and-real-time-safety), [architecture.md](../architecture.md), research report 01 §5–§7 (`_local/sessions/2026-10-19-p1a-headless-core/`).

The engine turns a `Patch` into stereo `f32` the same way on every platform and for every host block size. One reference saw goes through the whole path (module, plan, segment loop, render, golden) and hashes the same on x86-64 Linux and wasm32 (Windows runs in CI). Every other module is a clearly marked stub that its lane replaces.

## Data flow

```text
Patch ──plan::compile(patch, &PrepareCfg)──▶ (Plan, PlanMap) ──Engine::new(plan, render_seed)──▶ Engine   (offline)
                                                            └─Engine::with_link(plan, seed, cap)──▶ (Engine, Link)  (real time)
Engine::process(left, right, &[Event])      any number of frames, events timed by offset in the block
Link::send_plan(plan) / send_param(epoch, id, v) / send_macro / collect()      the main thread's end of the rings
enth_render::render(patch, perf, spec)      the offline wrapper: compile, schedule events, call process in blocks
```

- `compile` allocates: it prepares every module (`ModuleImpl::new`) and sizes every table. Nothing after it allocates.
- `PlanMap` maps parameter addresses (`filter1.cutoff`, `voice.pan`, `master.level`) to the plan's `ParamId`s. It stays off the audio thread, where an op or an automation point becomes a value event.
- The engine API added by S2b: `Engine::with_link` and `Link` (plan swap, epoch-tagged value events, macros, collecting old plans), `Engine::set_monitor_bypass`/`monitor_bypass` (the plugin's FX bypass), `Engine::note_ends` (CLAP `NOTE_END`), `Engine::epoch`, `is_crossfading`, `plans_held`, `voices`, `notes`; `EventKind::{Macro, ModWheel, Choke}`; `EngineStats::{stolen_voices, cut_voices, stale_events, swaps, crossfades, dropped_note_ends}`; `CompileError::BadRoute`; `VoiceState::Stolen`; `NoteEnd`, `LinkCapacity`, `SendError`, `MAX_PLANS_IN_FLIGHT`, `CROSSFADE_FRAMES`.

## Modules: four roles, enum dispatch

| Role | Trait | Kinds | Called |
| --- | --- | --- | --- |
| Voice processor | `VoiceProcessor` | `osc`, `noise` (generators), `svf` (per-voice effect) | `start_voice` at a note-on, `process` once per segment over the active voices |
| Voice modulator | `VoiceModulator` | `adsr`, `lfo` with `scope = voice` | `start_voice`, `tick` once per segment (writes one value per voice), `voice_finished` |
| Global modulator | `GlobalModulator` | `lfo` with `scope = global` | `reset` at start, `tick` once per segment (returns one value) |
| Global processor | `GlobalProcessor` | `reverb`, `comp` | `reset` at start, `process` once per segment on the stereo bus, `is_silent` |

Every role also has `copy_state_from` (for the plan swap; never allocates; a lane's module must copy *all* its state, which `tests/swap.rs` checks for every kind) and `subnormals` (counts subnormal state, for the tail test).

`modules::ModuleImpl` holds one enum per role (`VoiceProcessorImpl`, `VoiceModulatorImpl`, `GlobalModulatorImpl`, `GlobalProcessorImpl`). `ModuleImpl::new(kind, values, cfg)` is an exhaustive `match ModuleKind`, so a catalogue kind without an implementation does not compile (checked: removing the `comp` arm gives `error[E0004]`). The compiler sorts modules into per-role lists, so the segment loop never meets a module in the wrong role and has no `unreachable!`. Dispatch happens once per module per segment.

### The contract

- **`Type::new(values, cfg)` is `prepare`.** It may allocate, sized from `PrepareCfg` only (the sample rate). A structural parameter (`osc.table`) may choose *what* to compute, never how much memory.
- **One call per module per segment** with the active voices in ascending order. Per-voice state is `[T; MAX_VOICES]` (structure of arrays), so a later SIMD path can run 4–8 voices per lane group without changing the API.
- **Parameters** come from views: `params.base(p)` (the patch value or the latest value event), `params.cur(p, v)` and `params.prev(p, v)` (this and the previous segment's value, for per-sample ramps; equal on a voice's first segment). Index them with the catalogue enums (`params.cur(OscParam::Semi, v)`). Discrete parameters: `Wave::from_value(params.base(OscParam::Wave))`, which never panics.
- **Generators overwrite** `io[v]` for every active voice (the engine sums them in patch order); per-voice effects transform `io[v]` in place; global processors transform `left`/`right` in place.
- **Randomness** only from `VoiceStart::seed` / `GlobalStart::seed` (or `seed::seeded_voice_seed` with the module's own `seed` parameter in the `seeded` phase mode), through `enth_dsp::util::rng`.
- **The four DSP rules**: transcendentals through `enth_dsp::util::math` (pinned `libm`); no `mul_add`; fixed summation order; flush recursive state with `enth_dsp::util::flush::flush`, at least once per segment. `voice_finished` is true past the attack and below −100 dB (`math::VOICE_END_LEVEL`).
- `start_voice` resets the slot (it may hold a previous note) and reads `params.base`; `notes` already holds the new note.

### Stubs (S2a)

| Kind | File | Stub | Replaced by |
| --- | --- | --- | --- |
| `osc` | `modules/osc.rs` | Reference saw: naive saw minus a two-sample polyBLEP residual (Välimäki and Huovilainen 2007), pitch `key + semi + fine/100` per segment, level ramped per sample, the three phase modes; writes the hard-sync timing when it is the master. Other waves are silent. S0's `analog.rs` and `wavetable.rs` are merged here: one `osc` kind | D1 (Plaits port, sine, wavetables) |
| `noise` | `modules/noise.rs` | Silent | D1 |
| `svf` | `modules/svf.rs` | Pass-through | D2 |
| `adsr` | `modules/adsr.rs` | Reference envelope: linear attack, exponential decay and release that fall 60 dB in the set time, evaluated per segment, flushed. Not silent: it is every patch's amplitude envelope | D2 |
| `lfo` | `modules/lfo.rs` | Outputs 0 (both roles) | D2 |
| `reverb` | `modules/reverb.rs` | Pass-through, `is_silent` true | FX1 phase 2 |
| `comp` | `modules/comp.rs` | Pass-through, `is_silent` true | FX2 phase 2 |

## Parameters and smoothing

Each plan has two flat tables: the voice table (modules that run per voice, and `voice.*`) and the global table (global modules, `master.*`, then the eight macros and the mod wheel). A slot holds `base`, a smoother, and `cur` and `prev`, per voice in the voice table. `ParamId::Voice(i)` or `ParamId::Global(i)` names a slot.

- **Settled values are exact.** Without smoothing in flight and without a route, `cur` is `base`, the plain value from the patch, bit for bit. It never goes through `from_norm(to_norm(x))`, which is not guaranteed to round trip.
- A value event (`EventKind::Param`, or `Link::send_param` with the plan's epoch) sets `base`, clamped to the spec; NaN and unknown ids are ignored.
- **Smoothers** (`params::Smoother`, step 3) follow `base` in the normalised domain (`ParamSpec::to_norm`/`from_norm`, pinned `libm`), once per segment, as the catalogue's `Smoothing` says: `Linear { ms }` is a ramp of `max(1, round(ms·sr/1000/32))` segments at constant time (a new target mid-ramp restarts the full ramp from where the value is; 10 ms = 15 segments at 48 kHz); `OnePole { ms }` moves `n += (target − n)·(1 − e^(−32/frames))` until within 1e-6, then snaps; `None` steps at the boundary. A smoother's last step lands on the exact `base`. Smoothers start settled at load (no initial ramp). In P1a every smoothed parameter is `Linear` (10 ms, `master.level` and the reverb's filters 20 ms).
- **Smoothers are per parameter, shared by all voices**, so every voice reads the same smoothed value and parameter changes reach held voices. **A trigger restarts a voice's ramps** ("smoother reset on retrigger"): on a fresh voice's first segment and on a mono retrigger, `prev = cur` for that voice, so no module ramps from a stale value.
- **Quiet tables cost nothing.** When no base was written, no smoother is moving and `prev` is level with `cur`, step 3 writes only the restarting voices and the routed parameters' `prev`, which gives the same values as the full pass (the S2a loop that copied every parameter for every voice is gone; the `engine` bench went down 6–7 %).
- Macros and the mod wheel are global-table slots with the `MACRO_VALUE` spec (0–1, smoothed over 10 ms); the macros start at the patch's values, the mod wheel at 0. They are set by `EventKind::Macro { index, value }` / `EventKind::ModWheel { value }` or `Link::send_macro`/`send_mod_wheel`, and are never in the `PlanMap`.

## Modulation

`modulation.rs`; compiled by `plan::compile` into one list of destinations per evaluation stage, each with its routes in patch order.

```text
n   = clamp(smoothed_norm + Σ_routes (depth / 100) · source, 0, 1)      summed in patch route order
cur = from_norm(n)
```

- **Depth** is a percentage of the destination's range in its normalised domain (design.md §2): `+1 oct` on a 20 Hz–20 kHz Log cutoff is 10.034 %. **Polarity belongs to the source**: envelopes, velocity, macros and the mod wheel run 0–1, an LFO is bipolar or unipolar by its own `polarity` (lane D2 implements it in the module), and `key` is `(pitch − 60) / 120` with the voice's current, glided pitch.
- **Stages.** A global modulator's modulated parameters are evaluated just before it ticks (step 4), a voice modulator's just before it ticks (step 5), everything else (generators, `voice_fx`, `fx`, `voice.*`, `master.*`) in step 6. A route from a modulator that ticks later reads its previous segment's value: a deterministic one-segment delay that makes cycles harmless.
- Voice-rate destinations are evaluated per active voice in ascending order; one whose routes are all global is computed once and copied (the same bits). A parameter with a route always takes `from_norm(n)`; one without keeps its exact plain value.
- **The compiler checks routes** (a subset of S1b's `validate`, which is not merged yet): a module source must name a modulator, the destination must exist and be `modulatable`, `|depth| ≤ 100`, and a voice-rate source (velocity, key, a voice-scope modulator) never targets a global destination (`CompileError::BadRoute`, code `bad_route`, with a reason: `unknown_source`, `not_a_modulator`, `unknown_destination`, `not_modulatable`, `depth_out_of_range`, `route_rate_mismatch`). A route from or to a disabled module, or with depth 0, is left out (it contributes nothing).
- Cost: `from_norm` per modulated parameter, per voice, per segment (Log: two `libm` calls in `f64`).

## The segment grid and the host adapter

`SEG = 32` frames is the processing quantum. `Engine::process` copies frames out of a one-segment FIFO and renders a new segment whenever it runs dry. Before rendering the segment that starts at frame `32·k`, it applies every queued event whose frame is at or before `32·k`, in arrival order. So an event at frame `f` takes effect at `ceil(f / 32) · 32` (0.67 ms at 48 kHz), and the grid is counted from the engine's start, never from the start of a host call: any sequence of block sizes gives the same bits (tested for 1 to 36,000 frames and random block sequences). Blocks are never split at event times.

Events wait in a fixed-capacity queue (`segment::EVENT_CAPACITY` = 4,096). When it is full, a new event is dropped and counted (`EngineStats::dropped_events`); nothing reallocates.

## The order of one segment

1. The command ring (real-time engines only): plan swaps (state carried; a crossfade if modules were added or removed), epoch-tagged value events (stale ones dropped and counted), macros. See "Plan swap".
2. Due events, in arrival order: note-ons (the voice manager picks a slot or retriggers; a fresh voice gets `start_voice` on every voice module with its seed and its modulator outputs restart from 0), note-offs (gate down; in mono and legato, a return to the newest held key), value events, macros, the mod wheel, chokes. Then every gliding voice moves one segment.
3. Smoothers (see "Parameters and smoothing"); a trigger restarts the voice's ramps.
4. Global modulators, in patch order, each after its own modulated parameters.
5. Voice modulators, in patch order, each after its own modulated parameters, over the active voices.
6. The remaining modulated parameters (see "Modulation").
7. Voices: each voice bus is zeroed, generators write a scratch bus and are summed in patch order (the first enabled `osc` first writes the hard-sync timing that later generators read), then the `voice_fx` chain, then the amp stage (times the steal fade of a stolen voice) and the pan into the stereo sum in ascending voice order.
8. `fx` in order (a processor that `is_silent` with an all-zero input is skipped, which must be bit-identical to running it; with the monitor bypass on, `time_based` ones are skipped), then `master.level`.
9. Voices whose amplitude envelope reports `voice_finished` (in every plan that runs), or whose steal fade is over, are freed and reported (`Engine::note_ends`). Modules flush their own state; nothing in the engine recurses.
10. The FIFO is filled (a debug assertion checks every sample is finite).

During a crossfade, steps 3–8 run for both plans (the replaced one first) over the same notes and voices.

## Voices, the amp stage and the pan law

`voice.rs`. Deterministic: no randomness, no clock, no dependence on the host's block size; the same performance gives the same slots, so the same summation order (unit-tested). Notes are addressed by CLAP's (port, channel, key, note id); queries (note-off, choke) take −1 wildcards. Every note-on takes the next `note_index`, whatever happens to it, so stealing and mode changes never move a seed.

- **Poly** (`voice.mode = poly`): a note-on takes the lowest free of 32 slots while fewer than `voice.polyphony` (≤ 16) voices are held or releasing. When polyphony is full it **steals the oldest released voice, else the oldest held one** ("oldest" = the earliest note-on; ties cannot happen). The victim (`VoiceState::Stolen`) fades to silence linearly over 64 frames (two segments, `1 → ½ → 0`, per sample, times the amp ramp) and is freed; the new note starts in a spare slot. Only a burst of more than 16 steals within two segments can find all 32 slots busy: the new note then takes the fading voice with the least fade left (a cut without a fade, counted in `cut_voices`). Nothing is ever dropped (`dropped_notes` stays 0). A same-key note-on starts a new voice; a lower polyphony from a structural edit is enforced by the next note-on's steals.
- **Mono and legato** play one voice with a **16-key last-note stack** (newest on top; a 17th key pushes out the oldest). A note-on while a key is held: mono **retriggers** (envelopes restart their attack from their current level, the new velocity applies), legato changes the pitch only. A note-on with no key held retriggers the voice while it still sounds (both modes) or starts a fresh voice once it has ended. Releasing the sounding key returns to the newest key still held (a pitch change, gliding if glide is on, no retrigger); releasing the last key releases the voice. The voice keeps its first note's `note_index` (and its seeds) until it ends. `voice.polyphony` is ignored. A mode change (structural) adopts the newest held voice and rebuilds the stack from the held voices; left-over poly voices still release on their note-offs.
- **Glide** (`voice.glide` ms, 0 = off; `voice.glide_mode`): linear in semitones at constant time, `max(1, round(glide·sr/1000/32))` segments (100 ms = 150 at 48 kHz) from the current pitch to the new one, whatever the interval; a new note mid-glide starts a new glide from where the pitch is; the last step lands exactly. `always` glides every new note: a retrigger or legato change from the voice's pitch, a fresh voice (poly or mono) from the previous note-on's pitch; `legato` glides only when another key is held at the note-on. Glide writes `NoteState::pitch_prev` (the pitch at the segment start) and `pitch_cur` (at its end); the oscillator ramps between them. Glide works in poly mode too (report 01 had it off in poly; the catalogue's description and the voice-mode goldens include it).
- **Choke** (`EventKind::Choke`, CLAP `NOTE_CHOKE`): matching playing voices fade out over 64 frames like a steal (not counted as steals).
- **Voice end:** a voice is freed at the end of a segment in which the amplitude envelope (`voice.amp_env`) is past its attack and below −100 dB, or its steal fade is over. Its slot costs nothing afterwards. **Every end and every cut is reported** as `NoteEnd { key, frame }` (the absolute frame where it went silent, on the grid): `Engine::note_ends()` lists those of the last `process` call (up to 256; more are counted in `dropped_note_ends`), for CLAP `NOTE_END` in P1b.
- **`NoteState`** (per slot): `pitch_prev`/`pitch_cur` in semitones, `velocity`, `gate`, `trigger` (a note-on landed this segment: a fresh voice or a mono retrigger), `fresh` (a new voice), `release`, `note_index`. Unchanged by S2b (no new fields).
- **Amp stage:** the envelope's value is per segment; the gain ramps linearly from the previous segment's value to this one's across the 32 frames (`RAMP[i] = (i+1)/32`, exact).
- **Voice bus and pan (decided for F2):** voices are mono (one generator sum per voice, so the per-voice effects run once, not twice), placed into a stereo bus by `voice.pan` with a constant-power law: `θ = (pan + 1)·π/4`, gains `(cos θ, sin θ)` in `f64` via `libm`, −3 dB and exactly equal at the centre, exactly `(1, 0)` and `(0, 1)` at the ends. The pan gains ramp with the amp gain. Effects (`fx`) are stereo. Unison spread would need a stereo voice bus; it is not in P1.
- **Master:** `master.level` (dB → gain through `math::db_to_gain`) ramps per segment and is skipped at exactly 0 dB. The pan and master gain caches live in the plan (`plan::Output`) and are carried at a swap.

## Plan swap

`swap.rs` (rings, `Link`), `plan.rs` (`Plan::carry_from`), `engine.rs` (step 1, the crossfade). Design.md §3; report 01 §5.

- **One command ring** (`rtrb`, `Cmd::{Plan(Box<Plan>), Param { epoch, id, value }, Macro { index, value }}`), so a value event can never overtake the plan that defines its id. `Link::send_plan` stamps the next epoch on the plan and returns it; `Link::send_param(epoch, id, value)` carries the epoch of the plan whose `PlanMap` produced `id`; the engine applies it only if that is the plan it runs, else drops it (`stale_events`). Host automation (`EventKind::Param` through `process`) applies to the running plan: use the `PlanMap` of `Engine::epoch()`.
- **A return ring with a pending slot.** A replaced plan goes back on the return ring; if it is full, the engine keeps the plan in its pending slot and **stops reading commands** (the commands behind it stay queued, in order) until a later segment finds room. Nothing is dropped, so nothing is freed on the audio thread. `Link::collect` drops returned plans on the main thread.
- **At most two plans in flight** (`MAX_PLANS_IN_FLIGHT`): `send_plan` hands the plan back (`SendError::InFlight`) while two sent plans have not returned their predecessors; the session coalesces edits meanwhile. A full command ring gives `SendError::Full` with the value back.
- **State carry** (on the audio thread, never allocating): every module of the new plan whose (id, kind) matches one in the same role list of the old plan copies its state (`copy_state_from`), its modulator outputs, and its parameters' `cur`/`prev` per voice and smoothers; so do `voice.*`, `master.*`, the macros and the output ramps. Fresh voice modules start every active voice (`start_voice` with that voice's note index, so the seeds are the ones the note had); fresh global modules `reset`. The voice manager, `NoteState` and glides live in the engine, so held notes survive. Base values come from the new plan (compiled from the current patch); if one differs from the old, its smoother ramps to it. A scope change moves an LFO to another role list: removed and added. A surviving reverb copies its delay lines (about 1.6 MB at 48 kHz) on the audio thread at the swap: a plain copy, no allocation, but P1b should time it.
- **The crossfade, only when a module was added or removed:** both plans run for 256 frames (8 segments), and the output is `old + (new − old)·w` with `w = 0.5 − 0.5·cos(π·(i+1)/256)` (computed once through `libm`). That is an **equal-gain** raised-cosine fade, not design.md's equal-power one: the two plans share the held notes, so their outputs are highly correlated and an equal-power fade would bump the level by up to 3 dB mid-fade; and the form is exact wherever both plans give the same sample, so an unrelated edit (an added LFO with no route, a removed one, a silent oscillator) leaves the output bit-identical (tested). A plan that arrives during a crossfade waits in the ring. Without an added or removed module (a recompile, a structural parameter of a surviving module such as `osc.table` or `voice.mode`), the swap is instant and the old plan goes back at once.
- Swaps never happen in an offline render (`Engine::new` has no ring), so goldens cannot see them; CPU doubles during a crossfade (P1b measures it against the deadline).

## Monitor bypass (the plugin's FX bypass)

`Engine::set_monitor_bypass(on)` mutes every `fx` module whose kind is flagged `time_based` in the catalogue (the reverb in P1): the signal passes it unchanged. It is engine state, not patch state: **for the plugin only**. `enth-render` builds its engine internally and never calls it, and `RenderSpec` has no field for it, so renders, goldens and scores cannot see it by construction (design.md §5, ADR-0006). It takes effect at the next segment boundary; engaging it resets those modules (`reset`, allocation-free), so releasing it starts them from silence instead of resuming a stale tail. Non-time-based effects (the compressor) keep running. Plugins call it from the audio thread before `process` (for example from an atomic the UI sets).

## Seeds

```text
module_key = fnv1a64(module id bytes)                   seed::module_key
per voice:          derive(derive(render_seed, module_key), note_index)    seed::voice_seed
per global module:  derive(render_seed, module_key)                        seed::global_seed
seeded phase mode:  derive(derive(param_seed, module_key), note_index)     seed::seeded_voice_seed
```

The render seed comes from `RenderSpec::seed` (default `0x0E17_5EED`); the engine owns it and computes every seed, so plans are seed-free. `SplitMix64`, `derive` and `fnv1a64` are the Phase 0 spike's, pinned by tests. Nothing derives from parameter values or the patch hash; the seed-stability test guards it.

## Determinism and real time

- Transcendentals only through `enth_dsp::util::math` (wrappers named like `libm`: `sinf`/`sin`, `exp2f`/`exp2`, …), plus `tanh_approx`, `db_to_gain`, `midi_to_hz` and `constant_power_pan`.
- `process` never allocates, frees, locks, logs or does I/O, and does not panic on any input (mismatched channel lengths, offsets past the block, NaN, unknown ids, a full queue). Tested under `assert_no_alloc`.
- Renders are the same under FTZ off, FTZ only and FTZ+DAZ (the test sets MXCSR/FPCR to emulate hosts).

## `enth-render`

```rust
render(&Patch, &Performance, &RenderSpec) -> Result<RenderOutput, RenderError>
RenderSpec { sample_rate: u32, seed: u64 }                    // Default: 48 kHz, DEFAULT_SEED
Performance { notes: Vec<Note>, automation: Vec<Automation>, duration_s: f64 }
Note { key: u8, velocity: f32 /* 0–1 */, on_s: f64, off_s: f64 }
Automation { time_s: f64, addr: ParamAddr, value: f32 }       // base value jumps, clamped to the spec
RenderOutput { sample_rate, left: Vec<f32>, right: Vec<f32>, stats: RenderStats }   // interleaved(), mid(), peak()
render_with_block(.., block)                                  // the same bits for any block (tests)
```

Times become frames as `round(t · sample_rate)` in `f64`. Same-frame events: automation first, then notes in performance order, each note's on before its off. Automation sets the base value, which the engine smooths as the parameter's spec says (since S2b; `enth-render`'s own doc comment on `Automation` still says "jumps" and needs a one-line update by its owner). Automating a disabled module's parameter is not an error and changes nothing. Errors carry stable codes (`bad_spec`, `bad_performance`, `unknown_param`, and the compiler's codes, including `bad_route`).

## Goldens

A golden case (`tests/golden/cases/<name>.toml`) with `kind = "patch"` embeds the patch as a `[source.patch]` table with exactly the shape of a `*.patch.json` file, plus notes and optional automation in seconds and an optional render seed. Stereo renders hash both channels; the summary is computed on the mono mix. Until S1b's file layer lands, `tests/golden/src/patch.rs` converts the table with the catalogue's parsers; S1b's conversion replaces it without changing any case file.

| Case | Guards |
| --- | --- |
| `ref-saw-gate` | The F2 reference: the reference saw through a gate envelope. D1 updates it on purpose |
| `ref-saw-1v` | `Patch::init()` with the reference envelope, through the release. D1 and D2 update it |
| `engine-chord` | Spine composite: two saws, every kind, chord with off-grid times, a steal (S2b; was a dropped voice), smoothed automation. Updated whenever a stub is replaced, on purpose |
| `voice-poly-steal` | S2b: polyphony 2, steals with the 64-frame fade into spare slots, then a free slot |
| `voice-mono` | S2b: the 16-key stack, retriggers, the return to the newest held key, a fresh voice after the end |
| `voice-legato-glide` | S2b: legato with an 80 ms `always` glide, including the glide back to a held key and a detached note |
| `voice-poly-glide-legato` | S2b: poly with a 60 ms `legato` glide (overlapping notes glide, detached ones do not), two saws |
| `mod-routes` | S2b: routes from velocity, key and two macros; smoothed automation of a routed parameter and of `master.level` |
| `stand-in-sine` | The harness's own smoke test (no engine) |

The S2b cases use only the reference saw, the gate envelope and built-in sources (velocity, key, macros), so lane D2's modules do not move them; lane D1 updates them with the other saw goldens.

**Convention for module goldens: play them through a gate envelope** (`attack = 0, decay = 0, sustain = 100, release = 0`), so they do not move when D2 replaces the envelope. `tests/contract.rs` pins what that needs: the envelope's output is exactly 1.0 from the end of a note's first segment while held and exactly 0.0 one segment after the note-off. Composite cases (the 16-voice patch, a phrase with steals, the 40 s tail) belong to the spine lanes.

## Benchmarks

`crates/enth-engine/benches/common/mod.rs` builds a patch with one module of a kind (`patch_with`), prepares an engine with a held chord outside the measured function (`prepare`) and renders it in 512-frame calls (`render`). The `engine` bench (2026-10-19, x86-64, pinned toolchain): reference saw, 1 s at 48 kHz, 1 voice 3,660,346 instructions (76 per frame), 16 voices 40,367,014 (841 per frame); the budgets are those plus 10 %. After S2b (quiet parameter tables): 3,427,804 (71 per frame) and 37,679,499 (785 per frame), within the S2a budgets, which stay as they are. Every patch holds an `osc` and an `adsr`, so D1 and D2 re-measure these two budgets when they replace the stubs. A module bench subtracts the `engine` case with the same voice count to read its own cost.

The **`g1` bench** (`benches/g1.rs`, S2b) is the G1 reference patch: polyphony 16, two oscillators (saw, pulse an octave down) and noise into a resonant low-pass, an amp and a filter envelope, a vibrato LFO per voice and a slow global LFO, compressor and reverb, eight routes (envelope, LFOs, velocity, key, a macro), a held 16-note chord, 2 s in 512-frame calls. Its hard limit is G1's provisional cap, 12,000 instructions per frame (1,152,000,000), so module lanes can land without editing it unless they break G1; lane E re-bases it to the first full product measurement plus 10 %. Measured 2026-10-19 with the stub modules: **122,452,870 instructions (1,276 per frame)**.

## Tests

- `enth-engine` unit tests: parameter tables (exact settled values, ramps, clamping), plan compiler (role lists, disabled modules, the sync master, refused S2b features, invalid patches as errors), voice manager, event queue and FIFO, seeds (pinned), the registry test (every kind builds in the role its section and rate imply; every role variant is reachable), the reference saw.
- `tests/segment.rs`: event timing on the grid, block-size invariance (fixed sizes and a proptest over random block sequences), arrival order, voice end, dropped notes, the pan law, master level and value events, render-seed behaviour of the three phase modes, odd host calls.
- `tests/noalloc.rs`: `assert_no_alloc` around every `process` call, including 16 voices with dropped notes, a full event queue and junk events, with a negative control.
- `tests/tail.rs`: a 40 s release tail through one module of every kind: 0 subnormal output samples, 0 subnormal state, 0 voices left, every sample finite; the same bits under every FTZ/DAZ mode. The patch includes every kind, so each lane's module is covered when it lands.
- `tests/contract.rs`: the gate-envelope contract above.
- `tests/voices.rs` (S2b): the steal fade's shape and the stolen voice's exact silence after 64 frames, mono following the last note, mono retriggering vs legato, glide (linear, constant time, both glide modes, poly), voice-end reports, chokes, routes from velocity, key, macros and the mod wheel (with smoothing), a resting route changing nothing, block-size invariance with routes, smoothing, glide and steals in every voice mode (a proptest over random notes, macro, mod-wheel and automation events and random block sequences), 40 s tails in every mode with steals and routes: 0 voices, 0 subnormals.
- `tests/swap.rs` (S2b, under `assert_no_alloc`): a held chord survives recompiling the same patch and unrelated structural edits (an added LFO, a removed LFO, a silent added oscillator) **bit for bit**; an added oscillator fades in; for every `ModuleKind` (both LFO scopes) copying state into a fresh module gives identical output for 360 segments; stale-epoch events are ignored and current ones apply; a full command ring (plans, values, macros) drains in one segment; a swap while the return ring is full keeps the plan (held, then handed back) and a third plan is refused while two are in flight; a plan arriving during a crossfade waits; the monitor bypass removes the reverb exactly and leaves the compressor.
- `tests/noalloc.rs` also holds the "every voice stolen" overflow test: a 200-note burst that fills all 32 slots (steals and cuts), a stream that steals every voice again and again, and mono/legato with more keys than the stack.
- `enth-render/tests/render.rs`: the reference saw's pitch, level and release; render-level block invariance with automation; the render seed as the only randomness; **seed stability** (every non-zero float parameter of every kind, `voice` and `master`, moved by 1e-3: the 10 ms RMS envelope moves under 0.05 dB within 30 dB of the peak, measured 0.025 dB; the negative control, a reseed, moves it 6.3 dB); finite output for random parameter values; typed errors.

## Seams for the lanes after F2

| Lane | Files | What fills in |
| --- | --- | --- |
| **S2b** (done) | `voice.rs`, `modulation.rs`, `params.rs`, `swap.rs`, `engine.rs`, `plan.rs`, `benches/g1.rs` | Built as described above. Left for P1b: CLAP polyphonic modulation (per-voice macro offsets), wiring `Link` to `enth-session`, measuring compile time and crossfade CPU in the test host |
| **D1** | `modules/osc.rs`, `modules/noise.rs`, `benches/osc.rs`, `benches/noise.rs` | Replace the stubs; hard sync reads `VoiceCx::sync_in` (the master writes `sync_out`, the reset time per frame); wavetables generated in `new`; ramp pitch from `pitch_prev` to `pitch_cur` (glide); `copy_state_from` copies all state; re-measure the `engine` bench budgets; update `ref-saw-*`, `engine-chord` and the S2b cases (`voice-*`, `mod-routes`) |
| **D2** | `modules/svf.rs`, `modules/adsr.rs`, `modules/lfo.rs`, their benches | Replace the stubs; the SVF interpolates coefficients from `prev` to `cur` (modulated values change every segment); keep the gate-envelope contract and the voice-end rule; an envelope restarts its attack from its current level on `trigger` without `fresh` (a mono retrigger); the LFO's output range is its polarity (routes carry only a depth); re-measure the `engine` bench budgets; update `ref-saw-1v` and `engine-chord` |
| **FX1** phase 2 | `modules/reverb.rs`, `benches/reverb.rs` | Wrap `enth_dsp::reverb::Reverb` (`set_params` once per segment with the catalogue's conversions, `process_in_place`, `is_silent` exact); if `room` or `decay` legitimately fail the seed-stability test, list them in its `EXEMPT` with the reason |
| **FX2** phase 2 | `modules/comp.rs`, `benches/comp.rs` | Wrap `enth_dsp::compressor::Compressor` the same way |
| **C** phase 2 | — | `enth render` maps its perf JSON (note names, velocity 0–127) onto `Performance`; `--seed` sets `RenderSpec::seed` |
| **A**, **H** | — | Analyse `RenderOutput::mid()` (or each channel); nuisance renders change `RenderSpec::seed` only |

A lane replaces only its own files; nothing else in the engine changes when a module lands.

## Open points

- The crossfade is equal-gain (raised cosine), not design.md §3's equal-power ramp; the reasons are under "Plan swap". design.md should follow if the owner agrees.
- `crates/enth-dsp/src/smoothing.rs` (an S0 placeholder for S2b) stays empty: the smoother needs `ParamSpec`'s scale mapping (`enth-params`, the same layer as `enth-dsp`), so it lives in `enth-engine/src/params.rs`. Delete the placeholder or keep it for module-internal smoothers.
- `enth-render`'s doc comments on `Automation` ("jumps") and `RenderStats::dropped_notes` ("no stealing before lane S2b") are stale since S2b; its owner updates them (S2b does not edit that crate).
- S1b's `validate` is not merged: the compiler checks routes itself (`bad_route`); once `validate` lands, the two must agree.

- Windows: the golden hashes were produced on x86-64 Linux and match wasm32-wasip1 under wasmtime 48.0.6; the Windows leg runs in CI on the first push.
- The golden harness's PatchFile resolver (`tests/golden/src/patch.rs`) is temporary until S1b's conversion merges.
- Disabling the amplitude envelope: the engine runs it anyway; whether it is valid is S1b's `validate`.
