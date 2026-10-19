# Analysis (`enth-analysis`)

- Date: 2026-10-19. P1a lane A; 2026-10-20, P1b lane EV (the G1 eval findings: tonal pitch, the low-range pitch fallback, filter sweeps, the tremolo-corrected envelope, boolean thinness, the filter-LFO check in the beat estimator). Crate: `crates/enth-analysis` (L3; depends on `libm`, `rustfft`, `realfft`, `ebur128`, `serde`; no I/O, clocks, threads or global RNG; builds for wasm32).
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
| `pitch` | YIN tracker and its summary (`Pitch`); the tonal pitch of spectral peaks (`Tonal`, `tonal_pitch`) |
| `sweep` | Brightness sweeps from the centroid over time: filter envelopes and filter LFOs (`FilterSweep`) |
| `loudness` | `LoudnessMeter`: LUFS, sample peak, true peak, clipping (`Level`) |
| `features`, `distance` | Multi-resolution log-mel, envelope, centroid track, MFCC, f0 track (`FeatureExtractor`); metric v1 and its components |
| `fft` | `FftPath::{Scalar, Dispatched}` and the real-FFT planner |
| `units` | Descriptor units and note names |
| `test_signals` | Signals with known answers: sines, polyBLEP oscillators, detuned pairs, ADSR, tremolo, noise, the 10-parameter toy voice (with an optional cutoff LFO, `render_toy_lfo`) |

## Descriptors

All on the float render (never a clamped or quantised file), at any sample rate from 8 to 192 kHz (48 kHz everywhere in P1a). A signal must be finite and at least 20 ms long. Below −80 dBFS sample peak a clip is `silent` and only `loudness` is filled. `NoteTiming { on_s, off_s }` gives the performance: decay, sustain and release need a note-off; without one they are `None` (use `infer_note_off` for a foreign WAV, then pass its result).

| Path (alias) | Unit | Meaning (the one-line `Feature::meaning`, abridged) | How |
| --- | --- | --- | --- |
| `pitch.f0_hz` (`pitch`, `pitch.note`) | Hz | Fundamental measured from the audio: the period of the waveform | YIN, frame 2048, hop 512, 50–2000 Hz, threshold 0.15; median over voiced frames whose centre is within 40 dB of the envelope peak. **When that finds no pitch**, the same with a 4096-sample frame and a 25 Hz floor (below) |
| `tonal.f0_hz` (`tonal_pitch`, `tonal.note`) | Hz | Pitch of the tonal part, found under noise | Harmonic matching of the steady peaks of the noisiness spectrum (below) |
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
| `modulation.filter_sweep.hz` (`filter_lfo`), `.depth_oct` (`filter_sweep`), `.ms` (`sweep_ms`) | Hz, oct, ms | A brightness sweep: a filter LFO (`kind: lfo`, `hz`) or a filter envelope (`kind: env`, `dir`, `ms`) | The brightness STFT's centroid over time ([`sweep`](#brightness-sweeps-2026-10-20)) |
| `loudness.lufs_i` | LUFS | Integrated loudness | `ebur128` (BS.1770-4); ungated whole-clip fallback for very short sounds; `None` below −70 LUFS |
| `loudness.peak_dbfs` | dBFS | Highest sample | May exceed 0 |
| `loudness.true_peak_dbtp` | dBTP | Highest inter-sample level | `ebur128` true peak (4× polyphase below 96 kHz), never below `peak_dbfs` |
| `loudness.clipped_pct`, `loudness.clipping` | %, bool | Samples at or beyond full scale | Share with \|x\| ≥ 1.0; `clipping` = share > 0 |

`envelope.beat_corrected` is false only when a beat (or, without one, a tremolo) was found but could not be removed (fewer than one period in the held part); `envelope.beat_periods` says how many periods were fitted.

### Beat-corrected envelope (report 03 §4.1)

