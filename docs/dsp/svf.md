# State-variable filter (`enth-dsp::svf`, `enth-engine::modules::svf`)

- Lane: P1a D2. Code: `crates/enth-dsp/src/svf.rs` (the filter, the parameter maps, white-box tests), `crates/enth-engine/src/modules/svf.rs` (the per-voice module). Catalogue entry: `crates/enth-patch/src/catalogue/svf.rs` (listed in [catalogue.md](catalogue.md)). Bench: `crates/enth-engine/benches/svf.rs`. Goldens: `tests/golden/cases/svf-*.toml`.
- Decisions behind it: research report 02 Q3 and Q8 (`_local/sessions/2026-10-19-p1a-headless-core/02-dsp-modules.md`) and its prototype (`~/code/enth-scratch/2026-10-19-p1a/02/proto`).

A two-pole (12 dB/octave) state-variable filter with low-pass, band-pass, high-pass and notch outputs, a soft saturator in front (`drive`), and per-voice key tracking. It is our own code, written from the equations of Andrew Simper's *Linear Trapezoidal Integrated State Variable Filter* (Cytomic, 2013, typo fix 2016), which are the same as Vadim Zavalishin's TPT state-variable filter (*The Art of VA Filter Design*, Native Instruments, 2012): two trapezoidal integrators in a zero-delay loop, solved in closed form. No code is copied; the paper contributes the equations only.

## The filter

Per sample, with `v0` the input and `ic1`, `ic2` the two integrator states:

```text
g  = tan(π·fc/fs)        k = 1/Q        a1 = 1/(1 + g·(g + k))     a2 = g·a1     a3 = g·a2

v3  = v0 − ic2
v1  = a1·ic1 + a2·v3                       band
v2  = ic2 + a2·ic1 + a3·v3                 low
ic1 = 2·v1 − ic1        ic2 = 2·v2 − ic2

lp = v2      bp = k·v1 (unity at the cutoff)      hp = v0 − k·v1 − v2      notch = v0 − k·v1
```

There is no unit delay in the feedback, so the response is exactly the bilinear transform of the analogue prototype with the cutoff pre-warped by `tan`: the cutoff is where it is set right up to 0.45·fs. The tests compare the measured response of all four modes against the analytic one (a complex `f64` evaluation of the transfer function) at DC, at the cutoff, at Nyquist and at other frequencies, and require agreement within 0.1 dB; DC and Nyquist are exact (a low-pass passes DC with gain 1, a high-pass blocks it).

### Per-segment coefficients, interpolated

The engine runs modules once per 32-frame segment. A module that stepped the coefficients once per segment would put a 1.5 kHz zipper on every cutoff sweep, so the filter interpolates. The module computes `a1, a2, a3, k` for the *end* of the segment (one `tanf`) and `process_block` ramps all four **linearly per sample** from the previous segment's values to those, adding a precomputed increment each frame (four adds, no division). Report 02 measured the interpolation at 13 to 23 dB less error than stepping, and the filter stays stable along the interpolation path (largest eigenvalue magnitude of the update matrix 0.99995 between 20 Hz and 21.6 kHz at Q 25).

- The module keeps the previous segment's coefficients itself (`coeffs[voice]`) instead of recomputing them from the parameter view's `prev` value. That halves the `tanf` calls, and a mono retrigger or a smoother reset becomes a 32-frame glide of the coefficients instead of a step. A fresh voice starts at its first segment's values (`from = to`).
- The four inputs (cutoff, resonance, drive, key-tracking term) form a cache key. When they are bit-identical to the previous segment's, nothing is recomputed: **a static filter costs no transcendental**.
- When the old and new coefficients are equal, the ramp is skipped (adding exact zeros is bit-identical and saves four adds per sample).
- The integrator states are flushed (below 1e-15 to exactly 0) once per block, in code, not by FTZ/DAZ flags.

### Cutoff

The effective cutoff is `cutoff · 2^(keytrack · (pitch − 60)/1200)` Hz, with `pitch` the voice's pitch in semitones (`NoteState::pitch_cur`, so a glide moves the cutoff too) and `keytrack` in percent: 100 % moves the cutoff one octave per octave played, relative to middle C (key 60). It is clamped to `20 Hz … 0.45·fs` (21.6 kHz at 48 kHz, 19.8 kHz at 44.1 kHz), where `g = tan(0.45π) = 6.3`; a NaN becomes 20 Hz. The ceiling keeps `g` finite and well-conditioned; above it the response would be the mirror image of a lower cutoff anyway. At `keytrack = 0` the `exp2` is skipped.

## Resonance to Q (a deliberate choice)

`Q = 0.5 · 50^(resonance/100)`, that is `k = 2 · 50^(−resonance/100)`: **exponential in Q**, from Q 0.5 (critically damped, no peak) to Q 25 (a 28 dB peak at the cutoff).

| resonance | 0 % | 10 % (default) | 25 % | 50 % | 75 % | 100 % |
| --- | --- | --- | --- | --- | --- | --- |
| Q | 0.50 | 0.74 | 1.33 | 3.54 | 9.4 | 25 |
| peak at the cutoff (low-pass) | none | none (Butterworth is 0.707) | 2.5 dB | 11 dB | 19.5 dB | 28 dB |

