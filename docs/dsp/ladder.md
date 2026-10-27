# Ladder filter (`ladder`; `enth-dsp::ladder`, `enth-engine::modules::ladder`)

- **Status: implemented (S2a lane FLT, wave 3; last on the cut list, built after `comb` and `formant`).** Kind `ladder`, discriminant 17, section `voice_fx`, id prefix `ladder` (ladder1, ladder2…), rate voice. Fixed and persisted from F0m; still hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- Code: `crates/enth-dsp/src/ladder.rs` (the filter, the closed-form response, white-box tests), `crates/enth-engine/src/modules/ladder.rs` (the per-voice module, mirroring `svf`'s). Catalogue entry: `crates/enth-patch/src/catalogue/ladder.rs`. Engine-level acceptance tests: `crates/enth-engine/tests/flt.rs`. Bench: `crates/enth-engine/benches/ladder.rs`. Goldens: `tests/golden/cases/ladder-*.toml`.
- A four-pole ladder filter per voice (a zero-delay-feedback TPT Moog ladder with a saturator at its input): a 24 or 12 dB/octave low-pass, a band-pass or a high-pass from the stage taps, with the ladder's resonance, which thins the lows as it rises and rings on its own near the top; `mix` with the dry signal. It mirrors [svf.md](svf.md) (same keys, key-tracking rule, drive curve and per-sample coefficient ramps) so a patch can swap one for the other.
- Sources: V. Zavalishin, *The Art of VA Filter Design*, rev. 2.1.0 (Native Instruments, 2018), chapter "Moog ladder filter" (the one-pole stage, the feedback loop solved without a unit delay, the taps); research report 05 §5 for the choice of kind (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`). **Provenance: written from those equations; no code ported, no third-party source consulted, so no `THIRD_PARTY_NOTICES.md` entry.** The input saturator and the cutoff clamp are the svf's own (`svf::Drive`, `svf::MIN_CUTOFF_HZ`, `svf::MAX_CUTOFF_FRACTION`).

## Parameters (lane CAT, F1m)

The table lane FLT implements, generated from `crates/enth-patch/src/catalogue/ladder.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Mirrors `svf`'s table** where the meanings match (keys, units, ranges, defaults), so a patch can swap one for the other: the effective cutoff is `cutoff · 2^(keytrack · (pitch − 60) / 1200)`; `drive` is gain into the input saturator, level-compensated; `resonance` self-oscillates near the top (recorded below: from 95.2 %). `mode` taps the stages for 24 or 12 dB/octave low-pass, band-pass or high-pass; a diode ladder can be appended later.
- **Defaults:** a gentle useful setting, as a new `svf` is: a 24 dB low-pass at 2 kHz.

### `ladder` — Ladder filter (voice_fx, ids `ladder1`, `ladder2`…; 6 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mode` | none | `lp24`, `lp12`, `bp`, `hp` | enum | lp24 | auto | basic | The response: 24 or 12 dB/octave low-pass, band-pass or high-pass. |
| `cutoff` | hz | 20Hz … 20kHz | log | 2kHz | mod, auto | basic | The corner frequency: lower is darker for a low-pass; route an envelope or LFO here for sweeps and wobbles. |
| `resonance` | percent | 0% … 100% | linear | 10% | mod, auto | basic | Emphasis at the cutoff; near the top the ladder rings on its own (self-oscillation) and the lows thin out, as on a Moog. |
| `keytrack` | percent | 0% … 100% | linear | 0% | auto | intermediate | How much the cutoff follows the played note: 100 % moves it an octave per octave played. |
| `drive` | db | 0dB … 24dB | linear | 0dB | mod, auto | intermediate | Gain into the ladder's saturator, normalised back after: grit and a thicker, compressed tone. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged; lower values blend the dry signal back in. |

## The filter

```text
stage:   v = (x − s)·G;   y = v + s;   s = y + v                 G = g/(1 + g),  g = tan(π·fc/fs)     y = G·x + (1 − G)·s
loop:    y4 = G⁴·u + σ,   σ = (1 − G)·(G³·s1 + G²·s2 + G·s3 + s4),   u = x − k·y4
         ⇒  u = (x − k·σ) / (1 + k·G⁴)          one division, exact: no unit delay in the feedback
         u ← u / √(1 + u²/16)                    the loop limiter
taps:    y0 = u, y1 … y4          lp24 = y4;   lp12 = y2;   bp = y2 − 2·y3 + y4;   hp = y0 − 4·y1 + 6·y2 − 4·y3 + y4
out = (1 − mix)·dry + mix·tap
```

- **Four trapezoidal one-poles in series, the last fed back to the first.** Each stage is the TPT (trapezoidal) integrator of [svf.md](svf.md) as a one-pole low-pass; the loop is solved for the current sample, so the response is the bilinear transform of the analogue ladder `H = H1⁴ / (1 + k·H1⁴)` with every pole **pre-warped to the cutoff** (`tan`): the poles are where they are set up to 0.45·fs, and the self-oscillation is at the cutoff frequency exactly (below).
- **Resonance** is the feedback gain `k = 4.2 · resonance/100` (linear in `k`, as on a Moog). At the cutoff each stage is −3 dB and 45°, so the loop has 180° there and a gain of `k/4`: the response at the cutoff is `0.25 / (1 − k/4)` and the ladder rings from **`k = 4`, `resonance` 95.2 %**. The top of the knob (`k = 4.2`) is 5 % above that.

  | resonance | 0 % | 10 % (default) | 25 % | 50 % | 75 % | 90 % | 95 % |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | `k` | 0 | 0.42 | 1.05 | 2.1 | 3.15 | 3.78 | 3.99 |
  | passband (DC) gain `1/(1 + k)`, dB | 0.0 | −3.0 | −6.2 | −9.8 | −12.4 | −13.6 | −14.0 |
  | gain at the cutoff `0.25/(1 − k/4)`, dB | −12.0 | −11.1 | −9.4 | −5.6 | +1.4 | +13.2 | +40 |

  So **the passband falls as the resonance rises** (the lows thin out, which is the Moog sound and what CAT's text promises), by 3 dB at the default. There is no compensation for it; `drive` is the only level control in the module (below). A patch that wants the lows kept at high resonance adds a second signal path or gain after it.
- **The ring.** Above `k = 4` a linear loop grows without limit, so the summed signal `u` goes through a fixed soft limiter, `u / √(1 + u²/16)`, before the first stage: unity for small signals (−0.26 dB at `u` = 1), bounded at 4. The self-oscillation settles where the limiter's gain has fallen to `4/k`: a clean, steady sine at the cutoff of about −12 dBFS RMS (0.25) at `resonance` 100 %. Measured through the DSP core (`near_the_top_of_the_knob_the_ladder_rings_at_the_cutoff`, a noise burst in, 6 s later): the ring is at 100.00 / 439.98 / 1000.00 / 3999.99 / 10000.09 Hz for cutoffs of 100 / 440 / 1,000 / 4,000 / 10,000 Hz, **within 0.005 % of the cutoff** (acceptance: 2 %); RMS 0.250 (0.234 at 10 kHz), steady to four digits between the two halves of the window. Through the engine (`a_ladder_at_resonance_100_rings_at_its_cutoff`): 250.5 / 1000.5 / 4000.5 / 12000.0 Hz for cutoffs of 250 / 1,000 / 4,000 / 12,000 Hz (one bin of 1.46 Hz), RMS 0.20 after the amp envelope and pan. **The threshold:** at `resonance` 94 % (`k` = 3.95) a burst at 1 kHz has died to exactly zero after 10 s; at 96 % (`k` = 4.03) it rings at RMS 0.10 (`the_ring_starts_just_above_95_percent_and_dies_below`). The limiter is applied after the exact linear solve (the usual cheap treatment): while it is in its linear range the solve is exact; as it compresses, the loop is slightly weaker than the linear equations say, which only sets the ring's amplitude.
- **A ring needs something to start it.** From a zero state with a zero input the filter stays at exactly zero; any input (a note's attack is enough) starts it, and `start_voice` zeroes the state, so each note's ring is excited by that note. The ring lasts as long as the voice does: the amp envelope's release still ends it.
- **Drive** is the svf's input saturator (`svf::Drive`, an algebraic sigmoid `x/√(1 + a·x²)` with a make-up gain that keeps a full-level saw's loudness; see [svf.md](svf.md#drive)) in front of the loop, so it also reaches the limiter. At 0 dB it is skipped (bit-identical). The same 0–24 dB range and curve as the svf, so a patch that swaps one for the other keeps its drive.
- **Taps.** `lp12` is the second stage's output with the whole loop's feedback, so it resonates at the same `resonance` with the same passband gain `1/(1 + k)`, 12 dB/octave above the cutoff and, being tapped before two more poles, a peak at the cutoff twice (6 dB) that of `lp24`: `0.5/(1 − k/4)`. `bp` is `y2 − 2·y3 + y4` (two high-pass and two low-pass poles at the cutoff: a 12/12 band-pass), `hp` the binomial combination of the input and the four stages (a four-pole high-pass). Both resonate through the same loop and **self-oscillate at the same `resonance`**, at the cutoff. Their responses are the closed forms in `response_complex`.
- **Range of the cutoff:** 20 Hz … 0.45·fs after key tracking (`svf::Coeffs::new`'s clamp, so the module behaves like the svf at the ends); a NaN cutoff becomes 20 Hz, and `k` is clamped to 0 … 4.2.

### The slope

Four poles at the same frequency do not fall at 24 dB/octave from the corner; they approach it. From the closed form at a cutoff of 125 Hz (so the bilinear warp is negligible up to 2 kHz), resonance 0, and through the filter itself (a sine against the closed form within 0.05 dB):

| | at `fc` | 2 `fc` | 4 `fc` | 8 `fc` | 16 `fc` |
| --- | --- | --- | --- | --- | --- |
| gain, dB | −12.04 | −27.96 | −49.23 | −72.56 | −96.60 |
| drop over the octave below, dB | | 15.92 | 21.27 | 23.34 | 24.03 |

(At resonance 10 %: −11.08 / −27.92 / −49.24 / −72.57 / −96.60 and drops 16.84 / 21.32 / 23.33 / 24.03.) **The slope is within 1 dB of 24 from the third octave above the cutoff on** (4 `fc` → 8 `fc`: 23.3; then 24.0), **not at "1 and 2 octaves above the cutoff"** as the acceptance text says: the octave from `fc` to 2 `fc` falls 15.9 dB and the next 21.3, for any four equal poles at the cutoff, analogue or digital; no choice of `fc` convention changes the shape near the corner. The asymptote, relative to the passband, is reached to within 4.0 dB at 2 `fc`, 1.2 dB at 4 `fc` and 0.6 dB at 8 `fc`. Reported to the coordinator; the check is stated where it holds. Through the engine (`the_ladder_slope_through_the_engine_is_the_closed_forms`): 23.30 dB from 3.9 to 7.8 times the cutoff (a cutoff of 100 Hz, the sines of keys 67 and 79) and 21.19 dB from 2 to 3.9 times it (200 Hz). Above 8 kHz the bilinear transform bends the slope up (25.9 dB/octave from 4 to 8 kHz at a cutoff of 1 kHz), as it does for every TPT filter. `lp12` falls at 12 dB/octave, `hp` rises at 24 and `bp` at 12/12 far from the cutoff (`the_twelve_db_tap_and_the_band_pass_and_high_pass_slopes`).

### Key tracking

As `svf` ([svf.md](svf.md)): the effective cutoff is `cutoff · 2^(keytrack · (pitch − 60) / 1200)` octaves, with `pitch` the voice's current note pitch in semitones (so a glide moves it). A test (`the_cutoff_tracks_the_key_exactly_as_the_svf_does`) builds the svf's coefficients with the same rule for five cutoff/keytrack/key combinations and checks that the ladder's `G = g/(1 + g)` is `g/(1 + g)` of the svf's `g` to 1e-6; a second checks in Hz that key 72 at 100 % doubles a 1 kHz cutoff (the lp12's −6.02 dB point lands at 2 kHz).

### Zipper-free updates

As the svf: the module computes the coefficients for the *end* of the 32-frame segment, and `process_block` ramps `G`, `k` and the drive's `b` and `c` linearly per sample from the previous segment's values, plus `mix`. `G²`, `G³`, `G⁴` and the division are recomputed per sample only while ramping.

- The module keeps each voice's previous coefficients itself, so a retrigger or a smoother reset is a 32-frame glide, not a step; a fresh voice starts at its first segment's values (`from = to`).
- The four inputs (cutoff, resonance, drive, key-tracking term) are the cache key. When they are bit-identical to the previous segment's, nothing is recomputed: **a static ladder costs no transcendental**. A recompute is one `tanf` (plus `exp2f` for a nonzero key-tracking term and `expm1` for a nonzero drive).
- When the old and new coefficients are equal the ramp is skipped (adding exact zeros is bit-identical); with both drives off the saturator is skipped.
- `mix` 0 at both ends of a segment leaves `io` untouched: the input passes **bit for bit**, while the filter keeps running so that raising `mix` finds its state current. `start_voice` zeroes the voice's four stage states.
- The states are flushed (below 1e-15 to exactly 0, in code, not by FTZ/DAZ flags) once per block.
- **Plan swap:** `copy_state_from` copies the four floats per voice, the cached coefficients, the drive and their keys, so a swap that keeps a ladder keeps its ring.

## Closed-form checks

- **The measured response is the closed form** (`the_measured_response_matches_the_closed_form`, `enth-dsp`): the spectrum of a small impulse's response against `response` at every bin from 40 Hz to 0.45·fs above −70 dB of the peak, four modes, cutoffs 200 Hz, 2 kHz and 10 kHz, resonance 0, 10, 50 and 90 %: 48 settings, the worst difference **0.0119 dB** (`hp`, 200 Hz, 50 %); asserted 0.05.
- **Through the engine** (`the_ladder_gain_is_the_closed_form`): a quiet sine (osc level 5 %, so the limiter stays in its linear range) at six keys (110 Hz … 3.5 kHz) through `lp24` at cutoffs 1 kHz, 200 Hz and 100 Hz with resonance 0, 50 and 80 %, and `lp12`, `bp` and `hp` at 1 kHz with resonance 0 and 60 %: 73 points above −75 dB, worst **0.016 dB** from the closed form (asserted 0.05).
- **Self-oscillation:** above (frequency within 0.005 % of the cutoff through the core; one FFT bin through the engine).
- **Latency 0:** the output sample depends on the input sample (`the_filter_has_no_latency`): `h[0] ≠ 0` in every mode; no look-ahead, no delay line.

## Stability under modulation

Required: finite, bounded output and a tail that dies, with the cutoff (and resonance, drive, mix) jumping every segment at the parameter extremes. Tests:

- **Random-jump stress** (`enth-dsp`, `random_jumps_of_everything_stay_bounded_and_the_tail_dies`): white noise (±0.5) in; every segment a new random cutoff (20 Hz … 21.6 kHz, log-uniform), resonance (0 … 100 %), drive (0 … 24 dB), mode and mix, for 30,000 segments (20 s, a 1.5 kHz modulation of every parameter): every sample finite, **the output peak 1.10, the largest stage state 2.55** (the limiter bounds even the self-oscillation), then the parameters park and the state decays to exactly zero with no subnormal on the way.
- **Module stress** (`modules/ladder.rs`): the same through the engine module in each of the four modes, 15,000 segments, a new random cutoff, resonance, drive and mix every segment; finite, peak below 20 (asserted), then 30 s of silence at resonance 80 %: no subnormal in the module's state at any block, and the state is exactly zero.
- **Audio-rate LFOs** (`tests/flt.rs`): LFOs at 50, 37, 43 and 29 Hz at full depth on `cutoff`, `resonance`, `drive` and `mix` (resonance 90 % base) on two overlapping saws for 10 s: the peak is 1.13 against 1.36 for the dry saw (asserted below 3× dry).
- **Golden** `ladder-wobble`: a 6 Hz cutoff LFO and a 37 Hz resonance LFO at resonance 85–91 %.

## Aliasing

The ladder is not linear: the loop's limiter (a smooth odd curve) and, when `drive` is up, the input saturator act per sample at the base rate with no oversampling. **Bounds, against the sine's fundamental** (report 02's measure, `enth_dsp::alias`, [aliasing.md](aliasing.md)); the sine is at the same relation to the cutoff at every key, because the cutoff tracks the key (`keytrack` 100 % and a cutoff of `ratio · 261.63 Hz`), so the measure reads the filter, not a skirt:

| | sine | 55 Hz | 110 Hz | 220 Hz | 440 Hz | 880 Hz | 1,760 Hz | 3,520 Hz |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `lp24`, cutoff 4× the key, resonance 30 % | dB | −93.3 | −103.3 | −97.7 | −93.4 | −95.5 | −92.7 | −120.0 |
| `lp24`, cutoff 4× the key, resonance 90 % | dB | −93.3 | −103.4 | −97.8 | −93.4 | −95.5 | −92.7 | −119.8 |
| `lp12`, cutoff 4× the key, resonance 60 % | dB | −93.1 | −103.4 | −97.8 | −93.4 | −95.5 | −92.7 | −119.9 |
| `bp`, cutoff on the key, resonance 60 % | dB | −93.2 | −103.3 | −97.8 | −93.4 | −95.5 | −92.7 | −119.1 |
| `hp`, cutoff a quarter of the key, resonance 60 % | dB | −92.9 | −103.6 | −97.8 | −93.4 | −95.5 | −92.7 | −120.0 |
| `lp24`, resonance 100 %, ring on the sine | dB | −93.2 | −103.3 | −97.8 | −93.4 | −95.5 | −92.7 | −93.0 |
| `lp24`, resonance 100 %, ring an octave above the sine | dB | −97.2 | −99.7 | −94.8 | −94.0 | −93.8 | −95.3 | −88.3 |

(The ring is an exact harmonic of the sine in the last two rows, so the measure counts it as signal, as it does the comb's teeth. Pre-roll 0.5–2.5 s so the attack's ring is over.) Worst **−88.3 dB** (the ring at 7 kHz, from a 3.5 kHz sine); the rest are the engine's own sine floor (−93.3 … −119.8 dB, [aliasing.md](aliasing.md)). **Bound: −86.8 dB** (worst plus 1.5 dB), `tests/flt.rs::a_ladder_adds_almost_no_aliasing_without_drive`.

**A ring above a sixth of the sample rate aliases its limiter.** The self-oscillation passes through the limiter once per sample; at the ring's amplitude the limiter has a third harmonic of about −37 dB (the loop signal `u` peaks at 1.41 on the ring: the limiter's third harmonic is −42 dB at `u` = 1 and −37 dB at 1.41, before the filter's slope), which is above Nyquist when the ring is above 8 kHz and folds back. Key 105 with the cutoff at one to six times the key (rings at 3.5 … 21 kHz, each an exact harmonic of the sine, resonance 100 %):

| ring | 3.5 kHz | 7.0 kHz | 10.6 kHz | 14.1 kHz | 17.6 kHz | 21.1 kHz |
| --- | --- | --- | --- | --- | --- | --- |
| aliasing, dB | −93.0 | −88.3 | −55.6 | −38.5 | −34.2 | −34.9 |

Bound for the rings above 8 kHz: **−32.7 dB** (`a_ring_above_a_sixth_of_the_rate_folds_its_limiter`). This is a property of a non-oversampled saturator in a self-oscillating loop, like the svf's drive; it applies only at `resonance` above 95 % **and** a cutoff above 8 kHz, and the fix, if it ever matters, is to run the loop at twice the rate (about double the cost). Reported to the coordinator.

**Drive** is the svf's saturator (`tests/alias.rs` records −100.5, −93.9, −58.2, −37.3 and −25.2 dB for it at 24 dB of drive and a sine at 110 … 1,760 Hz). Through the ladder, cutoff open at 20 kHz (`a_driven_ladder_aliases_like_the_svf_drive`, keys 45 … 93):

| 24 dB of drive | 110 Hz | 220 Hz | 440 Hz | 880 Hz | 1,760 Hz |
| --- | --- | --- | --- | --- | --- |
| cutoff 20 kHz, dB | −100.5 | −94.6 | −59.2 | −37.9 | −25.6 |
| cutoff 4× the key, dB | −102.2 | −97.3 | −83.9 | −51.3 | −32.6 |

Bounds: −24.1 dB (open; the svf's own is −23.7) and −31.1 dB (tracked). With the cutoff below the folded harmonics the ladder's own slope removes most of what the saturator folds.

Fast modulation does not alias: the ramp is a linear interpolation of the coefficients, not a nonlinearity.

## Determinism

The transcendentals (`tanf` for the cutoff, `exp2f` for key tracking, `expm1` in the drive map) run per voice and changed segment through `enth_dsp::util::math` and the pinned `libm`; the per-sample code is `+ − × ÷` and `sqrt` (the limiter and the drive), all correctly rounded in IEEE 754, with no `mul_add`. Tests: the same bits under every FTZ/DAZ mode (`a_ladder_renders_the_same_bits_under_every_ftz_mode`), zero subnormals in the output or any engine state over a 40 s render of four notes through two ladders in series (a resonance-100 % ring at 100 Hz with 24 dB of drive, and a high-pass at 97 %), with a 3 s and a 20 s release, every voice below −100 dB at the end (`a_40_s_tail_through_ladders_holds_no_subnormals`), no allocation in `process` (`a_ladder_never_allocates_in_process`), two instances keep separate state (`two_ladders_keep_separate_state`, `modules/ladder.rs`). The same bits on six targets are the CI golden job's; the nine `ladder-*` goldens are the pin.

## Goldens

| Case | What it pins |
| --- | --- |
| `ladder-lp24` | a 110 Hz saw through `lp24` at 600 Hz, resonance 40 %, an envelope opening the cutoff by 35 % of its log range: the four-stage solve, the per-sample ramps along a sweep, the log-scale route depth |
| `ladder-ring` | resonance 100 %, a short saw note, the ring at 700 Hz until the release: the self-oscillation level, the limiter, the exact ring frequency |
| `ladder-lp12` | the 12 dB/octave tap at 800 Hz, resonance 60 % |
| `ladder-bp` | the band-pass tap at 1.2 kHz, resonance 60 %, an envelope sweeping the cutoff down |
| `ladder-hp` | the high-pass tap at 600 Hz, resonance 50 %: the binomial combination |
| `ladder-drive` | 18 dB of drive into the ladder at 1.5 kHz with a loud saw: the saturator and the limiter together |
| `ladder-keytrack` | three notes an octave apart with key tracking 100 %: the octave-per-octave map, the cached-coefficient path |
| `ladder-mix` | `mix` 50 % at resonance 80 %, drive 6 dB: the blend |
| `ladder-wobble` | a 6 Hz LFO on the cutoff and a 37 Hz LFO on the resonance: the ramps and the stability under fast modulation |

## Cost

Measured with callgrind (`cargo xtask bench -p enth-engine --bench ladder`), 1 s at 48 kHz, whole engine, on the build of 2026-10-26; the "marginal" column subtracts the `engine` bench of the same build with the same voices (3,615,181 at 1 voice, 37,272,521 at 16: the reference saw and the ADSR are in both):

| Case | Instructions | Budget (+10 %) | Marginal per sample per voice |
| --- | --- | --- | --- |
| `lp24_1v` | 6,427,120 | 7,070,000 | (fixed costs dominate at 1 voice) |
| `lp24_16v` | 78,720,770 | 86,593,000 | 54.0 |
| `drive_16v` (12 dB) | 86,666,546 | 95,334,000 | 64.3 |
| `moving_16v` (a cutoff event every segment) | 101,111,733 | 111,223,000 | 83.1 |

A four-pole ladder costs 54 instructions per sample per voice against the svf's 28 for one two-pole filter (four stages, the solve, and the limiter's `sqrt` and division). Drive adds 10 (a `sqrt`, a division and the ramp of two values). A moving cutoff adds the ramp of the coefficients, the powers of `G` and the division per sample and, per voice and segment, a new `tanf`: the extra 29 per sample together.

**Stereo pairs (lane PRF, 2026-10-27).** In a plan with the stereo voice path each voice runs the ladder for the left bus and again for the right ([engine.md](engine.md#stereo-voice-path)). `ladder::process_lanes` runs up to four instances at once, one per lane of an `f32x4`: two voices' left and right filters (`Ladder::process_pair` collects the voices two at a time). Each lane performs `process_block`'s `+ − × ÷` and `sqrt` in the same order on its own state, so every output sample and every state bit equals the instance run alone (`instances_in_lanes_are_process_block_bit_for_bit`, over modes, ramps, drive and mix; `process_pair_is_two_process_calls_bit_for_bit` through the engine). Instances run together only when their blocks take the same branches (mode, whether the coefficients ramp, whether the drive is on, whether the blend is off, full or partial); a group that differs, or a single leftover instance, runs `process_block` one after the other. On the stereo-forced bench (16 voices, 1 s) the static ladder went from 121,422,871 to 72,342,863 instructions (−40.4 %); in the G2 reference patch −33.8 M of 803 M. The mono path is unchanged (78,746,871 → 78,548,871 for `lp24_16v`, −0.25 %).

## Behaviour other lanes must know

- **The level falls as the resonance rises** (−3 dB at the default 10 %, −13.6 dB at 90 %) while the peak at the cutoff rises (+13 dB at 90 %); a ladder at 90 % resonance on a bass loses its lows. There is no compensation; it is the sound of a Moog. Self-oscillation is a −12 dBFS RMS sine, independent of the input.
- **Resonance 100 % is a sine oscillator at the cutoff**, in every mode (also `bp` and `hp`), started by any input. With key tracking at 100 % it plays in tune with the keyboard (the ring is at the effective cutoff to 0.005 %), which is how a patch uses the ladder as a whistle (the `ladder-whistle-sweep` WAV).
- **A voice start cuts the ring**: `start_voice` zeroes the stage states.
- **Rings above 8 kHz alias** (above), and the limiter puts a third harmonic of about −37 dB on any ring (before the filter's slope).
- **Latency 0**; no look-ahead; the output is bounded for any input (the limiter bounds the loop; an input that is itself large passes the stages linearly).

## Files

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/ladder.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/ladder.rs` | implemented, 11 white-box tests |
| Engine module | `crates/enth-engine/src/modules/ladder.rs` | implemented, 8 module tests; the dispatch arms are S0d's |
| Engine acceptance tests | `crates/enth-engine/tests/flt.rs` | 12 ladder tests (aliasing ×3, closed form, slope, ring, bit-exact mix 0, instances, modulation, tail, FTZ, no alloc) |
| Instruction budget | `crates/enth-engine/benches/ladder.rs` | measured + 10 % ([Cost](#cost)) |
| Golden cases | `tests/golden/cases/ladder-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | nine ([Goldens](#goldens)) |
| WAVs | `~/code/enth-scratch/s2/flt/wav/ladder-*.wav` | a resonant cutoff sweep, a whistling sweep at resonance 100 % (rendered, never played) |

## Acceptance (from phase-s2.md, every module lane)

| Row | Where it is checked |
| --- | --- |
| Same bits on six targets | the nine goldens (CI) |
| FTZ-independent | `tests/flt.rs::a_ladder_renders_the_same_bits_under_every_ftz_mode` |
| Zero subnormals after a 40 s tail | `tests/flt.rs::a_40_s_tail_through_ladders_holds_no_subnormals` |
| Latency 0 | [Closed-form checks](#closed-form-checks): `h[0] ≠ 0` in every mode |
| Budget +10 % | [Cost](#cost) |
| Two instances keep separate state | `tests/flt.rs::two_ladders_keep_separate_state`, `modules/ladder.rs` |
| `mix` 0 passes the input bit for bit | `tests/flt.rs::ladder_mix_zero_renders_the_same_bits_as_no_ladder`, `dsp/ladder.rs`, `modules/ladder.rs` |
| Aliasing measured spectrally, bound recorded | [Aliasing](#aliasing): −86.8 dB (linear sweeps and rings to 7 kHz), −32.7 dB (rings above 8 kHz), −24.1 / −31.1 dB (24 dB of drive) |
| −24 dB/octave within 1 dB at 1 and 2 octaves above the cutoff | **not met as worded**: 15.9 and 21.3 dB/octave there for any four equal poles; within 1 dB (23.3, then 24.0) from the third octave ([The slope](#the-slope)) |
| Self-oscillation at the cutoff within 2 % near maximum resonance | [The filter](#the-filter): within 0.005 % (core), one FFT bin (engine); starts at 95.2 % |
| Key tracking as `svf` | [Key tracking](#key-tracking) |
| Stability under fast modulation of the cutoff | [Stability under modulation](#stability-under-modulation) |

## References

- V. Zavalishin, *The Art of VA Filter Design*, rev. 2.1.0, Native Instruments, 2018: the one-pole stage, the Moog ladder with the feedback solved in the same sample, the stage taps.
- Research report 05 §5 (filters beyond the SVF).