1. `E` = RMS over 8 ms every 1 ms. Fit region: from the first time `E ≥ 0.3·max E` (+20 ms) to note-off − 15 ms (or the end).
2. Beat rate r from the estimator below; **with no beat, the tremolo rate** (2026-10-20: a tremolo is a periodic level wobble like a beat, and read raw it put the attack on its first crest and the sustain wherever note-off fell in its cycle; the `filter-wobble` witness read attack 307 ms and sustain +4.9 dB, now 15 ms and −0.8 dB). With neither, use `E` as it is. With fewer than 1.0 periods in the region, use `E` and set `beat_corrected = false`. The rate never depends on `Want` (the tremolo is always measured), so `describe_some` and `describe` read the same envelope.
3. Least squares over the region, every 2nd ms, points within 30 dB of the peak: `ln E(t) ≈ Σ_{p≤deg} c_p x^p + Σ_{k≤K} (a_k cos 2πkrt + b_k sin 2πkrt)`, x in −1..1; deg = 3 with ≥ 3 periods, 2 with ≥ 1.5, else 1; K = 6 with ≥ 2 periods, else 2. **Then 4 Huber refits** (weights `min(1, k/|residual|)` with k = 1.5 × the residuals' MAD × 1.4826; Huber 1964): a sharp decay knee that the polynomial trend cannot follow stops steering the beat pattern. This is new in this port (see the accuracy table).
4. Divide the periodic part `P(t)` out of the whole envelope (it is phase-locked to note-on, so it extends to attack and release), at most 20 dB; where `P < −0.7` (a null deeper than 6 dB) drop the point and bridge by linear interpolation of the log level.
5. Read the stages above on the corrected curve. The peak is the maximum of a 5 ms moving average (robust to a single spike).

Smoothing over a beat period was measured and rejected in the research: a boxcar reaches 23 % invariance and smears transients by a whole period; a crest (max-hold) envelope moves rising edges earlier by half its window.

### Beating and tremolo (report 03 §4.2)

1. Stretch: from the first time `E ≥ 0.5·max E` to note-off, or to where the level stays more than 30 dB down (a 400 ms gap ends it); at least 0.5 s.
2. For harmonics h = 1…12 with h·f0 < min(0.45·sr, 10 kHz): demodulate at h·f0 with a phasor recurrence (renormalised every 1,024 samples), boxcar of 4 periods of f0 (nulls the other harmonics), decimate to 500 Hz. Drop harmonics more than 30 dB below the strongest.
3. Per harmonic: divide out a cubic fit of the log magnitude; Hann, zero-pad ×8, real FFT; scale to the relative wobble amplitude `M_h(f)`.
4. `S(r)` = weight-averaged `M_h(h·r)` over usable harmonics (h·r·T ≥ 2.5 cycles, h·r < f0/8, h·r ≤ 60 Hz), r from 0.15 to 25 Hz in 0.2 % steps; take the maximum, prefer 2r or 3r if they score ≥ 0.85 of it, refine on a ±2 % grid.
5. Accept only if the best harmonic's wobble is ≥ 0.35, `S(r)` ≥ 6× its median over the band, at least one usable harmonic peaks within 3 % of h·r (two when r·T < 1), **the harmonics with a real wobble at h·r (at least 0.3 of the best one's) hold at least half of the usable harmonics' weight** (2026-10-20: detune moves every harmonic; a filter LFO at f moves only the harmonics near its cutoff, at f and its multiples, which line up with h·r for r = f/k on every k-th harmonic, so a 3 Hz filter LFO read as 1 Hz "beating" through harmonics 3, 6 and 9; the beat grid below is unchanged by the check), and it is not a tremolo: on harmonics ≥ 2 the reading at r itself is weaker than at h·r. **New here:** when h·r is outside the demodulator band for every harmonic ≥ 2 (a fast rate on a low note), the same-rate reading of those harmonics is compared with harmonic 1's instead (half of it or more means tremolo).
6. Tremolo (new here; report 03 only rejected it): the same-rate score `Σ w·M_h(r)`, accepted when it is ≥ 0.1 (about 1.7 dB peak to trough), ≥ 6× its median, every harmonic (at least two) peaks within 3 % of r, and the same-rate reading on harmonics ≥ 2 beats the h-scaled one. Reported only when no beating was accepted.

### The low-range pitch fallback (2026-10-20)

The main YIN's lag range stops at half its 2048-sample frame, 46.9 Hz. A sub oscillator an octave below a C2 is at 32.7 Hz: the waveform's period is then outside the range, the difference function has no dip below the threshold at the C2 lag (the sub sine is inverted there), and pitch read as nothing (`bass_sub` at C2, eval finding 3). When the main tracker finds no pitch, the same YIN runs again on a 4096-sample frame, 25–2000 Hz (lag up to 1,920 samples, integration window 2,176), with the same loud-frame mask and summary. It searches from the shortest lag, so a tone above 47 Hz that the main frame missed (noise, a short sound) is not read at a subharmonic; it only adds periods the main frame could not hold. `bass_sub` at C2 now reads C1 (32.70 Hz), the period of its waveform and the note a tuner shows; a weak sub would leave the C2 series to the tonal pitch (below). Every reading the main frame finds is unchanged (the pinned descriptor hash holds). Cost: one more YIN pass, only on sounds without a main pitch.

