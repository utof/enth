# EQ (`eq`; `enth-dsp::eq`, `enth-engine::modules::eq`)

- **Status: implemented (S2a lane EQ, wave 4).** Still hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears the kind's flag ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `eq`, discriminant 15, section `fx`, id prefix `eq` (eq1, eq2…), rate global.
- What it is: a stereo six-band parametric equaliser after the voices: a low cut, a low shelf, two bells, a high shelf and a high cut, in series, then `mix`. Each band is the response of the Audio EQ Cookbook (Robert Bristow-Johnson), realised as a trapezoidal-integrator (TPT) state-variable section, with per-segment ramps of the section's parameters. 300 instructions per frame ([Cost](#cost)), latency 0, no heap memory.
- **Provenance: written from equations; no code ported, so no `THIRD_PARTY_NOTICES.md` entry.** The responses are the cookbook's formulas, which the tests also write out independently as the reference. The realisation is the one in Andrew Simper's *Linear Trapezoidal Integrated State Variable Filter* (Cytomic, 2013, typo fix 2016), which is Vadim Zavalishin's TPT state-variable structure: the paper is public, and contributes the equations only (written out in [The section](#the-section-a-tpt-state-variable-filter) below); nothing was copied or adapted from any implementation, and in particular none from GPL code. The same equations are the ones [svf.md](svf.md) uses.
- **The main deviation from the brief: the bands are not `enth-dsp`'s `Biquad`.** The brief said "wrap the biquad with per-segment coefficient ramps". That was built first, and it fails the brief's own stability row: a ramped transposed-direct-form biquad blows up under random jumps of a bell's frequency. The bands are TPT sections instead, which give the same response and cannot pump. `biquad.rs` is untouched; its `Coeffs::design` and constants are the independent reference in the tests. The measurements are in [Why not the biquad](#why-not-the-biquad).

## Parameters (lane CAT, F1m)

The table lane EQ implements, generated from `crates/enth-patch/src/catalogue/eq.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Bands in series, in table order:** low cut, low shelf, bell 1, bell 2, high shelf, high cut. The cuts are 12 dB/octave Butterworth sections that **stay in the signal path at every value**: nothing switches them in or out at the ends of their ranges (`low_cut` 10 Hz, `high_cut` 20 kHz), because both are modulatable and a stage that appears at a range end would click as a source crosses it. At an end they are out of the way, not bypassed: `low_cut` at 10 Hz is −0.3 dB at 20 Hz and under 0.01 dB from 50 Hz up; `high_cut` at 20 kHz is −0.3 dB at 10 kHz (the analog value: the digital filter's is −0.008 dB at 48 kHz and −0.16 dB at 96 kHz, [Limits](#limits-and-decisions), 1) and −3 dB at 20 kHz itself; `mix` is the bypass. Shelves and bells are the cookbook's (within 0.1 dB, EQ's acceptance); a band at 0 dB does nothing. A bell at high Q and −24 dB is a notch.
- **Defaults:** near-transparent: every gain 0 dB, both cuts at the ends of their ranges, out of the way.

### `eq` — EQ (fx, ids `eq1`, `eq2`…; 13 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `low_cut` | hz | 10Hz … 1kHz | log | 10Hz | mod, auto | basic | Removes everything below this frequency (12 dB/octave), clearing mud or the sub from effects; at 10 Hz it is out of the way. |
| `low_freq` | hz | 20Hz … 1kHz | log | 100Hz | mod, auto | intermediate | The corner of the low shelf. |
| `low_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | basic | Boosts or cuts everything below the low shelf's corner: weight and warmth. |
| `bell1_freq` | hz | 20Hz … 20kHz | log | 400Hz | mod, auto | intermediate | The center of the first bell. |
| `bell1_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | basic | Boosts or cuts a band around the first bell's center: a cut near 300-500 Hz cleans boxiness. |
| `bell1_q` | none | 0.1 … 20 | log | 1 | mod, auto | intermediate | How narrow the first bell is: 0.7 is broad, 10 a narrow notch or peak. |
| `bell2_freq` | hz | 20Hz … 20kHz | log | 2.5kHz | mod, auto | intermediate | The center of the second bell. |
| `bell2_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | basic | Boosts or cuts a band around the second bell's center: presence and bite around 2-5 kHz. |
| `bell2_q` | none | 0.1 … 20 | log | 1 | mod, auto | intermediate | How narrow the second bell is: 0.7 is broad, 10 a narrow notch or peak. |
| `high_freq` | hz | 1kHz … 20kHz | log | 8kHz | mod, auto | intermediate | The corner of the high shelf. |
| `high_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | basic | Boosts or cuts everything above the high shelf's corner: air and sparkle, or a darker top. |
| `high_cut` | hz | 1kHz … 20kHz | log | 20kHz | mod, auto | basic | Removes everything above this frequency (12 dB/octave); at 20 kHz it is out of the way. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged. |

## Signal path

```text
bus L,R -> low cut -> low shelf -> bell 1 -> bell 2 -> high shelf -> high cut -> wet
                                                                                  |
