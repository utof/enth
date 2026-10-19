# Oscillator (`osc`)

- Lane: P1a D1 (oscillators, wavetables and noise). The catalogue entry is `crates/enth-patch/src/catalogue/osc.rs` (frozen at F1, listed in [catalogue.md](catalogue.md)); the module contract it plugs into is in [engine.md](engine.md).
- Code: DSP in `crates/enth-dsp/src/osc.rs` (the cores) and `crates/enth-dsp/src/wavetable.rs` (the tables); the module in `crates/enth-engine/src/modules/osc.rs`. Tests: the same files (white-box), `crates/enth-engine/tests/d1_modules.rs` (the module contract), `d1_engine.rs` (patches through the engine) and `d1_noalloc.rs`. Goldens: `tests/golden/cases/osc-*.toml`. Bench: `crates/enth-engine/benches/osc.rs`.
- Decisions behind it: research report 02 (`_local/sessions/2026-10-19-p1a-headless-core/02-dsp-modules.md`, Q1, Q2, Q8). Where this lane departs from the report, the section below says so and why.
- Sources and licences: [`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md) has the full MIT notices for the Plaits oscillator and stmlib's polyBLEP residuals and Hermite interpolation. Everything else (the sine polynomial, the wavetable generator and layout, the five tables, the sync-bus form) is our own code; the references for the formulas are cited where they are used.

One oscillator module with five waves: **sine, triangle, saw, pulse** and **table** (the wavetable chosen by the structural `table` parameter, scanned by `position`). Pitch comes from the played note plus `semi` and `fine`; `level` scales the output; `sync` restarts the oscillator at the cycles of the first enabled `osc`; `phase_mode`, `phase` and `seed` choose each note's start phase. All of it is `f32` `+ − × ÷` and comparisons in the per-sample path: no transcendentals, no `mul_add`, no allocation, so the stream is bit-identical on Linux, Windows and wasm (the 34 golden cases are the same on native and `wasm32-wasip1` under wasmtime).

## The analog core: saw, triangle, pulse

`Shape` in `enth-dsp::osc` is a port of Plaits' `VariableShapeOscillator::Render` (MIT, © 2016 Emilie Gillet) with stmlib's `polyblep.h` residuals (MIT, © 2017 Emilie Gillet). The oscillator computes the naive waveform for the *next* sample, adds a polynomial band-limited step (BLEP) wherever the waveform jumps and an integrated one (BLAMP) at the triangle's corners, and outputs the previous step's sum so a residual that straddles two samples can be split between them. Consequences:

- **The output is one sample late** (every wave of this module alike, so oscillators stay aligned with each other and a mixed-wave patch has no comb filtering from different delays). It is invisible in a render and unmeasurable in a patch; tests that compare a wave's phase with its input timing allow for it.
- A bounded transition loop (three passes) replaces upstream's `while (transition_during_reset || !reset)`. A transition time that falls outside its sample (an inconsistent state, e.g. after a wave switch) is clamped to the sample, so no input can produce NaN, infinity or a hang.
- The waveform is a compile-time constant (`SAW`, `TRIANGLE`, `PULSE`): upstream blends triangle, saw and square with a `waveshape` knob; enth has three separate waves, so the terms multiplied by a constant zero are not there. The naive pulse is exactly `{0, 1}` before the output map `2x − 1`, so a pulse is ±1 and **not DC-free** (Plaits does the same); a 50 % square has no DC, a 20 % pulse has a DC of 0.6.
- Frequency and pulse width **ramp linearly over each 32-sample segment** from the previous segment's value to this one's (`prev + (cur − prev)·(i+1)/32`, the weights are a `const` table that equals the engine's `RAMP`). Upstream accumulates an increment with `stmlib::ParameterInterpolator`; ours is the engine-wide ramp, so glide, vibrato and PWM have no zipper noise and no drift. This is a change from upstream (listed in the notice).
- The phase increment is clamped to `[1e-7, 0.25]` cycles per sample (a quarter of the sample rate, 12 kHz at 48 kHz, upstream's `kMaxFrequency`) and the pulse width to `[2f, 1 − 2f]` (exactly 0.5 above a quarter of the sample rate), as upstream. The module clamps `pw` 5…95 % on top of that, from the catalogue.

### Sine

A fifth-degree odd minimax polynomial in the phase, `sin(2πq)` on `q ∈ [−¼, ¼]` after folding the phase into a quarter turn, evaluated with `+ − ×`:

```text
q·(c1 + q²(c3 + q²(c5 + q²(c7 + q²·c9))))
c1 = 6.2831852   c3 = −41.341656   c5 = 81.601006   c7 = −76.54978   c9 = 39.536705
```

The coefficients are a Remez exchange computed in 50-digit arithmetic (the script is `~/code/enth-scratch/p1a/d1/remez_sin.py`, kept outside the repository; it needs `mpmath`): the **maximum error is 3.3e-9**, below the `f32` grid near 1, so the polynomial is as accurate as a correctly rounded `sinf`, at the cost of five multiplies. The same fold-and-evaluate is used for every phase, so there is no table and no `libm` call on the audio path. `c1` is deliberately a hair below `2π` (the minimax balances the error over the quarter turn); the unit test checks 200,000 phases over the whole turn against `libm::sin` in `f64` to under 1e-6 (the `f32` evaluation error, not the polynomial's, is what remains).

## The phased core: sine and table

`Phased` is a plain phase accumulator for any wave read as a function of the phase (the sine and the wavetable): the same one-sample delay, the same ramped increment, the same sync bus, and the same BLEP at a reset, so a synced sine or table has the step at its restart band-limited just like the analog waves. The phase increment is clamped to 0.45 of the sample rate (21.6 kHz at 48 kHz). The table position is also ramped over the segment, one weight per sample.

## Wavetables

Five tables, generated in code at module-prepare time (no audio assets; the data is the same on every platform, which the unit test pins with one FNV-1a fingerprint per table over the `f32` bits).

### Layout (measured change from report 02)

Each table has **13 frames** (so the 3-, 4- and 6-way segments of the tables below fall on whole frames) and each frame **9 mip levels**: level `l` holds the harmonics `1 … 256 >> l` (256, 128, … 1) and is stored at `max(32 × top harmonic, 1024)` samples (8192, 4096, 2048 and six levels of 1024), plus three guard samples (`x(−1), x(N), x(N+1)`) so the Hermite read never wraps an index. A table is about **1.07 MB** (report 02: 11 levels of 2048, 1.1 MB). All level lengths are powers of two, so the read index is a mask.

Why not report 02's 2048 × 11? Re-measured here with Hermite interpolation: the Hermite reconstruction of a 2048-point cycle leaves images of the *highest* harmonics of the level that the report's test pitches did not show, and with the frame-0 `tilt` table (the brightest) and a saw at 55 Hz they land in band: **−77.7 dB for a saw at 55 Hz, −60 dB for tilt frame 0 at 55 Hz**, against the −85 dB ceiling in the plan. Storing each level at 32 samples per top harmonic keeps every image of every stored harmonic far above 20 kHz. After the change the worst case over 5 tables × 5 frames × 7 pitches (55 … 7,040 Hz) is **−91.6 dB** (tilt frame 0 at 440 Hz), and the rest sit at the −93 dB measurement floor. The memory and the per-sample cost did not change.

### Reading

`level_for(dt, alias_limit(sample_rate))` picks the lowest (brightest) level whose top harmonic times the phase increment stays at or below `alias_limit = max(½, 1 − 20 kHz / fs)` cycles per sample: at 48 kHz harmonics may reach **28 kHz**, so whatever aliases folds into 20 – 24 kHz where nothing is audible (the report's recommendation). Below about 109 Hz the full 256 harmonics are used; above, the bandwidth is 14 – 28 kHz depending on where the pitch falls in its octave. The level is chosen per segment from the larger of the two ends' increments; there is no cross-fade between levels, so a glide across an octave boundary has a brightness step (a known limitation, as in report 02).

`Wavetable::cursor(level, frame_pos)` fixes the two frame rows and the cross-fade weight, and `Cursor::read(phase)` does the per-sample work: 4-point Hermite in phase (stmlib's `InterpolateHermite`, MIT, © 2012 Emilie Gillet) in each of the two frames, then a linear cross-fade `a + (b − a)·w`. The module builds one cursor per segment when the position holds still (the usual case: the smoothed parameter has settled to exactly its base value) and reads the lower frame alone when the position sits on a frame (0 %, 100 %, k/12), saving a Hermite read. `Wavetable::sample` is exactly `cursor(…).read(…)`.

### Generation

Additive synthesis in `f64` from a master sine table of 8,192 points indexed by integers (`sin(2π·i/8192)` through the pinned `libm`; harmonic `k` at sample `i` of a level with step `s` reads index `(k·i·s) mod 8192`, so no phase is ever computed in floating point), summed in a fixed order, rounded once to `f32`. Every frame is **peak-normalised at level 0** and the other levels of the frame use the same scale, so a note does not change loudness when its mip level changes. The cost is that a frame whose spectrum overshoots (the square with its Gibbs ripple) has a flat top below 1: the basic table's saw and square read about 0.92 and 0.85 of full scale, and thin pulses are quiet. It is deliberate: a table never clips and every frame peaks at exactly 1.

`Wavetable::shared(kind)` generates each table once per process (a `OnceLock` per table holding an `Arc`) and hands out clones, so preparing a plan with several oscillators on the same table, or a second plan, costs a refcount bump. The cache is a memo of a pure function; nothing on the audio path touches it. Generating a table takes about 70 ms in a release build (all five and their fingerprints take 0.37 s), at plan-compile time on the control thread, never on the audio thread.

### The five tables

All coefficients are for `x(t) = Σ sₖ sin(2πkt) + cₖ cos(2πkt)` and every shape *starts like its analog counterpart* (a sine rises from 0; saw, triangle, square and pulse start at their low value), so switching between a table at position 0 and the analog wave of the same shape keeps the phase meaningful.

| Table | Frames | Formula |
| --- | --- | --- |
| `basic` | 13 | Sine → triangle → saw → square in three equal blends. Triangle `−cos` series (odd `k`, `1/k²`); saw `−(2/π)·Σ sin(kx)/k`; square `−(4/π)·Σ_{k odd} sin(kx)/k` |
| `pulse` | 13 | A pulse whose duty falls from 50 % to 3 % along the frames (`d = 0.5 − 0.47·pos`), the DC removed, written with both sine and cosine terms |
| `tilt` | 13 | `sₖ = −k^(−p)` with `p = 0.5 + 2.5·pos`: from very bright (p = 0.5) through the saw (p = 1) to nearly a sine (p = 3) |
| `formant` | 13 | The vowels A, E, I, O, U on a saw spectrum (`1/k` plus a 0.02 floor) under three Lorentzian resonances at the male-average F1…F3 of Peterson and Barney (1952, *J. Acoust. Soc. Am.* 24, 175–184): A 730/1090/2440 Hz, E 530/1840/2480, I 270/2290/3010, O 570/840/2410, U 300/870/2240, expressed as harmonic numbers of a 110 Hz reference fundamental; half-widths 1.0, 1.2 and 1.8 harmonics, gains 1, 0.7 and 0.35; linear morph between neighbouring vowels. The formants are therefore at fixed *harmonic numbers*, not fixed Hz: they track the note, which is what a wavetable can do |
| `organ` | 13 | Seven drawbars on harmonics 1, 2, 3, 4, 5, 6 and 8 (8′, 4′, 2⅔′, 2′, 1⅗′, 1⅓′, 1′; the 16′ and 5⅓′ drawbars sit below the fundamental and cannot live in a one-cycle table). Settings 0 – 8, each step 3 dB, 0 is off; six equal blends between seven registrations (fundamental only; 8-8; 8-6-8; 8-8-8-8; 8-8-8-8-8; 6-8-6-8-6-8; all eight) |

## Hard sync

**The engine's sync-bus form works as designed; no change to the module contract is proposed.** The first enabled `osc` in `generators` is the master (the engine fills `VoiceCx::sync_out` with `NO_SYNC = −1` and passes it to that module); each later `osc` with `sync` on reads `VoiceCx::sync_in`. The master writes, for every frame where its phase wrapped, **Plaits' own `reset_time`**: the fraction of the frame that remained after the wrap, in `[0, 1)`. A slave restarts at that point, with the BLEP (and, for the triangle and pulse, the transition bookkeeping) of `Render<true, …>`.

What the tests establish (all in `crates/enth-dsp/src/osc.rs` unless noted):

- *Same numbers as upstream.* A master's `sync_out` equals what upstream computes inside its own `Render<true, …>` (`master_phase / master_frequency` at each wrap), **bit for bit**, at 55 Hz, 440 Hz, 3,520 Hz and 11.9 kHz from several start phases, for a saw, triangle and pulse master. The reference in that test is a transcription of upstream's formula; the C++ upstream itself was not linked in.
- *Periodic at the master's pitch.* A slave restarts bit-exactly every master cycle, for every combination of master and slave wave (`d1_engine.rs::hard_sync_makes_any_slave_periodic_at_any_masters_pitch`: 4 master waves × 5 slave waves, at least 99.97 % of the output power on the master's harmonics against about 1 % or less free-running, with a silent master).
- *Aliasing of the classic case.* A saw slave at ×2.37 the master's pitch (the plan's reference case): alias power over harmonic power **−43.6 dB at a 110 Hz master and −36.8 dB at 440 Hz**, the same as report 02's prototype.
- *Other waves.* A sine or table slave has its reset jump (`shape(0) − shape(at_reset)`) band-limited with the same step: a synced sine measures −50.2 dB against −26.3 dB with a hard reset.

A master with `sync` on ignores its own `sync`. The master writes the bus whether or not its own `level` is 0, so a silent master is a legitimate way to build a sync-only modulator. Any wave can be the master.

## Phase modes

- `free`: the start phase is `Rng::new(start.seed).unit_f32()` where the engine derives `start.seed` from the render seed, the module id and the note index (`voice_seed`). It never depends on a parameter value.
- `reset`: `phase` degrees / 360.
- `seeded`: `Rng::new(seeded_voice_seed(seed_param, module_key, note_index)).unit_f32()`: the module's own `seed` replaces the render seed, so the render seed does not move it and a patch alone fixes it.

The start phase is drawn in `start_voice` but applied when the voice first renders, so it does not depend on the order of parameter events in the segment that started the note. A voice that changes **wave class** while it sounds (`wave` is automatable) keeps its phase: the new class starts from it (`Shape::new` / `Phased::new`), so the only discontinuity is the change of waveform. `table` is *structural*: the plan generates the named table in `Osc::new` **even while `wave` is not `table`**, so automating `wave` to `table` later finds it ready (`d1_engine.rs` checks that the switched note then plays at the same level as a patch that started on `table`).

## Pitch

`pitch = note + semi + fine/100` semitones, taken at the start (`pitch_prev`, `prev`) and the end (`pitch_cur`, `cur`) of the segment, so glide and vibrato ramp the increment. `midi_to_hz` is the pinned `libm` `exp2f` (per voice and segment, not per sample), divided by the sample rate. The tests measure the frequency by interpolated zero crossings over 4 – 10 s at keys 33, 69 and 105 for every wave and two tables: **within 1e-4 of the nominal pitch** (the `f32` phase accumulator's error is far smaller; the bound is the plan's).

## Aliasing, measured

Report 02's measure: alias power over harmonic power, 20 Hz – 20 kHz, 32,768 samples, Blackman-Harris window, ±8 bins around each harmonic counted as signal.

| Case | Measured | Ceiling (plan / report 02) |
| --- | --- | --- |
| Saw, 440 Hz | **−40.6 dB** | ≤ −38 dB (report: −40.6) |
| Triangle, 440 Hz | **−84.0 dB** | ≤ −80 dB |
| Square (pulse 50 %), 440 Hz | −42.8 dB | |
| Pulse 20 %, 440 Hz | −41.0 dB | |
| Saw, 3,520 Hz | **−28.8 dB** | (report: −28.8) |
| Saw slave, sync ×2.37, 110 Hz master | −43.6 dB | |
| Saw slave, sync ×2.37, 440 Hz master | −36.8 dB | |
| Wavetable, every table / 5 frames / 7 pitches 55 – 7,040 Hz | worst **−91.6 dB** (tilt frame 0, 440 Hz); the rest at the −93 dB floor | ≤ −85 dB, also at 3,520 Hz |
| Wavetable through the engine (`d1_engine.rs`), basic / tilt / formant × 3 positions × keys 45, 69, 105 | all below −85 dB | ≤ −85 dB |

The same numbers come out of the engine-level test for the saw at keys 69 and 105 (−40.6 and −28.8 dB). The polyBLEP saw is the classic two-point residual; the aliasing at 3,520 Hz is what that polynomial gives (the report notes a better BLEP, Signalsmith's `elliptic-blep`, as a later upgrade).

## Cost, measured

Callgrind instructions (`cargo xtask bench --bench osc`, 1 s of audio at 48 kHz, 16 voices, 512-frame host blocks), as the difference to the same patch without the second oscillator (`benches/engine.rs`'s saw patch), per sample and voice. They include the engine's mixing of the extra generator into the voice, which is a few instructions.

| `osc` mode | Instructions per sample |
| --- | --- |
| saw | 38.6 (report 02: 37.7) |
| saw, hard-synced (the master `osc1` is the reference saw) | 41.9 (report 02: 53.5) |
| triangle | 55.0 |
| pulse | 49.0 |
| sine | 55.6 |
| table, position between two frames (two Hermite reads and a cross-fade) | 107.9 (report 02: 62.3 per read, about 2× with a morph) |
| table, position on a frame (one Hermite read) | 88.8 |

The `engine` bench (the reference saw `osc1` plus the envelope, amp stage and pan) is 3,935,626 instructions for one voice (82 per frame) and 43,647,952 for 16 voices (909 per frame, 56.8 per voice and sample). Budgets (measured + 10 %, in the bench files): see the commit message and `benches/osc.rs`.

## Notes for lane S2b and the spine

- **ENGINE_VERSION**: the oscillator replaces a stub, so every render with an `osc` changes. This lane does not bump `ENGINE_VERSION` (`crates/enth-engine/src/lib.rs` is S2a's file); the coordinator should bump it when the lanes merge.
- **Start phase** is set only by `start_voice`. A legato or mono retrigger that does not call `start_voice` keeps the phase running, which is what a mono synth's legato should do; if S2b wants a phase reset on retrigger it needs to call `start_voice` again.
- **Smoothing**: `semi`, `fine`, `level`, `pw` and `position` are smoothed parameters (F1 catalogue). The module reads `cur`/`prev` and nothing else, so S2b's smoothing and modulation arrive without a change here. `wave`, `table`, `sync`, `phase_mode`, `phase` and `seed` are read from `base`. Goldens that automate `position` (or `pw`, `level`, `semi`, `fine`) will change when S2b's smoothing lands; the D1 goldens avoid automating smoothed parameters (the only automation is `wave`, which is discrete).
- **Sync master**: the first enabled `osc` is the master; the engine decides which. If S2b adds an enable switch or a module reorder, the module needs nothing else.
- **Table memory**: 1.07 MB per table, shared process-wide. A plan swap that keeps the table keeps the same `Arc`.

## Limitations

- One sample of latency on every wave.
- Pulse (and thin pulses) carry DC (the levels are ±1); the table's pulse has its DC removed.
- The saw's aliasing rises with pitch (−28.8 dB at 3,520 Hz); use the wavetable `basic` table, which is band-limited by construction, when a clean top octave matters.
- No cross-fade between mip levels while gliding across an octave boundary.
- `fine` and `semi` change pitch per segment (ramped); there is no per-sample pitch modulation inside a segment beyond the linear ramp.