### Tonal pitch (2026-10-20)

YIN needs the waveform to repeat: with white noise added to a tone, the difference function's dip at the period stays above the 0.15 threshold once the noise is a fifth or so of the power, and pitch goes (eval finding 7: noisiness above 0.2–0.4). The tonal pitch reads the steady part of the spectrum instead, so a snare-like goal can ask for `noisiness in [0.4, 0.85], tonal_pitch ~ A3`.

1. The Welch spectrum of the held part that `noise_ratio` uses (8192 points, Hann, 50 % overlap). Peaks by the same rule: local maxima at least 15 dB above the median of their ±64 bins, over 20 Hz–16 kHz (the noisiness band starts at 40 Hz; the sub of a C1 is at 32.7 Hz). Each peak's frequency is the log-parabolic (Gaussian) interpolation of its bin and neighbours; its power is the peak bin and 3 bins either side, each bin counted once.
2. `share` = the peaks' power over the band's. Under 1 % (−20 dB), there is no tonal part: `None`.
3. The strongest 32 peaks within 40 dB of the strongest. Candidates: each of the 8 strongest divided by 1..16, inside 25–2000 Hz. A candidate scores the power of the peaks within tolerance of its harmonics 1..16 (tolerance max(0.6 bin, 1.2 % of the harmonic), at most a quarter of the candidate).
4. The **highest** candidate within 90 % of the best score wins (a series and its subharmonic explain the same peaks; the subharmonic never wins a tie), refined by a power-weighted least-squares fit through its matched peaks (`f0 = Σ P·h·f / Σ P·h²`). `harmonicity` = the matched share of the peaks' power; under 0.5, `None`.

A missing fundamental is restored (harmonics 2, 3, 4 of 100 Hz read 100 Hz), as the ear does; a strong sub an octave down is the fundamental, a sub under 10 % of the peaks' power is not. Judged in cents like `pitch` (`tonal_pitch ~ A3` is ±25 ct, `tonal.note ~ A3` ±50 ct). Needs `Want::noise`.

### Brightness sweeps (2026-10-20)

`modulation.filter_sweep` (module `sweep`) reads the brightness STFT (Hann 2048, hop 512, 93.75 frames per second at 48 kHz) as a track of `log2` centroid, in octaves, over frames within 40 dB of the loudest (quieter frames bridged). A level change does not move the centroid, so a tremolo never reads as a sweep, and a filter LFO reads as one whether or not it moves the level enough to be a tremolo (before, a filter wobble showed only as a tremolo, and only when deep: eval finding 6).

