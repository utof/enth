# Offline biquad (`enth-dsp::biquad`)

- Lane: R4 BQ. Code: `crates/enth-dsp/src/biquad.rs` (the design, the section, the white-box tests), `crates/enth-dsp/tests/biquad_golden.rs` and `tests/fixtures/biquad-golden.txt` (the golden and the no-allocation check). Consumer: `enth-challenge`'s `Clip::render` (`crates/enth-challenge/src/ear/question.rs`), which applies an ear question's peak or shelf band; S2's EQ module (lane EQ) uses `Coeffs::design` and `Biquad` as the independent reference its tests compare against, and not as its bands: a ramped transposed-form biquad pumps under modulation ([eq.md](eq.md#why-not-the-biquad)). Design of record: [ear.md](../ear.md) ("Rendering and playback", "The offline biquad as built").
- Source: Robert Bristow-Johnson, *Cookbook formulae for audio EQ biquad filter coefficients* (the Audio EQ Cookbook, W3C note), the `peakingEQ`, `lowShelf` and `highShelf` filters in their `Q` form. The equations only: our own code, nothing copied, so there is no entry in `THIRD_PARTY_NOTICES.md`. It is distinct from the reverb's private `reverb::biquad` (a port of Signalsmith DSP's `filters.h`, MIT), which stays as it is.

A peaking filter and a low and a high shelf, designed in `f64` and run over an `f32` buffer **offline**: not a voice module, not on the audio thread, never called by the engine. It is a new file, so no existing output and no existing golden moved.

## The equations

With `A = 10^(dBgain/40)`, `w0 = 2π·f0/fs`, `α = sin(w0) / 2Q` and `c = cos(w0)`, all divided by `a0`:

| | peaking | low shelf | high shelf |
| --- | --- | --- | --- |
| `b0` | `1 + α·A` | `A·((A+1) − (A−1)c + 2√A·α)` | `A·((A+1) + (A−1)c + 2√A·α)` |
| `b1` | `−2c` | `2A·((A−1) − (A+1)c)` | `−2A·((A−1) + (A+1)c)` |
| `b2` | `1 − α·A` | `A·((A+1) − (A−1)c − 2√A·α)` | `A·((A+1) + (A−1)c − 2√A·α)` |
| `a0` | `1 + α/A` | `(A+1) + (A−1)c + 2√A·α` | `(A+1) − (A−1)c + 2√A·α` |
| `a1` | `−2c` | `−2((A−1) + (A+1)c)` | `2((A−1) − (A+1)c)` |
| `a2` | `1 − α/A` | `(A+1) + (A−1)c − 2√A·α` | `(A+1) − (A−1)c − 2√A·α` |

`H(z) = (b0 + b1 z⁻¹ + b2 z⁻²) / (a0 + a1 z⁻¹ + a2 z⁻²)`. The shelves use the cookbook's `Q` form, so `Q = 1/√2` is its shelf slope `S = 1`. Landmarks, which the tests pin to `1e-9` dB: a peak is exactly `gain` at its centre and 0 dB at DC and Nyquist; a low shelf is `gain` at DC, `gain/2` (in dB) at its corner and 0 dB at Nyquist; a high shelf mirrors it. A boost and a cut of the same band are exact inverses.

## The API

