# Analysis (`enth-analysis`)

- Date: 2026-10-19. P1a lane A. Crate: `crates/enth-analysis` (L3; depends on `libm`, `rustfft`, `realfft`, `ebur128`, `serde`; no I/O, clocks, threads or global RNG; builds for wasm32).
- What it is: every number that `enth describe`, `enth check`, the hint solver (`enth-solver::hint`) and the sound score read. A descriptor bug lets agents "pass" for the wrong reasons (the Phase 0 trial's 15 of 15 was 13 of 15 under correct descriptors), so each measurement below is a plain, documented definition with a measured accuracy.
- Ported from in-repo MIT code and our own research prototype, no third-party source: `spikes/spike-metric` (features, YIN, loudness, distances), `spikes/spike-agent` (descriptors, expectation parser, units) and the P1a research prototype `proto03` (report 03 §4: fitted beat removal, the multi-harmonic beating estimator, clipping fields, margins and noise, note-off inference; `~/code/enth-scratch/2026-10-19-p1a/03/proto/`). Nothing needs a `THIRD_PARTY_NOTICES.md` entry.

## Module map

| Module | Holds |
| --- | --- |
| `descriptors` | `Describer` (one per worker thread), `Descriptors`, the vocabulary `Feature` (path, aliases, unit, scale, meaning, hint margin, missing reason), `NoteTiming`, `Want` |
| `expect` | Expectation parser (`parse_list`, `parse_item`), `Expect` (`compared`, `interval`, `shortfall`), the checker (`check`, `check_signals`) and its `ItemResult` / `CheckReport` |
| `margin` | `Interval`, signed margins, `margin_pct`, nuisance `noise`, `is_thin` (`THIN_FACTOR` = 2) |
| `envelope` | 8 ms RMS envelope, fitted beat removal, stage measurements (`Envelope`), `infer_note_off` |
| `beat` | Multi-harmonic beating estimator and tremolo detection |
| `pitch` | YIN tracker and its summary (`Pitch`) |
| `loudness` | `LoudnessMeter`: LUFS, sample peak, true peak, clipping (`Level`) |
| `features`, `distance` | Multi-resolution log-mel, envelope, centroid track, MFCC, f0 track (`FeatureExtractor`); metric v1 and its components |
| `fft` | `FftPath::{Scalar, Dispatched}` and the real-FFT planner |
| `units` | Descriptor units and note names |
| `test_signals` | Signals with known answers: sines, polyBLEP oscillators, detuned pairs, ADSR, tremolo, noise, the 10-parameter toy voice |

## Descriptors

All on the float render (never a clamped or quantised file), at any sample rate from 8 to 192 kHz (48 kHz everywhere in P1a). A signal must be finite and at least 20 ms long. Below −80 dBFS sample peak a clip is `silent` and only `loudness` is filled. `NoteTiming { on_s, off_s }` gives the performance: decay, sustain and release need a note-off; without one they are `None` (use `infer_note_off` for a foreign WAV, then pass its result).

| Path (alias) | Unit | Meaning (the one-line `Feature::meaning`, abridged) | How |
| --- | --- | --- | --- |
| `pitch.f0_hz` (`pitch`, `pitch.note`) | Hz | Fundamental measured from the audio | YIN, frame 2048, hop 512, 50–2000 Hz, threshold 0.15; median over voiced frames whose centre is within 40 dB of the envelope peak |
| `pitch.cents_dev` | ct | Offset from the nearest note (A4 = 440 Hz) | From `f0_hz` |
| `pitch.stability` | 0–1 | 1 = steady | 1 − (std of voiced frames in cents)/50, clamped |
| `envelope.attack_ms` | ms | Onset (−40 dB re peak) to within 1 dB of the peak | On the beat-corrected envelope (below) |
| `envelope.decay_ms` | ms | Peak until the level settles near the held level | First drop below `S + max(1.5 dB, 0.2·|S|)` (S floored at −40 dB) that stays below for max(30 ms, half a beat period); 0 when S is within 1 dB of the peak. A threshold time, not the ADSR parameter, and not the spike's "90 % of the way" |
| `envelope.sustain_db` | dB | Level just before note-off, re the peak | Straight line through the dB level over max(70 ms, one beat period when ≥ 2 periods), ending 10 ms before note-off, read at its end; floored at −80 |
| `envelope.release_ms` | ms | Note-off to −40 dB re the level just before it | Last time above that level; if the clip ends first, a line in dB over the tail is extrapolated (`release_extrapolated`) |
| `envelope.duration_ms` | ms | How long the sound stays within 40 dB of its peak | Onset to the last such frame; `duration_truncated` marks a lower bound |
| `envelope.ripple_db` | dB | Residual wobble after beat removal | p95 − p5 of the corrected dB curve against its 200 ms moving average (in dB, full windows only) over the held part |
| `spectrum.centroid_hz` (`brightness`) | Hz | Spectral centroid | Magnitude-weighted, Hann STFT 2048 / hop 512, frames within 50 dB of the loudest; `centroid_track_hz` = first, middle and last third of the audible span |
| `noise_ratio` (`noisiness`) | 0–1 | Energy not in steady peaks | 8192-point Welch spectrum of the held part; a peak is a local maximum 15 dB over the median of its ±64 bins (±3 bins count as tonal); share of 40 Hz–16 kHz power outside peaks |
| `detune.beating_hz` (`beating`), `detune.depth_db` | Hz, dB | Detune beating | Multi-harmonic estimator (below), 0.15–25 Hz |
| `modulation.amp_lfo.hz` (`tremolo`), `.depth_db` | Hz, dB | An amplitude LFO: every harmonic moves at the same rate | The same estimator's same-rate search |
| `loudness.lufs_i` | LUFS | Integrated loudness | `ebur128` (BS.1770-4); ungated whole-clip fallback for very short sounds; `None` below −70 LUFS |
| `loudness.peak_dbfs` | dBFS | Highest sample | May exceed 0 |
| `loudness.true_peak_dbtp` | dBTP | Highest inter-sample level | `ebur128` true peak (4× polyphase below 96 kHz), never below `peak_dbfs` |
| `loudness.clipped_pct`, `loudness.clipping` | %, bool | Samples at or beyond full scale | Share with \|x\| ≥ 1.0; `clipping` = share > 0 |

`envelope.beat_corrected` is false only when a beat was found but could not be removed (fewer than one period in the held part); `envelope.beat_periods` says how many periods were fitted.

### Beat-corrected envelope (report 03 §4.1)

1. `E` = RMS over 8 ms every 1 ms. Fit region: from the first time `E ≥ 0.3·max E` (+20 ms) to note-off − 15 ms (or the end).
2. Beat rate r from the estimator below. With no beat, use `E` as it is. With fewer than 1.0 periods in the region, use `E` and set `beat_corrected = false`.
3. Least squares over the region, every 2nd ms, points within 30 dB of the peak: `ln E(t) ≈ Σ_{p≤deg} c_p x^p + Σ_{k≤K} (a_k cos 2πkrt + b_k sin 2πkrt)`, x in −1..1; deg = 3 with ≥ 3 periods, 2 with ≥ 1.5, else 1; K = 6 with ≥ 2 periods, else 2. **Then 4 Huber refits** (weights `min(1, k/|residual|)` with k = 1.5 × the residuals' MAD × 1.4826; Huber 1964): a sharp decay knee that the polynomial trend cannot follow stops steering the beat pattern. This is new in this port (see the accuracy table).
4. Divide the periodic part `P(t)` out of the whole envelope (it is phase-locked to note-on, so it extends to attack and release), at most 20 dB; where `P < −0.7` (a null deeper than 6 dB) drop the point and bridge by linear interpolation of the log level.
5. Read the stages above on the corrected curve. The peak is the maximum of a 5 ms moving average (robust to a single spike).

Smoothing over a beat period was measured and rejected in the research: a boxcar reaches 23 % invariance and smears transients by a whole period; a crest (max-hold) envelope moves rising edges earlier by half its window.

### Beating and tremolo (report 03 §4.2)

1. Stretch: from the first time `E ≥ 0.5·max E` to note-off, or to where the level stays more than 30 dB down (a 400 ms gap ends it); at least 0.5 s.
2. For harmonics h = 1…12 with h·f0 < min(0.45·sr, 10 kHz): demodulate at h·f0 with a phasor recurrence (renormalised every 1,024 samples), boxcar of 4 periods of f0 (nulls the other harmonics), decimate to 500 Hz. Drop harmonics more than 30 dB below the strongest.
3. Per harmonic: divide out a cubic fit of the log magnitude; Hann, zero-pad ×8, real FFT; scale to the relative wobble amplitude `M_h(f)`.
4. `S(r)` = weight-averaged `M_h(h·r)` over usable harmonics (h·r·T ≥ 2.5 cycles, h·r < f0/8, h·r ≤ 60 Hz), r from 0.15 to 25 Hz in 0.2 % steps; take the maximum, prefer 2r or 3r if they score ≥ 0.85 of it, refine on a ±2 % grid.
5. Accept only if the best harmonic's wobble is ≥ 0.35, `S(r)` ≥ 6× its median over the band, at least one usable harmonic peaks within 3 % of h·r (two when r·T < 1), and it is not a tremolo: on harmonics ≥ 2 the reading at r itself is weaker than at h·r. **New here:** when h·r is outside the demodulator band for every harmonic ≥ 2 (a fast rate on a low note), the same-rate reading of those harmonics is compared with harmonic 1's instead (half of it or more means tremolo).
6. Tremolo (new here; report 03 only rejected it): the same-rate score `Σ w·M_h(r)`, accepted when it is ≥ 0.1 (about 1.7 dB peak to trough), ≥ 6× its median, every harmonic (at least two) peaks within 3 % of r, and the same-rate reading on harmonics ≥ 2 beats the h-scaled one. Reported only when no beating was accepted.

### Note-off inference (report 03 §F5)

`infer_note_off(x, sr)`: the latest knee where a held level (slope above −6 dB/s over ≥ 100 ms) turns into a fall ≥ 30 dB/s steeper that continues for 15 dB, both sides straight in dB within 1.5 dB RMS. `None` for sounds that only decay. The CLI's precedence is `--perf`, then the WAV's `enth` chunk, then this.

## Expectations and the checker

Grammar (one item per comma or semicolon; commas inside `[...]` belong to the item; features by canonical path or alias, case-insensitive):

```text
item    := feature op value [tol] | feature "in" "[" value "," value "]" | feature ("==" | "!=") "null"
op      := "<" | "<=" | ">" | ">=" | "=" | "==" | "~" | "!=" (true/false descriptors only)
value   := number with an optional unit (2kHz, 10ms, 0.5s, -6dB, -14LUFS, 30%, 12ct, +2st), a note for pitch (C3, A#4), true/false
tol     := ("±" | "+-") (value | number "%")      after "=", "==" or "~" only
```

- `=` and `~` without a tolerance use the feature's default: 25 ct for `pitch`, 50 ct for `pitch.note` (the same note), 10 % for Hz and ms features, 2 dB for dB features, 1.5 LU for loudness.
- Pitch with `=` or `~` is judged in cents from the target note, so its margin and noise are in cents. `pitch in [C3, D3]` and `pitch > 200Hz` are judged in Hz.
- Every item is echoed in canonical form with the canonical path: `brightness>2kHz` → `spectrum.centroid_hz > 2000`, `pitch=C3±20ct` → `pitch.f0_hz = C3 ± 20ct`, `beating in [3Hz, 9Hz]` → `detune.beating_hz in [3, 9]`. Canonical text parses back to the same item (a property test).
- Errors are typed: `bad_expect` (usage, exit 2 in the CLI) or `unknown_feature` (validation, exit 3) with `did_you_mean` and a `hint`.

Per item the checker returns `pass`, `got` (the feature's unit), `compared` (the cents error for pitch against a note), `unit`, `margin` (signed distance to the nearest edge, positive inside), `margin_pct` (Hz and ms features), `noise`, `thin`, `unstable`, `lower_bound` and `estimated`:

- **Noise** is the largest |change| of the compared value over the nuisance re-renders the caller passes in (`check(items, &main, &nuisance)` with descriptors, or `check_signals` with signals). Report 03's nuisance set: a common phase shift of the oscillators always, a different inter-oscillator phase only when they are detuned (at 0 ct the phase difference is a timbre change: it nearly cancelled fundamentals and YIN jumped an octave), and another seed for any per-voice randomness. Rendering is the caller's job.
- **Thin**: |margin| ≤ 2 × noise, or `unstable` (present in some renders, absent in others).
- **Lower bound**: a truncated duration can only pass `>` items. **Estimated**: an extrapolated release, or an envelope item with an uncorrected slow beat.
- For the hint solver: `Expect::shortfall` (0 when passing; nepers for Hz and ms features, the unit otherwise; infinite when absent), `Expect::interval`, `Expect::log_scale`, `Feature::hint_margin` (how far past a threshold to aim), `Descriptors::get`.

## Features and metric v1

`FeatureExtractor::extract` scales a clip to −23 LUFS, then computes natural-log mel magnitudes at FFT 2048, 1024 and 512 (hop n/2, Hann, 64 triangular bands, 30 Hz–16 kHz, `2595·log10(1 + f/700)`), a 10 ms RMS envelope in dB and its ±50 ms crest, an energy-weighted centroid track (log2 Hz, mid resolution), MFCC c0–c12 (DCT-II of the log-mel) and optionally a YIN f0 track. `distance::metric_v1` (`METRIC_V1 = "logmel-l1/1"`, provisional, ADR-0007) is the mean absolute log-mel difference averaged over the three resolutions, 1e3 when either clip is silent; `components` adds the envelope, centroid, MFCC and f0 distances beside it.

## Determinism and the scalar FFT path

`rustfft` chooses AVX, SSE, NEON or scalar kernels at run time, and they round differently. Everything that can decide pass/fail runs on `FftPath::Scalar` (the default): `rustfft`'s `FftPlannerScalar` for a half-length complex FFT, plus our own even-length real-FFT split whose twiddles come from `libm` in f64 (the textbook packing of Sorensen et al., "Real-valued fast Fourier transform algorithms", IEEE TASSP 35(6), 1987). `FftPath::Dispatched` uses `realfft`'s planner for callers that accept a tolerance. All other transcendentals go through `libm`; sums run in a fixed order.

Measured 2026-10-19 (`tests/determinism.rs`, pinned hashes): raw scalar real FFTs (lengths 512, 1024, 2048, 8192 and 1000, forward and inverse), the unnormalised features and every descriptor except LUFS and true peak give **the same bits on native x86_64 (debug, release and `-C target-cpu=native`, an AVX2/FMA machine) and on wasm32-wasip1 under wasmtime 48.0.6 (debug, release and `+simd128`)**. The dispatched path differs natively (AVX2); on wasm it equals the scalar one because `rustfft`'s `wasm_simd` feature is off. `rustfft` computes its own twiddles with `std`'s f64 `cos`/`sin` and rounds them to f32; that is the one outside input, which is why the pinned test exists. `ebur128` (LUFS, true peak) uses `std` maths for its filter coefficients and logarithms, so those two agree across platforms within a tolerance, not by construction; they happened to match exactly in the run above. Check on wasm with:

```text
CARGO_TARGET_WASM32_WASIP1_RUNNER=/path/to/wasmtime cargo test -p enth-analysis --target wasm32-wasip1 --test determinism -- --nocapture
```

## Cost

`examples/analysis_cost.rs`, release build, i7-9750H (AVX2). Instructions per call from two callgrind runs with different repetition counts (valgrind 3.22); wall-clock is best of 5 at load 8.5–10, so an upper bound and information only.

| Workload | Scalar | Dispatched (AVX2) | Scalar / dispatched |
| --- | --- | --- | --- |
| One real FFT, 2048 points | 108,300 instr, 11.1 µs | 32,700 instr, 3.6 µs | 3.3× (wall 3.1×) |
| Features of a 2 s clip (loudness + 3 log-mel resolutions + MFCC) | 62.6 M instr, 8.3 ms | 43.3 M instr, 6.2 ms | 1.45× (1.33×) |
| `describe` of a 3 s clip (all descriptors) | 312.7 M instr, 45.5 ms | 206.5 M instr, 31.8 ms | 1.51× (1.43×) |

Inside the scalar `describe`: YIN 38 %, the beating estimator 24 %, the envelope fit 12 % (its least squares 9 %), the centroid STFT 11 %, `ebur128` 8 % (true peak 4 %), noisiness 5 %. A hint call of 16 evaluations therefore spends about 0.7 s single-threaded in `describe` on 3 s clips; `Describer::describe_some(.., Want::for_features(..))` skips spectrum, noisiness and loudness when the list does not need them. YIN on loud frames only, or a hop of 1024, are the obvious savings if that ever matters.

## Measured accuracy

Synthetic signals and the toy voice of `test_signals` (as in the research); release build; raw outputs in `~/code/enth-scratch/p1a/a/sweeps-final.txt`.

| Descriptor | Result | Where |
| --- | --- | --- |
| Pitch | Sines 65–1047 Hz within 3 ct; toy saw, square and triangle C2–C6, worst error 0.33 ct (spike: 0.4) | `pitch` unit tests, `audit::pitch_and_brightness_table` |
| Brightness | A sine reads its own frequency within 25 Hz; 1 kHz + 3 kHz at 1:1 and 1:0.5 read 2,000 and 1,667 Hz within 30 Hz; cutoff 100→12,800 Hz reads 333, 355, 457, 650, 975, 1,470, 2,161, 3,056 Hz (monotone; spike 375/655/1,471/3,057 at 200/800/3,200/12,800); saw > square > triangle | unit tests, `audit` |
| Attack | A linear ramp of 30, 100 or 200 ms reads 0.90× its length (1 % → −1 dB of a ramp is 0.88×; the 8 ms window adds the rest); toy 2/10/100/500/1000 ms reads 4/10/88/426/846 ms | `envelope` tests, calibration table |
| Sustain | Toy 0/−3/−6/−12/−20/−45 dB read −0.1/−2.9/−5.7/−11.6/−19.5/−44.5 (decay 300 ms). After a decay spike with a time constant under 30 ms the 8 ms window and 5 ms robust peak blunt the peak, so the sustain reads up to 1.2 dB high (−20 → −18.8) | calibration table, `audit` |
| Release | 1.15 × the toy's release time (its −40 dB point) within 5 %; an ADSR sine's −40 dB time within 25 ms (461 ms for 460); an extrapolated release of a pure exponential within 0.1 % (2,301 ms for 2,303) | `audit`, `envelope` tests |
| Beating | Within 3 % of theory on **333 of 378** grid cells (C2…C6 × 3–50 ct × saw/square/triangle × held/bell/filter-envelope), wrong on 4 (all within 3.3 %), absent on 41 (beats below 0.25 Hz, or the 30 Hz of 50 ct at C6). The spike's estimator: 198 / 4 / 176. **0 of 400** detune-0 random patches report beating or tremolo | `sweeps::beat_accuracy_sweep`, `false_positive_sweep` |
| Tremolo | 72 of 72 grid cases (110–880 Hz × 2–8 Hz × depth 0.3–0.8) read within 3 %, none as beating; the 4 tremolo cases of report 03 read 5.0/2.99/5.98/4.03 Hz with no beat, and its 20 ct control reads 2.54 Hz beating, no tremolo; 0 tremolo reports on the 378 detuned grid cells | `sweeps::tremolo_sweep` |
| Clipping | `clipped_pct` exact; a +6 dB sine reads +6.02 dBFS; a fs/4 sine sampled at 45° reads 3.01 dB more true peak than sample peak (within 0.3 dB); report 03's g5 step-2 patch reads +6.73 dBFS with 3,192 samples over full scale (both as in the research); its centroid reads 2,994 Hz at 0 dB and at −12 dB master on the float render, and 3,712 Hz once clamped as a 16-bit file would be (the research: 2,995 and 3,712) | `loudness` tests, `sweeps::clipping_bends_brightness_and_master_gain_undoes_it` |
| Loudness | 997 Hz sine at amplitude 0.1 reads −23.01 ± 0.05 LUFS | `loudness` tests |
| Noisiness | Sine < 0.01; white noise > 0.9 | unit tests |
| Note-off inference | Of 143 random patches still sounding at note-off, 132 found, 110 within 30 ms; median error 3 ms, p10 −117 ms (an earlier decay knee), p90 9 ms; no knee on 37 of 57 already-decayed sounds (the same as the research) | `sweeps::note_off_inference_sweep` |

**Beat invariance** (report 03 §F2): 300 random toy patches (wave, note A2–A5, cutoff, resonance, filter envelope, ADSR; 3 s clip, 2 s hold), each detuned by 5, 8, 12, 20, 30 or 45 ct, against its detune-0 twin. Tolerances: attack within max(5 ms, 25 %), sustain within 1.5 dB (twin above −40 dB), decay and release within 20 %.

| Envelope | Attack ok | Sustain ok (p50 / p90 error) | Decay ok | Release ok | All four ok |
| --- | --- | --- | --- | --- | --- |
| Raw 8 ms RMS (the spike) | 72.3 % | 26.2 % (3.0 / 7.4 dB) | 40.2 % | 93.4 % | 19.0 % |
| Fit as in report 03 (before the Huber refits) | 94.7 % | 81.4 % (0.50 / 2.61 dB) | 73.7 % | 96.9 % | 65.3 % (report: 64 %) |
| **Fit with Huber refits (shipped)** | **95.3 %** | **88.6 % (0.27 / 1.62 dB)** | **79.4 %** | **97.3 %** | **74.0 %** |
| Shipped, on a held-out seed (`invariance_sweep_holdout`) | 94.7 % | 85.5 % (0.30 / 1.91 dB) | 78.4 % | 99.6 % | 71.3 % (67.0 % before the refits) |

A beat was found on 242 of the 300 detuned patches; 14 were flagged `beat_corrected = false`.

**The Phase 0 trial re-judged** (`audit::trial_rejudged`, report 03 §F1), with the two nuisance renders: g4 run 3 fails `attack>8ms` (reads 6 ms, programmed 6 ms; the spike read 38 ms), g5 run 1 fails `sustain>-4dB` (reads −4.50 dB, programmed −6; the spike read −2.0), g4 runs 1–2 pass `attack>8ms` at 9 ms with noise 1–2 ms (thin), and g5 run 3's `sustain>-4dB` reads −4.04 dB with noise 0.07 dB: a thin fail here, a thin pass at −4.0 in the research, which measured the 16-bit file. 12 of 15 pass (the research: 13), and every failure other than the two real ones is thin.

## Limits

- **Very slow beats are flagged, not corrected.** With fewer than one beat period in the held part (5 ct below about 250 Hz with a 2 s hold), the envelope is used raw and `beat_corrected` is false; sustain may then be off by a few dB. Beats below about 0.25 Hz (3 ct at C2/C3) are not even detected, so they are neither flagged nor corrected. An idea from report 03, not built: estimate the fundamental's beat phase from neighbouring harmonics.
- **Remaining envelope failures** (26 % of the invariance sweep): decays that settle within the residual ripple of a shallow sustain (decay is ill-conditioned when the sustain is within a few dB of the peak), triangle waves (near-sinusoidal partials beat to deep nulls), filter envelopes that change the spectral balance during the beat (the pattern is then not periodic), and short decaying sounds where no beat can be measured (attack then reads on the raw envelope).
- **Quiet sustains** below −30 dB re peak are not used for the beat estimate, so a beat there is not removed (calibration: −45 dB reads −46.0 at 25 ct against −44.5 at 0 ct).
- **Fast decay spikes** read the sustain up to about 1.2 dB high (the 8 ms window and the 5 ms robust peak blunt the peak). Definitional, not beat-induced.
- **Toy voice only.** Two equal oscillators, one voice, mono. Untested: unequal oscillator levels, three or more oscillators or unison (several beat rates, a non-periodic pattern), stereo, LFO-modulated sounds, noise, effects, and detune combined with tremolo. **Rerun the sweeps on the real engine once S2a lands** (`docs/phase-1a.md`, lane A).
- **Monophonic descriptors.** One note per analysis; chords and overlapping notes are out of scope for P1a.
- `ebur128`'s LUFS and true peak are tolerance-equal across platforms (see Determinism). Margins plus `noise` cover this for pass/fail.
- Debug-build cost: `describe` of a 3 s clip takes about 1 s unoptimised (the root profile optimises only `enth-dsp` and `enth-engine`), so the gate's fast tests use short clips and the sweeps run in release.

## Reproduce

```text
cargo test -p enth-analysis                                                         # unit, audit, fast sweep cases, determinism, fuzz-lite
cargo test --release -p enth-analysis -- --ignored --nocapture --test-threads 1      # all sweeps and tables (about 3 min)
cargo build --release -p enth-analysis --example analysis_cost && target/release/examples/analysis_cost
valgrind --tool=callgrind target/release/examples/analysis_cost describe scalar 4    # and 8; the difference / 4 = per call
```

## Sources

- A. de Cheveigné and H. Kawahara, "YIN, a fundamental frequency estimator for speech and music", JASA 111(4), 2002.
- ITU-R BS.1770-4 and EBU R 128 (loudness, true peak), through the `ebur128` crate 0.1.10 (MIT).
- H. V. Sorensen, D. L. Jones, M. T. Heideman and C. S. Burrus, "Real-valued fast Fourier transform algorithms", IEEE TASSP 35(6), 1987 (the real-FFT split).
- P. J. Huber, "Robust estimation of a location parameter", Annals of Mathematical Statistics 35(1), 1964 (the refits).
- D. O'Shaughnessy, *Speech Communication*, 1987 (the mel formula `2595·log10(1 + f/700)`).
- P1a research report 03 (`_local/sessions/2026-10-19-p1a-headless-core/03-cli-descriptors.md`) §4 and §F1–F6; `docs/spikes/agent-probe.md`; `docs/spikes/metric.md`; ADR-0007.
