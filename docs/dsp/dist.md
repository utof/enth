# Distortion (`dist`; `enth-dsp::dist`, `enth-dsp::resample`, `enth-engine::modules::dist`)

- **Status: built (S2a lane DST, wave 3).** The module replaced the S0d stub; every acceptance row is in [Acceptance](#acceptance-how-each-row-is-met). Kind `dist`, discriminant 7, section `voice_fx`, id prefix `dist` (dist1, dist2…), rate voice. Still hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag; the parameter table is lane CAT's F1m table and was not changed.
- Code: the DSP core is `crates/enth-dsp/src/dist.rs` (curves, ADAA, converters, crossover, tone, dry path, one voice) and the oversampler is `crates/enth-dsp/src/resample.rs` (written to be shared: a driven filter or a high-index PM pair may reuse it); the module is `crates/enth-engine/src/modules/dist.rs`. Tests: white-box in the same three files, `crates/enth-engine/tests/dist.rs` (aliasing sweeps, the chain of two, the plan swap, the 40 s tail, FTZ, no allocation). Goldens: `tests/golden/cases/dist-*.toml` (11 cases). Bench: `crates/enth-engine/benches/dist.rs`.
- **Provenance: nothing is ported.** Every structure is written from the papers cited under [Sources](#sources) and every equation is on this page; no code from any library was copied, and the half-band coefficients were designed by the method below, not looked up. So there is **no `THIRD_PARTY_NOTICES.md` entry** for this lane (the acceptance row asks for one "for every port"; there is none to make).
- Decisions behind it: research report 05 §2 (ADAA, the shaper family) and §7 (oversampling) in `_local/sessions/2026-10-25-s2-depth-and-fast-training/`, lane CAT's table and the F1m decisions ([catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m)). Where this lane departs from or settles something the table left open, the section says so; the list is under [Limits and decisions](#limits-and-decisions).

Distortion per voice: five shaping curves (soft, hard, asymmetric, sine fold, triangle fold) with first-order antiderivative anti-aliasing (ADAA-1) and zero-latency 1×, 2× or 4× IIR oversampling; a bitcrusher and a sample-and-hold downsampler; `drive`, `bias`, `tone`; a `keep_lows` crossover that leaves the sub clean; and `mix` against a phase-matched dry path. All of it is `f64` inside the module (the bus is `f32`), with the pinned `libm` and a fixed order of evaluation, so the stream is bit-identical on every target.

## Parameters (lane CAT, F1m)

*Built as specified. What the signal-path bullet leaves to lane DST, the phase-matched dry path, is in [The dry path and `mix`](#the-dry-path-and-mix); the whole path is drawn under [Signal path](#signal-path).*

The table lane DST implements, generated from `crates/enth-patch/src/catalogue/dist.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Signal path:** `keep_lows` splits off the band below it (no split at 20 Hz), which bypasses the curve; `drive` and `bias` go into the `mode` curve (oversampled by `oversample` for the clip and fold curves; `bitcrush` reads `bits`, `downsample` reads `rate`, and neither is oversampled); a DC blocker; `tone` (a low-pass, open at 20 kHz); then `mix`: 0 % is the input bit for bit, and otherwise the blend is against a **phase-matched dry path**. The oversampler's IIR half-bands, the `keep_lows` crossover and `tone` all shift phase, so a plain dry copy would comb the blend wherever the curve is nearly linear (a quiet signal, or `drive` near 0 dB); the dry signal `mix` blends is put through the same phase response as the wet path's linear stages (an all-pass with that phase, or the crossover's own all-pass output; how is lane DST's choice), and the test is a flat magnitude response at `mix` 50 %, within ±0.5 dB from 50 Hz to 10 kHz, at `drive` 0 dB and a level well below the knee. The step from the matched dry path to the raw input at exactly 0 % changes phase, never level.
- **`rate` goes up to 192 kHz**, so that its top is the identity at every supported sample rate: a downsample rate at or above the engine's sample rate holds nothing and passes the input unchanged. The default stays 6 kHz.
- **Defaults:** a gentle useful setting, 6 dB into the soft curve, 2× oversampling. `oversample` is structural (it rebuilds the plan, so a factor change never clicks).
- Report 04's S6 "downsample, MSEG on drive" is `mseg → dist.rate` here: the amount of downsampling is `rate`.

### `dist` — Distortion (voice_fx, ids `dist1`, `dist2`…; 9 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mode` | none | `soft`, `hard`, `asymmetric`, `sine_fold`, `triangle_fold`, `bitcrush`, `downsample` | enum | soft | auto | basic | The distortion curve: soft or hard clipping, asymmetric, sine or triangle folding, bitcrush or downsample. |
| `drive` | db | 0dB … 48dB | linear | 6dB | mod, auto | basic | Gain into the curve: more drive is more distortion; route an LFO or MSEG here for rhythmic grit. |
| `bias` | percent | -100% … 100% | linear | 0% | mod, auto | intermediate | An offset added before the curve, so the top and bottom distort differently: adds even harmonics and a thicker, rasping tone. |
| `bits` | none | 1 … 16 | linear | 8 | mod, auto | intermediate | Bit depth in bitcrush mode, fractions allowed: 16 is clean, 8 is gritty, 2 is a buzz. Other modes ignore it. |
| `rate` | hz | 100Hz … 192kHz | log | 6kHz | mod, auto | intermediate | The held sample rate in downsample mode: lower is more metallic and aliased; at the engine's rate or above it the input passes unchanged. Other modes ignore it. |
| `tone` | hz | 100Hz … 20kHz | log | 20kHz | mod, auto | intermediate | A low-pass after the curve that tames fizz: lower is darker; 20 kHz leaves it open. |
| `keep_lows` | hz | 20Hz … 1kHz | log | 20Hz | auto | intermediate | Frequencies below this bypass the distortion, so the sub stays clean and only the mids distort; at 20 Hz everything is distorted. |
| `oversample` | none | `off`, `x2`, `x4` | enum | x2 | structural | advanced | How much the clipping and folding curves are oversampled to cut aliasing: off, 2x or 4x (no latency either way); changing it rebuilds the plan. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged; lower values blend in the clean signal (parallel distortion). |

## Signal path

Per voice, per sample (the equation of each stage is in the sections below):

```text
 x ─┬─ crossover (LR4 at keep_lows) ─ lo ──────────────────────────────── phase chain ─┐
    │                              └ hi ─ ×drive + bias ─ curve ─ DC block ────────────┴─ + ─ tone LP ─ wet ─┐
    │                                                                                                         ├─ blend ─ out
    └─ (lo + hi, or x) ─ phase chain ─ tone all-pass ─ dry ───────────────────────────────────────────────────┘
```

- `keep_lows` at 20 Hz (the minimum) has no crossover: `hi = x`, `lo` is absent. Above it the low band skips `drive`, `bias` and the curve.
- `drive` is the gain `10^(dB/20)`; `bias` is an offset of `bias/100` added **after** the gain, so 100 % puts the curve's own unit level on the signal at any drive. The curve's input is `u = gain·hi + bias/100`.
- `bitcrush` and `downsample` replace the curve; they are not oversampled (their aliasing is the effect), but `drive`, `bias` and their clamp to ±1 still apply.
- The DC blocker sits after the curve (an asymmetric curve, a bias or a clipped wave makes DC) and before the low band is added back; `tone` is after both.
- The blend is `(1 − mix)·dry + mix·wet`, and the dry is not the raw input but a phase-matched copy ([The dry path](#the-dry-path-and-mix)).

## The curves

Every curve has slope 1 at 0 and a peak of 1, so changing `mode` changes the character of a loud signal and not the level of a quiet one. `F` is the antiderivative that ADAA needs (any constant: only differences matter).

| `mode` | Curve `f(u)` | Antiderivative `F(u)` | Corners and folds |
| --- | --- | --- | --- |
| `soft` | `tanh(u)` | `ln cosh(u)` | none; smooth, limits ±1 |
| `hard` | `clamp(u, −1, 1)` | `½u²` for `|u| ≤ 1`, `|u| − ½` beyond | corners at `|u|` = 1 |
| `asymmetric` | `tanh(u)` for `u ≥ 0`, `½·tanh(2u)` for `u < 0` | `ln cosh(u)`; `¼·ln cosh(2u)` | limits +1 and −½, slope 1 on both sides of 0; the slope and the second derivative are continuous there and the third derivative jumps (−2 against −8) |
| `sine_fold` | `sin(u)` | `−cos(u)` | first fold at `|u|` = π/2 ≈ 1.571 (the output is 1 there and falls back), then every π (peaks at π/2 + kπ) |
| `triangle_fold` | period-4 triangle with unit slope: `u` on `[−1, 1]`, `2 − u` on `[1, 3]`, `u − 4` on `[3, 5]`… | `½r² − r` on the first half of the period, `3r − ½r² − 4` on the second, with `r = (u + 1) mod 4` (periodic: the integral over a period is 0) | folds at `|u|` = 1, 3, 5…, a corner at each |
| `bitcrush` | clamp to ±1, then round to `2^(bits−1)` steps per side | — | mid-tread, so silence stays silence; fractional `bits` allowed; 1 bit is {−1, 0, +1} |
| `downsample` | clamp to ±1, then hold each sample for `fs/rate` samples | — | a zero-order hold on a phase accumulator; at `rate` ≥ `fs` it holds nothing and is the identity |

A fold starts to fold when `|u|` first exceeds its threshold, so in terms of the signal: a full-scale (±1) sine folds a `triangle_fold` at any `drive` above 0 dB and a `sine_fold` above 3.9 dB (`20·log10(π/2)`); a signal of peak 0.5 needs 6 dB and 9.9 dB. A `bias` of 100 % puts the signal's zero at `u` = 1: exactly on the first fold of `triangle_fold`, and at `sin(1)` = 0.84 on the way up to the first fold of `sine_fold`.

**The static transfer curve** is checked at DC through the whole shaper, oversampler included (a constant input to the ADAA stage inside the 1×, 2× and 4× oversamplers, 400 samples, 241 inputs from −6 to +6): the largest deviation from the formula is **2.0e-15** (the requirement is 1e-6), `a_constant_input_returns_the_static_curve`. Each curve is also checked against its formula to 1e-15 and each antiderivative against a central difference of the curve to 1e-6.

## Anti-aliasing: ADAA-1

Parker, Zavalishin and Le Bivic (DAFx-16, 2016) and Bilbao, Esqueda, Parker and Välimäki (IEEE SPL 24(7), 2017): replace the sample `f(u[n])` by the average of `f` over the straight line between the last two inputs,

```text
y[n] = (F(u[n]) − F(u[n−1])) / (u[n] − u[n−1])         midpoint fallback: y[n] = f((u[n] + u[n−1]) / 2) when |u[n] − u[n−1]| < 1e-5
```

which filters the continuous curve with a rectangular window of one sample before it is sampled, and so lowers the aliasing without a lookahead. The fallback is an exact match of the quotient's limit; the test `adaa_matches_the_midpoint_across_the_fallback_threshold` checks both sides of the threshold to 1e-8, and a constant input returns the curve exactly (above).

**The price is a droop.** Where the curve is linear (a quiet signal, or the middle of `hard`), the quotient is the two-sample average `(u[n] + u[n−1])/2`: a half-sample delay and a droop of `cos(ω/2)` at the rate the curve runs at. It is the same on every first-order ADAA design, and the only lever is the oversampling factor: the curve runs at 2× or 4× the rate, so a given frequency sits a half or a quarter as far up the `cos(ω/2)` curve. The droop of the **wet** path at `drive` 0 dB with a quiet sine, measured through the whole module (`print_response_tables`, dB re the input):

| Hz | 50 | 100 | 300 | 1k | 3k | 6k | 8k | 10k | 12k |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| off, wet | −0.04 | −0.01 | −0.00 | −0.02 | −0.17 | −0.69 | −1.25 | −2.01 | −3.01 |
| x2, wet | −0.04 | −0.01 | 0.00 | −0.00 | −0.04 | −0.17 | −0.30 | −0.47 | −0.69 |
| x4, wet | −0.04 | −0.01 | 0.00 | 0.00 | −0.01 | −0.04 | −0.07 | −0.11 | −0.17 |

(The −0.04 dB at 50 Hz is the 5 Hz DC blocker.) These are the exact `cos(ω/2)` values at the oversampled rate; the half-bands add less than 2e-6 dB.

### ADAA stays on at 2× and 4× (the open decision of the brief)

Is the antiderivative worth its cost when the signal is already oversampled? Measured: a sine at 16× drive (24 dB) into each curve at two pitches, the aliasing of the curve with the antiderivative quotient against the plain curve at the oversampled rate (`adaa_lowers_aliasing_inside_the_oversampler_too`, `Measure::REPORT_02`; dB lower with ADAA):

| Curve | 1.76 kHz: 1× / 2× / 4× | 3.52 kHz: 1× / 2× / 4× |
| --- | --- | --- |
| soft | 9.1 / 16.6 / 2.0 | 10.7 / 18.2 / 18.3 |
| hard | 9.8 / 18.2 / 21.1 | 10.9 / 18.4 / 21.0 |
| asymmetric | 9.6 / 17.2 / 11.7 | 10.9 / 18.2 / 21.9 |
| sine fold | 6.1 / 10.2 / 2.2 | 15.2 / 10.2 / 1.9 |
| triangle fold | 10.0 / 20.3 / 20.6 | 18.0 / 17.6 / 23.2 |

So ADAA lowers aliasing by 6 to 23 dB at every factor wherever the curve (not the half-bands' stopband) sets the floor; the two entries near 2 dB (soft and sine fold at 4×) are the stopband floor of the half-bands, where nothing is lost either. The extra cost is one antiderivative and one division per oversampled sample (`ln cosh` for the tanh pair, a multiply for the hard clip, a cosine for the sine fold, a few multiplies for the triangle). **The decision: ADAA is on at all three settings.** Switching it off at 2× and 4× would save that cost and recover 0.47 dB (2×) and 0.11 dB (4×) of the droop at 10 kHz, and it would cost 10 to 23 dB of aliasing on every curve except soft and sine fold at 4×, where the half-bands' floor makes the gain 2 dB.

## Zero-latency IIR oversampling

`oversample` off, `x2`, `x4`. The signal is interpolated by one or two half-band stages, the ADAA curve runs at the high rate and the result is decimated by the same stages. Nothing looks ahead, nothing is reported: the filters are causal IIR all-passes, so the latency is 0 (ADR-0006) and the price is a frequency-dependent group delay of the wet path, which the dry path copies ([below](#the-dry-path-and-mix)).

### Structure: Valenzuela and Constantinides (1983)

A half-band low-pass is the average of two all-pass filters in the polyphase form,

```text
H(z) = ½ · [ A0(z²) + z⁻¹ · A1(z²) ]       Ai(z) = ∏ (α + z⁻¹) / (1 + α·z⁻¹)    one first-order section per α, real α in (0, 1)
```

and both branches run at the **lower** rate: an interpolator by two costs one pass through each chain per input sample (the two output samples are the two branches), a decimator the same. A section is `y[n] = α·(x[n] − y[n−1]) + x[n−1]`: one multiply, three additions. The filter is exactly power-complementary, `|H(f)|² + |H(½ − f)|² = 1`, which the design test checks to 1e-12 on 2,000 points.

### Design: the elliptic half-band's poles

The coefficients are not tabulated from a book: `design::alphas(n, fp)` in the tests of `resample.rs` computes them, and the committed constants equal its output to 1e-13 (`the_committed_coefficients_are_the_design`). The method, for an odd order `n` and a passband edge `fp` (a fraction of the oversampled rate, `fp < ¼`):

1. A half-band has `ωp + ωs = π`; its analogue elliptic prototype therefore has **all its poles on one circle** `|s| = ρ` with `ρ = 1/tan(π·fp)` (the pre-warped stopband edge), one of them real at `−ρ`. The elliptic modulus is the selectivity `k = tan(π·fp) / tan(π·(½ − fp)) = tan²(π·fp) = 1/ρ²`, the ratio of the pre-warped band edges.
2. The poles come from Jacobi elliptic functions (parameter `m = k²`, complete integrals `K(m)` and `K(1 − m)`). The real pole fixes the auxiliary argument `v0` through `sc(v0 | 1 − m) = ρ` (a bisection). With `(sn_v, cn_v, dn_v)` the functions at `v0` in the complementary parameter `1 − m`, and `(sn, cn, dn)` at `i·K(m)/n` in `m`, the complex poles are `s_i = −(cn·dn·sn_v·cn_v + j·sn·dn_v) / (1 − dn²·sn_v²)` for `i = 2, 4, …, n − 1` (`j` the imaginary unit: the first term is the real part). The design asserts `|s_i| = ρ` to 1e-9 for every pole.
3. The bilinear map `z = (1 + s/ρ)/(1 − s/ρ)` sends the circle's pairs to pairs `±j·√α` on the imaginary axis of `z`, with `α = (1 − σ)/(1 + σ)` and `σ = −Re(s)/ρ`. These `α` are the section coefficients.
4. Sorted ascending, alternate coefficients go to `A0` (1st, 3rd, …) and `A1` (2nd, 4th, …): that puts the **low-pass** of the odd order `n` in the path.
5. The Jacobi functions are evaluated by the descending Landen transformation (the arithmetic-geometric mean; Abramowitz and Stegun 16.4) in `f64`, and `K` from the AGM.

The two stages used, designed once and committed as `HB_STAGE1` and `HB_STAGE2` (`cargo test -p enth-dsp --lib print_halfband_design -- --ignored --nocapture` reprints them):

| Stage | Order `n` | Passband edge `fp` | Stopband from | Stopband | Passband ripple | A0 coefficients | A1 coefficients |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 (2×, and the first of 4×; at the base rate in, 2× out) | 9 (four sections) | 0.2 | 0.3 | **70.04 dB** | 4e-7 dB | 0.07986642623635758, 0.5453236510711319 | 0.2838293448741102, 0.8344118914807372 |
| 2 (the second of 4×, at 2× on stage 1's output) | 7 (three sections) | 0.15 | 0.35 | **76.59 dB** | < 1e-5 dB | 0.08143002317661638, 0.7097708001024805 | 0.3156598402166598 |

Frequencies are fractions of the rate the stage runs at. Why these two, at 48 kHz: stage 1 runs at 96 kHz, so its passband edge is 19.2 kHz and its stopband starts at 28.8 kHz. The interpolator's image of anything below 19.2 kHz lies above 28.8 kHz (the image of `f` is `48 kHz − f`), and what the curve generates between 28.8 and 48 kHz folds into 0 – 19.2 kHz in the decimator only after 70 dB. Stage 2 runs at 192 kHz in a 4× chain; its passband edge, 0.15 of that, is 28.8 kHz, exactly where stage 1's stopband begins, so it passes everything stage 1 leaves and has to remove only the images from 67.2 kHz up: seven poles and 76.6 dB are enough there.

The measured responses (`the_half_bands_are_flat_in_the_passband_and_deep_in_the_stopband`): power complementary to 1e-12; passband ripple under 1e-5 dB; stopband above 69.9 dB and 76.5 dB. Through the whole identity chain (up, nothing, down; `the_identity_chains_are_flat_and_have_no_latency`), the magnitude is flat within 2e-6 dB from DC to 19 kHz at 48 kHz (asserted at 1e-4 dB from 50 Hz to 19 kHz; the transition band between 19.2 and 24 kHz rolls off to −6 dB at Nyquist, which is inaudible), the impulse response is causal with 90 % of its energy in the first 16 samples (no look-ahead, no pre-ringing), and a sine at 100 Hz, 1, 5 and 12 kHz comes back at the base RMS within 1e-3 dB.

| Group delay of the identity chain (base-rate samples) | 100 Hz | 10 kHz |
| --- | --- | --- |
| 2× | 2.29 | 2.78 |
| 4× | 3.32 | 3.85 |

This is the delay of the *wet path*, not a reported latency: the dry path is given the same phase so that nothing combs.

**Priming.** A voice starts with its oversampler in the steady state of its first input sample (`Oversampler::prime`), so a voice that starts on a nonzero input has no step from the half-bands (`a_fresh_voice_has_no_start_transient_from_a_constant_input`).

**Determinism.** `+ − × ×0.5` in `f64` in a fixed order; the states are flushed (below 1e-15 to exactly 0) once per 32-frame block by code, never by FTZ/DAZ flags.

## Converters

`bitcrush`: `y = round(clamp(u, −1, 1) · L)/L` with `L = 2^(bits−1)` (`exp2`, so fractional `bits` is smooth). `downsample`: a phase accumulator advances by `rate/fs` per sample (capped at 1) and takes a new clamped sample when it wraps; between wraps the last one is held. Both run at the base rate (not oversampled) after `drive` and `bias`; the DC blocker, `tone` and `mix` apply as for the curves. The test pins the quantisation grid (multiples of 1/4 at 3 bits), the hold lengths (four samples at a quarter of the rate) and that `rate` ≥ `fs` is the identity bit for bit (`bitcrush_quantises_and_downsample_holds`).

Their aliasing is the effect, so it is recorded and not bounded to a low level ([Aliasing](#aliasing)).

## `keep_lows`: the crossover

Above 20 Hz a fourth-order Linkwitz-Riley crossover (Linkwitz, JAES 24(1), 1976) splits the input at the corner: three second-order Butterworth sections (`Q = 1/√2`) in Simper's trapezoidal state-variable form (Cytomic, 2013; `g = tan(π·fc/fs)`, `k = √2`), one that makes the low-pass `LP1` and high-pass `HP1`, a second that low-passes `LP1` (the low band) and a third that high-passes `HP1` (the high band). The sum of the two bands is the second-order Butterworth **all-pass** of the input, with no magnitude error (`the_crossover_is_lr4_and_sums_to_an_all_pass`). The high band is driven into the curve; the low band bypasses it, goes through the same phase chain as the dry path and is added after the DC blocker. In the linear limit the whole thing is therefore the crossover's all-pass at `mix` 100 %, as at any `mix`.

Measured, a 0.5-amplitude sine through the hard clip at 24 dB of drive with the corner at 400 Hz (the fundamental's gain; the test is `keep_lows_passes_the_low_band_and_distorts_the_rest`, the table is `print_response_tables`):

| Sine | 20 Hz | 40 Hz | 60 Hz | 100 Hz |
| --- | --- | --- | --- | --- |
| fundamental gain, dB | +0.001 | +0.013 | +0.065 | +0.486 |

Within **0.1 dB up to 60 Hz (a sixth of the corner)** and +0.5 dB at a quarter of it. The cause is not the low band but the high band's stopband: the LR4 high-pass is −48 dB at a quarter of the corner, and a 24 dB drive multiplies that tail by 16 before the clip; it adds in phase with the low band. At a lower drive the band below the corner is flatter. Without a split the same sine is squared by the clip (`amp > 1.1×`).

The corner has no modulation source: a change of value moves the crossover's coefficients at the next segment with its state carried on, and only switching the split on or off (the 20 Hz minimum against anything above) re-primes the crossover and the phase chains to the steady state of the next input sample.

## `tone`

A one-pole trapezoidal low-pass after the curve (Zavalishin, *The Art of VA Filter Design*, 2018): `G = g/(1 + g)` with `g = tan(π·tone/fs)`, `v = G·(x − s)`, `y = v + s`, `s ← y + v`. Its magnitude is `1/√(1 + (tan(ω/2)/g)²)`: −3 dB at `tone` up to the bilinear warp. At the 20 kHz maximum the filter is out of the path (its state follows the signal so that closing it does not click). Measured at x2, drive 0 dB, against the exact one-pole magnitude (`tone_follows_its_one_pole_response`, tolerance 0.5 dB):

| `tone` | Test frequency | Measured | One-pole | Difference |
| --- | --- | --- | --- | --- |
| 500 Hz | 125 Hz | −0.27 | −0.26 | −0.00 |
| 500 Hz | 500 Hz | −3.01 | −3.01 | +0.00 |
| 500 Hz | 1 kHz | −7.00 | −7.00 | −0.00 |
| 2 kHz | 500 Hz | −0.26 | −0.26 | +0.00 |
| 2 kHz | 2 kHz | −3.03 | −3.01 | −0.02 |
| 2 kHz | 4 kHz | −7.18 | −7.11 | −0.07 |
| 4 kHz | 1 kHz | −0.25 | −0.25 | −0.00 |
| 4 kHz | 4 kHz | −3.08 | −3.01 | −0.07 |
| 4 kHz | 8 kHz | −7.81 | −7.51 | −0.30 |

The differences grow with frequency because they include the ADAA droop of the wet path (the droop table above: −0.30 dB at 8 kHz at x2); the filter itself is the bilinear one-pole.

## The DC blocker

A one-pole high-pass `y[n] = x[n] − x[n−1] + R·y[n−1]` with a corner at **5 Hz** (`R = 1 − 2π·5/fs`, a time constant of 32 ms): far below the lowest note, far above anything `bias` modulation does. The curve of `asymmetric`, a `bias`, or a clipped wave with an asymmetric start produces DC, and `tone`, `mix` and the master bus after it would carry it. The blocker's cost is the 5 Hz droop of the wet path (−0.04 dB at 50 Hz) and the one limit under [Limits](#limits-and-decisions): it starts empty, so an abruptly starting clipped wave has its mean removed over about 100 ms.

## The dry path and `mix`

The brief of lane CAT: a plain dry copy would comb the blend wherever the curve is nearly linear, because the wet path's linear stages shift phase. The wet path's linear response (the curve replaced by a line) is `H_wet(z) = tone(z) · ADAA(z) · [oversampler identity chain]`, and the dry path must have the same phase with unit magnitude:

- **The oversampler's identity chain** is `½·[A0(z²)² + z⁻¹·A1(z²)²]` for one stage, and the two halves are in phase to within 6e-4 rad, so **`A0` applied twice reproduces its phase with unit magnitude** (the `PhaseChain`: `A0` coefficients doubled, the sections `p0, p0, p1, p1`).
- **The ADAA's half-sample** (the two-tap average at the oversampled rate) and, at 4×, the second stage are carried by one more first-order all-pass section whose coefficient is a least-squares fit of the phase over 50 Hz to 12 kHz: `PHASE_ADAA_1 = 0.3933182303573387`, `PHASE_ADAA_2 = 0.6521487138786072`, `PHASE_ADAA_4 = −0.10712356954513495`. The chains are: off, the single section; x2, `p0, p0, p1, p1, PHASE_ADAA_2` (five sections); x4, the same with `PHASE_ADAA_4`. The phase error against the real linearised wet path is at most **2.1° (off), 1.3° (x2) and 2.6° (x4)** from 50 Hz to 12 kHz (`phase_chains_follow_the_linear_wet_path`, asserted at 2.2°, 1.4° and 2.8°); an error of 2.6° between two equal-magnitude signals mixed 50/50 is a dip of `20·log10(cos 1.3°)` = 0.002 dB.
- **The `keep_lows` crossover** is matched exactly by feeding the dry chain the crossover's own sum `lo + hi` (an all-pass), not the raw input.
- **`tone`** is matched by a first-order all-pass `2·lp − x` built from the same one-pole low-pass with its corner **doubled**: a first-order all-pass with corner `2ωc` has the group delay at DC of the low-pass with corner `ωc` (`2/ω_a = 1/ω_c`), so the two agree where it matters most; the phase differs by a few degrees above `tone`, where the wet path is already attenuated.

**`mix` 0 passes the input bit for bit**: when `mix` is 0 for a whole block (and the fade below is at 0) the voice is not processed at all; the states are cold and the bus is untouched (`mix_zero_is_the_input_bit_for_bit_and_costs_nothing`, `mix_zero_leaves_the_bus_untouched_in_every_mode`, and `mix0_16v` in the bench costs 4 instructions per voice-frame). When `mix` leaves 0, the states are primed at the steady state of the block's first sample and the output fades from the raw input to the blend over 8 blocks (256 frames, 5.3 ms at 48 kHz, `FADE_STEP` = 0.125 per block): the step from the matched dry path to the raw input changes phase, never level, and the fade makes that phase change gradual. The same fade runs down when `mix` returns to 0. A fresh voice with `mix` > 0 starts at full blend (a new note has no past to fade from).

**Measured, `mix` 50 % at `drive` 0 dB, soft, a 0.01-amplitude sine (flatness through the whole module; the requirement is ±0.5 dB from 50 Hz to 10 kHz):**

| Hz | 50 | 100 | 300 | 1k | 3k | 6k | 8k | 10k | 12k |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| off, mix 50 | −0.03 | −0.01 | −0.00 | −0.01 | −0.08 | −0.34 | −0.60 | **−0.95** | −1.38 |
| x2, mix 50 | −0.03 | −0.01 | 0.00 | −0.00 | −0.02 | −0.08 | −0.15 | −0.23 | −0.34 |
| x4, mix 50 | −0.03 | −0.01 | 0.00 | 0.00 | −0.01 | −0.02 | −0.04 | −0.06 | −0.08 |

Half the droop of the wet path, in decibels, because the dry path has the right phase and unit magnitude: the blend's level is the mean of 1 and the wet gain. **At the default (x2) and at x4 the criterion holds with margin (worst −0.23 dB at 10 kHz); with oversampling off it holds up to about 7 kHz and misses at 10 kHz by 0.45 dB**, which is the ADAA droop of the wet path and cannot be matched by a phase-only dry path. This is a limit, not a bug ([Limits](#limits-and-decisions)); the test (`mix_half_is_flat_below_the_knee`) asserts ±0.5 dB at x2 and x4.

## Controls, segments and state

The engine renders 32-frame segments. At the end of each segment the module converts the six modulatable values (`drive`, `bias`, `bits`, `rate`, `tone`, `mix`) to the values the block uses (`Ctl`: the drive's linear gain by one `exp2`, the bias offset, the bit levels by one `exp2`, the hold step, the tone's `tan` and `mix`) and caches them per voice, so **a static setting costs no transcendental**. The block interpolates each linearly per sample from the previous segment's `Ctl` to the new one: no zipper noise on a modulated `drive` and no step on an automation jump (`a_drive_change_ramps_from_the_previous_segment`). When the previous and new values are bit-identical the block takes a static path with no interpolation at all (the usual case; interpolating equal endpoints adds exact zeros, so the bits are the same, and `static_values_are_the_dsp_core_bit_for_bit_in_every_mode` pins the module to the core). `mode`, `oversample` and `keep_lows` come from the module's base values (they have no modulation source) and rebuild the `Setup` only when one changes.

The per-voice state is about a kilobyte (`Voice`), on the heap in a boxed slice of 32 voices; two `dist` modules in one chain are two modules with two states (`two_modules_in_one_chain`, `two_instances_keep_separate_state`), and each voice slot is independent of the others (`voices_are_independent_and_start_voice_resets_a_slot`).

**A plan swap carries the state** (`copy_state_from`): the voices, the cached `Ctl`s and the keys are copied, so an edit elsewhere in the patch leaves a held note sounding the same. A swap that changes `mode`, `oversample` or `keep_lows` re-primes the changed parts from the next input sample ([Limits](#limits-and-decisions) has the size of the step for an `oversample` change).

**Subnormals and FTZ.** Every recursive state is flushed once per block in code; after a 40 s tail through an asymmetric curve at 4× with bias, `keep_lows`, `tone` and `mix` 80 % there are zero subnormal samples and zero subnormal states, and every voice ended; the same holds for the bitcrusher and the downsampler (`a_40_s_tail_through_dist_has_no_subnormals_and_every_voice_ends`). The render is bit-identical under flush-to-zero and denormals-are-zero set on the CPU (`ftz::the_render_is_the_same_under_every_ftz_mode`), and `process` allocates nothing (`process_does_not_allocate`, under the no-alloc guard).

## Aliasing

The shared harness ([aliasing.md](aliasing.md)): an `osc` sine at the engine's default level through one `dist` at `drive` 24 dB (the default `mode` curve per row, `bias` 0, `tone` open, `mix` 100 %), at six keys from 110 Hz to 3.52 kHz (keys 45 to 105: at key 105 the fifth harmonic is already at 17.6 kHz), `Measure::REPORT_02` (alias power over harmonic power, 20 Hz – 20 kHz). The engine's sine measures −93 dB or lower on its own, the floor of the measure. Measured 2026-10-26, `cargo test -p enth-engine --test dist --release -- --nocapture` (`each_shape_aliases_below_its_recorded_bound_at_every_oversampling`); dB, lower is cleaner:

| Hz | soft off | soft x2 | soft x4 | hard off | hard x2 | hard x4 | asym off | asym x2 | asym x4 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 110 | −100.3 | −100.4 | −100.4 | −68.7 | −91.3 | −99.6 | −98.3 | −100.3 | −100.3 |
| 220 | −97.0 | −97.0 | −97.0 | −60.4 | −82.9 | −95.1 | −78.3 | −97.0 | −97.0 |
| 440 | −72.5 | −93.8 | −93.8 | −49.2 | −71.8 | −88.8 | −57.4 | −92.2 | −93.3 |
| 880 | −46.0 | −91.9 | −90.3 | −41.6 | −65.2 | −83.3 | −40.6 | −74.6 | −84.3 |
| 1760 | −33.5 | −65.2 | −78.0 | −31.2 | −61.8 | −73.2 | −31.6 | −56.9 | −74.6 |
| 3520 | −27.3 | −48.4 | −70.4 | −26.5 | −44.9 | −69.4 | −26.6 | −45.0 | −64.6 |

| Hz | sine fold off | sine fold x2 | sine fold x4 | triangle off | triangle x2 | triangle x4 |
| --- | --- | --- | --- | --- | --- | --- |
| 110 | −95.3 | −95.3 | −95.3 | −50.8 | −73.3 | −89.3 |
| 220 | −94.6 | −94.6 | −94.6 | −44.6 | −64.3 | −81.3 |
| 440 | −94.2 | −94.2 | −94.2 | −29.9 | −53.3 | −71.3 |
| 880 | −95.3 | −95.3 | −95.3 | −30.6 | −47.6 | −63.8 |
| 1760 | −14.9 | −67.9 | −63.7 | −7.1 | −38.1 | −47.9 |
| 3520 | −5.6 | −46.8 | −44.8 | −10.5 | −13.3 | −40.0 |

**The recorded bounds** (each test asserts the worst point of its sweep plus 1.5 dB, `BOUNDS_24_DB` in `crates/enth-engine/tests/dist.rs`):

| Shape, at 24 dB | off | x2 | x4 |
| --- | --- | --- | --- |
| soft | −25.8 | −46.9 | −68.9 |
| hard | −25.0 | −43.4 | −67.9 |
| asymmetric | −25.1 | −43.5 | −63.1 |
| sine fold | −4.1 | −45.3 | −43.3 |
| triangle fold | −5.6 | −11.8 | −38.5 |

**Oversampling measurably lowers aliasing** (`oversampling_lowers_aliasing`): the mean over the six keys, dB.

| Shape | off | x2 | x4 |
| --- | --- | --- | --- |
| soft | −62.8 | −82.8 | −88.3 |
| hard | −46.3 | −69.7 | −84.9 |
| asymmetric | −55.5 | −77.6 | −85.7 |
| sine fold | −66.7 | −82.4 | −81.3 |
| triangle fold | −28.9 | −48.3 | −65.6 |

The test asserts that 2× and 4× are each at least 10 dB below off for every shape, and that 4× is at least 5 dB below 2× for every shape except the sine fold. **The sine fold at 24 dB is the exception**: its harmonics (Bessel functions, significant out to about the order of the drive's linear gain, 16) reach about 16 times the fundamental, 56 kHz at the top key, without decaying, so at either factor the floor is the half-bands' stopband (70 dB and 76.5 dB) summed over the images that land in band, and 4× does not improve on 2× there (−44.8 against −46.8 dB at the top key).

**The defaults are clean at a moderate drive** (`the_defaults_are_clean_at_moderate_drive`): at `drive` 12 dB and 2×, the worst of the sweep is −89.9 dB (soft), −69.9 dB (asymmetric) and −92.7 dB (sine fold), asserted at −88.4, −68.4 and −91.2 dB.

**The triangle fold is the hardest case**: every fold is a corner, and a corner's spectrum decays only as `1/n²`, so ADAA-1 and oversampling help but cannot make 24 dB of drive clean at high pitches (−13.3 dB at 3.5 kHz at 2×, −40 dB at 4×). It is a character setting, and 4× is the right choice for it at high drive.

**The converters alias on purpose** (a 4-bit crush and an 8 kHz downsample of the engine's sine, 2× set but not used; recorded, and asserted only to be finite and not silent):

| Hz | 110 | 220 | 440 | 880 | 1760 | 3520 |
| --- | --- | --- | --- | --- | --- | --- |
| bitcrush, 4 bits | −37.8 | −34.4 | −31.6 | −27.9 | −27.6 | −27.1 |
| downsample, 8 kHz | −32.6 | −26.5 | −20.5 | −14.4 | −8.0 | −0.5 |

The crusher's number is the quantisation error that falls between the harmonics: it grows with pitch as the error spreads over more of the band (−38 dB at 110 Hz, −27 dB above 1 kHz); the downsampler's is the images of the hold, which reach 0 dB when the sine approaches the held rate (3.5 kHz against 8 kHz).

## Cost

Measured with gungraun (callgrind) on x86_64 Linux with the pinned toolchain, 2026-10-26: one second at 48 kHz (1,500 segments) of the held saw chord through one `dist`, instructions in total. The `engine` bench (the skeleton with `osc` and `adsr`: **3,628,657** instructions for 1 voice and **37,286,072** for 16) is subtracted to read the module's own cost per voice per output frame. The budget is the measured count plus 10 %, rounded up to 1,000 (`crates/enth-engine/benches/dist.rs`):

| Case | Setting | Instructions | Budget | Module, per voice-frame |
| --- | --- | --- | --- | --- |
| `default_1v` | catalogue defaults (soft, 6 dB, x2), 1 voice | 26,625,007 | 29,288,000 | 479 |
| `default_16v` | the same, 16 voices | 402,316,281 | 442,548,000 | 475 |
| `x4_16v` | x4 | 707,592,768 | 778,353,000 | 873 |
| `off_16v` | oversampling off (ADAA-1 alone) | 210,444,421 | 231,489,000 | 225 |
| `mix0_16v` | `mix` 0 (the bypass) | 40,482,383 | 44,531,000 | 4 |
| `full_16v` | x4, sine fold 24 dB, bias 20 %, `keep_lows` 200 Hz, `tone` 6 kHz, `mix` 70 % | 617,058,010 | 678,764,000 | 755 |

At 16 voices the default is 365 million instructions per second of audio (7,600 per frame) and x4 is 670 million; `mix` 0 costs the engine's call and nothing else. The per-sample cost is dominated by the tanh pair: each oversampled sample evaluates one `tanh` and one `ln cosh` (an `exp` and a `log`) through `libm`; the hard clip, which has no transcendental, is 201 per voice-frame at 2×. A polynomial replacement of the `exp` and `log` was tried and measured 7 % cheaper, which is not worth a second implementation of the curve. Instruction counts do not see subnormal penalties or cache effects; the subnormal tests stay mandatory, and no wall-clock time was measured.

## Limits and decisions

1. **`mix` 50 % is flat within ±0.5 dB from 50 Hz to 10 kHz at 2× and 4×, not with oversampling off** (−0.95 dB at 10 kHz, within 0.5 dB up to about 7 kHz). The droop is the first-order ADAA's `cos(ω/2)` at the base rate, half of it in the blend; a phase-only dry path cannot match a magnitude loss, and a compensating filter on the wet path would add another stage of the same kind it is meant to hide. Off is the setting for a lo-fi or CPU-lean sound, where the top octave is not the point.
2. **`keep_lows`: the band below the corner is within 0.1 dB up to a sixth of the corner at 24 dB of drive** (60 Hz at 400 Hz) and +0.5 dB at a quarter of it, because the LR4 high band's tail is amplified by the drive before the curve ([`keep_lows`](#keep_lows-the-crossover)). At lower drives it is flatter; at 0 dB the sum is the all-pass.
3. **The DC blocker is primed to the first sample**, so a wave that starts away from its mean (a saw beginning at its extreme into a hard clip) carries the difference as a decaying offset, up to about 0.3 of the curve's peak in the worst case, gone in about 100 ms (four time constants of 32 ms). A DC blocker whose corner is low enough to leave a bass untouched cannot be faster.
4. **Changing `oversample` while a note sounds is an instant swap, as for every structural parameter** ([engine.md](engine.md)): the plan swaps without a crossfade, the voices keep their filter states and the new oversampler is primed to the next input sample. In the test (a three-note chord through an asymmetric curve at 18 dB, swapped at segment 200) the swap segment differs from a fresh render of the new setting by up to 0.81 (full scale 1) for x4 to x2 and 0.82 for x4 to off, and the difference is 4e-5 over the last 67 ms of the 267 ms that follow (`the_plan_swap_carries_dist_state`); the old and new paths differ in group delay (2.3 against 3.3 samples at 100 Hz) and in aliasing, so expect a tick on a loud sustained note. A swap that keeps `oversample` is bit-identical to no swap. The catalogue's "a factor change never clicks" is true of the *parameter* (nothing smooths or modulates it); the swap is a step.
5. **The output can exceed the curve's limit** because the half-bands, the DC blocker and the tone filter are linear and band-limit the clipped wave: a saw into the soft clip at 18 dB reaches +2.6 dBFS before the master. Nothing clips inside the module (the bus is `f32`); the master limiter and the engine's output stage see it.
6. **The sine fold at extreme drive is not better at 4× than at 2×** ([Aliasing](#aliasing)), and the triangle fold aliases the most of all the shapes; both are properties of their spectra.
7. **The converters are not band-limited**: bitcrush and downsample take the base-rate signal, and their aliasing is the sound. `drive` and `bias` still apply before their clamp to ±1.
8. **The wet path is half a sample late on `off`** (the ADAA's delay) and a few samples late at 2× and 4× (group delays above); the dry path copies the phase of all of it to within 2.6° from 50 Hz to 12 kHz. The fit is not made above 12 kHz.

Decisions taken here, which the table did not fix: ADAA stays on at 2× and 4× (above); the DC blocker at 5 Hz (a higher corner colours a bass, a lower one takes longer to settle); the curves are normalised to slope 1 at 0 and a peak of 1, the asymmetric curve having limits +1 and −½; `bias` is added after `drive` as `bias/100` of the curve's unit level; the 8-block fade when `mix` leaves 0; the cold bypass at `mix` 0; ADAA at the oversampled rate with the midpoint fallback at 1e-5; `tone` bypassed at 20 kHz with its state following; the `bits` fraction by `exp2(bits − 1)`; the hold step capped at 1 so `rate` above the engine's rate is the identity; the half-band orders and edges.

Listening files for these (a saw and a growl through each shape and factor) are in the lane's scratch directory outside the repository, with the patches that made them.

## Files

| What | Where |
| --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/dist.rs` (lane CAT's F1m table, [Parameters](#parameters-lane-cat-f1m)); the module reads `DistParam` through `crates/enth-engine/src/params.rs` |
| DSP core | `crates/enth-dsp/src/dist.rs`: `Mode`, the five `Curve`s, `Adaa`, `Shaper`, `Setup`, `Ctl`, `Voice`; 19 unit tests and one ignored printer for this page (`print_response_tables`) |
| Oversampler | `crates/enth-dsp/src/resample.rs`: `Allpass`, `HB_STAGE1`/`HB_STAGE2`, `Oversampler`, `PhaseChain` and its constants; the designer (`design::alphas`) and its checks are in its 8 tests, and `print_halfband_design` (ignored) reprints the coefficients |
| Engine module | `crates/enth-engine/src/modules/dist.rs`: `Dist` (the `VoiceProcessor`), 8 unit tests |
| Engine tests | `crates/enth-engine/tests/dist.rs`: 9 tests (aliasing bounds, oversampling lowers aliasing, moderate drive, converters, chain of two, tail, FTZ, plan swap, and `process` under the no-allocation guard, which runs in debug builds) |
| Instruction budget | `crates/enth-engine/benches/dist.rs` |
| Golden cases | `tests/golden/cases/dist-*.toml`: `soft-default`, `hard-x4`, `asym-bias`, `sine-fold`, `triangle-fold-off`, `bitcrush`, `downsample`, `keep-lows`, `mix-half`, `drive-lfo`, `chain` |

## Acceptance: how each row is met

| Row (phase-s2.md, every module lane; then lane DST's own) | How | Where |
| --- | --- | --- |
| Same bits on six targets | `f64` with `util::math`, fixed order, no fused operations; 11 golden cases, equal on x86_64 Linux (`cargo xtask golden`) and on `wasm32-wasip1` under wasmtime (`cargo xtask golden --wasm`, 2026-10-26); the Windows and macOS targets run in CI | the goldens |
| FTZ-independent | every recursive state flushed in code per block; the render is the same bits under every FTZ/DAZ setting | `ftz::the_render_is_the_same_under_every_ftz_mode` |
| Zero subnormals after a 40 s tail | 0 subnormal output samples and states for three modes, every voice ended | `a_40_s_tail_through_dist_has_no_subnormals_and_every_voice_ends` |
| Latency 0 | the oversampler is causal IIR; the module reports none | [Zero-latency IIR oversampling](#zero-latency-iir-oversampling) |
| Budget within +10 % | six cases ([Cost](#cost)) | `benches/dist.rs` |
| Two instances keep separate state | two modules in one chain, and two instances in the unit tests | `two_modules_in_one_chain`, `two_instances_keep_separate_state` |
| `mix` 0 passes the input bit for bit | not processed at all; every mode | `mix_zero_is_the_input_bit_for_bit_and_costs_nothing`, `mix_zero_leaves_the_bus_untouched_in_every_mode` |
| Aliasing measured spectrally with a recorded bound | the bounds above, asserted at the worst plus 1.5 dB | `each_shape_aliases_below_its_recorded_bound_at_every_oversampling` |
| Provenance and a `THIRD_PARTY_NOTICES.md` entry per port | nothing is ported: no entry | the top of this page |
| Static transfer curve matches the formula at DC within 1e-6 | 2.0e-15 | `a_constant_input_returns_the_static_curve` |
| Fold thresholds documented | [The curves](#the-curves) | `folds_fold_at_their_thresholds` |
| `bias` makes even harmonics | soft at 12 dB, 440 Hz, 0.6 amplitude: the second harmonic is −209.7 dB (numerical zero) at bias 0 and −18.6 dB at 35 % (the third is −14.0 and −14.8 dB) | `bias_makes_even_harmonics` |
| `tone` follows its filter within 0.5 dB at three frequencies | three corners (500 Hz, 2 kHz, 4 kHz) at three frequencies each: within 0.30 dB (the 8 kHz point, which includes the ADAA droop), 0.07 dB or better at and below the corner's own frequency | `tone_follows_its_one_pole_response` |
| `keep_lows` passes the band below the crossover within 0.1 dB | 0.013 dB at a tenth of the corner, 0.065 dB at a sixth, at 24 dB of drive | `keep_lows_passes_the_low_band_and_distorts_the_rest` |
| `mix` 50 % flat ±0.5 dB, 50 Hz – 10 kHz, drive 0 dB | x2 and x4 within 0.23 dB; off −0.95 dB at 10 kHz ([Limits](#limits-and-decisions), 1) | `mix_half_is_flat_below_the_knee` |
| Aliasing sweep per shape at 1×, 2×, 4×, with oversampling measurably lowering it | the tables above | `oversampling_lowers_aliasing` |
| Two `dist` modules in one chain keep separate state | the pair differs from either alone, and swapping their settings changes the sound | `two_modules_in_one_chain` |

## Sources

- S. Parker, V. Zavalishin and E. Le Bivic, *Reducing the aliasing of nonlinear waveshaping using continuous-time convolution*, Proc. DAFx-16, 2016 (the first-order antiderivative quotient and its fallback).
- S. Bilbao, F. Esqueda, J. D. Parker and V. Välimäki, *Antiderivative antialiasing for memoryless nonlinearities*, IEEE Signal Processing Letters 24(7), 2017.
- M. Valenzuela and A. G. Constantinides, *Digital signal processing schemes for efficient interpolation and decimation*, IEE Proc. G 130(6), 1983 (the two-path all-pass half-band and its polyphase form).
- A. Antoniou, *Digital Signal Processing* (2005); L. Gazsi, *Explicit formulas for lattice wave digital filters*, IEEE Trans. CAS 32(1), 1985; M. Lutovac, D. Tosic and B. Evans, *Filter Design for Signal Processing Using MATLAB and Mathematica* (2001): the elliptic half-band and the relation of its poles to the all-pass coefficients.
- M. Abramowitz and I. Stegun, *Handbook of Mathematical Functions*, 16.4 (the descending Landen transformation for the Jacobi functions) and 17 (the complete integrals).
- S. Linkwitz, *Active crossover networks for noncoincident drivers*, JAES 24(1), 1976 (the fourth-order Linkwitz-Riley alignment).
- A. Simper (Cytomic), *Linear trapezoidal integrated state variable filter*, 2013; V. Zavalishin, *The Art of VA Filter Design*, 2018 (the trapezoidal one-pole and state-variable forms; the same equations as [svf.md](svf.md)).
- Research report 05 §2 and §7, `_local/sessions/2026-10-25-s2-depth-and-fast-training/`.

None of these is a source of code; each contributes equations, and the page above writes out the ones the module uses.