Why: the peak of a 12 dB/octave filter at its cutoff is about `Q` for `Q ≥ 1`, so an exponential map makes equal knob steps equal steps in decibels (about 0.34 dB per percent from Q 1 at 17.7 % to the top). The curve first proposed gave Q ≈ 1 at 50 % (the design review's note): half the travel of the knob was spent between Q 0.5 and 1, where there is nothing to hear, and the middle of the knob was still a plain filter. The default of 10 % lands within 0.5 dB of the maximally flat response (Q 0.707), so a new filter is neutral, not coloured. The map is documented on `k_from_resonance`, pinned by a landmark test, and part of the catalogue's meaning of `resonance`: changing it changes every patch that uses a filter.

## Drive

`drive` (0 to 24 dB) puts a soft saturator in front of the filter: `y = x / sqrt(1 + a·x²)` with `a = 10^(dB/10) − 1`. It is odd, monotonic, has unity gain at small signals, and is bounded by `1/sqrt(a)`: a sigmoid with gain `G = 10^(dB/20)` in front, normalised back, offset so that the small-signal level does not change. At 0 dB, `a = 0` and the saturator is skipped entirely, which is bit-identical to running it (`x / sqrt(1 + 0)` is exactly `x`). The drive amount `a` is ramped per sample across the segment, like the coefficients. The cost is one `sqrt` and one `div` per sample (both correctly rounded in IEEE 754).

The shape is the algebraic sigmoid above, not a tanh; the module comment in `catalogue/svf.rs` says so.

## Stability under modulation

Required: finite output, no stuck DC, at full resonance under audio-rate cutoff modulation. Tests (`enth-dsp` white-box and `modules/svf.rs`):

- **Random-jump stress**: all four modes, drive 0 and 24 dB, Q 25, white noise (±0.5) in, and the cutoff jumping to a new random value (log-uniform from 20 Hz to 0.45·fs) at the start of every one of 15,000 segments (10 s; an audio-rate 1.5 kHz modulation): every output sample is finite, the peak stays below 25 (report 02 measured 2.9), the mean of the output stays below 0.05 (no stuck DC), and once the input stops and the cutoff is parked the state decays to exactly 0.0 (no limit cycle, no subnormal tail).
- An impulse at 0.45·fs and Q 25 is below 1e-6 within 60 ms (report 02: 42 ms).
- `modules/svf.rs` repeats the random-jump stress through the module (high-pass, Q 25, a random cutoff from 20 Hz to 20 kHz every segment for 15,000 segments; the state then dies to exact zero with no subnormals), and the `moving_16v` bench renders 16 voices with a cutoff event every segment.

## Parameters

| Parameter | Reaches the core as | Notes |
| --- | --- | --- |
| `mode` | `Mode::{Lp, Bp, Hp, Notch}` | read from `base` (it is not modulatable) |
| `cutoff` | Hz, `cur` value | smoothing and modulation are applied by the engine before the module sees it |
| `resonance` | `k_from_resonance` | |
| `keytrack` | octaves per octave | 0 skips the `exp2` |
| `drive` | `drive_amount` | 0 dB skips the saturator |

## Determinism

The only transcendentals are `tanf` (per changed segment), `exp2` (the resonance map in `f64`, and key tracking) and `expm1` (the drive map), all through `enth_dsp::util::math` (pinned `libm`). The per-sample code is `+ − × ÷` and `sqrt`, all correctly rounded; no `mul_add`. 24 impulse-response hashes (4 modes × 3 cutoffs × 2 resonances) are pinned in `svf.rs`, and the `enth-dsp` unit tests were run under wasmtime (wasm32-wasip1) with the same results. The goldens `svf-*` are checked native = wasm by `cargo xtask golden --wasm`.

## Cost

Measured with callgrind (`cargo xtask bench --bench svf`), 1 s at 48 kHz, whole engine; the "marginal" column subtracts the `engine` bench with the same voices (the reference saw and the ADSR are in both):

| Case | Instructions | Budget (+10 %) | Marginal per sample per voice |
| --- | --- | --- | --- |
| `lp_1v` | 5,517,718 | 6,070,000 | (fixed costs dominate at 1 voice) |
| `lp_16v` | 65,201,335 | 71,722,000 | 28.4 |
| `drive_16v` (12 dB) | 71,442,823 | 78,588,000 | 36.6 |
| `moving_16v` (a cutoff event every segment) | 71,381,472 | 78,520,000 | 36.5 |

The inner loop of a static low-pass is 22 instructions per sample (13 floating-point operations, 6 register moves that SSE2's two-operand form needs, the load, the store and the loop). The rest of the marginal 28 is the engine handling one more voice processor. A moving cutoff adds the four ramp adds and one `tanf` per voice-segment. A form with `v2 = ic2 + g·v1` would save about five instructions per sample, but it ramps a different set of coefficients than the one report 02 validated for stability, so it was not adopted.

## Behaviour other lanes must know

- The module does not use `params.prev`. Whatever smoothing S2b puts on `cutoff`, `resonance` and `drive` (the catalogue gives them `SMOOTH`) arrives as `cur`, and the filter glides across 32 frames to it from where it was.
- `svf-cutoff-jump` automates `filter1.cutoff` at off-grid times with a resonance of 90 %: until S2b smooths the cutoff, it moves in one segment, and the golden will change on purpose when smoothing lands.
- The goldens `svf-*` play the reference saw (a stub until D1 lands); they re-baseline then.

## References

- A. Simper, *Linear Trapezoidal Integrated State Variable Filter with Low Noise Optimisation*, Cytomic, 2013 (typo fix 2016). The equations.
- V. Zavalishin, *The Art of VA Filter Design*, Native Instruments, rev. 2.1. The zero-delay feedback and trapezoidal integration derivation and pre-warping.
- Research report 02 (Q3, Q8) and its prototype `proto/src/bin/svftest.rs`: interpolation versus stepping and the eigenvalue scan.
