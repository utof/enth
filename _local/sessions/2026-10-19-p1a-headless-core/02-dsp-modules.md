# 02 — DSP modules and ports

- **Date:** 2026-10-19
- **Sub-researcher:** 02 (DSP modules and ports), P1a research session
- **Questions answered:** 1 oscillators, 2 noise, 3 multimode filter, 4 envelopes and LFOs, 5 the Signalsmith reverb port, 6 the compressor, 7 the global FX bypass, 8 the CPU budget
- **Evidence labels:** *measured* (I ran it here, today), *documented* (the source says so; linked and dated), *anecdotal* (forum or blog opinion), *inference* (my reasoning, not tested)
- **Scratch and prototype:** `~/code/enth-scratch/2026-10-19-p1a/02/` (`notes.md` is the running log; `proto/` is a Rust crate; `reverb-golden/` holds the C++ harness; `upstream/` holds the cloned sources, each in its own directory). Nothing in the repo was changed except this file.
- **Machine for every measurement:** i7-9750H, Linux Mint 22.3, rustc 1.99.1, `libm =0.2.16`, g++ 13.3.0, clang 18.1.3, node 22.22.3 (wasm under V8 via `node:wasi`), valgrind 3.22.0. Load average 6–7 during timings (other agents were running), so **wall-clock figures are ±20%**. Instruction counts from callgrind are exact.

## Answer in brief

1. **The Signalsmith reverb port works: it is bit-exact with the upstream C++.** *Measured.* A Rust port of `reverb.h` plus its `delay.h`, `mix.h` and `filters.h` helpers (727 lines of scratch code) gives output identical to the C++ built with g++ and clang at `-O2`. The test covers two parameter sets, 4 s of stereo audio and a mid-render parameter change. The Rust output also hashes the same in release, debug, `target-cpu=native` and `wasm32-wasip1` under V8, with and without `simd128`. The C++ itself changes once FMA contraction is allowed (`-march=native`, up to 1.8e-5) or with `-Ofast` (up to 5.2e-5).
2. **The reverb needs no RNG at all.** *Measured.* Its `std::mt19937` seeds are five hard-coded constants, so the delay lengths depend only on `roomMs`. The 40 random draws can therefore be a constant table. I reproduced libstdc++'s draws exactly in Rust (mt19937, plus GCC's `generate_canonical<float,24>` and Lemire's `uniform_int_distribution`). Those distributions are not portable across C++ standard libraries ([WG21 P2059R0](https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2020/p2059r0.pdf)), so upstream itself sounds different on libc++ and MSVC. **Do not seed the reverb from the patch hash** (ADR-0006, design.md §5). Doing so would change the reverb's character whenever any knob moves.
3. **Reverb behaviour is clean.** *Measured.*
   - **Allocation:** it allocates 1.64 MB at 48 kHz, in `prepare` only.
   - **Parameter changes:** a `roomMs` change re-computes delay offsets and cross-fades over 20 ms. It does not re-allocate and does not click: the peak second difference after a change is 1.15–1.47× the steady-state value.
   - **Denormals:** with the code flush, a 61 s tail has 0 subnormals and the output reaches exact zero at 15.1 s. Without it, there are 2.34 M subnormal output samples and the render is 4.2× slower.
   - **Sample rate:** the decay time is the same at 44.1, 48 and 96 kHz.
   - **Cost:** 2,574 instructions per stereo frame, about 1.6% of a core.
4. **Two upstream issues to handle.** *Documented.*
   - `reset()` forgets `delay4`.
   - `stfx-library.h`, the wrapper whose 20 ms parameter fade the port must reproduce for bit-exactness, still says "Boost Software License" in its header. The effect headers were corrected to MIT on 2025-07-17. Either add BSL-1.0 to ADR-0004's list of licences allowed for ported source, or ask upstream to fix the header.
5. **Oscillators: port Plaits' `VariableShapeOscillator`.** It is MIT, © 2016 Emilie Gillet, and gives saw, pulse with PWM, triangle, hard sync and a one-sample-delay polyBLEP/BLAMP. *Measured aliasing* (alias power over harmonic power, 20 Hz–20 kHz):

   | Waveform | 110 Hz | 440 Hz | 1,760 Hz | 7,040 Hz |
   | --- | --- | --- | --- | --- |
   | Naive saw | −27 dB | −21 dB | −14 dB | −7 dB |
   | polyBLEP saw | −47 dB | −41 dB | −33 dB | −23 dB |
   | Triangle | −99 dB | −84 dB | −67 dB | −55 dB |

   The saw costs 38 instructions per sample, and hard sync adds 42%. Plaits' saw aliases exactly like the classic 2-point polyBLEP in `spike-engine`: both use the same polynomial.
6. **Wavetables: generate the tables in code.** Each table is 2048 samples with 11 mip levels, built by exact integer-indexed additive synthesis in f64, with 4-point Hermite interpolation. *Measured:* aliasing at or below −93 dB (the measurement floor), 62 instructions per sample, 3.2 ms to generate each frame, and the same hash on native and wasm.
7. **Filter: a Cytomic/Simper TPT SVF** (LP/BP/HP/notch), with `g = libm::tanf` computed every 32 samples. *Measured:*
   - Stepping coefficients every 32 samples deviates from per-sample coefficients by −36 to −7 dB, worst at high Q.
   - Interpolating `a1..a3` linearly per sample costs 3 adds and no division, and cuts the deviation by 13–23 dB.
   - The filter stays stable under random cutoff jumps between 20 Hz and 21.6 kHz at Q 25.

   Correction: DaisySP's `svf` is not a TPT filter (see Plan check).
8. **Envelopes, LFOs and noise.** *Measured.*
   - **Envelopes:** RC-style exponential segments with an overshoot target. Each segment's target overshoots the end by a fraction of its span, so every segment reaches its end in exactly its set time, within one control step, from any start level.
   - **LFOs:** integer (u32) phase; rate error ≤ 4e-6.
   - **Noise:** xorshift32 white noise per voice, and pink noise through J.O. Smith's 3rd-order filter, which measured −10.00 dB/decade.
   - **Determinism:** the whole 16-voice prototype hashes identically on native, wasm and `target-cpu=native`.
9. **Compressor:** JOS's `compression_gain_mono` structure. That is an attack/release follower on |x|, then dB, then the static curve, then a one-pole smoother at attack/2, then back to linear gain. JOS's function has no quadratic knee and no make-up gain, so I add Giannoulis et al. (2012)'s soft knee and a make-up gain. Two links differ from JOS: stereo is linked by `max(|L|,|R|)` rather than JOS's `|L|+|R|`, and there is no look-ahead (web-audio-api-rs has a 6 ms one, per the Web Audio spec). *Measured:* the gain computer matches web-audio-api-rs's formula within 1e-6 dB, and the compressor costs 160 instructions per frame.
10. **Global FX bypass:** an instance-level monitor mask on effect modules flagged `time_based` (the reverb in P1). It lives in plugin state outside the `Patch`, and `enth-render` has no way to set it. Scoring and CLI renders therefore ignore it by construction.
11. **CPU budget.** *Measured:* a P1 voice costs 229 instructions per sample (32 ns). The 16-voice reference patch with compressor and reverb costs 6,353 instructions per frame, 920 ns per frame, 22.7× realtime (4.4% of one core). **Proposed G1 budget:**
    - at least 10× realtime on one core offline for that patch;
    - at most 12,000 instructions per frame as a gungraun hard limit, re-based to the first product measurement +10%;
    - p99 `process()` time at most 25% of the 64-sample deadline.

## Findings

### Q1 — Oscillators

**Source: Plaits `VariableShapeOscillator` and stmlib `polyblep.h`. MIT, verified per file.**