| Item | What |
| --- | --- |
| `Kind` | `Peak`, `LowShelf`, `HighShelf` (the type `enth-music` keeps for its integers, `EqKind`, has a fourth member, `BandPass`, which is the synth's svf and never reaches here) |
| `Coeffs::design(kind, freq_hz, gain_db, q, sample_rate)` | `f64` in, normalised coefficients out (`b0 b1 b2 a1 a2`, `a0 = 1`); `Coeffs::peak`, `low_shelf` and `high_shelf` are the same with the kind fixed |
| `Coeffs::IDENTITY`, `is_identity()` | `H = 1`; what a gain of exactly 0 designs |
| `Coeffs::response`, `magnitude_db` | `H(e^{jw})` (and `magnitude_db`, 20·log10 of its absolute value) in `f64` from the coefficients, for tests and later for displays |
| `Biquad::new(coeffs)`, `process(&mut [f32])`, `tick(f32)`, `reset()` | one section over a buffer in place, state kept between calls |

Arguments are sanitised, not trusted: a non-finite argument, or a sample rate too low to hold 1 Hz, designs the identity; the frequency is clamped to `1 Hz … 0.49·fs`, `Q` to `0.01 … 1000` and the gain to ±120 dB. Inside those ranges the section is stable for every argument (the analog denominators have positive coefficients, and the bilinear transform maps the left half-plane into the unit circle); a test checks `|a2| < 1` and `|a1| < 1 + a2` over the corners of the ranges and over random bands. The game's ranges (docs/challenge.md: 20 Hz to 20 kHz, ±0.01 to ±24 dB, `Q` 0.1 to 25) sit well inside.

## Why it is built this way

- **`f64` design and state, `f32` in and out.** A low or narrow band has its poles close to the unit circle (a 20 Hz band at 48 kHz is about `1e-3` away at `Q` 1, a boosted 40 Hz `Q` 8 band at 96 kHz about `1e-4`), and a biquad's rounding noise grows like `1/w0²`, about `1e5` at 20 Hz and 48 kHz. Offline there is nothing to save by rounding early, so rounding happens once, on the way out. A test compares the 96 kHz band with the cookbook to 0.01 dB.
- **Transposed direct form II**, written in one fixed order: `y = b0·x + s1`, `s1 = (b1·x − a1·y) + s2`, `s2 = b2·x − a2·y`. Two states, five products, no `mul_add` (Rust never contracts `a*b + c` by itself, and the build with `-C target-feature=+fma,+avx2` gives the same bits), so every target computes the same thing.
- **Flush in code.** Both states and the output go through `flush64` (below `1e-15` is exactly `+0.0`), so a decaying tail reaches exact zero and never lingers in the subnormal range, whatever the CPU's FTZ/DAZ say (AGENTS.md). A test rings a `Q` 4, +12 dB band and checks the last 4,096 samples of 600,000 are `+0.0` with both states zero. A non-finite input poisons the state until `reset()`, as for any recursive filter.
- **No gain is the identity bit for bit.** A gain of 0 dB designs `IDENTITY` and `process` returns without touching the buffer (NaN, tiny values and `-0.0` included). The game never asks for it (its bands have a gain), but S2's EQ will.
- **The host block size is not an input.** `process` continues from the state the last call left, so any chunking gives the same bits (a test checks block sizes 1, 7, 32, 100, 4,096 and the whole clip).
- **No allocation.** `process` and `tick` are plain loops over the slice; `tests/biquad_golden.rs` runs them under `assert_no_alloc` in debug builds.

## What "the cookbook's response" means here, and what we measured

The cookbook gives each filter as an analog prototype and a bilinear transform with the corner **pre-warped**: `s = j·tan(w/2) / tan(w0/2)`, so the corner is where it is set. The digital filter is that prototype evaluated at the warped frequency, exactly; the tests use that as the reference, written independently of the coefficient equations above (the prototypes in the `Q` form are the cookbook's own list, and complex arithmetic in the test):

| Test (all in `biquad::tests`) | Reference | Result |
| --- | --- | --- |
| Coefficients: 3 kinds × 3 sample rates (44.1, 48, 96 kHz) × 7 centres (20 Hz to 20 kHz) × 8 gains (±0.01 to ±24 dB) × 7 `Q` (0.1 to 25) × 24 frequencies | the prototype through the bilinear transform | worst 3.8e-8 dB over 84,672 points (bound 1e-6) |
| **Processed**: the impulse response through `Biquad::process` in `f32`, read at 35 frequencies (DC, the centre and 33 log-spaced from 20 Hz to 0.45·fs) | the same | worst 5.1e-5 dB over 285 sections (**bound 0.1 dB**, the acceptance); 3 sections of 288 (a boosted 20 Hz band at `Q` 25 rings for more than 10 s) are left to the coefficient test |
| Random bands in the game's ranges (property test, fixed seed, 256 cases) | the prototype, and the landmarks | stable; within 1e-6 dB |
| Landmarks, inverse sections, clamps, identity, block size, tails, corners of the ranges | closed forms | as above |

A mutation check (flipping the sign of the high shelf's `a1`) fails six of these tests, so they do detect a mistranscribed coefficient.

**The digital curve is not the analog curve at the unwarped frequency, and the cookbook does not claim it is.** Near Nyquist the bilinear transform squeezes the response. For a +12 dB, `Q` 1 band at 48 kHz, the largest difference from the analog prototype evaluated at the plain frequency, anywhere up to 0.45·fs:

| Centre | peak | shelf (low or high) |
| --- | --- | --- |
| 1 kHz | 0.04 dB | 0.02 dB |
| 4 kHz | 0.63 dB | 0.23 dB |
| 8 kHz | 2.2 dB | 0.9 dB |
| 16 kHz | 7.2 dB | 3.5 dB |

That is the cookbook's own filter, the same one every other player of this recipe gets, and it is what the game's questions are defined by: a high band sounds a little narrower and steeper than its analog ancestor. (A game that wanted the analog shape at 16 kHz would have to oversample, and nothing in R4 needs it.)

## Determinism

The only transcendentals are `pow` (once per design, `10^(dB/40)`), `sin` and `cos` (once each), all `f64` through `enth_dsp::util::math` (the pinned `libm`); the rest is `+ − × ÷` and `sqrt`, correctly rounded in IEEE 754. `Coeffs` carries `f64` bits and the golden pins them.

**The golden** (`tests/biquad_golden.rs`, `tests/fixtures/biquad-golden.txt`): 117 sections (peak, low shelf and high shelf × nine places at 44.1, 48 and 96 kHz from 40 Hz to 30 kHz × four shapes, plus the ends of the ranges and a band with no gain), each with the first 16 hex digits of the SHA-256 of its coefficients (the five `f64` bit patterns, so a different `libm` result shows first) and of its output over a fixed input (16,384 samples of seeded noise, then the same length of silence, so the tail and its flush are in the hash), and a `sum` line over every digest: `90dd238c4111f3b54d7b6af51d812762edc94c8782bf0040aa2b2366778f1d7a`. The cases are the integers an `EqBand` carries (Hz, centi-dB, thousandths of `Q`) converted by one IEEE division each, as `Clip::render` converts them.

It runs on the six golden targets: `.github/workflows/golden.yml` runs `cargo test -p enth-dsp --test biquad_golden` on Linux x86-64 (debug and release, and with `-C target-cpu=native` and `-C target-feature=+fma,+avx2`), Linux aarch64, Windows (MSVC) and macOS arm64, and on wasm32-wasip1 under wasmtime with and without `simd128`. The same sum came out natively (debug, release, native CPU flags and explicit FMA/AVX2) and on wasm32-wasip1 with and without `simd128` (run under node's WASI, which this machine has instead of wasmtime). The file is compiled in, so no directory is preopened. Regenerate on purpose only: `cargo test -p enth-dsp --test biquad_golden -- --ignored regenerate`, and the commit says why (a change here is an engine-version change for anything that renders through the biquad).

## Cost

Release build, measured once on the dev laptop (information only; instruction counts are the budget when a budget is set): 6.5 ns per sample per channel for `process` (a dependent chain of five products, five adds, three flushes), about 98 ns per `design`. A 3 s stereo clip is 288,000 samples, about 2 ms; a banded clip adds about 2 ms to its preparation, beside 15 to 30 ms for the render.

## Behaviour other lanes must know

- **A boost can exceed full scale** (a +24 dB peak on a loud clip): the biquad does not limit. Level matching is playback's (docs/ear.md), and the shell sets the gains.
- **`Clip::render` filters both channels after the engine, release tail included**, each from a cleared state. The band-pass band (`EqKind::BandPass`) never reaches the biquad: it is the timbre's own filter.
- **The pre-warped corner pins the corner, not the curve** (the table above): the labels of a band ladder (the "1 kHz" a player picks) name the centre, which is exact.
- **S2's EQ** does not wrap `Coeffs` and `Biquad` (it was built that way first and a biquad whose coefficients are ramped per sample blew up under random jumps of a bell: [eq.md](eq.md#why-not-the-biquad)); its bands are trapezoidal state-variable sections with the same response, and this module's `Coeffs::design` and constants are the reference its tests compare against.
