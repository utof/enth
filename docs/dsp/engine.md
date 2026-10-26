# Engine (`enth-engine`, `enth-render`)

- Lanes: P1a S2a, the walking skeleton (freeze point F2), then S2b: the voice manager, smoothing, the modulation matrix and the plan swap; then the module lanes D1 (oscillators, noise), D2 (filter, envelope, LFO) and FXW (reverb and compressor wrappers) replaced every stub. P1b lane E+ added the seams the plugin, the session and the editor need (see "P1b seams (lane E+)"). After F2, changes to the types below are additive only.
- Code: `crates/enth-engine/src/` (module contract, plan compiler, segment loop, voice manager, smoothers and modulation, plan swap, the modules), `crates/enth-render/src/lib.rs` (offline render), `crates/enth-dsp/src/util/{math,rng}.rs` (the `libm` door and seeded randomness), `tests/golden/` (cases and the patch source).
- Decisions behind it: [design.md §2, §3 and §5](../design.md#3-engine-and-real-time-safety), [architecture.md](../architecture.md), research report 01 §5–§7 (`_local/sessions/2026-10-19-p1a-headless-core/`); for the P1b seams, the P1b research session (`_local/sessions/2026-10-19-p1b-plugin-editor/`: summary, 01 R6, 02 D7, 03 F3).

The engine turns a `Patch` into stereo `f32` the same way on every platform and for every host block size. S2a ran one reference saw through the whole path (module, plan, segment loop, render, golden) so the spine could be frozen before the modules existed; every module is now its lane's real implementation, and the goldens hash the same on x86-64 Linux and wasm32 (Windows runs in CI).

## Data flow

```text
Patch ──plan::compile(patch, &PrepareCfg)──▶ (Plan, PlanMap) ──Engine::new(plan, render_seed)──▶ Engine   (offline)
                                                            └─Engine::with_link(plan, seed, cap)──▶ (Engine, Link)  (real time)
Engine::process(left, right, &[Event])      any number of frames, events timed by offset in the block
Link::send_plan(plan) / send_param(epoch, id, v) / send_depth(epoch, d, pct) / send_macro / send_event(kind) / collect()
                                            the main thread's end of the rings
Engine::attach_tap(Arc<Tap>)                main thread; the editor reads the Tap (telemetry), the engine only publishes
enth_render::render(patch, perf, spec)      the offline wrapper: compile, schedule events, call process in blocks
```

- `compile` allocates: it prepares every module (`ModuleImpl::new`) and sizes every table. Nothing after it allocates.
- `PlanMap` maps parameter addresses (`filter1.cutoff`, `voice.pan`, `master.level`) to the plan's `ParamId`s. It stays off the audio thread, where an op or an automation point becomes a value event.
- The engine API added by S2b: `Engine::with_link` and `Link` (plan swap, epoch-tagged value events, macros, collecting old plans), `Engine::set_monitor_bypass`/`monitor_bypass` (the plugin's FX bypass), `Engine::note_ends` (CLAP `NOTE_END`), `Engine::epoch`, `is_crossfading`, `plans_held`, `voices`, `notes`; `EventKind::{Macro, ModWheel, Choke}`; `EngineStats::{stolen_voices, cut_voices, stale_events, swaps, crossfades, dropped_note_ends}`; `CompileError::BadRoute`; `VoiceState::Stolen`; `NoteEnd`, `LinkCapacity`, `SendError`, `MAX_PLANS_IN_FLIGHT`, `CROSSFADE_FRAMES`.
- The engine API added by P1b lane E+ (all additive): `EventKind::{Slot, EpochParam, MacroVoice}`, `NoteKey::note_id`; `Link::{send_depth, send_event}`; `PlanMap::{depth, addr}`, `DepthId`; `Plan::{tap_params, tap_modulators, slot_target, route_depth}`; `Engine::{reset, position, attach_tap, tap, base, value, macro_offset, host_slot, is_bypass_fading}`; `NoteEnd::offset`; the `tap` module (`Tap`, `TapCapacity`, `TapFrame`); `GlobalProcessor::take_state_from` (default: copy), `ModuleImpl::take_state_from`; `MONITOR_BYPASS_FADE_MS`, `DEPTH_SMOOTHING_MS`, `POLY_MACROS`; `segment::EventQueue::clear`. In `enth-render`: `Performance::controls`, `Control`, `Controller`.

## Modules: four roles, enum dispatch

| Role | Trait | Kinds | Called |
| --- | --- | --- | --- |
| Voice processor | `VoiceProcessor` | `osc`, `noise` (generators), `svf` (per-voice effect) | `start_voice` at a note-on, `process` once per segment over the active voices |
| Voice modulator | `VoiceModulator` | `adsr`, `lfo` with `scope = voice` | `start_voice`, `tick` once per segment (writes one value per voice), `voice_finished` |
| Global modulator | `GlobalModulator` | `lfo` with `scope = global` | `reset` at start, `tick` once per segment (returns one value) |
| Global processor | `GlobalProcessor` | `reverb`, `comp` | `reset` at start, `process` once per segment on the stereo bus, `is_silent` |

Every role also has `copy_state_from` (for the plan swap; never allocates; a lane's module must copy *all* its state, which `tests/swap.rs` checks for every kind) and `subnormals` (counts subnormal state, for the tail test). Global processors also have `take_state_from(&mut other)` (P1b lane E+): an instant swap never runs the old plan again, so the new module may *take* the state; the default copies, the reverb overrides it with an O(1) exchange (see "Plan swap"). `tests/swap.rs` checks for every kind that taking gives the bits copying gives.

`modules::ModuleImpl` holds one enum per role (`VoiceProcessorImpl`, `VoiceModulatorImpl`, `GlobalModulatorImpl`, `GlobalProcessorImpl`). `ModuleImpl::new(kind, values, cfg)` is an exhaustive `match ModuleKind`, so a catalogue kind without an implementation does not compile (checked: removing the `comp` arm gives `error[E0004]`). The compiler sorts modules into per-role lists, so the segment loop never meets a module in the wrong role and has no `unreachable!`. Dispatch happens once per module per segment.

### The contract

- **`Type::new(values, cfg)` is `prepare`.** It may allocate, sized from `PrepareCfg` only (the sample rate). A structural parameter (`osc.table`) may choose *what* to compute, never how much memory.
- **One call per module per segment** with the active voices in ascending order. Per-voice state is `[T; MAX_VOICES]` (structure of arrays), so a later SIMD path can run 4–8 voices per lane group without changing the API.
- **Parameters** come from views: `params.base(p)` (the patch value or the latest value event), `params.cur(p, v)` and `params.prev(p, v)` (this and the previous segment's value, for per-sample ramps; equal on a voice's first segment). Index them with the catalogue enums (`params.cur(OscParam::Semi, v)`). Discrete parameters: `Wave::from_value(params.base(OscParam::Wave))`, which never panics.
- **Generators overwrite** `io[v]` for every active voice (the engine sums them in patch order); per-voice effects transform `io[v]` in place; global processors transform `left`/`right` in place.
- **Randomness** only from `VoiceStart::seed` / `GlobalStart::seed` (or `seed::seeded_voice_seed` with the module's own `seed` parameter in the `seeded` phase mode), through `enth_dsp::util::rng`.
- **The four DSP rules**: transcendentals through `enth_dsp::util::math` (pinned `libm`); no `mul_add`; fixed summation order; flush recursive state with `enth_dsp::util::flush::flush`, at least once per segment. `voice_finished` is true past the attack and below −100 dB (`math::VOICE_END_LEVEL`).
- `start_voice` resets the slot (it may hold a previous note) and reads `params.base`; `notes` already holds the new note.

### The modules (S2a's stubs, all replaced)

S2a left a marked stub in each file so the spine could be built and frozen first; each lane replaced only its own files.

| Kind | File | Implementation | Notes |
| --- | --- | --- | --- |
| `osc` | `modules/osc.rs` | Saw, triangle and pulse from the Plaits-derived polyBLEP/BLAMP core, a sine, wavetables, hard sync and the three phase modes (D1). S0's `analog.rs` and `wavetable.rs` are merged here: one `osc` kind | [osc.md](osc.md) |
| `noise` | `modules/noise.rs` | White (xorshift32) and pink noise, one stream per voice (D1) | [noise.md](noise.md) |
| `svf` | `modules/svf.rs` | Cytomic/Simper TPT state-variable filter, coefficients ramped from `prev` to `cur` (D2) | [svf.md](svf.md) |
| `adsr` | `modules/adsr.rs` | RC (exponential) envelope with a 1 ms minimum segment, evaluated per segment (D2). It is every patch's amplitude envelope | [envelope.md](envelope.md) |
| `lfo` | `modules/lfo.rs` | Seven shapes, per voice or global, with its own polarity (D2) | [lfo.md](lfo.md) |
| `reverb` | `modules/reverb.rs` | Wrapper around `enth_dsp::reverb::Reverb`, Signalsmith Basics' FDN port (FXW; core by FX1) | [reverb.md](reverb.md) |
| `comp` | `modules/comp.rs` | Wrapper around `enth_dsp::compressor::Compressor` (FXW; core by FX2) | [compressor.md](compressor.md) |

## Parameters and smoothing

Each plan has two flat tables: the voice table (modules that run per voice, and `voice.*`) and the global table (global modules, `master.*`, then the eight macros and the mod wheel). A slot holds `base`, a smoother, and `cur` and `prev`, per voice in the voice table. `ParamId::Voice(i)` or `ParamId::Global(i)` names a slot.

- **Settled values are exact.** Without smoothing in flight and without a route, `cur` is `base`, the plain value from the patch, bit for bit. It never goes through `from_norm(to_norm(x))`, which is not guaranteed to round trip.
- A value event (`EventKind::Param`, `EventKind::EpochParam` or `EventKind::Slot`, or `Link::send_param` with the plan's epoch) sets `base`, clamped to the spec; NaN and unknown ids are ignored. Real-time hosts use slots or epochs, never a bare `Param` (see "Host automation: slots and epochs").
- **Smoothers** (`params::Smoother`, step 3) follow `base` in the normalised domain (`ParamSpec::to_norm`/`from_norm`, pinned `libm`), once per segment, as the catalogue's `Smoothing` says: `Linear { ms }` is a ramp of `max(1, round(ms·sr/1000/32))` segments at constant time (a new target mid-ramp restarts the full ramp from where the value is; 10 ms = 15 segments at 48 kHz); `OnePole { ms }` moves `n += (target − n)·(1 − e^(−32/frames))` until within 1e-6, then snaps; `None` steps at the boundary. A smoother's last step lands on the exact `base`. Smoothers start settled at load (no initial ramp). In P1a every smoothed parameter is `Linear` (10 ms, `master.level` and the reverb's filters 20 ms).
- **Smoothers are per parameter, shared by all voices**, so every voice reads the same smoothed value and parameter changes reach held voices. **A trigger restarts a voice's ramps** ("smoother reset on retrigger"): on a fresh voice's first segment and on a mono retrigger, `prev = cur` for that voice, so no module ramps from a stale value.
- **Quiet tables cost nothing.** When no base was written, no smoother is moving and `prev` is level with `cur`, step 3 writes only the restarting voices and the routed parameters' `prev`, which gives the same values as the full pass (the S2a loop that copied every parameter for every voice is gone; the `engine` bench went down 6–7 %).
- Macros and the mod wheel are global-table slots with the `MACRO_VALUE` spec (0–1, smoothed over 10 ms); the macros start at the patch's values, the mod wheel at 0. They are set by `EventKind::Macro { index, value }` / `EventKind::ModWheel { value }` or `Link::send_macro`/`send_mod_wheel`, and are never in the `PlanMap`. In an offline render they come from `Performance::controls`.

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
- **Depth value events** (P1b lane E+; `Link::send_depth(epoch, DepthId, pct)`, the id from `PlanMap::depth(&RouteAddr)`): the route's term moves to `pct / 100` (clamped to ±100 %, NaN ignored, computed exactly as the compiler computes a route's depth) linearly over `DEPTH_SMOOTHING_MS` = 10 ms, `max(1, round(10·sr/1000/32))` segments (15 at 48 kHz), advanced in step 3; a new depth mid-ramp restarts the full ramp from where the depth is; the last step lands exactly, so once settled the plan holds the bits a recompiled plan holds (tested against a recompiled render). An event with another plan's epoch is dropped (`stale_events`). **A depth from or to 0 is a recompile**: a depth-0 route has no term (`PlanMap::depth` is `None`), which keeps an unrouted parameter's exact plain value. A ramp in flight at a plan swap is not carried: the new plan starts at its patch depth.
- **Per-voice macro offsets** (P1b lane E+, CLAP polyphonic modulation; `EventKind::MacroVoice { key, index, offset }`, addressed by `NoteKey::note_id(id)`, wildcards allowed): every playing voice matching `key` gets `offset` (normalised, clamped to ±1, NaN ignored) on macro `index` (0–7; the mod wheel takes none). For **voice-rate destinations only** a macro source reads `clamp(macro + offset[v], 0, 1)` per voice; global destinations keep the macro itself. While any offset of an active voice is non-zero, a destination with a macro route is evaluated per voice (otherwise computed once, as before: a voice whose offset is 0 gets the same bits either way). Offsets start at 0 whenever a note lands on a voice (a fresh voice, a mono retrigger, a legato change) and are cleared when it ends; they live in the engine, so they survive plan swaps. Renders with poly-mod are outside the CLI contract (a `Performance` cannot express them).

## The segment grid and the host adapter

`SEG = 32` frames is the processing quantum. `Engine::process` copies frames out of a one-segment FIFO and renders a new segment whenever it runs dry. Before rendering the segment that starts at frame `32·k`, it applies every queued event whose frame is at or before `32·k`, in arrival order. So an event at frame `f` takes effect at `ceil(f / 32) · 32` (0.67 ms at 48 kHz), and the grid is counted from the engine's start, never from the start of a host call: any sequence of block sizes gives the same bits (tested for 1 to 36,000 frames and random block sequences). Blocks are never split at event times.

Events wait in a fixed-capacity queue (`segment::EVENT_CAPACITY` = 4,096). When it is full, a new host event is dropped and counted (`EngineStats::dropped_events`); nothing reallocates. An event from the command ring (`Link::send_event`) is never dropped: it waits in the ring, with the commands behind it, until a segment has applied the due events (F1b review; host events are due within their block, so the queue drains).

## The order of one segment

1. The command ring (real-time engines only): plan swaps (state carried; a crossfade if modules were added or removed; remembered host slot values re-applied), epoch-tagged value and depth events (stale ones dropped and counted), macros, and events sent with `Link::send_event` (queued into the event queue at this boundary, behind the host events already due here; left in the ring while the queue is full). See "Plan swap".
2. Due events, in arrival order: note-ons (the voice manager picks a slot or retriggers; a fresh voice gets `start_voice` on every voice module with its seed and its modulator outputs restart from 0; the voice's poly-mod offsets restart at 0), note-offs (gate down; in mono and legato, a return to the newest held key), value events (`Param`, `EpochParam` if its plan runs, `Slot` through the slot table), macros, per-voice macro offsets, the mod wheel, chokes. Then every gliding voice moves one segment. The monitor bypass fade moves one segment towards its target.
3. Smoothers (see "Parameters and smoothing"); a trigger restarts the voice's ramps. Route depth ramps (depth value events) advance.
4. Global modulators, in patch order, each after its own modulated parameters.
5. Voice modulators, in patch order, each after its own modulated parameters, over the active voices.
6. The remaining modulated parameters (see "Modulation").
7. Voices: each voice bus is zeroed, generators write a scratch bus and are summed in patch order (the first enabled `osc` first writes the hard-sync timing that later generators read), then the `voice_fx` chain, then the amp stage (times the steal fade of a stolen voice) and the pan into the stereo sum in ascending voice order.
8. `fx` in order (a processor that `is_silent` with an all-zero input is skipped, which must be bit-identical to running it; the monitor bypass fades `time_based` ones out over 20 ms and then skips them), then `master.level`.
9. Voices whose amplitude envelope reports `voice_finished` (in every plan that runs), or whose steal fade is over, are freed and reported (`Engine::note_ends`). Modules flush their own state; nothing in the engine recurses.
10. The FIFO is filled (a debug assertion checks every sample is finite).

During a crossfade, steps 3–8 run for both plans (the replaced one first) over the same notes and voices. After the last segment a `process` call needs, an attached and enabled telemetry tap gets the block's values (it only reads; see "Telemetry tap").

## Voices, the amp stage and the pan law

`voice.rs`. Deterministic: no randomness, no clock, no dependence on the host's block size; the same performance gives the same slots, so the same summation order (unit-tested). Notes are addressed by CLAP's (port, channel, key, note id); queries (note-off, choke) take −1 wildcards. Every note-on takes the next `note_index`, whatever happens to it, so stealing and mode changes never move a seed.

- **Poly** (`voice.mode = poly`): a note-on takes the lowest free of 32 slots while fewer than `voice.polyphony` (≤ 16) voices are held or releasing. When polyphony is full it **steals the oldest released voice, else the oldest held one** ("oldest" = the earliest note-on; ties cannot happen). The victim (`VoiceState::Stolen`) fades to silence linearly over 64 frames (two segments, `1 → ½ → 0`, per sample, times the amp ramp) and is freed; the new note starts in a spare slot. Only a burst of more than 16 steals within two segments can find all 32 slots busy: the new note then takes the fading voice with the least fade left (a cut without a fade, counted in `cut_voices`). Nothing is ever dropped (`dropped_notes` stays 0). A same-key note-on starts a new voice; a lower polyphony from a structural edit is enforced by the next note-on's steals.
- **Mono and legato** play one voice with a **16-key last-note stack** (newest on top; a 17th key pushes out the oldest). A note-on while a key is held: mono **retriggers** (envelopes restart their attack from their current level, the new velocity applies), legato changes the pitch only. A note-on with no key held retriggers the voice while it still sounds (both modes) or starts a fresh voice once it has ended. Releasing the sounding key returns to the newest key still held (a pitch change, gliding if glide is on, no retrigger); releasing the last key releases the voice. The voice keeps its first note's `note_index` (and its seeds) until it ends. `voice.polyphony` is ignored. A mode change (structural) adopts the newest held voice and rebuilds the stack from the held voices; left-over poly voices still release on their note-offs.
- **Glide** (`voice.glide` ms, 0 = off; `voice.glide_mode`): linear in semitones at constant time, `max(1, round(glide·sr/1000/32))` segments (100 ms = 150 at 48 kHz) from the current pitch to the new one, whatever the interval; a new note mid-glide starts a new glide from where the pitch is; the last step lands exactly. `always` glides every new note: a retrigger or legato change from the voice's pitch, a fresh voice (poly or mono) from the previous note-on's pitch; `legato` glides only when another key is held at the note-on. Glide writes `NoteState::pitch_prev` (the pitch at the segment start) and `pitch_cur` (at its end); the oscillator ramps between them. Glide works in poly mode too (report 01 had it off in poly; the catalogue's description and the voice-mode goldens include it).
- **Choke** (`EventKind::Choke`, CLAP `NOTE_CHOKE`): matching playing voices fade out over 64 frames like a steal (not counted as steals).
- **Voice end:** a voice is freed at the end of a segment in which the amplitude envelope (`voice.amp_env`) is past its attack and below −100 dB, or its steal fade is over. Its slot costs nothing afterwards. **Every end and every cut is reported** as `NoteEnd { key, frame, offset }`: `frame` is the absolute frame where it went silent (on the grid), `offset` that frame as an offset into the `process` call that reported it, **clamped into the block** (`0..len`; P1b lane E+). The FIFO renders a whole segment ahead, so `frame` can lie up to 32 frames past the block; such an end is reported at the block's last frame (the voice is below −100 dB by then). `offset` is the timing for CLAP `NOTE_END`. `Engine::note_ends()` lists those of the last `process` call (up to 256; more are counted in `dropped_note_ends`).
- **`NoteState`** (per slot): `pitch_prev`/`pitch_cur` in semitones, `velocity`, `gate`, `trigger` (a note-on landed this segment: a fresh voice or a mono retrigger), `fresh` (a new voice), `release`, `note_index`. Unchanged by S2b (no new fields).
- **Amp stage:** the envelope's value is per segment; the gain ramps linearly from the previous segment's value to this one's across the 32 frames (`RAMP[i] = (i+1)/32`, exact).
- **Voice bus and pan (decided for F2):** voices are mono (one generator sum per voice, so the per-voice effects run once, not twice), placed into a stereo bus by `voice.pan` with a constant-power law: `θ = (pan + 1)·π/4`, gains `(cos θ, sin θ)` in `f64` via `libm`, −3 dB and exactly equal at the centre, exactly `(1, 0)` and `(0, 1)` at the ends. The pan gains ramp with the amp gain. Effects (`fx`) are stereo. Unison spread would need a stereo voice bus; it is not in P1.
- **Master:** `master.level` (dB → gain through `math::db_to_gain`) ramps per segment and is skipped at exactly 0 dB. The pan and master gain caches live in the plan (`plan::Output`) and are carried at a swap.

## Stereo voice path

Lane SVP (S2a, freeze F1m); `stereo.rs`, with call sites in `plan.rs` and `engine.rs`. Until S2a a voice was mono from the generator sum to the pan stage (the bullet "Voice bus and pan" above). Unison spread (lane UNI) and a stereo voice effect need two channels in between, so a plan can now run its voices in stereo. Research report 06 §3.2; phase-s2.md, the SVP row.

**The flag.** A plan carries one `stereo` flag (`Plan::is_stereo`), set in `compile_with(patch, cfg, mode)`; `compile` is its `StereoMode::Auto` case. `Auto` takes `stereo::patch_needs_stereo(patch)`, which is **`false` for every patch today**: no generator or voice effect makes a per-voice stereo signal. Lane UNI replaces its body (a `wt` with unison spread above 0). `StereoMode::Forced` sets the flag for any patch, which is how tests and the bench run the stereo path with mono content. The flag is a property of a compiled plan: changing it is a recompile and a swap, never an edit.

**While the flag is clear nothing changes.** There are no right-channel instances, no bus copy and no second balance loop. The engine tests the flag once per segment and once per active voice. On the G1 bench the mono path moved by **+246,505 instructions of 602,615,188 (+0.04 %, 2.6 per frame)**, measured on the same machine before and after (2026-10-26, gungraun 0.20.0); all 73 goldens are unchanged.

**While it is set,** step 7 changes in three places and nowhere else:

1. After the generator sum, `StereoPath::split` copies each active voice's left bus to the right bus: a mono source is the same signal on both channels. (Lane UNI's stereo generator will write its right channel here instead; `split` is the one function that changes.)
2. The `voice_fx` chain runs on the left bus as before, then **the same chain runs again on the right bus**: every `voice_fx` module has a second instance, built in `compile` from the same patch values (`StereoPath::mirror`), in the same order. A right instance shares its left twin's **parameter span**, so there is one set of knobs, smoothers and modulation routes and no new `ParamId`, `PlanMap` entry or answer-code field; its **state is its own**. Two modules of one kind keep separate state on each channel, as S2 rule 3 asks.
3. The amp stage and the pan become `stereo::mix_voice`, the balance below.

**Seeds** come from the module id and the channel. The left instance keeps the mono key, `fnv1a64(id)`, so everything it does is the mono path's. The right instance gets `channel_key(key, Right) = derive(key, RIGHT_STREAM)` in both places a module sees a key: `VoiceStart::seed` (`voice_seed(render_seed, key, note_index)`) and `VoiceStart::module_key` (the `seeded` phase mode's rule). Seeds still never depend on parameter values or on the patch hash. No voice effect reads its seed today, so the channels are bit-equal at zero spread. A voice effect that does use random numbers in its audio path would decorrelate its right channel, which is what a stereo effect wants; one that must equal mono at spread 0 must not.

**The balance law.** `voice.pan` (−100…+100 %) on a stereo voice is a **constant-power balance**: each channel is scaled by the mono pan law's gain for its own side and is never mixed into the other. With `(gl, gr) = (cos θ, sin θ)`, `θ = (pan + 1)·π/4` (`constant_power_pan`, in `f64` via `libm`) and `amp` the amp envelope's per-sample ramp:

```text
out_l += x_l · amp · gl            out_r += x_r · amp · gr
```

- **A mono source gives the mono path's bits on both channels.** This fixes the law. Stereo with zero spread must equal mono, so the gains must be the mono law's, and `mix_voice` forms each channel's product exactly as the mono stage does (the same cached and ramped `(gl, gr)`, the same `a0·q`/`a1·p − a0·q` ramp, the same steal-fade factor, in the same order). A unity-centre balance (the console kind) cannot meet it: it would make a stereo voice 3 dB louder than the same voice with the flag clear.
- **The centre is −3 dB per channel**, hard left passes the left channel at unity and silences the right, hard right the opposite: a balance drops the far channel, it does not fold it in.
- **Power:** for two uncorrelated channels of equal power the sum is constant over the whole range (`cos²θ + sin²θ = 1`), as a panned mono voice's is.
- The pan gains, their ramps and the steal fades are the mono stage's, so the swap carries them (`Plan::out`) and a glide or a `key → voice.pan` route behaves the same.

**Swap carry** (`Plan::carry_from` → `StereoPath::carry_from`, on the audio thread, no allocation). The right instances of the new plan exist already: `compile` built them on the main thread.

| Old plan | New plan | Right instances | Crossfade |
| --- | --- | --- | --- |
| stereo | stereo | pair with the old right instance of the same (id, kind) and copy its state; one with no partner starts every active voice (right-channel seeds) | only if a module was added or removed, as for any edit |
| mono | stereo | copy the **old left** instance's state: the signal was mono, so the right channel continues exactly as the left was | **yes** |
| stereo | mono | none; the old plan's go back with it on the return ring and are freed on the main thread | **yes** |
| mono | mono | none | as before |

A flag change counts as a structural change, so the swap crossfades (256 frames, the equal-gain raised cosine; the old plan keeps running its own path). With mono content the two plans give the same bits and the fade is the identity, so turning the flag on or off cannot click; with real stereo content (lane UNI) the fade carries the change over 256 frames instead of one sample. A toggle during a crossfade waits in the ring like any plan. The cost of a flag change is the crossfade's: both plans run for 8 segments, and the right instances' state is copied with the rest.

**Real-time.** The right-channel instances, the 4 KB right voice bus (`Buses::voice_r`, in the boxed `Buses`) and the right chain's parameter views are all allocated in `compile` or `Engine::new`. `process` and the swap allocate nothing with the flag on (`assert_no_alloc` over rendering, a burst of steals through a four-effect chain, and 30 toggles with a one-slot return ring).

**CPU** (instruction counts, gungraun, G1 = `benches/g1.rs`, 16 voices, 2 s, 2026-10-26):

| Run | Instructions | Per frame | Against |
| --- | --- | --- | --- |
| G1, main `2809273` | 602,615,188 | 6,277 | — |
| G1, this branch (flag clear) | 602,861,693 | 6,280 | +0.04 %; budget 650,000,000 |
| G1, flag forced on (`benches/stereo.rs`) | 647,369,152 | 6,743 | +7.4 % over the mono path |

Forced stereo on G1 costs the second `svf` per voice, the bus copy and the two-channel balance: **+464 instructions per frame**; the reverb and the compressor run once. A patch whose voice chain is heavier (S2a's distortion, comb, formant and ladder) pays that chain twice, which is the price of a stereo voice effect; the G2 reference patch (lane G2B) measures it with unison. Lane UNI's cost is its own.

**What lane UNI and the later module lanes use.** UNI turns the flag on in `patch_needs_stereo` (a function of the patch's values only, never of a running plan) and writes its right channel where `split` is called. A voice effect needs nothing: it is instantiated twice by the engine, and its module API did not change. A module that wants to know its channel does not exist yet; if one does, it gets a field in `VoiceStart` rather than a second trait.

**Tests.** `stereo.rs` (unit): the key rule (pinned) and the right instance's start; the right chain equals a second mono instance fed the right input; the balance arithmetic against the mono stage, hard left and right, no leakage, constant power; the carry for stereo to stereo (each channel keeps its own state, with different states in the channels so the test is not vacuous), mono to stereo (right from the old left) and an added module. `tests/stereo.rs`: forced stereo equals mono on both channels for G1, every voice mode with steals and glide, every voice-effect kind (two filters, `dist`, `comb`, `formant`, `ladder`), pans from −100 to +100 with a `key → voice.pan` route and a cutoff LFO, at host blocks 32, 100 and 512; a recompile, an edit, an added and a removed effect, and both toggles; thirty toggles with a return ring of one; steals through a four-effect chain; a 40 s tail (no subnormals in output or in the right instances' state, no voices left) — all under `assert_no_alloc`. `tests/golden/tests/stereo_path.rs`: the flag is clear for each of the 72 patch cases in `cases/`, and each renders the same bits on both channels with the flag forced on (`enth_render::render_with_stereo`), which ties the stereo path to the existing hashes on every target without new files. `RenderStats::stereo` reports the flag.

## Plan swap

`swap.rs` (rings, `Link`), `plan.rs` (`Plan::carry_from`), `engine.rs` (step 1, the crossfade). Design.md §3; report 01 §5.

- **One command ring** (`rtrb`, `Cmd::{Plan(Box<Plan>), Param { epoch, id, value }, Macro { index, value }, Depth { epoch, id, depth }, Event(EventKind)}`), so a value event can never overtake the plan that defines its id. `Link::send_plan` stamps the next epoch on the plan and returns it; `Link::send_param(epoch, id, value)` and `Link::send_depth(epoch, id, pct)` carry the epoch of the plan whose `PlanMap` produced the id; the engine applies them only if that is the plan it runs, else drops them (`stale_events`). A `Cmd::Param` also updates the patch value of any slot that targets the parameter (see "Host automation"). `Link::send_event(kind)` (audition notes) queues the event at the next boundary. Host automation through `process` must not use a bare `EventKind::Param`: the swap happens at a segment boundary *inside* `process`, so an id looked up before the call can name another parameter of the new plan (report 01, measured as a bug in the code). It uses `EventKind::Slot` or `EventKind::EpochParam`.
- **A return ring with a pending slot.** A replaced plan goes back on the return ring; if it is full, the engine keeps the plan in its pending slot and **stops reading commands** (the commands behind it stay queued, in order) until a later segment finds room. Nothing is dropped, so nothing is freed on the audio thread. `Link::collect` drops returned plans on the main thread.
- **At most two plans in flight** (`MAX_PLANS_IN_FLIGHT`): `send_plan` hands the plan back (`SendError::InFlight`) while two sent plans have not returned their predecessors; the session coalesces edits meanwhile. A full command ring gives `SendError::Full` with the value back.
- **State carry** (on the audio thread, never allocating): every module of the new plan whose (id, kind) matches one in the same role list of the old plan copies its state (`copy_state_from`), its modulator outputs, and its parameters' `cur`/`prev` per voice and smoothers; so do `voice.*`, `master.*`, the macros and the output ramps. Fresh voice modules start every active voice (`start_voice` with that voice's note index, so the seeds are the ones the note had); fresh global modules `reset`. The voice manager, `NoteState`, glides and poly-mod offsets live in the engine, so held notes survive. Base values come from the new plan (compiled from the current patch); if one differs from the old, its smoother ramps to it. Then every remembered host slot value is re-applied to the new plan where the slot still names the same parameter (see "Host automation"). A scope change moves an LFO to another role list: removed and added.
- **Effect state moves at an instant swap** (P1b lane E+, report 03). `carry_from` first pairs all five role lists (cheap) to learn whether the swap crossfades. If not, the old plan never runs again, so each surviving effect *takes* its state (`GlobalProcessor::take_state_from`, default: copy). The reverb exchanges its whole `enth_dsp` core with the old module's: an O(1) move of its 1.6 MB of delay lines instead of a copy. Every field the copy leaves alone (ring sizes and masks, the detune depth, the mixer and interpolator tables) is a constant of the sample rate, so the result is the copy's, bit for bit (the recompile tests run through it). The old plan leaves with the new plan's fresh lines and is dropped by `Link::collect` on the main thread, as before: nothing is freed on the audio thread. With a crossfade both plans run, so the state is copied as before (03's alternative, crossfading the voice sums before one shared effect chain, was not adopted: it changes the crossfade's definition).
  - Measured (scratch `rtbench`, the G1 patch, 64-frame blocks, 2026-10-19, load 5–9): the instant-swap block went from p50 151–179 µs, p99 251–327, max 257–347 to p50 54–58, p99 99–101, max 105–129 single-threaded and pinned; paced under SCHED_RR with a swap every 250 ms, from p50 177–195, p99 334–362, max 338–400 to p50 62–65, p99 137–250, max 139–286 (report 03 measured max 997 → 149 µs at load 15–17). A crossfading swap still copies (unchanged: p50 about 190–215 µs pinned).
- **The crossfade, only when a module was added or removed:** both plans run for 256 frames (8 segments), and the output is `old + (new − old)·w` with `w = 0.5 − 0.5·cos(π·(i+1)/256)` (computed once through `libm`). That is an **equal-gain** raised-cosine fade, not design.md's equal-power one: the two plans share the held notes, so their outputs are highly correlated and an equal-power fade would bump the level by up to 3 dB mid-fade; and the form is exact wherever both plans give the same sample, so an unrelated edit (an added LFO with no route, a removed one, a silent oscillator) leaves the output bit-identical (tested). A plan that arrives during a crossfade waits in the ring. Without an added or removed module (a recompile, a structural parameter of a surviving module such as `osc.table` or `voice.mode`), the swap is instant and the old plan goes back at once.
- Swaps never happen in an offline render (`Engine::new` has no ring), so goldens cannot see them; CPU doubles during a crossfade (report 03: crossfade blocks 1.6–2× a normal block).

## Monitor bypass (the plugin's FX bypass)

`Engine::set_monitor_bypass(on)` bypasses every `fx` module whose kind is flagged `time_based` in the catalogue (the reverb in P1). It is engine state, not patch state: **for the plugin only**. `enth-render` builds its engine internally and never calls it, and `RenderSpec` has no field for it, so renders, goldens and scores cannot see it by construction (design.md §5, ADR-0006). Non-time-based effects (the compressor) keep running. Plugins call it from the audio thread before `process` (for example from an atomic the UI sets).

**The 20 ms fade** (ADR-0006; P1b lane E+; before, the switch was instant): engaging it fades those modules to the identity over `MONITOR_BYPASS_FADE_MS` = 20 ms, `max(1, round(20·sr/1000/32))` segments (30 at 48 kHz), then resets them (allocation-free) and stops calling them, so they cost nothing; releasing it fades them back in from that clean state over 20 ms. During the fade a module runs on its input as usual and its output is mixed per sample, `wet·g + dry·(1 − g)` with `g` ramping linearly (1 = not bypassed); the form is exactly the dry signal at `g = 0` and the wet one at `g = 1`. Reversing mid-fade turns the fade around (no reset). The engine holds one fade position for both plans of a crossfade, and a plan swap carries the modules' state, so a swap during a fade continues it. Before the engine has rendered anything (and right after `reset`) the switch is immediate: a project that loads with the bypass on starts without the reverb. Tested: after engaging, the output equals the patch without the reverb once the fade has settled (ADR-0006's test); after releasing, it equals adding the reverb by a plan swap at the same segment once both fades have settled; a reversal mid-fade leaves no trace.

## Host automation: slots and epochs (P1b lane E+)

Report 01 found that host automation sent as `EventKind::Param` with an id from the shell's `PlanMap` can land on the wrong parameter: a plan swap happens at a segment boundary inside `process`, after the shell computed the ids. Two safe forms replace it:

- **`EventKind::Slot { slot, value }`** (the plugin's 64 slot parameters): `value` is the host's normalised value. Each plan holds a **slot table** compiled from `Patch::slots`: per slot, the target's `ParamId`, its spec, its patch value and that value's `to_norm`, and a key (`fnv1a64` of the target address; 0 for an empty slot). The engine resolves the event through the table of the plan that runs when it is due: the **exact patch value** if `value` is bit-equal to `to_norm(patch value)` (`from_norm(to_norm(x))` is not `x` for 33–82 % of values, report 01 F3), else `from_norm(value)`. A target in a disabled module, or a non-automatable one (a patch that skipped `validate`), drives nothing. During a crossfade both plans resolve it through their own tables.
- The engine **remembers the last host value per slot** (`Engine::host_slot`) and **re-applies it after a plan swap** whose slot key is unchanged, so host automation survives an edit; a slot that now names another parameter, or none, forgets it (the session pushes the new target's value to the host). So does every slot the new plan names with **`Plan::forget_host_slots(mask)`** (bit `i` = slot index `i`; F1b review): the session marks all of them on a plan compiled from a loaded patch, and the slots of the values a recompiling edit sets, so the plan's patch value wins instead of the old host value (an in-editor preset load, or a host state load, which nice-plug 0.4.2 applies without re-activating). A value event from the command ring (`Cmd::Param`, an editor edit) updates the slot's patch value, so the host's echo of that edit, `to_norm` of the same value, lands exactly. `reset` keeps the remembered values (CLAP: a reset leaves parameter values alone).
- **`EventKind::EpochParam { epoch, id, value }`**: host automation by id, tagged with the epoch of the plan whose `PlanMap` gave the id; dropped and counted in `stale_events` unless that plan runs when it is due.
- `EventKind::Param` stays for engines without a link (the offline renderer), which never swap.

Tested (`tests/slots.rs`): with a structural edit that moves every id, a plan swap inside the block never routes a value to the wrong parameter (host blocks of 32, 48, 64, 100 and 333 frames; a slot event equals the epoch-tagged event with the new plan's id, bit for bit; an epoch-tagged event for the old plan is dropped; the untagged `Param` is shown misrouted as a negative control); the echo of the patch value is exact; remembered values survive recompiles and are forgotten when the slot moves; a disabled target remembers.

## Telemetry tap (P1b lane E+)

`tap.rs`. A read-only, lock-free, allocation-free window for the editor (report 02, D7): `Tap::new(TapCapacity { params, modulators, scope })` (allocates; main thread) returns an `Arc<Tap>`; `Engine::attach_tap` (main thread, before the engine moves to the audio thread) gives the engine its reference; the editor keeps a clone. At the end of each `process` call, while the tap is enabled (`Tap::set_enabled`, the shell's flag for "editor open and shown"), the engine publishes:

- the running plan's epoch, the routed parameters' modulated values (plain units) in `Plan::tap_params` order (voice-table routed parameters, then global ones) and the modulator outputs in `Plan::tap_modulators` order (voice modulators, then global ones); per-voice values are those of the **display voice**: the newest held voice (highest note index), else the newest releasing one, else the newest fading one; with no voice a voice-table value is its base and a voice modulator's output 0;
- active and held voices, the display voice and its note index, the eight macros and the mod wheel (smoothed), the block's peaks, `EngineStats`, and the block's mono mix `(L + R)/2` into a scope ring (a power of two, default 4,096 samples).

Each value is one atomic holding `f32` bits, overwritten by every publish; a reader copies them with `Tap::read(&mut TapFrame)` (pre-sized by `Tap::frame`, no allocation) and the newest scope samples with `Tap::scope(&mut [f32])`. The frame is one publish's, also across a plan swap: a publish brackets its stores with a version counter (odd while it writes), and `Tap::read` keeps a copy only if the counter was even and unchanged around it (a seqlock, F1b review issue 1; the writer never waits, the reader retries up to 32 times and, if the audio thread was preempted mid-publish, returns no routed values and the generation it had instead of a torn frame). The scope ring is separate and may show the newer block's samples during a copy, harmless for a display. **Overflow drops, never blocks:** values beyond the capacities are not published and are counted (`TapFrame::dropped`); a block longer than the scope ring keeps its newest samples. Once the ring holds a full ring of silence and no voice plays, a block publishes only if a value the editor shows differs from the one the tap holds (**the quiet rule**, lane ED2: a new plan's epoch, a global modulator still running, a macro or a parameter moved in silence, a value that changed while the tap was disabled, so an editor opened in silence sees the current plan at once); such a publish leaves the silent scope ring alone. Otherwise the generation stops and the editor can sleep (the check is one pass over the tap's own last stores: +235 instructions per silent 64-frame block, 3,099 against 2,864, callgrind on release): `Tap::sleep(seen_generation)` (returns `false` if newer data already arrived), and the next publish with new data sets a flag the shell takes with `Tap::take_wake()` after `process` to request a repaint. Publishing only reads engine state: tested to leave the output bit-identical, including with a tap that overflows everywhere; the reader maps `tap_params` to addresses with `PlanMap::addr` and ignores values whose epoch it has no map for. `enth-render` never attaches a tap.

## Reset, position and audition notes (P1b lane E+)

- **`Engine::reset()`** (CLAP `reset`, called on the audio thread; allocation-free): frees every voice (no `NOTE_END` reported), resets every module's state (global modules `reset`, parameters settled on their bases, depth ramps landed, modulator outputs 0, the output ramps at the current master level; voice modules need nothing, `start_voice` resets a slot), empties the FIFO and the event queue, clears poly-mod offsets, and restarts the position, the segment grid and the note index at 0. What follows renders exactly like `Engine::new` with a plan compiled from a patch holding the same bases (tested bit for bit after a performance that left voices, glides, an LFO phase, a reverb tail, the compressor's envelope and a smoother in flight). Kept: the plan and its epoch, the base values (including macros and the mod wheel), the remembered slot values, the commands queued in the ring, the monitor bypass (snapped), the tap, and the link counters (`stale_events`, `swaps`, `crossfades`) and `dropped_events`. A crossfade in progress ends: the replaced plan goes back on the return ring, or into the pending slot if the ring is full, never freed.
- **`Engine::position()`**: the absolute frame of the next host frame, counted from `new` or `reset`; a block passed to `process` starts there.
- **Audition notes** (the editor's keyboard, Recreate): `Link::send_event(EventKind::NoteOn { … })` plays a note whether or not the host transport runs. The event is queued into the event queue at the next segment boundary (it waits in the ring while the queue is full, so an audition note-off is never dropped) and applied in step 2 exactly like a host event at that boundary (tested bit for bit). Give audition notes their own `NoteKey::port` so their `NOTE_END`s can be filtered out. Offline engines have no link, so renders cannot see them, by construction.

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
Performance { notes: Vec<Note>, automation: Vec<Automation>, controls: Vec<Control>, duration_s: f64 }   // ..Default::default()
Note { key: u8, velocity: f32 /* 0–1 */, on_s: f64, off_s: f64 }
Automation { time_s: f64, addr: ParamAddr, value: f32 }       // sets the base value (clamped to the spec); the engine smooths it
Control { time_s: f64, controller: Controller, value: f32 }   // P1b: a macro or the mod wheel, 0–1, smoothed over 10 ms
Controller::{Macro(MacroIndex), ModWheel}                     // text form macro1..macro8, modwheel (FromStr, Display)
RenderOutput { sample_rate, left: Vec<f32>, right: Vec<f32>, stats: RenderStats }   // interleaved(), mid(), peak()
render_with_block(.., block)                                  // the same bits for any block (tests)
```

Times become frames as `round(t · sample_rate)` in `f64`. Same-frame events: automation first, then controls in performance order, then notes in performance order, each note's on before its off. **Controls** (P1b lane E+) let a CLI render express what the plugin does with host macro automation and a MIDI mod wheel: each becomes `EventKind::Macro` or `EventKind::ModWheel` at its frame (tested equal to driving the engine by hand); a value outside 0–1 or a bad time is `bad_performance`. Automation sets the base value, which the engine smooths as the parameter's spec says. Automating a disabled module's parameter is not an error and changes nothing. Errors carry stable codes (`bad_spec`, `bad_performance`, `unknown_param`, and the compiler's codes, including `bad_route`).

## Goldens

A golden case (`tests/golden/cases/<name>.toml`) with `kind = "patch"` embeds the patch as a `[source.patch]` table with exactly the shape of a `*.patch.json` file, plus notes, optional automation and optional controls (`controls = [{ time_s, source = "macro1" | … | "modwheel", value }]`) in seconds and an optional render seed. Stereo renders hash both channels; the summary is computed on the mono mix. `tests/golden/src/patch.rs` converts the table through `enth-patch`'s file layer.

| Case | Guards |
| --- | --- |
| `ref-saw-gate` | The F2 reference: one note of an oscillator through a gate envelope (the S2a reference saw, then D1's Plaits saw, which updated it on purpose) |
| `ref-saw-1v` | `Patch::init()` through the release. D1 and D2 updated it when the real saw and envelope replaced the stubs |
| `engine-chord` | Spine composite: two saws, every kind, chord with off-grid times, a steal (S2b; was a dropped voice), smoothed automation. Updated on purpose whenever a module changes its output |
| `voice-poly-steal` | S2b: polyphony 2, steals with the 64-frame fade into spare slots, then a free slot |
| `voice-mono` | S2b: the 16-key stack, retriggers, the return to the newest held key, a fresh voice after the end |
| `voice-legato-glide` | S2b: legato with an 80 ms `always` glide, including the glide back to a held key and a detached note |
| `voice-poly-glide-legato` | S2b: poly with a 60 ms `legato` glide (overlapping notes glide, detached ones do not), two saws |
| `mod-routes` | S2b: routes from velocity, key and two macros; smoothed automation of a routed parameter and of `master.level` |
| `mod-controls` | P1b lane E+: macro and mod-wheel moves from `Performance::controls` (macro1 on the level, macro3 on the fine tune, the mod wheel on the pan), smoothed over 10 ms |
| `osc-*`, `noise-*` | D1: every wave at three pitches, sync, phase modes, tables, white and pink noise |
| `svf-*`, `adsr-*` | D2: every filter mode, drive, keytrack and a cutoff jump; envelope shapes, retrigger, velocity, zero times |
| `lfo-*` | The integration pass (CH): every shape routed to `filter1.cutoff` through the gate envelope, unipolar, the random shapes with their seeds, a global LFO (also into `comp1.mix`), vibrato on `osc1.fine`, an LFO on an LFO's `rate` |
| `reverb-*`, `comp-*` | FXW: the reverb's impulse, wake, chord and parameters; the compressor's steps and parallel mix |
| `stand-in-sine` | The harness's own smoke test (no engine) |

The S2b cases use only an oscillator, the gate envelope and built-in sources (velocity, key, macros), so the envelope's curves do not move them; D1 updated them with the other saw goldens when the Plaits saw replaced the reference saw.

**Convention for module goldens: play them through a gate envelope** (`attack = 0, decay = 0, sustain = 100, release = 0`), so they do not move when the envelope's curves change. `tests/contract.rs` pins what that needs, in terms of the envelope's **1 ms minimum ramp**: no envelope segment is shorter than 1 ms, rounded up to whole 32-frame segments, `n = adsr::min_ramp_steps(sample_rate)` (2 at 44.1 and 48 kHz, 3 at 96 kHz, 12 at 384 kHz, 1 at 8 kHz). A gate envelope therefore rises strictly for `n − 1` segments and is **exactly 1.0 from the end of segment `n` while the key is held**; after the note-off it falls strictly for `n − 1` segments and is **exactly 0.0 `n` segments after it**. (S2a's reference envelope had `n = 1`: exactly 1.0 after one segment and 0.0 one segment after the note-off. A 0 ms attack or release is never a jump.) Composite cases (the 16-voice patch, a phrase with steals, the 40 s tail) belong to the spine lanes.

## Benchmarks

`crates/enth-engine/benches/common/mod.rs` builds a patch with one module of a kind (`patch_with`), prepares an engine with a held chord outside the measured function (`prepare`) and renders it in 512-frame calls (`render`). The `engine` bench (S2a and S2b, 2026-10-19, x86-64, pinned toolchain; with the reference saw and envelope, so historical, and D1 and D2 re-measured the budgets with the real modules, see [osc.md](osc.md) and [envelope.md](envelope.md)): 1 s at 48 kHz, 1 voice 3,660,346 instructions (76 per frame), 16 voices 40,367,014 (841 per frame); the budgets are those plus 10 %. After S2b (quiet parameter tables): 3,427,804 (71 per frame) and 37,679,499 (785 per frame), within the S2a budgets, which stay as they are. Every patch holds an `osc` and an `adsr`, so these two budgets move whenever either module does. A module bench subtracts the `engine` case with the same voice count to read its own cost.

The **`g1` bench** (`benches/g1.rs`, S2b) is the G1 reference patch: polyphony 16, two oscillators (saw, pulse an octave down) and noise into a resonant low-pass, an amp and a filter envelope, a vibrato LFO per voice and a slow global LFO, compressor and reverb, eight routes (envelope, LFOs, velocity, key, a macro), a held 16-note chord, 2 s in 512-frame calls. Its hard limit is G1's provisional cap, 12,000 instructions per frame (1,152,000,000), so module lanes can land without editing it unless they break G1; lane E re-bases it to the first full product measurement plus 10 %. Measured 2026-10-19 with the stub modules (before D1, D2 and FXW): **122,452,870 instructions (1,276 per frame)**.

## Tests

- `enth-engine` unit tests: parameter tables (exact settled values, ramps, clamping), plan compiler (role lists, disabled modules, the sync master, refused S2b features, invalid patches as errors), voice manager, event queue and FIFO, seeds (pinned), the registry test (every kind builds in the role its section and rate imply; every role variant is reachable); each module's own white-box tests live in its file.
- `tests/segment.rs`: event timing on the grid, block-size invariance (fixed sizes and a proptest over random block sequences), arrival order, voice end, dropped notes, the pan law, master level and value events, render-seed behaviour of the three phase modes, odd host calls.
- `tests/noalloc.rs`: `assert_no_alloc` around every `process` call, including 16 voices with dropped notes, a full event queue and junk events, with a negative control.
- `tests/tail.rs`: a 40 s release tail through one module of every kind: 0 subnormal output samples, 0 subnormal state, 0 voices left, every sample finite; the same bits under every FTZ/DAZ mode. The patch includes every kind, so each lane's module is covered when it lands.
- `tests/contract.rs`: the gate-envelope contract above, at 8, 44.1, 48, 96 and 384 kHz.
- `tests/voices.rs` (S2b): the steal fade's shape and the stolen voice's exact silence after 64 frames, mono following the last note, mono retriggering vs legato, glide (linear, constant time, both glide modes, poly), voice-end reports, chokes, routes from velocity, key, macros and the mod wheel (with smoothing), a resting route changing nothing, block-size invariance with routes, smoothing, glide and steals in every voice mode (a proptest over random notes, macro, mod-wheel and automation events and random block sequences), 40 s tails in every mode with steals and routes: 0 voices, 0 subnormals.
- `tests/swap.rs` (S2b, under `assert_no_alloc`): a held chord survives recompiling the same patch and unrelated structural edits (an added LFO, a removed LFO, a silent added oscillator) **bit for bit**; an added oscillator fades in; for every `ModuleKind` (both LFO scopes) copying state into a fresh module gives identical output for 360 segments; stale-epoch events are ignored and current ones apply; a full command ring (plans, values, macros) drains in one segment; a swap while the return ring is full keeps the plan (held, then handed back) and a third plan is refused while two are in flight; a plan arriving during a crossfade waits; the monitor bypass removes the reverb exactly and leaves the compressor.
- `tests/noalloc.rs` also holds the "every voice stolen" overflow test: a 200-note burst that fills all 32 slots (steals and cuts), a stream that steals every voice again and again, and mono/legato with more keys than the stack.
- P1b lane E+, all under `assert_no_alloc`: `tests/swap.rs` (taking state equals copying it for every kind; the bypass fade out, in and reversed), `tests/slots.rs` (above, and a plan marked with `forget_host_slots` drops the remembered values of the slots it names), `tests/depth.rs` (a depth event equals a recompiled render once its ramp settles; ids only for terms in the plan; stale, NaN and clamped events; a full command ring of depth events; a ramp restarted mid-way), `tests/polymod.rs` (an offset moves only the matching voice and only voice-rate destinations; zero or unmatched offsets change nothing; clamping, wildcards, a reset on a new note and at the voice's end), `tests/tap.rs` (no change to the output; the published values equal the engine's for the display voice; the scope holds the newest mono samples; overflow drops and counts; silence stops publishing; the wake handshake; a plan swap changes the layout with the epoch), `tests/reset.rs` (reset equals a fresh engine; reset during a crossfade with a full return ring; position; audition notes equal host events; a full ring of audition events; audition events finding the event queue full wait in the ring, in order, never dropped), `tests/voices.rs` (`NoteEnd::offset` clamped inside the block for host blocks of 1 to 333 frames), the plan unit test that an instant swap moves the reverb's lines and a crossfade copies them.
- `enth-render/tests/controls.rs`: controls equal the engine's macro and mod-wheel events, block-size invariance, a control to the patch value is a no-op, typed errors and the controllers' text form.
- `enth-render/tests/render.rs`: a saw's pitch, level and release; render-level block invariance with automation; the render seed as the only randomness; **seed stability** (every non-zero float parameter of every kind, `voice` and `master`, moved by 1e-3: the 10 ms RMS envelope moves under 0.05 dB within 30 dB of the peak, measured 0.025 dB; the negative control, a reseed, moves it 6.3 dB); finite output for random parameter values; typed errors.

## Seams for the lanes after F2

| Lane | Files | What fills in |
| --- | --- | --- |
| **S2b** (done) | `voice.rs`, `modulation.rs`, `params.rs`, `swap.rs`, `engine.rs`, `plan.rs`, `benches/g1.rs` | Built as described above. Left for P1b: CLAP polyphonic modulation (per-voice macro offsets, done by E+), wiring `Link` to `enth-session`, measuring compile time and crossfade CPU in the test host |
| **E+** (P1b, done) | `engine.rs`, `event.rs`, `plan.rs`, `modulation.rs`, `params.rs`, `swap.rs`, `segment.rs`, `tap.rs`, `voice.rs`, `module.rs`, `modules/{mod,reverb}.rs`, `enth-render/src/lib.rs` | Slot events and epochs, depth events, poly-mod offsets, the telemetry tap, `reset`/`position`, audition notes, the 20 ms bypass fade, `NoteEnd::offset`, effect state moved at an instant swap, `Performance::controls`; see "P1b seams (lane E+)" |
| **D1** (done) | `modules/osc.rs`, `modules/noise.rs`, `benches/osc.rs`, `benches/noise.rs` | Replaced the stubs; hard sync reads `VoiceCx::sync_in` (the master writes `sync_out`, the reset time per frame); wavetables generated in `new`; ramp pitch from `pitch_prev` to `pitch_cur` (glide); `copy_state_from` copies all state; the `engine` bench budgets were re-measured; `ref-saw-*`, `engine-chord` and the S2b cases (`voice-*`, `mod-routes`) were updated |
| **D2** (done) | `modules/svf.rs`, `modules/adsr.rs`, `modules/lfo.rs`, their benches | Replaced the stubs; the SVF interpolates coefficients from `prev` to `cur` (modulated values change every segment); the gate-envelope contract (restated for the 1 ms minimum ramp) and the voice-end rule are kept; an envelope restarts its attack from its current level on `trigger` without `fresh` (a mono retrigger); the LFO's output range is its polarity (routes carry only a depth); the `engine` bench budgets were re-measured; `ref-saw-1v` and `engine-chord` were updated |
| **FX1** phase 2 (done, as lane FXW) | `modules/reverb.rs`, `benches/reverb.rs` | Wraps `enth_dsp::reverb::Reverb` (`set_params` once per segment with the catalogue's conversions, `process_in_place`, `is_silent` exact); the seed-stability test's `EXEMPT` list holds any parameter that legitimately fails it, with the reason |
| **FX2** phase 2 (done, as lane FXW) | `modules/comp.rs`, `benches/comp.rs` | Wraps `enth_dsp::compressor::Compressor` the same way |
| **C** phase 2 (done) | — | `enth render` maps its perf JSON (note names, velocity 0–127) onto `Performance`; `--seed` sets `RenderSpec::seed` |
| **A**, **H** (done) | — | Analyse `RenderOutput::mid()` (or each channel); nuisance renders change `RenderSpec::seed` only |

A lane replaces only its own files; nothing else in the engine changes when a module lands. All the seams above are filled.

## P1b seams (lane E+): what the session, plugin and editor lanes rely on

| Need | Engine side | Session / plugin / editor side |
| --- | --- | --- |
| Host automation | `EventKind::Slot { slot, value }` (normalised; the engine applies I6, remembers and re-applies); `Plan::forget_host_slots(mask)` | Send a `Slot` event when a slot's host value changes, the modulated value for CLAP mono modulation. Never `EventKind::Param` in the plugin. `EpochParam` if a shell ever automates by id. The session marks plans after a load (all slots) and after a recompiling edit (the written slots) |
| Editor edits | `Link::send_param(epoch, id, plain)` (updates the slot table's patch value) | The epoch and `PlanMap` of the latest *sent* plan |
| Depth drags | `Link::send_depth(epoch, PlanMap::depth(route), pct)`, 10 ms ramp | A recompile when the depth goes from or to 0, or `PlanMap::depth` is `None` |
| Poly-mod | `EventKind::MacroVoice { key: NoteKey::note_id(id), index, offset }` | nice-plug's `PolyModulation { voice_id, poly_modulation_id, normalized_offset }`; ignore `MonoAutomation` |
| `NOTE_END` | `NoteEnd::offset` (clamped into the block) | `timing = offset`; filter the audition port |
| CLAP `reset` | `Engine::reset()` on the audio thread | Nothing else: values and slots are kept |
| Audition | `Link::send_event(NoteOn/NoteOff)` with an audition `NoteKey::port` | Release them on editor close, deactivate and load |
| FX bypass | `set_monitor_bypass` with the 20 ms fade | On a new engine (before its first `process`) or right after `reset` the switch is immediate: set the persisted state there |
| Editor feedback | `Tap` (`attach_tap` before the engine goes to the audio thread); `TapFrame` is `non_exhaustive` (so are `EngineStats` and `NoteEnd`: they may grow) | Enable it while the editor is open and shown; `take_wake()` after `process` → repaint; map `Plan::tap_params` with `PlanMap::addr` per epoch. Every engine counts epochs from 0 |
| Plugin = CLI | `Performance::controls` for macros and the mod wheel | The CLI's perf JSON does not read controls yet (lane C's format) |

## Open points

- The crossfade is equal-gain (raised cosine), not design.md §3's equal-power ramp; the reasons are under "Plan swap". design.md should follow if the owner agrees.
- `crates/enth-dsp/src/smoothing.rs` (an S0 placeholder) holds D2's module-internal smoothers, `LinearRamp` and `OnePole`, which settle exactly on their target. The engine's parameter smoothers (`params::Smoother`) stay in `enth-engine/src/params.rs`: they need `ParamSpec`'s scale mapping.
- S1b's `enth_patch::validate` is merged, and the compiler still checks routes itself (`bad_route`), as a subset of it; the two must agree.

- Windows: the golden hashes were produced on x86-64 Linux and match wasm32-wasip1 under wasmtime 48.0.6; the Windows leg runs in CI on the first push.
- The golden harness's `PatchFile` resolver (`tests/golden/src/patch.rs`) goes through `enth-patch`'s file layer, which checks every value against its `ParamSpec`.
- Disabling the amplitude envelope: the engine runs it anyway; whether it is valid is S1b's `validate`.
- F1b review: the tap's layout tore for one frame across a plan swap; fixed by lane S2s with a seqlock (`Tap::begin`/`finish`, `tests/tap.rs`, and unit tests in `tap.rs` against a synthetic writer). The quiet rule that stopped publishing global modulators, macros and a new epoch after a ring of silence (an editor opened in silence got nothing until a note played) is fixed by lane ED2: in silence the tap publishes what changed ("Telemetry tap" above; `tests/tap.rs`). For PL: `reset` reports no `NOTE_END` for the voices it frees, and a mono or legato return to a held key keeps the released note's poly-mod offsets.
- P1b lane E+: a depth ramp in flight at a plan swap is not carried (the new plan starts at its patch depth); poly-mod offsets are not smoothed (a step per segment, ramped across it by the modules like any modulator); `EventKind::Param` sent through `Link::send_event` names whatever plan runs (send values with `send_param`); the CLI's perf JSON cannot express `controls` yet; `NoteEnd::offset` reports an end the FIFO rendered ahead at the block's last frame (up to 31 frames early).