- [`plaits/dsp/oscillator/variable_shape_oscillator.h`](https://github.com/pichenettes/eurorack/blob/f4cfa7697095a28d292bff758beb51deb49b5a86/plaits/dsp/oscillator/variable_shape_oscillator.h) carries a full MIT header ("Copyright 2016 Emilie Gillet."). The file was last changed 2023-05-29; the repo was last pushed 2026-10-12.
- The eurorack README says AVR projects are GPL-3.0 and STM32F projects are MIT. Plaits is an STM32 project and its file header is MIT, so it is permitted. *Documented.*
- [`stmlib/dsp/polyblep.h`](https://github.com/pichenettes/stmlib/blob/d18def816c51d1da0c108236928b2bbd25c17481/dsp/polyblep.h) is MIT ("Copyright 2017 Emilie Gillet."). It holds four one-line polynomials: `ThisBlepSample`, `NextBlepSample` and their integrated (BLAMP) forms.

**Algorithm (from the source).**
- The oscillator delays its output by one sample (`next_sample_`). When a discontinuity happens at fractional position `t` inside a sample, it adds the step residual `0.5t²` to this sample and `−0.5(1−t)²` to the next. For a slope change (the triangle corners) it uses the integrated residual `0.1875 − t/2 + 1.5(t/2)² − (t/2)⁴`.
- `waveshape` morphs triangle (0) → saw (0.5) → square (1). `pw` sets the pulse width and the triangle's slope.
- Hard sync runs a master phase. At reset it subtracts the slave's value with a BLEP at `reset_time`, and handles transitions that fall inside the reset sample.
- PWM uses `previous_pw` to place the transition correctly while the width moves.
- Frequency is clamped to `kMaxFrequency = 0.25` (fs/4). The pulse width is clamped to `[2f, 1−2f]`.

**Port notes (inference, from reading the code).**
- The core is f32 `+ − × ÷` with no transcendentals per sample, so it is deterministic under our rules. The prototype hashes the same on native and wasm (*measured*).
- Pitch is computed per control step with `libm::exp2f`.
- The Plaits code interpolates its parameters per sample with `stmlib::ParameterInterpolator`. In enth, frequency and `pw` should ramp linearly across each 32-sample control segment the same way, for glide and vibrato. A step per segment would cause small pitch zipper.
- **Sync in enth:** osc 2 synced to osc 1 should use osc 1's increment as `master_frequency`, with the master phase initialised to osc 1's start phase. The two f32 accumulations then stay bit-identical.
- **Raise the frequency cap:** `kMaxFrequency = 0.25` caps the oscillator at 12 kHz at 48 kHz. Raise it to 0.45 and check that the pulse-width clamp still holds, or keep 0.25 and document it. A key at C8 is 4.2 kHz, so this only matters for extreme pitch modulation.

**Aliasing, measured.**
- Method: 1 s at 48 kHz; a 4-term Blackman-Harris FFT of length 32,768; power within ±8 bins of each harmonic counted as signal, everything else from 20 Hz to 20 kHz as alias.
- The bracketed figures count only aliases below 8 kHz, a rough proxy for audibility.
- The measurement floor is about −93 dB. Script: `alias.py`; data: `alias-results.txt`.

| Oscillator | 110 Hz | 440 Hz | 1,046 Hz | 1,760 Hz | 3,520 Hz | 5,000 Hz | 7,040 Hz |
| --- | --- | --- | --- | --- | --- | --- | --- |
| naive saw | −27.0 [−31.5] | −20.6 [−25.0] | −16.7 [−21.2] | −14.3 [−18.8] | −10.8 [−15.8] | −9.5 [−13.7] | −6.8 [−12.1] |
| 2-point polyBLEP saw (spike) | −46.8 [−70.2] | −40.6 [−63.7] | −36.4 [−61.1] | −33.4 [−57.1] | −28.8 [−57.1] | −28.6 [−49.2] | −22.9 [−50.6] |
| Plaits saw | −46.8 [−70.2] | −40.6 [−63.7] | −36.4 [−61.1] | −33.4 [−57.1] | −28.8 [−57.1] | −28.6 [−49.2] | −22.9 [−50.6] |
| Plaits square | −48.5 [−72.1] | −42.8 [−64.9] | −37.6 [−61.6] | −36.7 [−57.1] | −34.0 [−64.4] | −36.0 [−56.9] | −32.7 [−73.0] |
| Plaits pulse 20% | −46.7 [−70.2] | −41.0 [−63.1] | −35.2 [−62.2] | −32.7 [−55.4] | −27.0 [−54.8] | −27.8 [−47.4] | −26.0 [−52.2] |
| Plaits triangle | −99.1 [−101.3] | −83.9 [−88.6] | −71.4 [−79.5] | −67.0 [−72.4] | −59.5 [−65.2] | −58.6 [−58.9] | −55.2 [−56.2] |
| Plaits saw, hard sync ×2.37 | −43.6 [−67.0] | −36.8 [−61.5] | −34.2 [−59.2] | −30.0 [−54.4] | −28.6 [−49.5] | −27.4 [−53.5] | −20.9 [−57.9] |
| wavetable saw, linear | −75.1 [−77.1] | −89.8 [−90.5] | −92.1 [−92.6] | −92.5 [−92.8] | ≤ −93 | ≤ −93 | ≤ −93 |
| wavetable saw, Hermite | ≤ −93 | ≤ −93 | ≤ −93 | ≤ −93 | ≤ −93 | ≤ −93 | ≤ −93 |

What the table shows:
- Plaits' saw and the spike's 2-point polyBLEP are identical to 0.1 dB in every column. They are the same polynomial arranged differently.
- So the choice between them is about features (sync, PWM, triangle, morph), not quality.
- A 2-point polyBLEP buys 20–25 dB over a naive saw. The wavetable is in another class for pure saws, but cannot do sync or PWM cheaply.
- Better BLEPs (Signalsmith's MIT [`elliptic-blep`](https://github.com/Signalsmith-Audio/elliptic-blep), last commit 2025-08-21) stay a later upgrade, as the effects research said.

**Cost, measured.** Callgrind instructions per sample, one oscillator, 220 Hz, rendered in 32-sample segments:

| Oscillator | Instructions per sample | ns per sample (wall) |
| --- | --- | --- |
| naive saw | – | 9.2 |
| 2-point polyBLEP saw | 23.4 | 10.0 |
| Plaits saw | 37.7 | 10.6 |
| Plaits saw with hard sync | 53.5 (+42%) | 12.2 |
| wavetable, linear | 44.3 | 11.8 |
| wavetable, Hermite | 62.3 | 15.1 |

Hard sync is cheap enough to ship in P1.

**Detune and unison readiness (inference).**
- Treat each oscillator as an array of sub-oscillators: `unison` (count, default 1), `detune` (cents) and `spread` (%). Each sub-oscillator gets a start phase from `derive(seed, note_index, osc, sub)`.
- P1 can ship with unison fixed at 1 and add the parameters later, with defaults that need no migration.
- Plaits' render loop is branchy (a `while` loop over transitions), so it does not map onto `wide` lanes.
- If unison or voices go into SIMD lanes (design.md §3), use the branch-free 2-point polyBLEP that `spike-engine` already runs bit-identically in f32x4/f32x8. It has the same alias performance as the table shows. Sync and moving PWM in lanes need masked selects, which is a separate task.

**Wavetables: format, band-limiting, interpolation and generation.**

*Format:*
- `TABLE_LEN = 2048` f32 samples per cycle, `LEVELS = 11` mip levels. Level *m* holds harmonics 1…(1024 >> m).
- Three guard samples per level (x[−1], x[N], x[N+1]) for Hermite interpolation.
- Layout `[frame][level][N+3]`. Frames per table: 8–16.
- Memory: 16 frames × 11 levels × 2051 × 4 B = 1.44 MB per table.

*Mip selection:*
- The prototype picks the smallest level whose top harmonic is at or below Nyquist (`2^m ≥ dt·2048`). Bandwidth is then 12–24 kHz depending on where the pitch falls within the octave. *Measured:* at 110 Hz it reaches 14.1 kHz.
- Recommendation (*inference*): allow the top harmonic up to `fs − 20 kHz`, i.e. 28 kHz at 48 kHz. Any alias then lands above 20 kHz, and the worst-case bandwidth rises to about 14 kHz.
- Level cross-fading (to avoid a brightness step at octave boundaries during glides) is not in P1. It is a known limitation.

*Interpolation:*
- 4-point Hermite (the form in Plaits' MIT `wavetable_oscillator.h`; it is textbook).
- Linear interpolation measured −75 dB at 110 Hz against Hermite's floor of −93 dB, for 18 fewer instructions per sample.
- Frame morph is a linear cross-fade of two adjacent frames, which doubles the reads. *Measured:* 323 instructions per sample for a full voice with a morphing table oscillator, against 229 for the full voice.
- Plaits' own approach (integrated tables plus a differentiator, no mip-maps) is designed for small memory and is not needed here.

*Generation in code:*
- `sin(2πi/N)` for i < 2048 comes from `libm::sin` in f64. Harmonic *k* at sample *n* is `SINE[(k·n) mod N]`, an exact integer index.
- Sums run in f64 in a fixed k order, then round to f32. Each frame is normalised by its level-0 peak, so all levels share one scale.
- *Measured:* 3.2 ms per frame. The table data hashes the same on native, wasm and `target-cpu=native` (`78f7a6c0…`).
- Five tables × 16 frames would take about 0.26 s. So generate lazily, only the tables a plan uses, off the audio thread. Cache them in an `Arc<WavetableBank>` on the engine, not per voice.
- A deterministic scalar f64 radix-2 FFT would be roughly 10–30× faster (*inference*). Do not use `rustfft`: it dispatches by CPU.
- Use no `build.rs`-baked blob: a host build script would use host maths, and the blob would bloat the wasm binary.

*Proposed tables* (each frame is a formula over harmonic amplitudes, so there is no asset licence question):
1. `basic`: sine → triangle → saw → square.
2. `pulse`: duty 50% → 3%.
3. `tilt`: amplitudes `k^−p` with p from 0.5 to 3, a smooth brightness axis that suits CMA-ES.
4. `formant`: vowel resonances A–E–I–O–U over a saw spectrum.
5. `organ`: drawbar registrations.

### Q2 — Noise

- **White noise:** xorshift32 (Marsaglia 2003), one `u32` per sample; `bipolar = (x >> 8)·2/2^24 − 1`, which is exact in f32.
  - Seed each voice's stream from `derive(seed, note_index)` with the spike's SplitMix64 `derive`, forced non-zero, because 0 is a fixed point of xorshift32.
  - *Measured:* flat to ±0.1 dB across the band; RMS 0.577.
  - SplitMix64 per sample would also do; xorshift32 is cheaper.
- **Pink noise:** white noise through J.O. Smith's 3rd-order pinking filter, `B = [0.049922035, −0.095993537, 0.050612699, −0.004408786]`, `A = [1, −2.494956002, 2.017265875, −0.522189400]`.
  - These coefficients are published in *Spectral Audio Signal Processing*, "Example: Synthesis of 1/F Noise" ([CCRMA page](https://ccrma.stanford.edu/~jos/sasp/Example_Synthesis_1_F_Noise.html), page copyright 2026-08-21). They also appear in Faust's `noises.lib` `pink_filter`, which carries no licence declaration.
  - *Measured:* −10.00 dB/decade from 100 Hz to 1 kHz and −9.99 dB/decade from 1 kHz to 10 kHz. Output RMS is 0.050 for uniform white input, so add about +21 dB of make-up (Faust uses ×12.5).
  - It is direct-form f32 with a pole near 0.999. If f32 precision ever shows, factor it into three first-order sections or keep the state in f64 (*inference*).
- **Rejected pink sources, for licence reasons:**
  - Paul Kellett's filter ([musicdsp archive `pink.txt`](https://github.com/bdejong/musicdsp/blob/master/source/files/pink.txt)) comes from a repo with no licence.
  - Stefan Stenzel's "new shade of pink" ([repo](https://github.com/Stenzel/newshadeofpink)) has a LICENSE saying "public domain" (2026-05-03), but its file headers still say "If used in a commercial product, you should give me one". The two disagree.
- **Determinism:** noise is integer-only. The streams depend on the render seed and the note, never on the patch hash (see the cross-cutting note below).

### Q3 — Multimode filter: TPT/ZDF SVF

**Source.**
- Andrew Simper, "Linear Trapezoidal Integrated State Variable Filter", [`SvfLinearTrapOptimised2.pdf`](https://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf): © 2013, last updated 23 November 2013, typo fixed 3 August 2016, all-pass added 20 November 2016. *Documented.*
- The paper says the structure stays "stable and smooth even with audio rate modulation". Aliasing can still result if the modulation sidebands pass Nyquist.
- The same equations are in fundsp's MIT/Apache [`svf.rs`](https://github.com/SamiPerttu/fundsp/blob/559584043aa3369e92d5aad12a5e3b003cfacfd5/src/svf.rs) and, as Zavalishin's TPT form, in stmlib's MIT `Svf` class.

**Equations.** f32 throughout; `libm::tanf` once per control step.
```
g  = tan(π·fc/fs)            fc clamped to [20 Hz, 0.45·fs]
k  = 1/Q                     Q from res: k = 2 − 1.96·res   (res 0..1 → Q 0.5..25)
a1 = 1/(1 + g·(g + k));  a2 = g·a1;  a3 = g·a2
per sample:  v3 = v0 − ic2
             v1 = a1·ic1 + a2·v3            (band)
             v2 = ic2 + a2·ic1 + a3·v3      (low)
             ic1 = 2·v1 − ic1;  ic2 = 2·v2 − ic2
outputs:     lp = v2;  bp = k·v1 (unity peak);  hp = v0 − k·v1 − v2;  notch = lp + hp
```

**Parameters.**
- **Keytrack:** `fc = cutoff · 2^(keytrack·(note − 60)/12)`.
- **Envelope and LFO:** applied in octaves through the modulation matrix: `fc = cutoff·2^(Σ depth·src)`.
- **Drive:** pre-filter gain into the spike's rational tanh (`x(27+x²)/(27+9x²)`, clamped to ±3), normalised back by 1/drive.
  - Nothing is solved inside the feedback loop: a nonlinear ZDF needs an iterative solve, which is not P1 work.
  - The saturation is not oversampled, so it aliases at high drive. ADAA from Faust's MIT `aanl.lib` comes later.

**Zipper, measured.**
- Test: a 110 Hz polyBLEP saw through a sweep from 100 Hz to 10 kHz, compared with a per-sample `tanf` reference. Error energy relative to the signal (`svf-results.txt`).

| Q | Sweep time | Step every 32 samples | `g` interpolated (needs a division) | `a1..a3` interpolated linearly |
| --- | --- | --- | --- | --- |
| 0.7 | 5 / 20 / 200 ms | −35.7 / −41.2 / −50.8 dB | −54.8 / −62.0 / −74.4 dB | −55.2 / −62.1 / −74.4 dB |
| 5 | 5 / 20 / 200 ms | −18.4 / −19.8 / −30.6 dB | −31.2 / −40.6 / −54.1 dB | −31.4 / −40.6 / −54.1 dB |
| 25 | 5 / 20 / 200 ms | −10.4 / −7.1 / −15.0 dB | −16.0 / −23.4 / −38.0 dB | −17.5 / −23.6 / −38.0 dB |

- Part of the stepped error is a lag of half a block rather than audible zipper, so read these as an upper bound on the artefact.
- **Recommendation:** compute coefficients at the 32-sample control rate and interpolate `a1, a2, a3` linearly per sample, which costs 3 adds and no division.
- **Stability of the interpolation:**
  - *Measured* numerically: along the interpolation path between 20 Hz and 21.6 kHz at Q 25, the eigenvalues of the 2×2 state matrix `[[2a1−1, −2a2], [2a2, 1−2a3]]` stay at |λ| ≤ 0.99995.
  - *Measured* stress test: 10 s of white noise at 0.5, with a random cutoff every 32 samples between 20 Hz and 21.6 kHz at Q 25. No non-finite sample; peak 7.9 when stepped, 2.9 when interpolated.
  - An impulse at fc = 21.6 kHz, Q 25 decays below 1e-6 in 42 ms.

**Determinism and denormals.**
- The only transcendental is `tanf` at control rate.
- Flush `ic1` and `ic2` at 1e-15 once per control step. That is enough for a feed-forward voice that ends at −100 dB, and the spike measured it.
- The cost is folded into the voice figures under Q8. Taking `voice-min` (98 instructions per sample) minus one oscillator (38) leaves about 60 instructions per sample for SVF, drive, amplitude ramp and mixing.

### Q4 — Envelopes and LFOs

**ADSR: own code, from the analog RC model.**
- The "target ratio" idea is described by Nigel Redmon ([EarLevel, 2013-06-03](https://www.earlevel.com/main/2013/06/03/envelope-generators-adsr-code/)). That page states no licence for its code, so it is used for the idea only.
- Each segment runs `level ← base + level·c` per control step: `c = exp(ln(r/(1+r))/N)`, `target = end + (end − start)·r`, `base = target·(1 − c)`, with N = segment time in control steps.
- Because the overshoot is proportional to the segment's span, the segment reaches its end in exactly N steps from any start level.
- Ratios: attack r = 0.3 (gently convex); decay and release r = 1e-4, a strongly exponential curve whose overshoot is 80 dB below the span.
- *Measured* (`misc-results.txt`; within one control step, 0.67 ms):

  | Segment | Set time | Reached after |
  | --- | --- | --- |
  | 0 → 1 | 5 ms | 5.33 ms |
  | 0.5 → 1 | 5 ms | 5.33 ms |
  | 1 → 0.7 | 300 ms | 300.67 ms |
  | 0.7 → 0 | 400 ms | 400.00 ms |
  | 0.18 → 0 | 400 ms | 400.00 ms |
  | 1 → 0.5 | 1,000 ms | 998 ms |

- I first prototyped fixed targets (EarLevel's form), which gives "rate" semantics: decay 300 ms with sustain 0.7 reached sustain after 261 ms. **Recommend segment-duration semantics.** `envelope.attack_ms`, the agent's hints and the lessons then match the knob 1:1.
- **Rates:** evaluate at the 32-sample control rate. The amplitude ramps linearly per sample within each segment, as in the spike. The minimum effective attack is one control step (0.67 ms), which is click-free.
- **Retrigger and legato:**
  - Poly retrigger and mono non-legato restart the attack from the current level. There is no reset to zero, so no click; *measured* 0.181 → 0.285 on the next step.
  - Mono legato does not retrigger the envelopes.
  - A voice ends when the amplitude envelope falls below −100 dB after the attack (spike rule). With r = 1e-4, release reaches 0 at its set time anyway.
- **Determinism:** `expf` and `logf` run only when a coefficient is computed, at most once per segment start; the rest is `+` and `×`.

**LFO.**
- Phase is a `u32` accumulator: `inc = round(rate·CTRL/fs·2^32)`, and the 24-bit phase converts exactly to f32. The phase never drifts and is exact on every platform. *Measured* rate error over 60 s: −4.0e-6 at 0.01 Hz and −4.6e-8 at 5 Hz. The prototype truncates `inc`; rounding halves the error.
- Shapes: sine (`libm::sinf` at control rate), triangle, saw up, saw down, square, sample & hold, and smooth random (cosine-interpolated S&H).
  - S&H draws come from a per-LFO xorshift stream seeded from `derive(seed, module_id ⊕ note_index)`.
- Modes:
  - poly (per voice, phase reset to `phase` at note-on; `retrig = note`);
  - free per voice (seeded random start phase);
  - global (one LFO for all voices, phase 0 at render start, as in the spike).
- **Smoothness:** square and saw edges step at control rate (1.5 kHz). That is acceptable in P1; a `smooth` (slew) parameter can follow.
- **Tempo sync later:** compute phase from song position, `phase = frac(beats·division + phase0)`. That needs a tempo map in the `Performance`. It stays deterministic and stays put across block boundaries.

### Q5 — Reverb port (Signalsmith Basics `reverb.h`)

**Pins as of today (2026-10-19).**

| Repo | Commit | Date and tag | Licence | Notes |
| --- | --- | --- | --- | --- |
| [Signalsmith-Audio/basics](https://github.com/Signalsmith-Audio/basics) | `369e906e03760ec219d07b06fa06dc4fd099f377` (HEAD of `main`) | 2026-09-30, tag 1.2.1 | `LICENSE.txt` is MIT, "Copyright 2022-2025 Signalsmith Audio / Geraint Luff" | 121 stars. `reverb.h` last changed 2026-01-20 (`5fbe38c1`), and its header says "MIT License - see LICENSE.txt and SUPPORT.txt". `SUPPORT.txt`: support expected until 2028-01-01 |
| [Signalsmith-Audio/dsp](https://github.com/Signalsmith-Audio/dsp) | `ca390d28c41ea5108a528e2e81faa4088f2f8ac8` | 2026-10-13, v1.7.3 | MIT, "Copyright (c) 2021 Geraint Luff / Signalsmith Audio Ltd." | Basics' CMake pins v1.7.1. `git diff v1.7.1 v1.7.3` touches only `curves.h` and `windows.h`, so `delay.h`, `mix.h` and `filters.h` are identical |

**What to port.**

From `reverb.h` (409 lines), lines 23–407, `ReverbSTFX`:

| Lines | Part | Port? |
| --- | --- | --- |
| 31–38 | parameters and defaults | Yes |
| 40 | constructor (`maxRoomMs = 200`, `detuneDepthMs = 2`) | Yes |
| 107–123 | `configureSTFX`, which sizes the buffers | Yes |
| 125–138 | `reset` | Yes, and also clear `delay4`, which upstream omits |
| 140–146 | latency 0; `tailSamples = round(sr·rt20·3)` | Yes |
| 148–271 | `processSTFX`, including the fading and non-fading branches | Yes |
| 284–294 | `updateFilters` | Yes |
| 296–300 | `getDecayDb` | Yes |
| 304–396 | `MultiDelay`: `configure`, `reset`, `updateLengths`, `updateLengthsExponential`, `write`, `read`, `read(fade)`, `readDetuned` ×2 | Yes |
| 399–406 | `updateDelays` with its five constant seeds | Yes |
| 42–90 | `state()`, UI metadata | No: becomes `ParamSpec`s |
| 92–105 | `presets()` (the "ambient" values) | No: becomes a patch preset |

From `dsp/delay.h`:

| Lines | Part |
| --- | --- |
| 43–188 | `Buffer`: power-of-two vector, mask, head index; only `resize`, `reset`, `operator[]`, `++` |
| 198–343 | `MultiBuffer`: one buffer with stride = capacity; channel *c* is at offset `c·stride` |
| 394–460 | `_franck_impl::ProductRange` and `InterpolatorLagrangeN<7>` (Franck 2009 product tree; the evaluation order must be kept exactly) |
| 586–608 | `Reader::read` |

From `dsp/mix.h`:

| Lines | Part |
| --- | --- |
| 23–59 | `Hadamard<8>::unscaledInPlace`; the butterfly order is (0,1),(2,3),(0,2),(1,3),(4,5),(6,7),(4,6),(5,7),(0,4),(1,5),(2,6),(3,7) |
| 100–123 | `Householder<8>::inPlace` |
| 158–201 | `StereoMultiMixer<8>`: coefficients from `cos`/`sin(πi/8)` in f64; `scalingFactor2() = 0.5` |

From `dsp/filters.h`, `BiquadStatic` (46–444), only these parts:
- `FreqSpec` (53–76), `octaveSpec` (77–87), `qSpec` (88–94);
- `configure` (100–239), the bilinear and one-sided branches for high-pass, low-pass, low-shelf and high-shelf only (skip `vicanek`);
- `operator()` (244–251), `reset` (253–255);
- `lowpassQ` (282), `highpassQ` (300), `highShelf` (375), `lowShelfQ` (405).

From `stfx/stfx-library.h`, re-implement rather than copy: `LibraryParam::_libStartFade` (333–337), `Block::fade`, `smooth` and `setupFade` (198–244), and the fade bookkeeping in `LibraryEffect::process` (626–675). This is the 20 ms A/B fade:
```
fadeSamples = 20·0.001·sr
end  = fadeRatio + n/fadeSamples
step = end ≥ 1 ? (1 − fadeRatio)/n : 1/fadeSamples
if fadeRatio == 0 { start a fade if any param.current ≠ param.to }
```
- Skip `StopDenormals`: it sets FTZ/DAZ through MXCSR, which our rules forbid.

**Evaluation order matters.** The C++ mixes `double` and `float` deliberately:
- Parameters, smoothing, the LFO phase and filter design are f64; the data path and coefficients are f32.
- `(unit + i)` is a float addition before being widened to double.
- `b1*0.5` in the low-pass uses the already-rounded float `b1`.
- `-2*A*(…)` is `((-2)·A)·(…)`.

The prototype (`proto/src/reverb.rs`) follows every such cast and was bit-exact on its first build. An agent porting it should copy the prototype's casts, not reinvent them.

**Replacing `std::mt19937`.**
- `MultiDelay::updateLengths(seed, …)` is called only with the constants `0x6DD09EE5`, `0x876753A5`, `0x5974DF44`, `0x8CDBF7E6` and `0x0BDDE171`.
- Its 8 `uniform_real_distribution<float>(0,1)` draws and 8 `uniform_int_distribution<int>(0,i)` swap indices per seed do not depend on `roomMs`.
- So the port needs a constant table of 5 × 8 (f32 bits, swap index), plus a test that regenerates it.
- I implemented mt19937, libstdc++'s `generate_canonical<float,24>` (one draw, `float(u)/2^32`, clamped below 1) and libstdc++'s Lemire `uniform_int_distribution` (GCC ≥ 11) in about 80 lines of Rust. *Measured:* all 40 values equal GCC 13's output.
- Distribution algorithms differ between standard libraries. MSVC prints 53 where other platforms print 824 and 925 for the same seed ([WG21 P2059R0](https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2020/p2059r0.pdf)). GROMACS ships its own distributions because "stdlibc++ and libc++ provide different sequences" ([GROMACS docs](https://manual.gromacs.org/current/doxygen/html-lib/uniformintdistribution_8h.xhtml)).
- libc++ was not installed here, so this is *documented*, not measured. The constant table also makes the port immune to a libstdc++ change; one landed in 2026-08 for non-zero-min generators ([PR118665](https://gcc.gnu.org/pipermail/libstdc++/2026-August/067645.html)), which does not affect mt19937.
- **Do not replace the table with the engine's seeded PRNG from the patch hash and module id** (ADR-0006's wording). It would retune the reverb's delay lengths every time any parameter of the patch changed. If variety is wanted later, use a `seed` parameter on the reverb module, stored in the patch.

**Allocation, parameter changes, denormals and sample rate.**
- **Allocation:** in `prepare`, sized from `maxRoomMs = 200` and the sample rate. At 48 kHz that is 409,600 floats (1.64 MB) over six multi-delays, each rounded up to a power of two; at 96 kHz about 3.3 MB.
- **Room size:**
  - A `roomMs` change never re-allocates. At the start of a fade, `updateDelays` recomputes 48 integer offsets, and reads cross-fade from the old taps to the new ones over 20 ms. That is 15 blocks of 64, which is deterministic because the internal block is fixed.
  - Upstream does not clamp `roomMs`. A value above `maxRoomMs` would read outside the per-channel stride and wrap into other channels' data, so the port must clamp to 10–200 ms.
- **Click test** (*measured*, `reverb-tests.txt`): wet-only output of a 440 Hz sine, comparing the maximum |second difference| in the 50 ms after a change with the 0.5 s before:

  | Change | After / before |
  | --- | --- |
  | `roomMs` 80 → 150 | 1.47 |
  | `roomMs` 80 → 20 | 1.17 |
  | `rt20` 1 → 6 s | 1.15 |
  | high cut 12 kHz → 2 kHz | 1.44 |
  | detune 2 → 20 | 1.17 |

  There are no spikes. This is a proxy metric; nobody listened.
- **Filters and modulation:** filter coefficients are recomputed every block without smoothing. `detune` and `roomMs` feed the LFO rate from their current values, not the faded ones.
  - So `roomMs` and `decay` should be `modulatable: false` in `ParamSpec`.
  - Reverb parameters should use `smoothing: None`: the internal fade already smooths them, and our smoother would add a second ramp.
  - A macro moving them every block would retrigger fades continuously (*inference*).
- **Denormals** (*measured*: 1 s noise burst, then 60 s of silence, default parameters):
  - Flushing at 1e-15 on every feedback write and on every biquad output: 0 subnormal output samples, 0 subnormal state values, output exactly zero from 15.1 s, 0.99 s wall time.
  - Without flushing: 2,337,601 subnormal output samples, 229,118 subnormal state values, never reaches zero, 4.15 s (4.2× slower).
  - The flush did not change the golden comparison, because nothing falls below 1e-15 in 4 s with signal.
- **Sample rate:** delays in samples are `roomMs·sr`; filters use `Hz/sr`; the LFO's depth is in ms and its rate is independent of sr. *Measured* impulse responses: −20 dB energy decay at 0.58 s (44.1 kHz), 0.58 s (48 kHz) and 0.61 s (96 kHz). Total energy scales with the low-pass's share of the band, as expected for a unit impulse.

**Bit-exact golden test against the C++: yes, it works.**
- *Measured* (`reverb-golden/harness.cpp`, 51 lines, using upstream headers unmodified).
- Input: a deterministic SplitMix64 noise burst plus an impulse; block 64; 4 s stereo.
- Scenario 0 is the defaults; scenario 1 is the "ambient" preset. Both change `roomMs`, `rt20` and the high cut at 2 s.

| Build | Result against the Rust port |
| --- | --- |
| g++ 13.3 `-O2 -ffp-contract=off` | identical (SHA-256 `d3e56e9b…849c` and `28fd00f1…e86b`) |
| clang 18.1.3 `-O2 -ffp-contract=off` | identical |
| clang `-O2` with default contraction, baseline x86-64 | identical (no FMA instructions to contract into) |
| g++ `-O2 -march=native` (FMA) | 383,549 of 384,000 samples differ, max 1.8e-5 |
| clang `-O2 -march=native` | differs, and also differs from g++ |
| g++ `-Ofast` | max difference 5.2e-5 |
| Rust: release, debug, `target-cpu=native`, `wasm32-wasip1` on V8, with and without `simd128` | one hash: fnv64 `f490ace427692965` (scenario 0), `e5c4525ffa49771c` (scenario 1) |

Conditions for the C++ reference build:
- baseline x86-64 (or `-ffp-contract=off`), no fast-math;
- libstdc++, though this only matters if the RNG is kept;
- glibc's f64 `cos`/`sin`/`pow`/`tan`/`sinh` may differ from Rust `libm` by an ulp, but the results are rounded to f32 before they reach the data path. *Measured:* no difference in 768,000 samples.

Treat the C++ comparison as a one-time validation of the port:
- Commit the C++ output hashes and the harness, for example as `tools/reverb-cxx-golden/` outside the cargo build, with the build commands in `docs/dsp/reverb.md`.
- Make the Rust hash the cross-platform golden. CI then needs no C++ toolchain, as AGENTS.md asks.

**Cost.**
- *Measured:* 2,574 instructions per stereo frame (callgrind), 342 ns per frame (wall), 61× realtime, about 1.6% of a core.
- Removing bounds checks (an `unsafe` experiment that changed no output bits) saved only 7% (2,404).
- The per-sample f64 `cos`+`sin` for the detune LFO is about 6%. Replacing it with a rotating phasor would break the match with upstream.
- One reverb costs about 11 full voices. It is still affordable as a single global instance.

**fundsp fallback status.** *Measured.*
- fundsp 0.23.0 is still the latest release (crates.io, 2026-01-07); the last commit is still `5595840` (2026-03-03).
- `default-features = false, features = ["std"]` builds for `wasm32-unknown-unknown` with 38 crates in the tree, against 81 with default features. Without `std` it fails (`Vec`/`vec!` not found), as ADR-0006 says.
- Its scalar transcendentals go through `libm` (good). Its SIMD paths use `wide`'s `f32x8::exp` and `mul_add` (`oversample.rs`), so a fallback would need its own cross-platform hash check.
- With the port bit-exact, the fallback is unlikely to be needed. Keep it only as the A/B reference.

### Q6 — Compressor

**Sources, checked per function.**
- Faust [`compressors.lib`](https://github.com/grame-cncm/faustlibraries/blob/2edec0537f536eaa130d42c354884d7db971663c/compressors.lib), repo HEAD `2edec053` (2026-10-15); the file was last changed in `9a8080eb` (2026-10-09). `compressor_lad_mono` (line 958), `compressor_mono` (999), `compressor_stereo` (1036) and `compression_gain_mono` (1080) declare `LicenseRef-STK-4.3`, "Copyright (C) 2014-2020 by Julius O. Smith III <jos@ccrma.stanford.edu>".
- The helpers they call carry no licence declaration: `an.amp_follower_ar` (`analyzers.lib` 292, author Jonatan Liljedahl, revised by Romain Michon), `si.onePoleSwitching` (`signals.lib` 593), `ba.tau2pole`, `ba.linear2db`, `ba.db2linear` and `si.smooth`. They are one-line textbook formulas, so implement them from the equations.
- JOS's book page ([Dynamic Range Compression](https://ccrma.stanford.edu/~jos/filters/Nonlinear_Filter_Example_Dynamic.html), Introduction to Digital Filters, page copyright 2026-08-21) describes compression only qualitatively. The equations below come from the Faust code.

**JOS `compression_gain_mono(ratio, thresh, att, rel)`, exactly.**
```
p(τ)    = τ ≈ 0 ? 0 : exp(−1/(τ·fs))                      (ba.tau2pole)
env[n]  = c·env[n−1] + (1−c)·|x[n]|,  c = |x[n]| > env[n−1] ? p(att) : p(rel)   (amp_follower_ar)
L[n]    = 20·log10(max(MIN, env[n]))
G[n]    = max(L[n] − thresh, 0) · (1/max(ε, ratio) − 1)         (outminusindb; ≤ 0 dB)
Gs[n]   = b·Gs[n−1] + (1−b)·G[n],  b = p(att/2)                 ("kneesmooth" = si.smooth)
gain[n] = 10^(Gs[n]/20)
compressor_mono:   y = x · gain                                  (compressor_lad_mono with lad = 0)
compressor_stereo: gain from |x| + |y|, applied to both channels
```
- JOS's function has no quadratic soft knee. What it calls the "knee" is the one-pole smoothing of the gain in dB.
- It has no make-up gain, and its stereo link sums |L| + |R|. A centred mono signal therefore reads 6 dB hot.

**Proposed enth compressor.** JOS's structure with three documented changes:
1. Detector `e = max(|L|, |R|)`, the Chromium and web-audio-api-rs link.
2. The Giannoulis–Massberg–Reiss soft knee replaces the hard `max(·, 0)`. *Documented:* JAES 60(6), 399–408, 2012 ([AES e-library](https://aes2.org/publications/elibrary-page/?id=16354)). It reduces to JOS's curve at W = 0.
3. Make-up gain M in dB.

```
d = L − T
G = 0                                  if 2d ≤ −W
G = (1/R − 1)·(d + W/2)² / (2W)        if |2d| < W   (W > 0)
G = (1/R − 1)·d                        if 2d ≥ W
Gs = b·Gs + (1−b)·G;  gain = 2^((Gs + M)·log2(10)/20);  yL = gain·L, yR = gain·R
```
- Implement it with `libm::log2f` and `libm::exp2f`: `20·log10(x) = 6.0206·log2(x)`.
- Flush `env` and `Gs` at 1e-15.
- No look-ahead and zero latency, as ADR-0006 requires.

*Measured* (`misc-results.txt`):
- The static curve at T −24 dB, R 4: 0 dB at or below −24 when W = 0; −4.50 dB at −18; −13.50 dB at −6. The knee is continuous: −0.56 dB at −24 when W = 6, and −1.12 dB at −24 when W = 12.
- A 1 kHz sine at −6 dBFS settles at −12.93 dB of gain. The static value on the peak is −13.5; the follower ripples just under the peak.
- Step response with attack 10 ms and release 200 ms: 63% of the final reduction after 12.5 ms; release to 36.8% after 267 ms.
- The effective times are longer than the nominal ones because two smoothers are cascaded (JOS's design). Document this in `docs/dsp/compressor.md`, or define attack and release as the measured 63% times.
- Cost: 160 instructions per stereo frame, about 48 ns per frame.

**Cross-check: web-audio-api-rs `dynamics_compressor.rs`.**
- [File at `03d0cb28`](https://github.com/orottier/web-audio-api-rs/blob/03d0cb28983896be5cde38d3961a47bdefb8ea8f/src/node/dynamics_compressor.rs), 615 lines. MIT, "Copyright (c) 2021 Otto Rottier".

Provenance:
- Written by contributor b-ma between 2022-07-27 ("first working version") and 2022-07-28; 29 commits on the file since, the latest 2025-11-28.
- The comments cite the Web Audio spec's compression curve and the Giannoulis 2012 paper (eq. 4, 7, 16), and add "seems coherent with chrome implementation". It is not a copy of Chromium code (*documented* from the file and `git log`).

How it differs from our design:
- **Look-ahead:** yes, a ring of render quanta sized `ceil(fs·0.006/128) + 1`. At 48 kHz that is 4 quanta, 3 of which are delay (384 samples). The ring also grows with `resize` on the first `process()` call, which is an allocation on the audio thread.
- **Detector:** log-domain, after the gain computer (the static curve on the instantaneous level, then a branching attack/release one-pole on the reduction); stereo is linked by max.
- **Knee:** the threshold is shifted by +knee/2 per the spec, which the comment itself calls "probably wrong".
- **Make-up:** automatic, `(1/fullRangeGain)^0.6`.
- **Maths:** `std` `powf`, `log10` and `exp`, which are not deterministic across platforms.

*Measured:* with WA's threshold set to `T − W/2`, our gain computer equals WA's formula within 9.5e-7 dB from −60 to 0 dB (W = 12, R = 4).

Use it as a test oracle for the static curve only. Do not port its detector or delay. Its log-domain "smooth branching after the gain computer" is the variant the Giannoulis paper recommends, so it is the obvious alternative if listening finds JOS's cascaded smoothers sluggish.

### Q7 — Global FX bypass

**Placement** (*inference*, consistent with ADR-0006 and design.md §5):
- **The switch:** `MonitorState { fx_bypass: bool }`, owned by the plugin instance (`enth-plugin`, L8). It is persisted in the plugin's state (a nice-plug persisted field), never in `Patch`, so it survives preset changes. The engine receives it as a real-time event or atomic, not as a parameter.
- **Which modules it bypasses:** effect modules whose `ModuleKind` metadata has `time_based: true`. That is the reverb in P1, and later delay and chorus. Users asked for "a global control for turning preset time based FX on and off" (VI-CONTROL, cited in the effects research). The compressor and future EQ or saturation stay on, because they set level and tone.
- **Behaviour:** a bypassed module becomes identity (output = input, not the patch's `dry` gain).
  - It cross-fades over 20 ms, then stops processing, so it costs zero CPU.
  - On un-bypass it calls `reset()` (a memset of 1.64 MB, roughly 0.1 ms, inside one block; *inference*) and fades back in.
  - None of this needs to be deterministic, because none of it is scored.
- **Why scoring and the CLI ignore it by construction:**
  - `enth-render`'s offline API takes (patch, performance, sample rate, block, seed, engine version) and builds the engine with `MonitorState::default()`.
  - It has no parameter or CLI flag for the bypass, so `enth render`, `describe`, scoring and lessons cannot see it.
  - Test: one engine with the bypass on, and another with `time_based` modules set `enabled = false` in the patch, must give equal output once the fade has settled. Separately, the golden test proves `enth render` does not depend on any `MonitorState`.
- **Not the same as `enabled`:** `ModuleInstance.enabled` is patch content and is scored. The bypass is a listening preference.

### Q8 — CPU budget

**Prototype voice** (`proto/src/bin/bench.rs`; 48 kHz; block 64; control rate 32; module-at-a-time over 32-sample segments; scalar):

| Case | Contents | Instructions per sample (callgrind) | Wall | ×realtime (one core) |
| --- | --- | --- | --- | --- |
| `voice-min` | 1 Plaits saw, SVF low-pass, drive, 1 ADSR | 97.8 | 15.6 ns | 1,333× |
| `voice-full` (the P1 voice) | 3 Plaits oscillators (saw, saw +7 ct, square −12 st), pink noise, SVF with interpolated `a1..a3`, drive, amp and filter ADSR, triangle LFO to vibrato and cutoff | 229.0 | 32.2 ns | 647× |
| `voice-wt` | as `voice-full`, but osc 3 is a 2-frame morphing Hermite wavetable | 323.0 | 46.5 ns | 449× |
| `v16-full` | 16 × `voice-full`, stereo sum | 3,661 per frame | 497 ns per frame | 42× |
| `reverb` | Signalsmith port, stereo | 2,574 per frame | 342 ns | 61× |
| `comp` | stereo, link by max | about 160 per frame | about 48 ns | about 377× |
| **`full16`** | 16 voices + compressor + reverb | **6,353 per frame (305 M per second of audio)** | **920 ns per frame** | **22.7× (4.4% of a core)** |

- The 16-voice prototype's hash is identical on native, wasm (V8) and `target-cpu=native`: `42119d226886b9f6`.
- Wall-clock figures are best of 7 under `flock -x` at load 6.9–7.1.
- For comparison, the spike's P1-like voice was 146 instructions per sample (2-point polyBLEP saws, no noise, coefficients stepped).
- Lanes (f32x8) cut the spike's instructions 1.8–1.95× at 16 voices. Plaits' oscillator would need a branch-free rewrite to benefit.

**Proposed G1 budget.**

*Reference patch "G1-16":* the `full16` patch above, with all 16 voices sounding and the effects on.

*Gates:*
1. Offline render at least 10× realtime on one core of the dev laptop (no more than 10% of a core). Best of N under the quiet lock, with load average and environment recorded per `docs/benchmarks.md`. The prototype gives 22.7×, which leaves 2.3× headroom for the modulation matrix, smoothing, plan dispatch and the voice manager.
2. At most 12,000 instructions per stereo frame for G1-16, as a gungraun hard limit. Re-base it to the first product measurement +10%, as the spike did. Sub-budgets for diagnosis:
   - voice ≤ 400 instructions per sample;
   - reverb ≤ 3,000 instructions per frame;
   - compressor ≤ 250 instructions per frame.
3. p99 `process()` time at most 25% of the 64-sample deadline (0.33 ms of 1.33 ms) in the test host. This row is already in `docs/benchmarks.md`.
4. The subnormal test (a 40 s tail with zero subnormals) on G1-16, effects included.

*Stretch target:* 20× realtime. *Wasm:* expect 80–90% of native, per the spike.

### Cross-cutting: seeds must not come from the patch hash

design.md §5 says: "per voice from the patch hash and note index; per effect from the patch hash and module id".

The problem (*inference*):
- Every op changes the patch hash. Under that rule, any knob move would also change every voice's start phases, noise streams, S&H values and any seeded effect.
- `render()` would no longer be a continuous function of the parameters for a fixed seed. Finite-difference sensitivity hints and the Jacobian (§7), CMA-ES and the "how far were your knobs" scores would all see seed noise.
- Two patches that differ only in an inaudible parameter would also differ in phase.

Recommendation:
- Derive streams from the render's `seed` input plus stable structural ids: `derive(seed, note_index)` per voice, `derive(seed, module_id ⊕ note_index)` per poly module, `derive(seed, module_id)` per global module.
- If a patch wants a different "random" character, give the module an explicit `seed` parameter, stored in the patch.
- `spike-engine` already uses the render seed, not the patch hash, so the spike is right and the doc is wrong.

## Recommended module set

| Module | Algorithm | Source and licence | Parameters (plain units; default) | CPU (measured) | Golden test |
| --- | --- | --- | --- | --- | --- |
| `osc` (×3) | Plaits variable-shape core: one-sample-delay polyBLEP/BLAMP, PWM, hard sync; pitch from `libm::exp2f` at control rate, increments ramped per sample | Port of `variable_shape_oscillator.h` (MIT, © 2016 Emilie Gillet) and `stmlib/dsp/polyblep.h` (MIT, © 2017 Emilie Gillet) | `wave` enum {sine, triangle, saw, pulse, table}, saw; `octave` −3…+3 stepped, 0; `semi` −12…+12 st, 0; `fine` −100…+100 ct, 0; `pw` 5…95 %, 50; `level` 0…100 %, 100; `sync` toggle (osc 2/3 to osc 1), off; `phase` enum {free, reset}, free; later `unison` 1…8, `detune` ct, `spread` % | 37.7 instr/sample (saw), 53.5 with sync | Hash of 1 s at 55, 440 and 3,520 Hz for saw, pulse 50%, pulse 20%, triangle and sync ×2.37. Alias ceilings at 440 Hz: saw ≤ −38 dB, triangle ≤ −80 dB. 10 s zero-crossing frequency accuracy ≤ 1e-4. Peak ≤ 1.1 |
| `osc` table mode | Mip-mapped tables, 2048 × 11 levels, Hermite in phase, linear across frames; level chosen so the top harmonic is ≤ fs − 20 kHz | Own code; tables generated from harmonic formulas (no assets) | `table` enum {basic, pulse, tilt, formant, organ}; `position` 0…100 % | 62.3 instr/sample per read, about 2× with morph; 3.2 ms per frame to generate | Hash of each table's generated data; render hash; alias ≤ −85 dB at 440 and 3,520 Hz |
| `noise` | xorshift32 white; JOS 3rd-order pink filter | Own code; coefficients from JOS *SASP* (book example) | `color` enum {white, pink}; `level` 0…100 % | about 10–25 instr/sample (estimate from voice differences) | Hash of 1 s per seed; distinct streams per note; pink slope −10 ± 0.3 dB/decade over 100 Hz–10 kHz |
| `filter` | Cytomic/Simper TPT SVF; `tanf` per 32 samples; `a1..a3` interpolated per sample; pre-filter rational-tanh drive | Own code from the paper (© Simper 2013–2016; cite) | `mode` enum {lp, bp, hp, notch}, lp; `cutoff` 20…20,000 Hz log, 2,000; `res` 0…100 %, 10 (Q 0.5…25); `keytrack` 0…100 %, 0; `drive` 0…24 dB, 0. Envelope and LFO through mod routes in octaves | about 60 instr/sample incl. drive and amplitude ramp | Impulse-response hashes for 4 modes × 3 cutoffs × 2 Q values; \|H\| at DC, fc and Nyquist against the analytic value within 0.1 dB; random-jump stress with no NaN; state 0 after silence |
| `env` (×2: amp, filter) | RC exponential with span-proportional overshoot (segment-duration semantics), control rate 32 | Own code (idea: EarLevel 2013) | `attack` 0…10,000 ms log, 5 (min one 0.67 ms step); `decay` 1…10,000 ms, 300; `sustain` 0…100 %, 70; `release` 1…20,000 ms, 400; amp only: `velocity` 0…100 %, 100 | negligible (control rate) | Timing within one control step; retrigger continuity; voice ends at −100 dB by its release time; hash of the envelope output |
| `lfo` (×1–2) | u32 phase at control rate; `sinf`; seeded S&H | Own code | `shape` enum {sine, triangle, saw_up, saw_down, square, sample_hold, smooth_random}, sine; `rate` 0.01…50 Hz log, 2; `phase` 0…360°, 0; `retrig` enum {note, free}, note; `scope` enum {poly, global}, poly; later `sync` divisions | negligible | 60 s rate accuracy ≤ 1e-5; phase-reset test; hash per shape; S&H depends on the seed and not on the patch hash |
| `reverb` (global) | Signalsmith Basics FDN: 4 diffusers + 8-channel feedback, Householder, Lagrange-7 detune, shelving damping; 20 ms A/B parameter fade | Port of `reverb.h` (MIT, © 2022–2025 Signalsmith Audio / Geraint Luff) and dsp `delay.h`, `mix.h`, `filters.h` (MIT, © 2021); fade re-implemented from `stfx-library.h` (BSL-1.0 header) | `dry` −60…+6 dB, 0; `wet` −60…+6 dB, −6; `room` 10…200 ms, 80; `decay` (RT60 = 3·rt20) 100…30,000 ms, 3,000; `early` 0…250 %, 150; `detune` 0…50, 2; `low_cut` 10…500 Hz, 80; `high_cut` 1,000…20,000 Hz, 12,000; `low_damp` 1…10 ratio, 1.5; `high_damp` 1…10 ratio, 2.5. Basic tier: `wet`, `room`, `decay`. `modulatable: false` for `room` and `decay`; `smoothing: None` for all | 2,574 instr/frame, about 1.6% of a core | Rust == C++ `-O2` (the two committed SHA-256s); Rust hash on every platform; 61 s tail with 0 subnormals; `assert_no_alloc` around `process`; parameter-change second-difference ratio < 3; the mt19937 table regenerated in a test |
| `comp` (global) | JOS structure, link by max, Giannoulis soft knee, make-up gain, no look-ahead | Own code, following JOS `compression_gain_mono` (LicenseRef-STK-4.3, © 2014–2020 J.O. Smith III); WA-rs (MIT) as a test oracle | `threshold` −60…0 dB, −18; `ratio` 1…20, 4; `attack` 0.1…100 ms log, 10; `release` 10…2,000 ms log, 200; `knee` 0…24 dB, 6; `makeup` 0…24 dB, 0 | about 160 instr/frame | Static-curve table; agreement with the WA formula within 1e-5 dB; steady-state sine; step-response 63% times; render hash |
| FX bypass (not a module) | Monitor mask on `time_based` effects; 20 ms fade; identity when bypassed | – | Plugin state only | 0 when bypassed | Bypass on == `enabled = false` after the fade; `enth render` has no bypass input |

## THIRD_PARTY_NOTICES entries needed

In the format of the repo's `THIRD_PARTY_NOTICES.md`. Each entry also needs the full licence text appended, and the `enth` paths are proposals.

```markdown
### Signalsmith Basics (reverb)

- Upstream: <https://github.com/Signalsmith-Audio/basics> at commit `369e906e03760ec219d07b06fa06dc4fd099f377`
- Licence: MIT
- Copyright: Copyright 2022-2025 Signalsmith Audio / Geraint Luff
- Derived enth files: `crates/enth-dsp/src/reverb.rs`
- Changes from upstream: `std::mt19937` and the libstdc++ distributions replaced by a constant table of their draws for the five
  constant seeds (regenerated by a test); denormals flushed in code (feedback writes and biquad outputs below 1e-15) instead of
  FTZ/DAZ (`StopDenormals` dropped); `reset()` also clears `delay4`; `roomMs` clamped to `maxRoomMs`; the STFX wrapper's 20 ms
  parameter fade re-implemented; parameters exposed in dB and ms.

### Signalsmith DSP library (delay, mix, filters)

- Upstream: <https://github.com/Signalsmith-Audio/dsp> at commit `ca390d28c41ea5108a528e2e81faa4088f2f8ac8` (v1.7.3; these
  files are identical in v1.7.1)
- Licence: MIT
- Copyright: Copyright (c) 2021 Geraint Luff / Signalsmith Audio Ltd.
- Derived enth files: `crates/enth-dsp/src/reverb.rs` (or `delay.rs`, `mix.rs`, `biquad.rs`)
- Changes from upstream: only `Buffer`/`MultiBuffer`, `InterpolatorLagrangeN<7>`, `Reader::read`, `Hadamard<8>`, `Householder<8>`,
  `StereoMultiMixer<8>` and the high-pass, low-pass, low-shelf and high-shelf `BiquadStatic` designs; f64 transcendentals through
  `libm`.

### Signalsmith STFX library (parameter fade) — only if the re-implemented fade counts as derived

- Upstream: <https://github.com/Signalsmith-Audio/basics/blob/369e906e03760ec219d07b06fa06dc4fd099f377/include/stfx/stfx-library.h>
- Licence: BSL-1.0 (file header: "Released under the Boost Software License"; the repo LICENSE.txt is MIT)
- Copyright: Copyright 2021-2022 Geraint Luff / Signalsmith Audio Ltd
- Derived enth files: `crates/enth-dsp/src/reverb.rs` (fade bookkeeping only)
- Changes from upstream: re-implemented arithmetic of the 20 ms A/B fade; no code copied.

### Mutable Instruments Plaits (variable-shape oscillator)

- Upstream: <https://github.com/pichenettes/eurorack> at commit `f4cfa7697095a28d292bff758beb51deb49b5a86`,
  `plaits/dsp/oscillator/variable_shape_oscillator.h` (and the Hermite interpolator in `wavetable_oscillator.h`)
- Licence: MIT
- Copyright: Copyright 2016 Emilie Gillet.
- Derived enth files: `crates/enth-dsp/src/osc.rs`
- Changes from upstream: parameters ramped at enth's 32-sample control grid; max frequency and enth's sync wiring; Rust.

### Mutable Instruments stmlib (polyBLEP)

- Upstream: <https://github.com/pichenettes/stmlib> at commit `d18def816c51d1da0c108236928b2bbd25c17481`, `dsp/polyblep.h`
- Licence: MIT
- Copyright: Copyright 2017 Emilie Gillet.
- Derived enth files: `crates/enth-dsp/src/osc.rs`
- Changes from upstream: none beyond translation to Rust.

### Julius O. Smith III — Faust compression_gain_mono / compressor_mono

- Upstream: <https://github.com/grame-cncm/faustlibraries> at commit `2edec0537f536eaa130d42c354884d7db971663c`,
  `compressors.lib` (`compressor_mono`, `compression_gain_mono`)
- Licence: LicenseRef-STK-4.3
- Copyright: Copyright (C) 2014-2020 by Julius O. Smith III <jos@ccrma.stanford.edu>
- Derived enth files: `crates/enth-dsp/src/compressor.rs`
- Changes from upstream: hand-written Rust, not Faust compiler output; stereo link by max(|L|,|R|) instead of |L|+|R|; Giannoulis et
  al. (2012) quadratic soft knee; make-up gain; denormal flush.
```

No entry is needed for web-audio-api-rs unless its formula is copied into a test. In that case add MIT, "Copyright (c) 2021 Otto Rottier".

The Cytomic paper, the Giannoulis paper and the JOS pink-filter coefficients are citations for `docs/dsp/*.md`, not notices.

## Plan check

Claims in the docs and earlier research, checked against today's evidence:

| Claim (where) | Verdict |
| --- | --- |
| ADR-0006: port `reverb.h` with the MIT helpers `delay.h`, `mix.h`, `filters.h` | **Confirmed**, with one catch: matching upstream's parameter behaviour needs the fade logic of `stfx-library.h`, whose header says BSL-1.0. Add BSL-1.0 to ADR-0004's ported-source list (it is permissive and OSI-approved; deny.toml already allows it as a scoped exception), or re-implement the fade from a written spec. Optionally the owner could ask upstream to correct the header; opening that issue is the owner's call |
| ADR-0006: "replace `std::mt19937` with the engine's seeded PRNG (seeded from the patch hash and module id), or reproduce mt19937 exactly" | **Change.** The seeds are constants, so use a constant table: it is exact, needs no RNG and is portable. Seeding from the patch hash would retune the reverb on every op |
| ADR-0006: a bit-exact golden test against C++ `-O2` without fast-math | **Confirmed, measured.** Add "no FMA contraction (baseline x86-64 or `-ffp-contract=off`)" to the condition |
| ADR-0006: the port takes 2–4 days (unvalidated) | **Partly validated.** The scratch port became bit-exact on its first build within this session. Product hardening (ParamSpecs, tests, docs, notices) is the remaining work, plausibly 1–2 days |
| ADR-0006 and the roadmap: compressor "feed-forward, stereo-linked, attack/release, soft knee, make-up gain … follows JOS `compressor_mono`" | **Partly wrong.** JOS has neither a soft knee nor make-up gain, and links by \|L\|+\|R\|. The knee comes from Giannoulis 2012 and the link by max is our choice; say both in `docs/dsp/compressor.md` |
| ADR-0006: fundsp fallback with `features = ["std"]` | **Confirmed:** builds for `wasm32-unknown-unknown`, 38 crates, still 0.23.0. Its SIMD paths use `wide::mul_add` and `f32x8::exp`, so a fallback needs its own hash check |
| Effects summary: "TPT SVF after Cytomic's paper, which fundsp and DaisySP already follow" | **Wrong for DaisySP.** [`Source/Filters/svf.cpp`](https://github.com/electro-smith/DaisySP/blob/2c72eaf9eac5fc0dca1919d65d606da907832618/Source/Filters/svf.cpp) is the double-sampled Chamberlin SVF from Simper's old musicdsp post. fundsp and stmlib are TPT |
| design.md §5: seeds "from the patch hash" | **Change** (see the cross-cutting note): derive from the render seed, the note index and the module id |
| design.md §3: control rate 32 with interpolation | **Confirmed** for envelopes and LFOs. The filter needs per-sample interpolation of `a1..a3` (measured 13–23 dB better than stepping) |
| design.md §5, rule 4: flush below 1e-15 and end voices at −100 dB | **Confirmed** for the reverb: 0 subnormals against 2.34 M unflushed; 4.2× wall |
| Roadmap G1: "16-voice patch within the CPU budget *(set in P0)*" | **Not set yet.** Proposal under Q8: ≥ 10× realtime, ≤ 12,000 instructions per frame, p99 ≤ 25% of the deadline |
| `docs/benchmarks.md`: ≥ 100× realtime per core for a basic patch | Comfortably met in the prototype: 1,333× for the minimal voice, 647× for the full voice |
| AGENTS.md: effects report zero latency | **Confirmed:** the reverb's `latencySamples()` is 0; the compressor has no look-ahead (web-audio-api-rs's 6 ms look-ahead must not be ported) |
| Roadmap P1: "2–3 oscillators (classic waveforms plus a few wavetables)" | Feasible as one `osc` module with a table mode: 5 generated tables, about 51 ms to generate each 16-frame table on first use |

## Gaps

- **No listening.** I may not play audio, so the blind A/B for the reverb, zipper audibility, envelope curve shapes and compressor feel are untested. Every quality statement here is a metric.
- **libc++ and MSVC distribution differences** are documented (P2059R0, GROMACS), not measured: libc++ is not installed. This does not matter once the table is constant.
- **Prototype hashes** were run on Linux x86-64 native, wasm32 under V8 (node 22) and `target-cpu=native` only. Not wasmtime (not on PATH here), not Windows, not aarch64.
- **Wall-clock timings ran at load average about 7**, so read them as ±20%. Instruction counts are exact but tied to this toolchain and `libm =0.2.16`.
- **The SVF zipper figure** is error energy against a per-sample reference, which includes lag. No spectral or perceptual zipper analysis was done.
- **Drive aliasing** (tanh without oversampling) was not measured.
- **The compressor** was not compared with Faust compiler output (Faust is not installed). It was checked only against the equations and the web-audio-api-rs formula.
- **Wavetables:**
  - the five table definitions are a proposal;
  - level cross-fading and the "≤ fs − 20 kHz" level rule were not prototyped (the prototype uses strict Nyquist);
  - the deterministic f64 FFT generator was not written.
- **Unison and SIMD lanes for Plaits' oscillator** were not prototyped.
- **Noise cost** is estimated from differences between voice variants, not isolated.
- **Licence edge cases:** the JOS pink-filter coefficients (a book example; low risk), the BSL-1.0 header on `stfx-library.h` (needs an ADR-0004 decision), and Plaits' `kMaxFrequency` and other upstream constants are not licence questions but need deciding.
- **Owner decision needed:** envelope time semantics. I recommend segment duration; "rate" semantics is the analog-style alternative.
- **Not verified:** the venue of Välimäki & Huovilainen's 2007 polyBLEP article (not needed: the algorithm is taken from the MIT stmlib code), and the full text of the Giannoulis paper (paywalled; the knee formula was cross-checked through web-audio-api-rs's comments and code).
- **Process note:** one early crates.io API call sent a User-Agent containing the owner's email before the coordinator's rule arrived. All later requests used none, or a generic one.
