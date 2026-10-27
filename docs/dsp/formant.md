# Formant filter (`formant`; `enth-dsp::formant`, `enth-engine::modules::formant`)

- **Status: implemented (S2a lane FLT, wave 3).** Kind `formant`, discriminant 11, section `voice_fx`, id prefix `formant` (formant1, formant2…), rate voice. Fixed and persisted from F0m; still hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- Code: `crates/enth-dsp/src/formant.rs` (the filter, the closed-form response, white-box tests), `crates/enth-engine/src/modules/formant.rs` (the per-voice module). Catalogue entry: `crates/enth-patch/src/catalogue/formant.rs`. Engine-level acceptance tests: `crates/enth-engine/tests/flt.rs`. Bench: `crates/enth-engine/benches/formant.rs`. Goldens: `tests/golden/cases/formant-*.toml`.
- A formant (vowel) filter per voice: three resonant band-passes in parallel, at the two formants the user sets in Hz (`f1`, `f2`) and a fixed third at 2.7 kHz, all moved together by `shift`, of one Q from `resonance`; `mix` with the dry signal. A vowel is a point in the (`f1`, `f2`) plane; a vowel path is one modulator routed to both.
- Sources: K. N. Klatt, "Software for a cascade/parallel formant synthesizer", *JASA* 67(3), 1980 (the parallel branch, its alternating signs and a third formant near 2.5–3 kHz); report 04 §1 and Catford 2001 for the vowel table (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`); research report 05 §5 for the choice of kind; A. Simper (Cytomic, 2013) and V. Zavalishin (*The Art of VA Filter Design*) for the TPT state-variable section, as in [svf.md](svf.md). **Provenance: written from those equations; no code ported, no third-party source consulted, so no `THIRD_PARTY_NOTICES.md` entry.** The section's coefficients come from `svf::Coeffs::new`; its integrator update is the svf's three lines, inlined for the three sections.

## Parameters (lane CAT, F1m)

The table lane FLT implements, generated from `crates/enth-patch/src/catalogue/formant.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Two formant frequencies, not a vowel knob:** F1 (openness) and F2 (frontness) in Hz place any vowel; a straight move between vowels is one modulator routed to both at two depths. No single vowel path makes report 04's moves (a "yoi" swings F2 while F1 stays low; "a → o" lowers both). The vowel table is the knob settings: i 240/2400, e 390/2300, a 850/1610, ɑ 750/940, o 360/640, u 250/595 (Catford 2001, report 04 §1). FLT's acceptance check: peaks at these settings ([Peaks](#peaks-on-report-04s-vowels)).
- `shift` moves F1, F2 and a fixed third formant (2.7 kHz) together; `resonance` sets the peaks' Q; coefficients once per segment (interpolated per sample, below).
- **Defaults:** a gentle useful setting, the vowel "a" at a moderate resonance (a vowel filter fully wet is never transparent).

### `formant` — Formant filter (voice_fx, ids `formant1`, `formant2`…; 5 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `f1` | hz | 100Hz … 1kHz | log | 850Hz | mod, auto | basic | The first formant: how open the vowel is, from 'ee' and 'oo' (about 250 Hz) to 'ah' (about 850 Hz). |
| `f2` | hz | 400Hz … 4kHz | log | 1.61kHz | mod, auto | basic | The second formant: how far forward the vowel is, from 'oo' (about 600 Hz) through 'ah' (1.6 kHz) to 'ee' (2.4 kHz); sweep it for 'yoi' and 'wow'. |
| `shift` | semitones | -24st … +24st | linear | 0st | mod, auto | intermediate | Moves every formant up or down together, keeping the vowel: a smaller, brighter voice or a bigger, darker one. |
| `resonance` | percent | 0% … 100% | linear | 50% | mod, auto | intermediate | How narrow and strong the formant peaks are: low is a soft, breathy vowel, high a sharp, whistling one. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged; lower values keep some of the unfiltered body. |

## The filter

```text
y = M(Q) · ( 1·bp(f1') − 1·bp(f2') + ½·bp(f3') )         f_i' = f_i · 2^(shift/12),  f3 = 2.7 kHz
Q = 2 · 25^(resonance/100)        M(Q) = 1.5 · √Q        bp(f) = the unity-peak 2-pole band-pass at f, k = 1/Q
out = (1 − mix)·x + mix·y
```

- **Each `bp` is the TPT state-variable filter of [svf.md](svf.md)** (two trapezoidal integrators in a zero-delay loop, band output `k·v1`), sharing one Q. Its response is the bilinear transform of `k·s/(s² + k·s + 1)` with the centre pre-warped by `tan`, so a section is exactly where it is set up to 0.45·fs, with gain 1 there, and its −3 dB bandwidth is `f/Q`.
- **Constant Q, on purpose.** Every section has the same width relative to its centre, so `shift` moves the whole response along the log-frequency axis and keeps the vowel (a test checks every peak is at `f·2^(shift/12)` within 1.5 %). The price: a real vowel's formant bandwidths are closer to constant in Hz (about 60–150 Hz), so at the default Q of 10 the "i" has a narrow F1 (24 Hz) and a wide F2 (240 Hz), where speech has about 60 and 100. Resonance 50 % lands F1 of an open "a" (850 Hz) at 85 Hz, which is speech-like.
- **Alternating signs (Klatt).** F1 and F3 add, F2 subtracts. A band-pass is in phase at its centre, +90° below and −90° above; two neighbours of the same sign cancel between them (a zero, 40–50 dB deep for "i" and "e"), and of opposite signs they fill the valley to 20–25 dB, which is what an all-pole vocal tract does. The sign is by role, not by order, so it stays right when `f2` crosses the third formant: with `f2` from 2.6 to 4 kHz the peak stays within about 2 % of `f2` at Q 5 and 10, and the fixed 2.7 kHz formant shows as a smaller peak beside it (computed, not asserted).
- **The third formant** sits at 2.7 kHz before `shift` and has half the weight (−6 dB), so it colours the top without taking the F2 peak anywhere (with it, "i" and "e" F2 are +0.8 % and +0.6 % high at Q 10; without it −0.05 % and −0.09 %). Its level and width are FLT's refinement of CAT's text ("lane FLT may refine").
- **Range of the centres:** 20 Hz … 0.45·fs after `shift` (`svf::Coeffs::new`'s clamp); `shift` is clamped to ±48 semitones in the core (the catalogue's maximum is 24); a NaN centre becomes 20 Hz and a NaN shift 0.

### Resonance, Q and bandwidth

`Q = 2 · 25^(resonance/100)`: exponential, so equal knob steps are equal steps in the height of the peaks (and the make-up below). Measured on one isolated section at 1 kHz (the others weighted 0, `a_sections_bandwidth_is_its_centre_over_q`): the −3 dB bandwidth is `f/Q` within 0.3 %.

| resonance | 0 % | 25 % | 50 % (default) | 75 % | 100 % |
| --- | --- | --- | --- | --- | --- |
| Q | 2.0 | 4.47 | 10.0 | 22.4 | 50.0 |
| bandwidth as a fraction of `f` | 1/2 | 1/4.5 | 1/10 | 1/22 | 1/50 |
| measured bandwidth at 1 kHz, Hz | 499.0 | 223.5 | 100.0 | 45.0 | 20.0 |
| make-up `M`, dB | +6.5 | +10.0 | +13.5 | +17.0 | +20.5 |
| default vowel "a": F1 / F2 / F3 bandwidth, Hz | 425 / 805 / 1350 | 190 / 360 / 600 | 85 / 161 / 270 | 38 / 72 / 121 | 17 / 32 / 54 |

The ring of a section is `τ = Q / (π f)` (amplitude e-fold): 4 ms for "a" F1 at the default, 0.16 s at Q 50 and 100 Hz, and at most 0.64 s at Q 50 with `f1` 100 Hz and `shift` −24 (a 25 Hz centre).

### Level

A unity-peak band-pass passes `1/Q` of a white noise's power, so a plain bank would lose 13–20 dB on a saw at the default and 25 dB or more at Q 50. The make-up `M = 1.5·√Q` cancels the narrowing (noise power through a section ∝ `1/Q`, amplitude ∝ `1/√Q`) and the constant 1.5 puts a saw near its input's level. The peaks are therefore `M` above the input (a unity-gain `bp` times `M`): +13.5 dB at the default. **The response never exceeds `M`**: a unity band-pass's response lies on the circle with diameter [0, 1] in the complex plane, so two sections of opposite sign differ by at most 1, and the third at half weight adds nothing measurable; max |H| / `M` is 1.000 over the six vowels at five resonances (test `the_peak_level_is_bounded_by_the_make_up`).

Measured from the closed form (test `the_make_up_keeps_a_harmonic_source_near_its_input_level`, six vowels, five resonances): the power gain on white noise is −9.1 … −3.2 dB, and the saw's (harmonic amplitudes 1/n):

| Saw | resonance 0 % | 50 % | 100 % |
| --- | --- | --- | --- |
| 110 Hz, vowels i / e / a / ɑ / o / u | 0.0, −2.2, −5.7, −9.3, −2.0, +0.3 dB | 0.0, −4.0, −3.8, −2.9, −0.9, −0.9 | −4.5, −6.3, −7.0, −5.1, −5.1, −7.0 |
| 220 Hz | +4.2, +0.5, −2.7, −6.3, +0.6, +3.9 | +5.5, −0.8, +0.1, −1.5, +1.9, +2.9 | +1.1, −6.2, −3.0, −6.2, −0.9, −3.3 |

So a saw keeps its loudness within about −9 … +6 dB whatever the vowel and the resonance: "near" its input, not equal; the level depends on where the harmonics fall, which is the point of a formant filter. A patch that needs it louder or quieter adds gain after it.

### Zipper-free updates

The engine runs modules once per 32-frame segment. As the svf does, the module computes the tuning for the *end* of the segment and `process_block` ramps **every section's `a1, a2, a3` and its output weight (fifteen values) linearly per sample** from the previous segment's values, plus `mix`; a sweep of `f2` never steps. Report 02 validated the ramp for the svf up to Q 25; here it holds at Q 50: the eigenvalues of the update matrix `[[2a1 − 1, −2a2], [2a2, 1 − 2a3]]` stay inside the unit circle along the straight line between any two coefficient sets from {20, 25, 100, 400, 1,000, 4,000, 10,000, 21,600 Hz}, at Q 2 and Q 50: largest magnitude **0.999974** (test `the_coefficients_stay_stable_along_every_interpolation_path`).

- The module keeps each voice's previous tuning itself, so a retrigger or a smoother reset is a 32-frame glide, not a step; a fresh voice starts at its first segment's values (`from = to`).
- The four inputs (`f1`, `f2`, `shift`, `resonance`) are the cache key. When they are bit-identical to the previous segment's, nothing is recomputed: **a static vowel costs no transcendental**. A recompute is two `exp2` and three `tanf`.
- When the old and new tunings are equal the ramp is skipped (adding exact zeros is bit-identical).
- `mix` 0 at both ends of a segment leaves `io` untouched: the input passes **bit for bit**, while the filter keeps running so that raising `mix` finds its state current. `start_voice` zeroes the voice's six integrators.
- The states are flushed (below 1e-15 to exactly 0, in code, not by FTZ/DAZ flags) once per block.
- **Plan swap:** `copy_state_from` copies the six floats per voice, the cached tunings and their keys, so a swap that keeps a formant keeps its ring.

## Peaks on report 04's vowels

Acceptance: peaks within 3 % of the table for at least five vowels, set through `f1` and `f2`. The location of the local maxima of the **closed-form response** (`enth_dsp::formant::response`, from the filter's own coefficients, on a 24,000-point log grid, 20 Hz–20 kHz) against the table, error in percent of the nearest maximum, F1 / F2:

| Vowel (F1/F2 Hz) | resonance 25 % (Q 4.5) | 50 % (default, Q 10) | 75 % (Q 22) | 100 % (Q 50) |
| --- | --- | --- | --- | --- |
| i 240/2400 | +0.13 / +0.85 | +0.02 / +0.80 | +0.02 / +0.19 | −0.01 / +0.04 |
| e 390/2300 | +0.25 / +1.29 | +0.05 / +0.59 | +0.02 / +0.13 | −0.01 / +0.04 |
| a 850/1610 | +1.39 / −0.84 | +0.29 / −0.15 | +0.05 / −0.04 | 0.00 / −0.01 |
| ɑ 750/940 | +7.30 / −14.4 (merged) | +1.06 / −1.05 | +0.19 / −0.20 | +0.05 / −0.05 |
| o 360/640 | +1.97 / −1.80 | +0.37 / −0.35 | +0.08 / −0.06 | +0.02 / −0.01 |
| u 250/595 | +1.18 / −1.00 | +0.22 / −0.20 | +0.05 / −0.03 | +0.02 / +0.00 |

- **At the default resonance and above, all six vowels are within 1.1 %** (asserted below 1.5 %); at 25 % five are within 2 % and "ɑ", whose two formants are a quarter apart, merges toward its middle (asserted: at least five within 3 %).
- **Below 25 % the peaks are humps, not peaks.** At 0 % (Q 2) the sections are half their centre wide and neighbours pull each other 4–12 % toward the middle (computed: i −3.7 % on F2, a +6.8 / −10.1 %, ɑ +9.6 / −12.5 %, o +11.8 / −11.7 %, u +6.3 / −5.6 %). That is the intended "soft, breathy vowel"; the acceptance is stated for 25 % and above.
- The same check **through the module** (the spectrum of the module's impulse response, peaks nearest the table): i +0.10 / +0.78, e 0.00 / +0.60, a +0.30 / −0.14, ɑ +1.07 / −1.05, o +0.40 / −0.38, u +0.20 / −0.17 percent (`report_04s_vowels_peak_where_f1_and_f2_say_through_the_module`).
- **The measured response is the closed form**: the impulse response's spectrum against `response` at every bin from 60 Hz to 20 kHz above −50 dB of the peak, for the six vowels at 0, 50 and 100 %: worst 0.0013 dB (asserted 0.05). Through the engine, a sine at four keys on each of the six vowels, "a" at 0 % and 100 %: the gain is the closed form's within 0.05 dB (`the_formant_gain_is_the_closed_form`).

## Stability under modulation

Required: finite, bounded output and a tail that dies, with `f1`, `f2` (and `shift`, `resonance`) jumping every segment at the parameter extremes. Tests:

- **Random-jump stress** (`enth-dsp`): white noise (±0.5) in; every segment a new random `f1` (100 Hz … 1 kHz, log-uniform), `f2` (400 Hz … 4 kHz), `shift` (±24), resonance (random, and fixed at 100 %) and mix, for 30,000 segments (20 s, a 1.5 kHz modulation of every parameter): every sample finite, the peak output 0.98 (random Q) and 0.66 (Q 50) for an input of ±0.5 (asserted below 4), the integrators' peak 3.0 and 4.2. Then the parameters park on the "a" at resonance 100 % with silence in: after 30 s no subnormal appeared on the way and the state is exactly zero.
- **Module stress** (`modules/formant.rs`): the same through the engine module, 15,000 segments, the same ranges; finite, peak below 4, then the same park-and-die-out check.
- **Audio-rate LFOs** (`tests/flt.rs`): LFOs at 50, 37, 43 and 29 Hz at full depth on `f1`, `f2`, `shift` and `resonance` (resonance 90 % base) on two overlapping saws for 10 s: the peak is 1.98 against 1.36 for the dry saw (asserted below 3× dry).
- **Retune** (`a_retune_across_one_block_has_no_click`): a 300 Hz sine through the filter retuned from "i" to "o" across one block steps by at most 0.08 against a peak of 1.75.

## Aliasing

**Bound: −91.0 dB** against the sine's fundamental (report 02's measure, `enth_dsp::alias`, [aliasing.md](aliasing.md)), enforced in `tests/flt.rs::a_formant_adds_no_aliasing`. The filter is linear: it adds nothing but the engine's own sine floor, shaped by the response. The measure divides alias power by harmonic power, so the sine has to be at a peak, not in a valley (a sine 40 dB down a skirt would read the floor 40 dB higher without any aliasing): for each key the test sets `f1` to the sine's frequency (clamped to 100 Hz … 1 kHz) and `shift` to bring it exactly onto the peak, so the fundamental is `M` high. Keys 33 … 105 (55 Hz … 3.5 kHz), at resonance 50 %, 100 % (with 2.5 s of pre-roll: Q 50 at 100 Hz rings for 0.16 s per e-fold) and 0 %:

| Sine | 55 Hz | 110 Hz | 220 Hz | 440 Hz | 880 Hz | 1,760 Hz | 3,520 Hz |
| --- | --- | --- | --- | --- | --- | --- | --- |
| resonance 50 %, dB | −93.2 | −103.3 | −97.8 | −93.3 | −95.5 | −92.7 | −119.4 |
| resonance 100 %, dB | −93.3 | −103.3 | −97.7 | −93.3 | −95.5 | −92.7 | −118.3 |
| resonance 0 %, dB | −93.3 | −103.5 | −97.8 | −93.4 | −95.5 | −92.6 | −119.7 |

Worst −92.6 dB, the same floor as the bare sine ([aliasing.md](aliasing.md) records −93.3 … −119.8 for it); the bound is set 1.6 dB above it. Fast modulation does not alias either: the ramp is a linear interpolation of coefficients of a linear filter, not a nonlinearity.

## Determinism

Transcendentals (`exp2` for the shift and the Q, `tanf` for the centres) run per voice and changed segment through `enth_dsp::util::math` and the pinned `libm`; the per-sample code is `+ − ×`, with no `mul_add`. Tests: the same bits under every FTZ/DAZ mode (`a_formant_renders_the_same_bits_under_every_ftz_mode`), zero subnormals in the output or any engine state over a 40 s render of four notes through two formants in series (resonance 100 % at 25 Hz, and the "i"), with a 3 s and a 20 s release, every voice below −100 dB at the end (`a_40_s_tail_through_formants_holds_no_subnormals`), no allocation in `process` (`a_formant_never_allocates_in_process`), two instances keep separate state (`two_formants_keep_separate_state`, `modules/formant.rs`). **Latency 0**: the output sample uses the input sample directly (`h[0] = Σ weight_i · a2_i ≠ 0`); no look-ahead, no delay line. The same bits on six targets are the CI golden job's; the seven `formant-*` goldens are the pin.

## Goldens

| Case | What it pins |
| --- | --- |
| `formant-a` | a saw at 110 Hz through the default vowel: the bank, its signs and make-up, the cached tuning |
| `formant-i` | 240/2400 Hz at resonance 70 %: the two ends of the formant range, the third formant just above F2 |
| `formant-u` | 250/595 Hz: two formants a factor 2.4 apart in the low corner |
| `formant-yoi` | an envelope sweeping `f2` down 57.4 % and `f1` up 17.6 % (i → o → i): the per-sample ramps along a sweep, the log-scale route depths |
| `formant-shift` | "a" at resonance 80 % up 7 semitones: the shift ratio and the third formant's move |
| `formant-mix` | "o" at `mix` 40 %: the blend |
| `formant-whisper` | white noise through "o" at resonance 90 %: the narrow-band ring on a broadband input |

## Cost

Measured with callgrind (`cargo xtask bench -p enth-engine --bench formant`), 1 s at 48 kHz, whole engine, on the build of 2026-10-26; the "marginal" column subtracts the `engine` bench of the same build with the same voices (3,621,181 at 1 voice, 37,278,521 at 16: the reference saw and the ADSR are in both):

| Case | Instructions | Budget (+10 %) | Marginal per sample per voice |
| --- | --- | --- | --- |
| `static_1v` | 6,068,771 | 6,676,000 | (fixed costs dominate at 1 voice) |
| `static_16v` | 74,135,421 | 81,549,000 | 48.0 |
| `moving_16v` (an `f2` event every segment) | 110,086,193 | 121,095,000 | 94.8 |

Three sections cost 48 instructions per sample per voice against the svf's 28 for one. A moving formant adds the ramp of fifteen values per sample and, per voice and segment, a new tuning (three `tanf`, one `exp2` for the Q and one for a nonzero shift): the extra 47 per sample together.

**Stereo pairs (lane PRF, 2026-10-27).** As for the [ladder](ladder.md): `formant::process_lanes` runs up to four instances (two voices × left and right) in the lanes of an `f32x4`, each performing `process_block`'s operations in the same order on its own state, so every bit equals the instance alone (`instances_in_lanes_are_process_block_bit_for_bit`, `process_pair_is_two_process_calls_bit_for_bit`). Instances whose blocks differ in ramp or blend run one after the other. On the stereo-forced bench (16 voices, 1 s) the static formant went from 112,252,367 to 88,690,335 instructions (−21.0 %); in the G2 reference patch −26.7 M of 769 M. The mono path is unchanged within 0.3 %.

## Behaviour other lanes must know

- **The peaks are `M` above the input** (+13.5 dB at the default), and a saw keeps roughly its loudness (previous section). A patch that stacks `formant` after a hot oscillator and a resonant filter can exceed 0 dBFS; the module has no limiter.
- **`mix` 100 % is never transparent**: below 25 % resonance the filter is a broad hump, above it a narrow ring; `mix` below 100 % keeps the dry body.
- **A voice start cuts the ring**: `start_voice` zeroes the integrators.
- **The `f2` range reaches above the fixed third formant** (2.7 kHz): at `f2` above it the third remains a smaller peak below `f2`, and `shift` moves it with the rest.
- **`f1` and `f2` are independent, so `f1 > f2` is possible** (it swaps the roles, and the alternating signs follow the roles): the response is still bounded by `M`.

## Files

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/formant.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/formant.rs` | implemented, 13 white-box tests |
| Engine module | `crates/enth-engine/src/modules/formant.rs` | implemented, 8 module tests; the dispatch arms are S0d's |
| Engine acceptance tests | `crates/enth-engine/tests/flt.rs` | 8 tests (aliasing, closed form, bit-exact mix 0, instances, modulation, tail, FTZ, no alloc) |
| Instruction budget | `crates/enth-engine/benches/formant.rs` | measured + 10 % ([Cost](#cost)) |
| Golden cases | `tests/golden/cases/formant-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | seven ([Goldens](#goldens)) |
| WAVs | `~/code/enth-scratch/s2/flt/wav/formant-*.wav` | the "yoi" sweep, a whispered sweep (rendered, never played) |

## Acceptance (from phase-s2.md, every module lane)

| Row | Where it is checked |
| --- | --- |
| Same bits on six targets | the seven goldens (CI) |
| FTZ-independent | `tests/flt.rs::a_formant_renders_the_same_bits_under_every_ftz_mode` |
| Zero subnormals after a 40 s tail | `tests/flt.rs::a_40_s_tail_through_formants_holds_no_subnormals` |
| Latency 0 | [Determinism](#determinism): no delay, `h[0] ≠ 0` |
| Budget +10 % | [Cost](#cost) |
| Two instances keep separate state | `tests/flt.rs::two_formants_keep_separate_state`, `modules/formant.rs` |
| `mix` 0 passes the input bit for bit | `tests/flt.rs::formant_mix_zero_renders_the_same_bits_as_no_formant`, `dsp/formant.rs`, `modules/formant.rs` |
| Aliasing measured spectrally, bound recorded | [Aliasing](#aliasing): −91.0 dB, `tests/flt.rs::a_formant_adds_no_aliasing` |
| Peaks within 3 % for ≥ 5 of report 04's vowels, bandwidths documented | [Peaks](#peaks-on-report-04s-vowels), [Resonance, Q and bandwidth](#resonance-q-and-bandwidth) |
| Stability under fast modulation of `f1`/`f2` | [Stability under modulation](#stability-under-modulation) |

## References

- K. N. Klatt, "Software for a cascade/parallel formant synthesizer", *Journal of the Acoustical Society of America* 67(3), 1980. The parallel branch and its alternating signs.
- Report 04 §1 (Catford 2001, male speech): the vowel table and the "yoi" move.
- A. Simper, *Linear Trapezoidal Integrated State Variable Filter*, Cytomic, 2013; V. Zavalishin, *The Art of VA Filter Design*, rev. 2.1. The section.
- Research report 05 §5 (filters beyond the SVF).