- **LFO** (`kind: lfo`, `hz`, `depth_oct` the peak-to-trough swing), over the held part (end of the attack to note-off), at least 0.5 s: remove a cubic trend, take the spectrum (Hann, zero-padded ×8, scaled to a sinusoid's amplitude) between max(0.1 Hz, 2 cycles in the stretch) and min(20 Hz, a quarter of the frame rate); the peak must be interior and 6× the band's median. Rate and swing are refined by fitting the cubic trend and a sinusoid jointly on a grid within one bin of the peak (the spectrum alone read 0.75 Hz LFOs 4 % low with 2.5 cycles). Accepted with a swing of at least 0.2 octave and a rate not within 3 % (or one cycle over the stretch) of 1–4 times the detune beat rate: beating swings the centroid at its rate and first multiples.
- **Env** (`kind: env`, `dir` up or down, `ms`, `depth_oct` the one-way change), over the audible part of the held note, when no LFO was found: the start level (mean of the first 5 frames, about 53 ms; over one beat period when the oscillators beat) against the end level (median of the last quarter); a change of at least 0.5 octave is a sweep, and `ms` is the time from the onset until 90 % of it is done.

Features: `modulation.filter_sweep.hz` (`filter_lfo`, `sweep_rate`), `.depth_oct` (`filter_sweep`, `modulation.filter_sweep`, `sweep_depth`; unit `oct`, which also takes `st`), `.ms` (`sweep_ms`). Needs `Want::spectrum`.

### Booleans and `thin` (2026-10-20)

A true/false item (`clipping = false`) has a margin of 0 by construction, so the rule |margin| ≤ 2 × noise made every such item thin even when every render agreed (eval finding 5). A boolean is now thin only when a nuisance render disagrees with it (noise 1) or the reading comes and goes; its margin is still printed as 0.

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

2026-10-20 additions, not re-measured: the tonal pitch reuses the noisiness spectrum and the filter sweep the brightness STFT (a few hundred candidate scores and one small FFT plus a 41-point least-squares grid on a few hundred centroid frames); the low-range YIN is a second pass on a 4096 frame, run only on sounds where the main YIN finds no pitch (noise, sub-bass), which roughly doubles YIN's share for those.

Inside the scalar `describe`: YIN 38 %, the beating estimator 24 %, the envelope fit 12 % (its least squares 9 %), the centroid STFT 11 %, `ebur128` 8 % (true peak 4 %), noisiness 5 %. A hint call of 16 evaluations therefore spends about 0.7 s single-threaded in `describe` on 3 s clips; `Describer::describe_some(.., Want::for_features(..))` skips spectrum, noisiness and loudness when the list does not need them. YIN on loud frames only, or a hop of 1024, are the obvious savings if that ever matters.

## Measured accuracy

Synthetic signals and the toy voice of `test_signals` (as in the research); release build; raw outputs in `~/code/enth-scratch/p1a/a/sweeps-final.txt`.

| Descriptor | Result | Where |
| --- | --- | --- |
| Pitch | Sines 65–1047 Hz within 3 ct; toy saw, square and triangle C2–C6, worst error 0.33 ct (spike: 0.4). A saw at C2 over a sine at C1 reads C1 (−0.05 ct); a C1 saw reads 32.70 Hz; white noise reads none | `pitch` unit tests, `audit::pitch_and_brightness_table`, `findings::a_sub_octave_reads_as_the_sub` |
| Tonal pitch | A3 triangle, saw and square under white noise (6 levels each): within 0.2 ct up to noisiness 0.96, where YIN's pitch went at about 0.23 (gone in all 9+ cases at 0.4 and above); `share` within 0.1 of 1 − noisiness. Series tests: saw, odd-harmonic, lone sine, missing fundamental and a strong sub all read their fundamental within 1 ct, a weak sub does not win. The snare-crack witness reads A3 (−0.7 ct, noise 2.2 ct) at noisiness 0.62 | `findings::the_tonal_pitch_survives_noise_that_hides_pitch`, `pitch` unit tests |
| Filter sweep | Filter LFO grid (notes 110–880 Hz × cutoff 600/2000 Hz × 0.75–12 Hz × ±0.25–4 oct, 240 cases, 3.5 s held): 221 found, 217 within 3 %. Misses: a cutoff below the fundamental (880 Hz note, 600 Hz cutoff: every harmonic is above the cutoff, so the sweep only scales the level), and deep 0.75 Hz sweeps (2.5 cycles, a 5 % low reading or none). Swing read for a ±0.25/0.5/1 octave cutoff LFO: median 0.31/0.62/1.21 octaves of centroid. **No** sweep on the 72 tremolo cases, **no** filter LFO on the 378 detuned grid cells or the 400 detune-0 random patches (28 of those read a one-shot sweep, from their filter envelopes). A one-shot toy pluck (cutoff 500 Hz + 4 oct, 300 ms) reads `env down`; without the envelope, nothing | `sweeps::filter_lfo_sweep`, `sweeps::beat_accuracy_sweep`, `sweeps::false_positive_sweep`, `sweeps::tremolo_sweep`, `findings` |
| Envelope under tremolo | A 30 ms attack to full sustain under a 4 Hz, 6 dB tremolo starting and ending in a trough: attack within 5 ms and sustain within 1.5 dB of the twin without tremolo; read raw, the attack ran on to the first crest (more than 50 ms late) and the sustain read the trough (4 dB and more low) | `findings::tremolo_no_longer_bends_attack_and_sustain` |
| Brightness | A sine reads its own frequency within 25 Hz; 1 kHz + 3 kHz at 1:1 and 1:0.5 read 2,000 and 1,667 Hz within 30 Hz; cutoff 100→12,800 Hz reads 333, 355, 457, 650, 975, 1,470, 2,161, 3,056 Hz (monotone; spike 375/655/1,471/3,057 at 200/800/3,200/12,800); saw > square > triangle | unit tests, `audit` |
| Attack | A linear ramp of 30, 100 or 200 ms reads 0.90× its length (1 % → −1 dB of a ramp is 0.88×; the 8 ms window adds the rest); toy 2/10/100/500/1000 ms reads 4/10/88/426/846 ms | `envelope` tests, calibration table |
| Sustain | Toy 0/−3/−6/−12/−20/−45 dB read −0.1/−2.9/−5.7/−11.6/−19.5/−44.5 (decay 300 ms). After a decay spike with a time constant under 30 ms the 8 ms window and 5 ms robust peak blunt the peak, so the sustain reads up to 1.2 dB high (−20 → −18.8) | calibration table, `audit` |
| Release | 1.15 × the toy's release time (its −40 dB point) within 5 %; an ADSR sine's −40 dB time within 25 ms (461 ms for 460); an extrapolated release of a pure exponential within 0.1 % (2,301 ms for 2,303) | `audit`, `envelope` tests |
| Beating | (Unchanged by the 2026-10-20 filter-LFO check.) Within 3 % of theory on **333 of 378** grid cells (C2…C6 × 3–50 ct × saw/square/triangle × held/bell/filter-envelope), wrong on 4 (all within 3.3 %), absent on 41 (beats below 0.25 Hz, or the 30 Hz of 50 ct at C6). The spike's estimator: 198 / 4 / 176. **0 of 400** detune-0 random patches report beating or tremolo | `sweeps::beat_accuracy_sweep`, `false_positive_sweep` |
| Tremolo | 72 of 72 grid cases (110–880 Hz × 2–8 Hz × depth 0.3–0.8) read within 3 %, none as beating; the 4 tremolo cases of report 03 read 5.0/2.99/5.98/4.03 Hz with no beat, and its 20 ct control reads 2.54 Hz beating, no tremolo; 0 tremolo reports on the 378 detuned grid cells (all unchanged on 2026-10-20) | `sweeps::tremolo_sweep` |
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
- **Attack on low notes** (a goal-design note, 2026-10-19, still open): a 3 ms attack knob reads 15 ms at A1, 12 ms at C2, 5 ms at C3 and above, with 12/10/5 ms of nuisance noise (measured through the CLI on the real engine, 2026-10-20). A level envelope cannot resolve an onset much shorter than one period of the note (18 ms at A1), and the 8 ms window's ripple at the period moves the reading with the start phase. A period-length RMS window would remove the ripple (less noise) but not the period's worth of rise (about 0.8 of a period), and would change every envelope reading below 125 Hz; not done. Goals on low notes keep wide attack limits (`bass-buzzy` < 40 ms, `kick-thump` < 15 ms).
- **Pitch is the waveform's period.** A strong sub oscillator an octave down reads the octave below (a tuner's answer, and the eval's `bass_sub` case); `tonal_pitch` agrees, and only drops a sub under 10 % of the peaks' power. Below 25 Hz there is no pitch.
- **Tonal pitch** needs about 171 ms of held sound (the Welch frame), peaks 15 dB above their neighbourhood, and a mostly harmonic series: an inharmonic body (a drum membrane's modes) reads its strongest harmonic-looking series or nothing.
- **Filter sweeps:** a cyclic sweep needs two cycles in the held part (a 0.3 Hz LFO needs a 7 s hold) and is read 4–5 % low with 2.5 cycles; with fewer, it can read as a one-shot sweep. A filter LFO at the rate of a detune beat (or 2–4 times it) is not reported. A cutoff below every harmonic moves the level, not the balance: that is a tremolo, not a sweep. The one-shot start level is the mean of the first 53 ms, so a sweep faster than that reads smaller than the cutoff move.
- **Toy voice only.** Two equal oscillators, one voice, mono. Untested: unequal oscillator levels, three or more oscillators or unison (several beat rates, a non-periodic pattern), stereo, noise, effects, and detune combined with tremolo. (LFO-modulated sounds: the tremolo and filter-LFO grids above, 2026-10-20.) **Rerun the sweeps on the real engine once S2a lands** (`docs/phase-1a.md`, lane A).
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
