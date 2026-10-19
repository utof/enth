# Engine (`enth-engine`, `enth-render`)

- Lane: P1a S2a, the walking skeleton (freeze point F2). After F2, changes to the types below are additive only.
- Code: `crates/enth-engine/src/` (module contract, plan compiler, segment loop, stub modules), `crates/enth-render/src/lib.rs` (offline render), `crates/enth-dsp/src/util/{math,rng}.rs` (the `libm` door and seeded randomness), `tests/golden/` (cases and the patch source).
- Decisions behind it: [design.md §3 and §5](../design.md#3-engine-and-real-time-safety), [architecture.md](../architecture.md), research report 01 §5–§7 (`_local/sessions/2026-10-19-p1a-headless-core/`).

The engine turns a `Patch` into stereo `f32` the same way on every platform and for every host block size. One reference saw goes through the whole path (module, plan, segment loop, render, golden) and hashes the same on x86-64 Linux and wasm32 (Windows runs in CI). Every other module is a clearly marked stub that its lane replaces.

## Data flow

```text
Patch ──plan::compile(patch, &PrepareCfg)──▶ (Plan, PlanMap) ──Engine::new(plan, render_seed)──▶ Engine
Engine::process(left, right, &[Event])      any number of frames, events timed by offset in the block
enth_render::render(patch, perf, spec)      the offline wrapper: compile, schedule events, call process in blocks
```

- `compile` allocates: it prepares every module (`ModuleImpl::new`) and sizes every table. Nothing after it allocates.
- `PlanMap` maps parameter addresses (`filter1.cutoff`, `voice.pan`, `master.level`) to the plan's `ParamId`s. It stays off the audio thread, where an op or an automation point becomes a value event.

## Modules: four roles, enum dispatch

| Role | Trait | Kinds | Called |
| --- | --- | --- | --- |
| Voice processor | `VoiceProcessor` | `osc`, `noise` (generators), `svf` (per-voice effect) | `start_voice` at a note-on, `process` once per segment over the active voices |
| Voice modulator | `VoiceModulator` | `adsr`, `lfo` with `scope = voice` | `start_voice`, `tick` once per segment (writes one value per voice), `voice_finished` |
| Global modulator | `GlobalModulator` | `lfo` with `scope = global` | `reset` at start, `tick` once per segment (returns one value) |
| Global processor | `GlobalProcessor` | `reverb`, `comp` | `reset` at start, `process` once per segment on the stereo bus, `is_silent` |

Every role also has `copy_state_from` (for S2b's plan swap; never allocates) and `subnormals` (counts subnormal state, for the tail test).

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

## Parameters

Each plan has two flat tables: the voice table (modules that run per voice, and `voice.*`) and the global table (global modules, and `master.*`). A slot holds `base`, `cur` and `prev`, per voice in the voice table. `ParamId::Voice(i)` or `ParamId::Global(i)` names a slot.

- **Settled values are exact.** Without smoothing in flight and without modulation, `cur` is `base`, the plain value from the patch, bit for bit. It never goes through `from_norm(to_norm(x))`, which is not guaranteed to round trip.
- A value event (`EventKind::Param`) sets `base`, clamped to the spec; NaN and unknown ids are ignored.
- S2a has no smoothing and no routes: step 3 copies `base` into `cur`. Smoothing and modulation (S2b) run in the normalised domain through `ParamSpec::to_norm`/`from_norm`, write `cur`, and snap back to `base` when settled.

## The segment grid and the host adapter

`SEG = 32` frames is the processing quantum. `Engine::process` copies frames out of a one-segment FIFO and renders a new segment whenever it runs dry. Before rendering the segment that starts at frame `32·k`, it applies every queued event whose frame is at or before `32·k`, in arrival order. So an event at frame `f` takes effect at `ceil(f / 32) · 32` (0.67 ms at 48 kHz), and the grid is counted from the engine's start, never from the start of a host call: any sequence of block sizes gives the same bits (tested for 1 to 36,000 frames and random block sequences). Blocks are never split at event times.

Events wait in a fixed-capacity queue (`segment::EVENT_CAPACITY` = 4,096). When it is full, a new event is dropped and counted (`EngineStats::dropped_events`); nothing reallocates.

## The order of one segment

1. Command ring (plan swaps, epoch-tagged value events): **TODO(S2b)**.
2. Due events, in arrival order: note-ons (the voice manager picks a slot; every voice module gets `start_voice` with its seed; the slot's modulator outputs restart from 0), note-offs (gate down), value events.
3. Smoothers: S2a copies `base` into `cur` (a fresh voice gets `prev = cur`). **TODO(S2b)**: smoothing.
4. Global modulators, in patch order.
5. Voice modulators, in patch order, each over the active voices.
6. Modulated parameters: **TODO(S2b)**: `n = clamp(base_norm + Σ depth/100 · src, 0, 1)` in patch route order.
7. Voices: each voice bus is zeroed, generators write a scratch bus and are summed in patch order (the first enabled `osc` first writes the hard-sync timing that later generators read), then the `voice_fx` chain, then the amp stage and the pan into the stereo sum in ascending voice order.
8. `fx` in order (a processor that `is_silent` with an all-zero input is skipped, which must be bit-identical to running it), then `master.level`.
9. Voices whose amplitude envelope reports `voice_finished` are freed. Modules flush their own state; nothing in the engine recurses.
10. The FIFO is filled (a debug assertion checks every sample is finite).

## Voices, the amp stage and the pan law

- **Allocation (S2a, minimal):** poly only. A note-on takes the lowest free of 32 slots while fewer than `voice.polyphony` voices are held or releasing; otherwise the note is dropped and counted (`dropped_notes`). Note-offs match CLAP's (port, channel, key, note id) with −1 wildcards. Every note-on, dropped or not, takes the next `note_index`, so S2b's stealing will not move any seed.
- **`NoteState`** (per slot): `pitch_prev`/`pitch_cur` in semitones, `velocity`, `gate`, `trigger` (a note-on landed this segment), `fresh` (a new voice), `release`, `note_index`.
- **Voice end:** a voice is freed at the end of a segment in which the amplitude envelope (`voice.amp_env`) is past its attack and below −100 dB. Its slot costs nothing afterwards.
- **Amp stage:** the envelope's value is per segment; the gain ramps linearly from the previous segment's value to this one's across the 32 frames (`RAMP[i] = (i+1)/32`, exact).
- **Voice bus and pan (decided for F2):** voices are mono (one generator sum per voice, so the per-voice effects run once, not twice), placed into a stereo bus by `voice.pan` with a constant-power law: `θ = (pan + 1)·π/4`, gains `(cos θ, sin θ)` in `f64` via `libm`, −3 dB and exactly equal at the centre, exactly `(1, 0)` and `(0, 1)` at the ends. The pan gains ramp with the amp gain. Effects (`fx`) are stereo. Unison spread would need a stereo voice bus; it is not in P1.
- **Master:** `master.level` (dB → gain through `math::db_to_gain`) ramps per segment and is skipped at exactly 0 dB.

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

Times become frames as `round(t · sample_rate)` in `f64`. Same-frame events: automation first, then notes in performance order, each note's on before its off. Automating a disabled module's parameter is not an error and changes nothing. Errors carry stable codes (`bad_spec`, `bad_performance`, `unknown_param`, and the compiler's codes).

## Goldens

A golden case (`tests/golden/cases/<name>.toml`) with `kind = "patch"` embeds the patch as a `[source.patch]` table with exactly the shape of a `*.patch.json` file, plus notes and optional automation in seconds and an optional render seed. Stereo renders hash both channels; the summary is computed on the mono mix. Until S1b's file layer lands, `tests/golden/src/patch.rs` converts the table with the catalogue's parsers; S1b's conversion replaces it without changing any case file.

| Case | Guards |
| --- | --- |
| `ref-saw-gate` | The F2 reference: the reference saw through a gate envelope. D1 updates it on purpose |
| `ref-saw-1v` | `Patch::init()` with the reference envelope, through the release. D1 and D2 update it |
| `engine-chord` | Spine composite: two saws, every kind, chord with off-grid times, a dropped voice, automation. Updated whenever a stub is replaced, on purpose |
| `stand-in-sine` | The harness's own smoke test (no engine) |

**Convention for module goldens: play them through a gate envelope** (`attack = 0, decay = 0, sustain = 100, release = 0`), so they do not move when D2 replaces the envelope. `tests/contract.rs` pins what that needs: the envelope's output is exactly 1.0 from the end of a note's first segment while held and exactly 0.0 one segment after the note-off. Composite cases (the 16-voice patch, a phrase with steals, the 40 s tail) belong to the spine lanes.

## Benchmarks

`crates/enth-engine/benches/common/mod.rs` builds a patch with one module of a kind (`patch_with`), prepares an engine with a held chord outside the measured function (`prepare`) and renders it in 512-frame calls (`render`). The `engine` bench (2026-10-19, x86-64, pinned toolchain): reference saw, 1 s at 48 kHz, 1 voice 3,660,346 instructions (76 per frame), 16 voices 40,367,014 (841 per frame); the budgets are those plus 10 %. Every patch holds an `osc` and an `adsr`, so D1 and D2 re-measure these two budgets when they replace the stubs. A module bench subtracts the `engine` case with the same voice count to read its own cost.

## Tests

- `enth-engine` unit tests: parameter tables (exact settled values, ramps, clamping), plan compiler (role lists, disabled modules, the sync master, refused S2b features, invalid patches as errors), voice manager, event queue and FIFO, seeds (pinned), the registry test (every kind builds in the role its section and rate imply; every role variant is reachable), the reference saw.
- `tests/segment.rs`: event timing on the grid, block-size invariance (fixed sizes and a proptest over random block sequences), arrival order, voice end, dropped notes, the pan law, master level and value events, render-seed behaviour of the three phase modes, odd host calls.
- `tests/noalloc.rs`: `assert_no_alloc` around every `process` call, including 16 voices with dropped notes, a full event queue and junk events, with a negative control.
- `tests/tail.rs`: a 40 s release tail through one module of every kind: 0 subnormal output samples, 0 subnormal state, 0 voices left, every sample finite; the same bits under every FTZ/DAZ mode. The patch includes every kind, so each lane's module is covered when it lands.
- `tests/contract.rs`: the gate-envelope contract above.
- `enth-render/tests/render.rs`: the reference saw's pitch, level and release; render-level block invariance with automation; the render seed as the only randomness; **seed stability** (every non-zero float parameter of every kind, `voice` and `master`, moved by 1e-3: the 10 ms RMS envelope moves under 0.05 dB within 30 dB of the peak, measured 0.025 dB; the negative control, a reseed, moves it 6.3 dB); finite output for random parameter values; typed errors.

## Seams for the lanes after F2

| Lane | Files | What fills in |
| --- | --- | --- |
| **S2b** | `voice.rs` | Stealing (oldest released, else oldest held, 64-frame fade into a spare slot), mono/legato with the 16-key stack, glide (writes `NoteState::pitch_prev/pitch_cur`), choke, voice-end queue for CLAP `NOTE_END` |
| | `modulation.rs`, `params.rs` (`VoiceTable::advance`, `GlobalTable::advance`) | Smoothers (step 3), routes (step 6; modulator parameters just before each modulator ticks), macros and the mod wheel as sources; then remove the `NotYet` refusals in `plan.rs` |
| | `swap.rs`, `engine.rs` step 1, `plan.rs` | Command and return rings (`rtrb`), epochs, `Plan` state carry by (`Slot::key`, kind) with `copy_state_from`, the 256-frame crossfade; overflow tests under `assert_no_alloc` |
| | `benches/g1.rs` | The 16-voice reference patch, using `benches/common` |
| **D1** | `modules/osc.rs`, `modules/noise.rs`, `benches/osc.rs`, `benches/noise.rs` | Replace the stubs; hard sync reads `VoiceCx::sync_in` (the master writes `sync_out`, the reset time per frame); wavetables generated in `new`; re-measure the `engine` bench budgets; update `ref-saw-*` and `engine-chord` |
| **D2** | `modules/svf.rs`, `modules/adsr.rs`, `modules/lfo.rs`, their benches | Replace the stubs; the SVF interpolates coefficients from `prev` to `cur`; keep the gate-envelope contract and the voice-end rule; re-measure the `engine` bench budgets; update `ref-saw-1v` and `engine-chord` |
| **FX1** phase 2 | `modules/reverb.rs`, `benches/reverb.rs` | Wrap `enth_dsp::reverb::Reverb` (`set_params` once per segment with the catalogue's conversions, `process_in_place`, `is_silent` exact); if `room` or `decay` legitimately fail the seed-stability test, list them in its `EXEMPT` with the reason |
| **FX2** phase 2 | `modules/comp.rs`, `benches/comp.rs` | Wrap `enth_dsp::compressor::Compressor` the same way |
| **C** phase 2 | — | `enth render` maps its perf JSON (note names, velocity 0–127) onto `Performance`; `--seed` sets `RenderSpec::seed` |
| **A**, **H** | — | Analyse `RenderOutput::mid()` (or each channel); nuisance renders change `RenderSpec::seed` only |

A lane replaces only its own files; nothing else in the engine changes when a module lands.

## Open points

- Windows: the golden hashes were produced on x86-64 Linux and match wasm32-wasip1 under wasmtime 48.0.6; the Windows leg runs in CI on the first push.
- The golden harness's PatchFile resolver (`tests/golden/src/patch.rs`) is temporary until S1b's conversion merges.
- Disabling the amplitude envelope: the engine runs it anyway; whether it is valid is S1b's `validate`.