bus L,R (dry) -----------------------------------------------------------------> mix -> bus
```

Both channels take the same coefficients; each band holds two integrator voltages per channel, and the two channels run in lock step through the same operations (a two-lane array, which the compiler turns into one vector register: the bits are those of the scalar form, which the goldens recorded first). Between the sections the signal is rounded to `f32`, a relative 6e-8 per section; state and design are `f64`, because a low band has `g` near 1e-3.

The engine calls the module once per 32-frame segment with the segment's previous and current parameter values (`GlobalParams::prev` and `cur`); the wrapper reads them in the catalogue's units and hands both to the core, which does the rest ([Segments and ramps](#segments-and-ramps)). The catalogue gives every parameter the engine's 10 ms smoother, so what arrives is already a smooth line from segment to segment; the core's ramps make it smooth inside the segment too.

## The section: a TPT state-variable filter

With `g = tan(π·f/fs)` (the corner pre-warped, so the corner lands where it should), a damping `k` and a choice of three mixing coefficients `m0, m1, m2`, one frame is (Simper's equations; `ic1`, `ic2` are the integrator voltages):

```text
a1 = 1/(1 + g(g + k))     a2 = g·a1     a3 = g·a2
v3 = in − ic2
v1 = a1·ic1 + a2·v3                      (band-pass, scaled)
v2 = ic2 + a2·ic1 + a3·v3                (low-pass)
ic1 = 2·v1 − ic1
ic2 = 2·v2 − ic2
out = m0·in + m1·v1 + m2·v2
```

The analog prototype is `(m0·(s² + k·s + 1) + m1·s + m2)/(s² + k·s + 1)` (with `s` the normalised frequency), and the bilinear transform with the corner pre-warped is exactly the filter the cookbook designs. The mapping, with `A = 10^(dB/40)`:

| Band | `g` | `k` | `m0, m1, m2` | Cookbook |
| --- | --- | --- | --- | --- |
| low cut | `tan(π·f/fs)` | `√2` | `1, −k, −1` | `HPF`, Q = 1/√2 |
| high cut | `tan(π·f/fs)` | `√2` | `0, 0, 1` | `LPF`, Q = 1/√2 |
| bell | `tan(π·f/fs)` | `1/(Q·A)` | `1, k(A²−1), 0` | `peakingEQ` |
| low shelf | `tan(π·f/fs)/√A` | `√2` | `1, k(A−1), A²−1` | `lowShelf`, S = 1 |
| high shelf | `tan(π·f/fs)·√A` | `√2` | `A², k(1−A)·A, 1−A²` | `highShelf`, S = 1 |

Python checks of these mixes against the cookbook's biquads agreed to 1e-13 dB before any Rust was written; the tests below repeat the comparison on the Rust code. The frequency is clamped to `1 Hz … 0.49·fs` and the gain to ±120 dB and the Q to `0.01 … 1000` (BQ's constants, `biquad.rs`), far outside the table; a non-finite control is replaced by its default before the design.

A band at exactly 0 dB has `m = [1, 0, 0]`: its output is its input (to the bit, but for a sample under 1e-15, which the flush takes to zero). It keeps its real `g` and `k`, so its integrators keep following the signal, and raising its gain later starts from a warm section, not from silence.

## Why not the biquad

The brief said to wrap `enth-dsp`'s `Biquad` (lane BQ) and ramp its coefficients per segment. The first version did exactly that, with `f64` state, `Coeffs::design` for the bells and shelves and the cookbook's `HPF`/`LPF` for the cuts, and a per-sample linear ramp of the five coefficients from the previous segment's design to the new one. Its **static response was perfect** (the designs equal the cookbook's to 2e-8 dB) and its **modulated behaviour was not**:

- a bell that picks 20 Hz or 20 kHz at random for every segment, +24 dB and Q 20, under full-scale noise, gave `inf` within four seconds;
- the same with 100 Hz ↔ 5 kHz, `inf`; 50 Hz ↔ 2 kHz at Q 10, 6.6e237; 20 Hz ↔ 20 kHz at +12 dB and Q 5, 1.6e7. A strict alternation did not show it (peak 4.5), a random order did;
- a low shelf with a jumping corner amplified by ten;
- with every band jumping to a random corner of the table every segment the first version's output had an rms of 62 and peaks of 26,000 at 96 kHz, against 31 for the TPT version on the same control sequence (the static rms of the settings visited has a median of 0.3 and a maximum of 22).

The mechanism is the form: the transposed direct form II holds sums of past inputs and outputs, so changing its coefficients re-reads that state with different numbers, and a high-Q resonator whose poles hop around the unit circle every 32 samples pumps energy into it. That is the failure report 05 §4 warned about ("a DF1 biquad … is the one that clicks under modulation"), and the brief's own acceptance asks for "no blow-up under audio-rate modulation of frequency, gain or Q, or at range extremes".

The TPT section keeps the two integrator voltages of an analog state-variable filter, and its state update `[[2·a1−1, −2·a2], [2·a2, 1−2·a3]]` has a largest singular value that is **exactly 1 for every `g > 0` and `k ≥ 0`**: it is a damped rotation by `2·atan g`. So no sequence of parameter changes, at any rate, can add energy to the state; the input can only add what it carries. Interpolating `(g, k, m0, m1, m2)` per sample keeps every frame a real section (`g > 0`, `k ≥ 0` along the whole line between two designs), so the guarantee holds along the ramp. A transposed biquad with ramped coefficients has no such guarantee. Same transfer function, different behaviour while the controls move; that is the whole difference.

What is kept from the biquad: its constants (`MIN_FREQ_HZ`, `MAX_FREQ_FRACTION`, `MIN_Q`, `MAX_Q`, `MAX_GAIN_DB`), and `Coeffs::design` and `Biquad` as the reference the tests compare against. `a_static_core_matches_bqs_biquads_to_rounding` runs the core and six `Biquad`s on the cookbook's coefficients on the same noise and finds no difference at all in `f32` output (the two realisations differ by about 1e-16 relative, which the rounding of every section's output hides). `the_same_ramp_through_a_biquad_pumps_where_the_tpt_section_does_not` keeps the measurement above in the tree: a local transposed-form section on `Coeffs::design` with the same per-sample ramps against the core on the identical control sequences.

| Bell, random pick of the two frequencies each segment, 4 s of full-scale noise | Biquad (ramped) peak | TPT peak | Frozen ends' peak |
| --- | --- | --- | --- |
| 20 Hz ↔ 20 kHz, +24 dB, Q 20 | inf | 6.3 | 2.4 |
| 100 Hz ↔ 5 kHz, +24 dB, Q 20 | inf | 2.9 | 3.0 |
| 50 Hz ↔ 2 kHz, +24 dB, Q 10 | 6.6e237 | 2.7 | 2.8 |

## Segments and ramps

`Eq::process(from, to, left, right)` takes the previous and the current controls. For each band:

1. the design at the start is the design of `from`'s inputs and the design at the end is the design of `to`'s (the last design is cached by its three `f32` inputs, so an unchanged band designs nothing and a static eq costs no transcendental);
2. if they are equal, the segment runs with fixed coefficients;
3. otherwise `g`, `k`, `m0`, `m1`, `m2` move by a precomputed increment per frame (`1/32` of the difference, exact because 32 is a power of two) and the last frame is snapped to the new design, whatever the increments added up to; the three coefficients `a1, a2, a3` are computed from the interpolated `g` and `k` each frame (one division).

The ramp starts from the design of *the previous controls*, not from anything the module remembers, so the output depends only on the state, the input and the two control sets. That makes **skipping a silent segment the same as running it**: when `is_silent()` holds (every integrator exactly zero) and both buses are exactly zero, a run produces zeros and leaves zeros, which is what skipping does; the cache is a pure function of its inputs, so it does not matter that a skipped segment does not refresh it (`an_idle_eq_skipped_by_the_engine_gives_the_same_bits_as_one_that_is_always_called`, with the controls moving through the silence).

**`mix`** is `wet·m + dry·(1 − m)`, with `m` ramped per frame from the previous segment's value to this one's with the engine's own table (`(i+1)/32`, exact in `f32`). At `m = 1` the output is the wet signal, as computed. At `m = 0` the dry samples are put back, so the input passes bit for bit (NaN, subnormals and `-0.0` included) while the sections keep running on it: their state follows the signal, and `mix` can come up without a transient. The case is the same hash as the render without the `eq` (`mix_zero_passes_the_engine_output_bit_for_bit`).

## Stability

Two layers. **Structural:** the state update is non-expansive for every section the table allows and every point of every ramp. The test takes the corners and middles of the table's ranges (frequencies, gains, Qs, six shelves and cuts and bells) at five sample rates from 8 to 192 kHz, plus 38,700 points on the straight lines between pairs of designs, and the largest singular value found is 1.000000000007 (the same quantity computed through the square root of a difference that is zero for a lossless section, so the 7e-12 is the check's own rounding: `every_section_the_table_allows_is_passive_and_so_is_every_point_of_a_ramp`). **Measured,** with the real module:

| Test | Setting | Result |
| --- | --- | --- |
| One control at a time between its two extremes, random every segment, 5 s of full-scale noise (`enth_dsp::eq`) | eight controls: low cut, low shelf frequency (+24 dB), low gain, bell frequency (+24 dB, Q 20), bell gain (Q 20), bell Q (+24 dB), high shelf frequency (+24 dB), high cut | the moving peak is at most **2.4 times** the louder of the two frozen ends (bell frequency); the bound is 4. The biquad: `inf` |
| Every pair of settings of one band from a grid of the table's corners and middles, alternating every segment, 1 s of noise (`enth_dsp::eq`) | frequency × {−24, 0, +24} dB × Q {0.1, 1, 20}; 27 settings of a bell, 9 of a shelf, 3 of a cut; 780 pairs in all | the worst ratio is **9.3** (a +24 dB, Q 1 bell at 20 Hz against a 0 dB, Q 20 bell at 20 kHz: the ramp passes through a narrow +21 dB bell in the middle of the segment, which is a loud setting of its own); the bound is 16. A 60 s run of that pair is stationary (peaks of 17 to 21 in every 5 s, rms flat) |
| All six bands jump to random corners of the table every segment, 20 s of full-scale noise at 32, 44.1, 48, 96 and 192 kHz (`enth_dsp::eq`) | | finite, stationary (the second half's power is within 4 times the first's), peaks 1.8e3 to 1.3e4. The cascade of four +24 dB sections is loud when frozen (the sum of the impulse response's absolute values for the loudest of 40 random corner settings is 305); every switch adds the energy of its own transient, so this is a regression guard (5e4) and not a proof |
| A 50 Hz unipolar square global LFO at full depth on one control, a chord of three saws and noise through the engine (`tests/eq.rs`) | the same eight controls | the moving peak is at most **3.1 times** the louder frozen end (`bell1_q`, bell at 100 Hz, +24 dB); the bound is 4 |
| Random corner and middle settings of all twelve controls held static (`enth_dsp::eq`) | 24 at each of 44.1, 48 and 96 kHz, those whose impulse response settles within 2^19 frames are checked (at least 40) | the output never exceeds the sum of the impulse response's absolute values times the peak input, a rigorous bound for a time-invariant section chain |

The engine's modulation rate is bounded by its segment rate (1.5 kHz at 48 kHz, `prev` to `cur` every 32 frames) and by the LFO's 50 Hz top; the tests drive the core at the segment rate, the fastest the engine can deliver. A control in a plugin host can move faster than a segment only through the host's own automation, which the engine also reads once per segment.

## Response against the cookbook

The reference is written out in the tests (`enth_dsp::eq`: the cookbook's `b0…a2` for `HPF`, `LPF`, `peakingEQ`, `lowShelf` and `highShelf` with S = 1, evaluated in `f64` as a complex ratio; `tests/eq.rs` has its own copy), not taken from `biquad.rs`, and the BQ designs are a second, separate check.

| Check | Setting | Worst difference |
| --- | --- | --- |
| The designed sections against the cookbook (`the_designed_sections_equal_the_cookbooks_formulas`) | every band type over {20 … 20 kHz} × {±0.1 … ±24 dB} × Q {0.1 … 20} at 44.1, 48 and 96 kHz, 40 probes each, 167,280 comparisons; the cuts over their whole ranges | **1.9e-8 dB** |
| The module's processed response (`the_processed_response_is_within_a_tenth_of_a_decibel_of_the_cookbook`) | the impulse response through `Eq::process` (the `f32` hand-offs, the flushes and the ramp code included), each band type alone: bells over 6 frequencies × 4 gains × 4 Qs, shelves, cuts; 153 settings at 48 kHz | **6.5e-4 dB** |
| Six bands at once (`all_six_bands_at_once_stay_within_a_tenth_of_a_decibel`) | 71 random settings across the table's ranges at 44.1, 48 and 96 kHz | **7.1e-3 dB** |
| Through the engine (`tests/eq.rs`, `the_engine_response_is_within_a_tenth_of_a_decibel_of_the_cookbook`) | a held `osc` sine at 11 keys from 65 Hz to 10.5 kHz through eight settings (defaults, a boost bell, a notch with a second bell, both shelves, both cuts, the low-mid cut and air boost, the table's extreme gains and Qs, cuts mid-range); the gain is the tone's power with the `eq` over without it | **1e-4 dB** |

(Each of these asserts 0.1 dB, EQ's acceptance; the margins are three orders.) The bilinear transform's warping near Nyquist is the cookbook's: a bell at 16 kHz at 48 kHz is narrower than its analog twin, and the processed response matches the cookbook's, not the analog's.

## Flatness at the defaults

At the defaults the six bands are the two cuts and four sections at 0 dB. Measured on the Rust code (`the_defaults_are_flat_where_the_table_says`): within **0.02 dB of flat from 40 Hz to 5 kHz** at 44.1, 48, 96 and 192 kHz; −3.010 dB at 20 kHz at every rate (the cookbook pre-warps the corner); −0.263 dB at 20 Hz from the low cut. The table gives the high cut's rest value at other frequencies:

| `high_cut` 20 kHz, response at | 5 kHz | 10 kHz | 15 kHz | 19 kHz | 20 kHz |
| --- | --- | --- | --- | --- | --- |
| 44.1 kHz | −0.000 dB | −0.001 | −0.022 | −0.781 | −3.010 |
| 48 kHz | −0.000 | −0.008 | −0.111 | −1.425 | −3.010 |
| 96 kHz | −0.009 | −0.163 | −0.918 | −2.457 | −3.010 |
| 192 kHz | −0.015 | −0.236 | −1.128 | −2.559 | −3.010 |

and the low cut at 10 Hz is the same at every rate: −3.010 dB at 10 Hz, −0.263 at 20 Hz, −0.053 at 30 Hz, −0.007 at 50 Hz.

## Aliasing

An equaliser is a linear filter, and a static setting adds no aliasing; what the harness measures is the floor of the engine's own arithmetic. Measured with `enth_dsp::alias` (`Measure::REPORT_02`, `FFT_LEN` 32768 after a 8,192-frame lead-in) on an `osc` sine at keys 45, 57, 69, 81, 93 and 105 (110 Hz to 3.5 kHz) through the engine, `static_settings_alias_no_more_than_the_engine_floor`, 2026-10-27:

| Setting | 110 Hz | 220 Hz | 440 Hz | 880 Hz | 1760 Hz | 3520 Hz | Worst |
| --- | --- | --- | --- | --- | --- | --- | --- |
| no `eq` at all (the floor) | −103.4 | −97.8 | −93.4 | −95.5 | −92.7 | −119.8 | −92.7 |
| defaults | −103.4 | −97.8 | −93.4 | −95.5 | −92.7 | −119.8 | −92.7 |
| low-mid cut and air boost | −103.4 | −97.8 | −93.4 | −95.5 | −92.7 | −119.8 | −92.7 |
| four bands at the table's corners (low +24, bell −24 at Q 20, bell +24 at Q 0.1, high −24) | −103.4 | −97.7 | −93.4 | −95.5 | −92.7 | −119.8 | −92.7 |

**Bound: −91.2 dB** (the worst measure plus 1.5 dB, [aliasing.md](aliasing.md)), asserted for the floor and for each setting, and no more than 3 dB above the floor of the same key.

**A swept band** (`a_swept_bell_has_a_deep_skirt`, `tests/eq.rs`): a +15 dB, Q 4 bell on the carrier, its centre (±0.24 decades), gain (±12 dB) or Q (about 0.7 to 20) moved by a global sine LFO. The skirt is the strongest bin in 2 to 4, 4 to 8 and 8 to 16 kHz above the carrier, in dB re the carrier's strongest bin (the MSEG page's measure). Worst over the three controls and carriers at keys 69 and 93:

| LFO rate | 2 – 4 kHz | 4 – 8 kHz | 8 – 16 kHz | Bound (worst + 2 dB) |
| --- | --- | --- | --- | --- |
| 5 Hz | −110.5 | −117.4 | −128.6 | −108.5 / −115.4 / −126.6 |
| 25 Hz | −82.6 | −89.5 | −100.8 | −80.6 / −87.5 / −98.8 |

Ramped inside the segment against stepped at the segment, on the core alone (`a_sine_through_a_swept_band_has_no_zipper_that_a_ramp_does_not_remove`: a 1 kHz sine, a +18 dB, Q 8 bell swept in the log domain, skirt in 3 – 5, 5 – 9 and 9 – 17 kHz):

| Sweep | Ramped | Stepped (previous = current) |
| --- | --- | --- |
| 10 Hz, 500 Hz … 2 kHz | −125.1 / −135.7 / −151.0 dB | −77.1 / −84.5 / −97.4 dB |
| 80 Hz, 700 Hz … 1.4 kHz | −81.7 / −99.6 / −118.0 dB | −55.8 / −63.1 / −75.9 dB |

so the in-segment ramp is worth 25 to 55 dB of skirt at ordinary sweep speeds. At 80 Hz the bell passes the carrier 160 times a second and the modulation itself, not the stepping, sets the first band.

## Precision, determinism and tails

- Design and state are `f64`; the input and output are `f32`. The only transcendentals are `pow` (the gain) and `tan` (the corner), once per design, from the pinned `libm` through `util::math`; everything else is `+ − × ÷`, written in one order, with no fused operation. The same bits on x86_64 Linux and on `wasm32-wasip1` under wasmtime (`cargo xtask golden` and `--wasm`, 134 cases, 13 of them `eq-*`, 2026-10-27); the Windows, macOS and aarch64 targets are CI's.
- The integrator voltages are flushed (|x| < 1e-15 → +0.0) once per segment and each section's output per sample; FTZ and DAZ are not relied on (`the_render_is_the_same_under_every_ftz_mode`).
- A NaN or infinity in the input clears the channel's integrators of the sections it reached at the end of the segment, so a bad block does not poison the equaliser (`a_nan_in_the_input_does_not_poison_the_equaliser`).
- **Tails:** a typical setting (low cut 60 Hz, bell +12 dB at Q 6, high shelf +6 dB) is exactly zero 0.3 s after a burst; the longest ring the table allows (a +24 dB, Q 20 bell at 20 Hz with a +24 dB low shelf there, a time constant of about 1.3 s) is zero after 39.6 s. After 40 s of silence neither has a subnormal in its state or its output, and the engine's tail test (a chord through that setting with a 3 s release, 40 s) finds no subnormal output sample, no subnormal state, no non-finite sample, and every voice ended (`a_40_s_tail_through_eq_has_no_subnormals`).
- **Latency:** 0. The first output of an impulse is the product of the six sections' direct terms, which the test compares with the cookbook's `b0/a0` (`latency_is_zero`).
- State: 536 bytes for the whole module (`size_of::<enth_dsp::eq::Eq>()`): six bands × (two integrator pairs, the cached design and its key). `copy_state_from` is a clone of that; there is no heap anywhere, so `take_state_from`'s default copy is the whole swap.

## Cost

Measured with gungraun (callgrind) on x86_64 Linux with the pinned toolchain, 2026-10-27: one second at 48 kHz (1,500 segments) of the held chord through one `eq`, instructions in total. The `engine` bench in the same tree (**3,610,649** instructions for 1 voice and **37,290,489** for 16) is subtracted to read the module's own cost. Budgets are the measured count plus 10 %, rounded up to 1,000 (`crates/enth-engine/benches/eq.rs`):

| Case | Setting | Instructions | Budget | Module, per frame |
| --- | --- | --- | --- | --- |
| `static_1v` | catalogue defaults (all six bands in the path), 1 voice | 18,034,747 | 19,839,000 | 300.5 |
| `static_16v` | the same, 16 voices | 51,714,587 | 56,887,000 | 300.5 (14,424,098 for the second, the same at 1 and 16 voices) |
| `routes_16v` | a global LFO routed to the twelve band controls at depth 0 | 52,135,425 | 57,349,000 | 300.5 + 9 for the LFO and routes |
| `moving_16v` | the same routes at depth 8 %: every band redesigns and ramps every segment | 63,960,661 | 70,357,000 | 547 (246 more than static) |

A static `eq` costs 14.4 million instructions per second of audio at 48 kHz (about 0.5 % of a 3 GHz core), independent of the voice count. Moving every control costs 26 million: six designs (`pow` and `tan` through `libm`) per segment and the per-frame ramp (one division per band). The pairing of the two channels brought the static cost from 450 to 300 instructions per frame with the same bits. Instruction counts do not see cache effects or subnormal penalties; the subnormal tests stay mandatory, and no wall-clock time was measured.

**The plan-swap blocks** (`benches/eq.rs`, the method of `benches/swap.rs`: G1 with a held 16-note chord, 64-frame blocks, with and without an `eq` at the end of the chain). A crossfading swap lands in the block where a module is added elsewhere; both plans then run, and the surviving `eq`'s state is copied into the new plan.

| Block | G1 | G1 with an `eq` | The `eq`'s share |
| --- | --- | --- | --- |
| ordinary (`swap_normal`) | 408,454 | 427,560 | 19,106 (298 per frame) |
| crossfading swap lands (`swap_crossfade`) | 2,494,771 | 2,534,638 | **39,867** (1.6 % of the block) |

The copy is 536 bytes: it disappears next to the second plan's run of the module (two segments of 64 × 298 = 38,000 instructions), which is most of the difference. Budgets: 450,000, 471,000, 2,745,000 and 2,789,000. Through the engine, the crossfading swap of a chord through a swept `eq` with a module added differs from the unswapped render by exactly 0.0 (the copied state is the old one; `the_plan_swap_carries_the_eq_state`), and an instant swap of the same patch is bit-identical to no swap.

## Limits and decisions

1. **`high_cut` at 20 kHz is not −0.3 dB at 10 kHz.** The table's comment (and the paragraph above, which copies it) gives the analog Butterworth's value; the digital filter with the corner pre-warped is −0.001 dB at 44.1 kHz, −0.008 dB at 48 kHz, −0.16 dB at 96 kHz and −0.24 dB at 192 kHz ([Flatness](#flatness-at-the-defaults)). The corner itself is exact (−3.010 dB at 20 kHz at every rate), and `low_cut` at 10 Hz is as the table says (−0.263 dB at 20 Hz). The table's text should read "out of the way: under 0.01 dB below 10 kHz at 48 kHz"; it is lane CAT's, so it is reported, not edited. The frequency clamp of 0.49·fs means that at sample rates under about 41 kHz the cut cannot sit at 20 kHz: at 32 kHz `high_cut` 20 kHz becomes 15.7 kHz, which is audible. From 44.1 kHz up (0.49·fs is 21.6 kHz) the clamp does not bind.
2. **The shelves use the cookbook's slope S = 1** (`Q = 1/√2` in the `Q` form of the formulas), the steepest monotonic shelf, because the table gives a corner and a gain and no shape. The cookbook's half-gain-in-dB-at-the-corner property holds. A table with a shelf slope control would change this.
3. **All six bands always run.** A band at 0 dB is the identity in output but its integrators track the signal, so raising its gain mid-note starts warm (no swell, no click); that costs the four idle bands' arithmetic (about 200 of the 300 instructions per frame). Skipping them would save two thirds and make a raised band start from silence.
4. **A ramp is linear in `(g, k, m)`, not in frequency, gain and Q**, so between two very different settings the segment passes through sections neither end contains (the 9.3 ratio above). They are real, stable sections (non-expansive), and the engine's own smoothers make adjacent segments' settings close for any real control; the cases above are jumps over the whole range every 21 ms. Interpolating in the musical parameters would need a design (a `pow` and a `tan`, several hundred instructions) per band per frame, an order of magnitude more than the section itself.
5. **The output can exceed full scale.** Boosts add; nothing clips inside the module (the bus is `f32`); the master limiter and the engine's output stage see it. A saw chord through the extreme setting of `eq-extremes` is at +6 dB rms.
6. **Mixed `mix`** is the arithmetic `wet·m + dry·(1 − m)` per frame; at 0 the exact dry samples, so `mix` from 100 to 0 is a click-free crossfade through that arithmetic (it was tested at a half: `mix_one_is_the_wet_signal_and_half_is_their_average`, `the_mix_ramps_across_a_segment_and_the_filters_stay_warm`).
7. **A bell near Nyquist is narrower than its analog twin** (the bilinear warp); the cookbook is the reference, so the response matches the cookbook's, which is what the table promises.

Decisions taken here which the table did not fix: TPT sections instead of the biquad ([Why not the biquad](#why-not-the-biquad)); shelf slope 1; all bands always running; the ramp from the design of `prev` to the design of `cur`, snapped on the last frame; the design cache keyed by the three `f32` inputs; `f64` state with an `f32` hand-off between sections; `mix` as above.

Listening files (a saw chord with a low-mid cut and an air boost, and one with a swept bell) are in the lane's scratch directory outside the repository, with the patches that made them, exported and never played.

## Files

| What | Where |
| --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/eq.rs` (lane CAT's F1m table, [Parameters](#parameters-lane-cat-f1m)); the module reads `EqParam` through `.index()`, so `crates/enth-engine/src/params.rs` is not touched |
| DSP core | `crates/enth-dsp/src/eq.rs`: `Controls`, `Params`, the designs (`highpass`, `lowpass`, `bell`, `low_shelf`, `high_shelf`, `settled`, `magnitude_db`), `State` (two lanes), `Section`, `Eq`; 24 unit tests |
| Engine module | `crates/enth-engine/src/modules/eq.rs`: `Eq` (the `GlobalProcessor`) and `core_controls`, 6 unit tests |
| Engine tests | `crates/enth-engine/tests/eq.rs`: 13 tests (response against the cookbook, the alias floor, the swept-bell skirt, the square-LFO stability, `mix` 0, the idle skip, two instances, two modules, the 40 s tail, FTZ, the plan swap, and `process` under the no-allocation guard in debug builds) |
| Instruction budget | `crates/enth-engine/benches/eq.rs` (four render cases and the four swap blocks) |
| Golden cases | `tests/golden/cases/eq-*.toml`: `default`, `low-mid-cut-air`, `bells`, `shelves`, `cuts`, `notch`, `extremes`, `swept-bell`, `wobble`, `mix-half`, `mix-zero`, `chain`, `wake` |
| Reference | `biquad.rs` (lane BQ's, unchanged) for `Coeffs::design`, `Biquad` and the constants |

## Acceptance: how each row is met

| Row (phase-s2.md, every module lane; then lane EQ's own) | How | Where |
| --- | --- | --- |
| Same bits on six targets | `f64` with `util::math`, fixed order, no fused operations; 13 golden cases, equal on x86_64 Linux (`cargo xtask golden`, 134 cases) and on `wasm32-wasip1` under wasmtime 48.0.6 (`cargo xtask golden --wasm`, 2026-10-27); the Windows, macOS and aarch64 targets run in CI | the goldens |
| FTZ-independent | every recursive state flushed in code; the render of the tail patch is the same bits under every FTZ/DAZ setting | `ftz::the_render_is_the_same_under_every_ftz_mode` |
| Zero subnormals after a 40 s tail | none in output or state; every voice ended; the core reaches exact zero in 0.3 s (typical) and 39.6 s (the longest ring) | `a_40_s_tail_through_eq_has_no_subnormals`, `a_long_tail_has_no_subnormals_and_reaches_exact_zero` |
| Latency 0 | the first output of an impulse is the product of the sections' direct terms | `latency_is_zero` |
| Budget within +10 % | [Cost](#cost) | `benches/eq.rs` |
| Two instances keep separate state | interleaved on different signals and settings, each is bit-identical to its solo run; two modules in one patch | `two_instances_keep_separate_state`, `two_modules_in_one_patch` |
| No allocation | `process` and `copy_state_from` under `assert_no_alloc` | `guarded::process_does_not_allocate`, `guarded::the_module_and_its_state_copy_do_not_allocate` |
| `mix` 0 passes the input bit for bit | the dry samples are put back; the engine output equals the render without the `eq` while an LFO sweeps it | `mix_zero_passes_the_input_bit_for_bit`, `mix_zero_passes_the_engine_output_bit_for_bit` |
| Aliasing measured spectrally with a recorded bound | −91.2 dB (the engine's floor, with or without the `eq`), and the swept-band skirts | [Aliasing](#aliasing), `static_settings_alias_no_more_than_the_engine_floor`, `a_swept_bell_has_a_deep_skirt` |
| Within 0.1 dB of the cookbook, each band type, gains and Qs across the table's ranges, formulas written independently | 1.9e-8 dB (designs), 6.5e-4 dB (processed, single bands), 7.1e-3 dB (six bands), 1e-4 dB (through the engine) | [Response against the cookbook](#response-against-the-cookbook) |
| The crossfade swap's copied buffers measured; the swap block recorded | 536 bytes; +39,867 instructions in the block, 1.6 % | [Cost](#cost), `the_plan_swap_carries_the_eq_state` |
| Flat at the defaults within 0.1 dB, or what the table documents | within 0.02 dB from 40 Hz to 5 kHz; the cuts' rest values and the one correction to the table's text | [Flatness](#flatness-at-the-defaults), [Limits](#limits-and-decisions) 1 |
| Stability: no blow-up under audio-rate modulation of frequency, gain or Q, or at range extremes | non-expansive state update; the measured ratios ([Stability](#stability)). The wrapped biquad failed this row | [Why not the biquad](#why-not-the-biquad) |
| Zipper measured | the swept-band skirts, and ramped against stepped | [Aliasing](#aliasing) |
| Provenance and a `THIRD_PARTY_NOTICES.md` entry per port | nothing is ported (the cookbook is formulas; the Cytomic paper contributes equations): no entry | the top of this page |

## Sources

- R. Bristow-Johnson, *Cookbook formulae for audio EQ biquad filter coefficients* (the Audio EQ Cookbook): the `HPF`, `LPF`, `peakingEQ`, `lowShelf` and `highShelf` responses; formulas only. The independent reference in the tests.
- A. Simper (Cytomic), *Linear Trapezoidal Integrated State Variable Filter with Low Noise Optimisation*, 2013 (typo fix 2016): the section's equations and the bell and shelf mixing coefficients (verified against the cookbook before use). No code consulted or copied.
- V. Zavalishin, *The Art of VA Filter Design*, rev. 2.1: the trapezoidal integrators and why the structure is stable under modulation.
- Research report 05 §4 (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`): "Cytomic/Simper SVF … safe under modulation, unlike a DF1 biquad"; [biquad.md](biquad.md) (lane BQ's biquad, the reference).
