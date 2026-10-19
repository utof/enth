# Noise (`noise`)

- Lane: P1a D1. The catalogue entry is `crates/enth-patch/src/catalogue/noise.rs` (frozen at F1, listed in [catalogue.md](catalogue.md)); the module contract is in [engine.md](engine.md).
- Code: DSP in `crates/enth-dsp/src/noise.rs`; the module in `crates/enth-engine/src/modules/noise.rs`. Tests: the DSP file (white-box), `crates/enth-engine/tests/d1_modules.rs`, `d1_engine.rs` and `d1_noalloc.rs`. Goldens: `tests/golden/cases/noise-*.toml`. Bench: `crates/enth-engine/benches/noise.rs`.
- Decisions behind it: research report 02 (`_local/sessions/2026-10-19-p1a-headless-core/02-dsp-modules.md`, Q2).
- Sources: all own code. The generator is George Marsaglia's xorshift32 (below) and the pink filter uses the coefficients Julius O. Smith III published in *Spectral Audio Signal Processing*; algorithms and published numbers need no notice entry (`THIRD_PARTY_NOTICES.md` lists ported source), but both are cited here.

White or pink noise, one independent stream per voice. Integer state and `+ − ×` only, so the streams are bit-identical on Linux, Windows and wasm.

## White: xorshift32

Marsaglia, G. (2003), "Xorshift RNGs", *Journal of Statistical Software* 8(14), the 13-17-5 triple:

```text
x ^= x << 13;  x ^= x >> 17;  x ^= x << 5;
sample = (x >> 8) · 2⁻²³ − 1          24 random bits, exact in f32, uniform in [−1, 1)
```

A unit test pins the three outputs after Marsaglia's own example state 2,463,534,242 (723,471,715; 2,497,366,906; 2,064,144,800, also verified with a separate Python implementation). *Measured:* mean ≈ 0, RMS 0.577 (the test allows ±0.003), flat within 0.5 dB across three bands of an averaged spectrum (about ±0.1 dB by eye). The generator has period 2³² − 1, far beyond any note.

## Pink: Smith's third-order pinking filter

White noise through the filter from "Example: Synthesis of 1/F Noise" in J. O. Smith III, *Spectral Audio Signal Processing* (<https://ccrma.stanford.edu/~jos/sasp/Example_Synthesis_1_F_Noise.html>):

```text
B = [ 0.049922035, −0.095993537,  0.050612699, −0.004408786 ]
A = [ 1,           −2.494956002,  2.017265875, −0.522189400 ]
y[n] = B·(w[n], w[n−1], w[n−2], w[n−3]) − A[1]·y[n−1] − A[2]·y[n−2] − A[3]·y[n−3]
out  = PINK_GAIN · y,   PINK_GAIN = 4.0
```

- **`f64` state.** The filter has a pole near 0.999; in `f32` direct form that loses the lowest octaves. The white sample is widened to `f64`, the recursion runs in `f64`, and the output is rounded once to `f32`. Same bits everywhere (no transcendentals, no fused multiply-add).
- **Gain.** Smith's filter has an RMS of 0.050 for white input of RMS 0.577. `PINK_GAIN = 4.0` brings pink to **RMS 0.200** (−9.2 dB below white's 0.577), which keeps the peaks near ±1: the measured peak over 1,000,000 samples is 0.956 and the test requires under 1.3, so pink at level 100 % does not clip where white does not.
- **Slope.** *Measured* over 100 Hz – 10 kHz (a least-squares line through an averaged periodogram, one point per 5 % of frequency, 2²⁰ samples): **−10.003 dB/decade** (the plan's tolerance is −10 ± 0.3).
- **Denormals.** The filter state is flushed to zero (`flush64`) once per 32-sample block, in code, with no FTZ/DAZ flags. After a long pink release tail the engine reports 0 subnormal values.
- The filter starts cold at a note's start (all state zero), so the first tens of milliseconds of pink lack their lowest octaves. Switching `color` to pink mid-note starts the filter from the state it had, which is whatever the last pink block left (cold the first time). Both are stable and deliberate.

Rejected sources, as in the report: Kellett's filter (no licence in its repository) and Stenzel's "new shade of pink" (a "public domain" file at odds with its headers).

## Seeding

A voice's stream starts from `VoiceStart::seed`, which the engine derives from **the render seed, the module id and the note index** (`crate::seed::voice_seed`). `White::from_seed` takes one `SplitMix64` step of it (`Rng::new(seed).next_u32()`) and uses the result as the xorshift state, replacing 0 (the fixed point) with `0x9E3779B9`. So:

- every note has its own stream (the test compares two notes of one render), and every voice slot a different one for simultaneous notes;
- a render is reproducible, and a different render seed gives different noise;
- **no parameter ever reseeds anything**: `color` and `level` changes leave the white stream untouched (a test automates `color` mid-note and checks the render is bit-identical up to that moment), and the seed-stability test in `d1_engine.rs` (a four-note chord on a patch with a table oscillator, a synced pulse, a triangle and pink noise) moves each of the ten non-zero continuous `osc` and `noise` parameters by 1e-3 relative: the 10 ms RMS envelope moves by at most 0.009 dB (the limit is 0.05 dB; the worst is `osc2.semi`, a real change of pitch), while changing the render seed moves it by 3.3 dB.

## Level and the module

`level` (percent, smoothed once S2b lands) ramps per sample from the previous segment's value to this one's, `out *= prev + (cur − prev)·RAMP[i]`; at exactly 100 % in both ends the multiply is skipped, so full-level noise is the generator's bits. `color` is read from `base` (it is discrete). White-noise output is in `[−1, 1)`; pink is RMS 0.2 with rare peaks near ±1.

## Cost, measured

Callgrind instructions (`cargo xtask bench --bench noise`, 1 s of audio at 48 kHz, 16 voices), as the difference to the `engine` bench's saw patch without the extra generator, per sample and voice, including the engine's mixing of the generator into the voice:

| Colour | Instructions per sample |
| --- | --- |
| white | 20.9 |
| pink | 57.3 |

Report 02 estimated 10 – 25 for the module; white is inside that, and pink adds 36 for the `f64` filter and the conversions. The `engine` bench's baseline is 56.8 per voice and sample (the reference saw, the envelope, the amp stage and the pan).

## Notes for lane S2b

- `Noise::new` allocates nothing and ignores its values; every state is per voice slot and reset by `start_voice`.
- A mono or legato retrigger that does not call `start_voice` keeps the stream running, which is the right behaviour for noise.
