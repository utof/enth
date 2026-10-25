# 05 — DSP algorithms and permissive sources for the next modules

- **Sub-researcher:** 05, session 2026-10-25 (S2 Depth and a faster training loop).
- **Question:** for each candidate S2 module: algorithm, aliasing strategy, cost against enth's budget, state, determinism and real-time risks, best permissive source (licence verified), study-only GPL references, effort. Then a library scan and a ranked build list.
- **Method:** read enth's engine and `enth-dsp` (`main` at `1c99e80`); shallow/sparse clones of candidate repos into `~/code/enth-scratch/2026-10-25-s2ft/05/src/` for LICENSE files and per-file headers (nothing built or run); GitHub and crates.io APIs (User-Agent `enth-research/0.1`); a callgrind micro-benchmark of candidate kernels (`…/05/bench/`, Rust 1.99.1, `libm =0.2.16`, x86-64 baseline).
- **Labels:** *measured* (I ran it, or enth's recorded benches), *documented* (source code, paper, manual), *anecdotal* (forums, secondary write-ups).

## TL;DR

1. **The engine's shape matters more than the algorithms.** Generators sum into a **mono** voice bus, per-voice FX run on it, and modulators tick once per 32-frame segment (`engine.rs` step 7). Three S2 features change that: **FM/PM/AM/RM between generators** (a buffer per generator, DAG order like the sync bus), **stereo unison** (a stereo voice bus, doubling per-voice FX cost) and **tempo sync** (no tempo exists in patch, engine or `render()`; delay, gate, sequencer and synced MSEG need it as a render input).
2. **Distortion: ADAA per voice, IIR oversampling globally.** ADAA-1 with polynomial antiderivatives costs 20–25 instructions per sample in `f32`, ADAA-1 `tanh` (f64 `log cosh`) 151, a 2× polyphase-allpass half-band up/down with a shaper 59 (all measured). The IIR reports no latency; a FIR half-band does (Signalsmith `crunch.h`: 32 samples), which ADR-0006 forbids.
3. **FM = DX-style phase modulation on the sine/wavetable cores**, not the polyBLEP core: 52 instructions per sample for a 2-op pair with feedback using a `u32` phase and sine table, 111 with enth's polynomial sine, 140 with `libm::sinf` (measured). **Plaits' six-op FM (MIT) and Dexed's MSFA core (Apache-2.0 per file) are portable.**
4. **Unison is the CPU risk and the natural SIMD target:** 7 saw copies × 16 notes ≈ 4,700 instructions per frame against ≈ 5,700 of G1 headroom (6,274 of 12,000 measured). Identical code on different phases suits `wide` f32x4/f32x8, which the Phase 0 spike measured bit-identical to scalar.
5. **`wide` 1.7.1 has traps the clippy bans miss** (its source): `recip`/`recip_sqrt` are approximate `_mm_rcp_ps`/`_mm_rsqrt_ps` on SSE but exact division on wasm32/NEON, and `exp`, `exp2`, `ln`, `sin_cos`, `pow` call `mul_add` internally. Only the `mul_add` family is banned today. `std::simd` is still nightly (2026).
6. **Every modulation effect has a verified permissive source** (Plaits/Rings ensemble and chorus, Signalsmith Basics chorus/freq-shifter/limiter, Signalsmith `hilbert-iir` 0BSD, DaisySP, Faust `phaflangers.lib` STK-4.3); each global FX costs 100–1,000 instructions per stereo frame.
7. **Latency comes only from a lookahead limiter, FIR oversampling and true through-zero flanging.** Keep zero latency: a no-lookahead limiter (instant-attack follower + ADAA soft clip), IIR oversampling, and a TZ flanger whose dry path is a fixed short delay (an effect property, not reported latency).
8. **Filters:** a TPT Moog ladder with one input nonlinearity costs 54 per sample (measured); ISC, public-domain and STK-4.3 sources exist. The formant filter is parallel TPT band-passes with Plaits' MIT vowel table; Faust's formant data is LGPL.
9. **Modulators are nearly free on the segment grid; their cost is in the contracts.** MSEG breakpoints are variable-length data that the catalogue, patch v1 and the 102-value model layout cannot hold (fix a point count); an envelope follower is the first modulator that reads audio.
10. **Build order:** distortion → FM/PM/RM routing → unison → modulated-delay core (chorus, flanger, phaser) → EQ → comb, formant → delay + tempo → warps, phase distortion → multiband/OTT → ladder → MSEG/sequencer → limiter, frequency shifter, follower, transient shaper (section 9).

## 0. What the engine imposes

| Fact (2026-10-25) | Consequence |
| --- | --- |
| One call per module per 32-frame segment; per-voice state `[T; MAX_VOICES]` (32) (`module.rs`) | SoA already suits SIMD across voices; buffers sized at `prepare` |
| Generators render into one shared `scratch` bus, summed into `buses.voice[v]` in patch order (`engine.rs` step 7) | An FM/AM/RM source needs its own buffer (32 × 32 × 4 B = 4 KB per generator) and an earlier slot |
| The first enabled `osc` writes a per-frame `SyncBus` that later generators read | The precedent for audio-rate routing between generators: a DAG in patch order |
| Per-voice FX transform a **mono** bus; the amp stage pans it into the stereo sum | Stereo unison needs a stereo voice bus (per-voice FX ×2) or a spread applied after the per-voice FX |
| No tempo in `enth-patch`, `enth-engine`, `enth-render` (grep) | Tempo sync is a render-input change (CLAP transport → `Performance`) |
| Budget ≤ 12,000 instructions per stereo frame at 16 voices; G1 6,274; P1-like voice 146 per voice-sample (`docs/benchmarks.md`) | ≈ 5,700 per frame of headroom; a per-voice module at *c* per sample spends 16·*c* per frame |

**Kernel costs (measured; instructions per call or sample, loop and input subtracted):**

| Kernel | Instr. | Kernel | Instr. |
| --- | --- | --- | --- |
| `libm::sinf` / `tanhf` | 39 / 102 | `tanh_approx` (enth's clamped Padé) | 11 |
| `libm::expf` / `exp2f` | 58 / 42 | `osc::sine` polynomial + phase wrap | 37 |
| `libm::logf` / `log` (f64) | 53 / 68 | phase wrap via `floor` / truncation | 27 / 17 |
| ADAA-1 hard clip / cubic (f32) | 25 / 20 | Cytomic TPT SVF step | 20 |
| ADAA-1 `tanh` (f64, `log1p`+`exp`) | 151 | TPT 4-pole ladder + `tanh_approx` | 54 |
| delay write + LFO + linear read | 78 | 6 first-order allpasses | 43 |
| same, 4-point Lagrange read | 120 | Hilbert, 12 complex one-poles | 160 |
| 2-op PM + feedback: poly sine / `sinf` / `u32`+LUT | 111 / 140 / 52 | 2× half-band IIR up+down + 2 shapers | 59 |
| Linkwitz–Riley 4 split (4 SVFs) | 69 | | |

One stream, ±30 %; enth's `osc` saw costs about 42 per voice-sample in context (derived from `docs/benchmarks.md`). Each module still needs a gungraun budget.

## 1. Oscillator interactions

| Module | Algorithm and aliasing | Cost, state | Determinism | Sources (verified) · study-only | Effort |
| --- | --- | --- | --- | --- | --- |
| **PM ("FM") between generators** | DX7-style `y = sin(2π(φ_c + I·m))`; linear through-zero FM is PM by the modulator's integral, and PM does not drift when `m` has DC. Units: ratio (coarse + fine), index *I* (rad). Only on the sine/wavetable cores: the polyBLEP core clamps `dt` to `[1e-7, 0.25]` and assumes a steady slope. Aliasing (Carson bandwidth ≈ 2(*I*+1)·*f_m*): (a) accept it, as Plaits' six-op engine (no oversampling in its source); (b) **index tracking** (scale *I* down near Nyquist; free, deterministic); (c) 2× the pair (Plaits' 2-op engine uses 4× + FIR). Tables: mip from `f_c + I·f_m` | +10–40 per voice-sample over a plain carrier; 52–111 for a 2-op pair. State: 2 phases, 2 feedback samples. **Self-feedback:** `pm = (y[n−1] + y[n−2])·fb` (the two-sample average stops "hunting"; Plaits `operator.h`, MSFA), +5 | `u32` phase + LUT exact; polynomial sine `+ − ×` | **Plaits `dsp/fm/*`, `engine2/six_op_engine.*` (MIT, © 2021 E. Gillet, per-file)**: operator, 32 algorithms, DX7 patches. **Dexed `source/msfa/*` (Apache-2.0 per file; `tuning.*` no header)**: int32 fixed-point. Chowning, JAES 1973 · Dexed plugin (GPL-3), Vital, Surge | M routing + S per mode |
| **AM / ring mod** | AM `c·(1 − d + d·m)`, RM `c·m`. A product's bandwidth is the sum of the inputs': little aliasing below fs/4. Warps' "analog" RM `Diode(m+c) + Diode(m−c)` adds grit. Warps runs its cross-mods at 6× with a 48-coefficient polyphase FIR (documented), which is overkill here | +2–5; +15 diode | Exact | **Warps `dsp/modulator.cc` (MIT, © 2014 E. Gillet)**: ring, diode, XOR, comparator, fold | S after PM routing |
| **Phase distortion (CZ)** | Remap the phase before a cosine: a piecewise-linear knee at `d` (CZ saw), or a window × synced cosine (CZ resonant waves). The remap's slope (up to `1/(2d)`) sets the bandwidth: clamp `d` by pitch or render 2×. Plaits renders 2× and decimates naively | ~35–45 | `+ − × ÷` + polynomial sine | **Plaits `engine2/phase_distortion_engine.cc` (MIT)**; Kleimola et al., *Phaseshaping oscillator algorithms*, SMC 2010; *Vector phaseshaping*, DAFx-11 | S |
| **Wavetable morph + warps** | Morph = today's frame crossfade. Warps remap the phase before the read (as Serum/Vital are described; anecdotal): **bend** `p/(p + k(1−p))` (rational, no `pow`); **asym** (bend per half); **mirror** `1 − |2p−1|`; **squeeze/PWM** `min(p/w, 1)`; **sync** `frac(r·p)`; **quantise** `⌊N·p⌋/N`; **remap** (a fixed number of curve points); **FM-from** (PM above). Aliasing: **slope-aware mip selection** (level for `dt·max|dp'/dp|`; conservative) plus the phased core's BLEP at hard edges | +10–30 over the table read (≈ 76 per voice-sample today, derived) | Exact if `k` comes from `libm` once per segment | No permissive Serum-style warp code found; write from first principles · Vital (GPL-3) | M |
| **Unison** | 8–16 copies: offsets `s_k·detune`, `s_k` spread over [−1, 1] with an optional curve (Szabo fits a non-linear JP-8000 curve; anecdotal); centre/side blend. **Phase:** random start per copy from `derive(voice_seed, k)` (keyed by note index and render seed, never parameters; design.md §5) or reset. **Spread:** alternating pans `±spread·|s_k|` | N × base (saw ≈ 42, table ≈ 76 per voice-sample); 7 × saw × 16 ≈ 4,700 per frame. State: N phases per voice | Fixed summation order over copies | Kleimola et al. 2010 (supersaw by phaseshaping); A. Szabo, *How to emulate the super saw* (2010) · Vital, Surge unison (GPL) | M mono, L stereo |
| **Sub oscillator** | A half- or quarter-rate phase toggled at each main wrap; square with BLEP, or sine | +10–15 | Exact | Plaits `super_square_oscillator.h` (MIT, BLEP'd); or a second `osc` at −12 st (possible today) | S |

**Routing.** Keep generator-to-generator modulation a DAG in patch order: each modulator renders its whole segment before the carrier reads it, which matches one call per module per segment. Two-way cross-modulation in the same sample would need per-sample interleaving; self-feedback stays inside one module. A generator with mix level 0 is a pure operator, as on a DX7.

## 2. Distortion family

| Module | Algorithm and aliasing | Cost | Determinism | Sources | Effort |
| --- | --- | --- | --- | --- | --- |
| **Soft clip** | Cubic `x − x³/3` or `tanh_approx`; ADAA-1 with a polynomial antiderivative (cubic: `x²/2 − x⁴/12`); midpoint fallback when `|x − x₋₁| < 1e-4` | 20 | The fallback is a plain compare; `f32` is fine for polynomial antiderivatives | **Faust `aanl.lib` (MIT per function: clippers, folds, `tanh1`, `asinh1`; ADAA-1 and -2)**; Parker, Zavalishin, Le Bivic, DAFx-16; Bilbao, Esqueda, Parker, Välimäki, IEEE SPL 24(7), 2017 | S |
| **Hard clip** | ADAA-1 (`x²/2` inside, `|x| − ½` outside), or polyBLAMP at the corners (Esqueda, Välimäki, Bilbao, DAFx-16) | 25 | As above | `aanl.lib hardclip` (MIT) | S |
| **tanh / tube** | ADAA-1 `tanh` needs `log cosh` = `|x| + log1p(e^{−2|x|}) − ln 2` in `f64`; ADAA-2 needs a dilogarithm (`aanl.lib` notes). Asymmetry: a bias before, a DC blocker after; or Signalsmith's unequal positive/negative "fuzz" gains | 151 (f64 ADAA-1); ~40–70 with 2× IIR + `tanh_approx` | `libm` only; `f64` avoids the cancellation in `(F(x) − F(x₋₁))/(x − x₋₁)` | Signalsmith Basics `crunch.h` (MIT; ADAA-1 of a `1/√(1+x²)` gain shaper inside 2× FIR) · chowdsp `ADAATanhClipper` (**GPL-3**) | S |
| **Foldback / wavefolder** | Triangle fold (piecewise linear, so ADAA-1 is cheap); sine fold (antiderivative `−cos`); Lockhart/Serge need Lambert-W (use a fixed iteration count) | ~30 (triangle, est.); ~90 (sine) | Fixed iteration counts | `aanl.lib triangleFold1/sineFold1` (MIT); Esqueda, Pöntynen, Parker, Bilbao, *Appl. Sci.* 7(12):1328, 2017; DaisySP `wavefolder.cpp` (MIT) | S |
| **Bitcrusher** | `round(x·q)/q`, `q = 2^(bits−1)`, fractional bits allowed; the aliasing is the point | ~8 | `round` is exact | DaisySP `decimator.cpp` (MIT) | S |
| **Sample-rate reducer** | Hold at a variable rate; the "clean" mode adds a BLEP at each new held value | 15–25 | Exact | **Plaits `fx/sample_rate_reducer.h` (MIT)**; DaisySP `sampleratereducer.cpp` (MIT); Plaits `fx/overdrive.h` (MIT) for a loudness-compensated drive | S |

**ADAA or oversampling?**

| | Per-voice insert (×16) | Global insert (×2 channels) |
| --- | --- | --- |
| Use | **ADAA-1 on polynomial-antiderivative shapers in `f32`**, with an optional "HQ" 2× IIR half-band (+~40) | **2× or 4× polyphase-allpass half-band IIR** around any shaper, optional ADAA-1 inside |
| Cost | 320–480 per frame; ~1,100 with 2× | 120–500 per frame |
| Latency | ADAA-1: half a sample of group delay | IIR: a few samples of frequency-dependent group delay, no lookahead, nothing to report. FIR: true latency |
| Limits | Memoryless shapers only. In the linear region ADAA-1 is a two-tap average, `|H| = cos(ω/2)`: −0.5 dB at 5 kHz, −2.0 dB at 10 kHz, −3 dB at 12 kHz, zero at Nyquist (48 kHz; computed). Inside a feedback loop (filter drive) it needs Holters' stateful extension (DAFx-19; *Appl. Sci.* 2020) | Cost scales with the factor |

## 3. Modulation effects

| Module | Algorithm | Interpolation | Cost per stereo frame, state | Determinism / RT | Sources | Effort |
| --- | --- | --- | --- | --- | --- | --- |
| **Chorus** | 2–6 modulated taps (5–25 ms), sine LFOs at fixed phase offsets | Cubic/4-point Lagrange; Signalsmith uses 8-point Lagrange × 6 taps | 300–450; 25 ms rings ≈ 5 KB per channel at 48 kHz | Replace Signalsmith's per-sample `std::polar` with a rotating phasor renormalised per segment, or the polynomial sine | **Signalsmith Basics `chorus.h` + DSP `delay.h` (MIT; interpolators linear, cubic, Lagrange-N, Kaiser-sinc)** | S |
| **Ensemble** | 3 taps per channel, LFOs at 0.75 Hz and 6.57 Hz, phases 0/120/240° | Linear | 150–250 | Constants are samples at 48 kHz: rescale | **Plaits `fx/ensemble.h`, Rings `fx/ensemble.h`, `fx/chorus.h` (MIT)** | S |
| **Flanger** | 0.1–10 ms modulated delay, ± feedback | Cubic | 100–200 | Flush the feedback write | DaisySP `flanger.cpp` (MIT); Faust `phaflangers.lib` (STK-4.3, JOS); JOS, *PASP* | S |
| **Through-zero flanger** | The dry path is a fixed delay `D` and the wet tap sweeps across it, so the output is `D` (2–5 ms) late while TZF is on. Document it as an effect property, like pre-delay; it is not reported latency | Cubic | 150–250 | As flanger | JOS *PASP* | S |
| **Phaser** | 4–12 first-order TPT allpasses (Zavalishin; stable under fast sweeps), LFO sweep, feedback; `tan` from `libm` once per segment, ramped | — | 100–150 (6 stages ≈ 43 per channel, measured) | Flush allpass states | DaisySP `phaser.cpp` (MIT); Faust `phaser2_stereo` (STK-4.3) | S |

**Fractional delays.** All are `+ − ×`, so bit-exact with a fixed operation order. **Linear** for ensembles; **cubic Hermite/4-point Lagrange** for chorus and flanger (+42 per tap, measured); **first-order allpass (Thiran)** only for slow, pitch-tuned delays (comb, Karplus–Strong): flat magnitude, but recursive, so fast modulation makes transients and the state needs flushing (JOS *PASP*; Laakso et al. 1996). One **modulated-delay core** (ring sized at `prepare`, interpolator as a type parameter) serves chorus, ensemble, flanger, delay and comb.

## 4. Delay, EQ, multiband, OTT, limiter, transient shaper, trance gate

| Module | Algorithm | Cost per stereo frame, state | Determinism / architecture | Sources | Effort |
| --- | --- | --- | --- | --- | --- |
| **Delay** (stereo, ping-pong, filtered feedback) | Two rings sized at `prepare` (2 s at 48 kHz = 384 KB per channel; 1.5 MB at 192 kHz). Ping-pong by cross-feed; LP/HP and optional `tanh_approx` in the loop; time changes by a two-head crossfade or slewed time | 80–200 | **Tempo sync needs a tempo input** in `render()`. Flag it `time_based` (FX bypass) | Signalsmith `delay.h` (MIT); DaisySP `delayline.h` (MIT) | S, + M for tempo |
| **EQ** (3–5 bands) | Cytomic trapezoidal SVF with mixed outputs: bell, shelves, LP/HP and notch from one structure. Safe under modulation, unlike a DF1 biquad. `libm::tanf` once per segment | ~25 per band per channel; 4 bands ≈ 200. 2 floats per band per channel | Exact; reuse `svf.rs` | **Cytomic, *SvfLinearTrapOptimised2.pdf* ("placed in the public domain")**; enth `biquad.rs` | S |
| **Multiband split** | Linkwitz–Riley 4 = two Butterworth TPT SVFs per side. 3 bands: split at f1, split the high side at f2, and an allpass at f2 on the low band for a flat sum | 69 per split per channel (measured); 3-band stereo ≈ 350–420 | Exact | Faust `crossover2/3LR4`, `lowpassLR4`, `highpassLR4` (STK-4.3 per function); Linkwitz, JAES 1976 | S |
| **OTT-style compression** | Per band: downward above threshold, upward below it (gain cap, e.g. ≤ +36 dB, and a floor so silence is not lifted); depth (dry/wet); band gains. Reuses enth's compressor detector | split ≈ 400 + 3 × ~100 (compressor ≈ 94, derived) ≈ 700–1,000 | Upward gain lifts noise and tiny values: gate below about −90 dB and flush | enth `compressor.rs`; Giannoulis, Massberg, Reiss, JAES 2012 · Xfer OTT (closed), Vital (GPL) | M |
| **Limiter** | **Zero latency:** an instant-attack peak follower with release, plus an ADAA-1 soft clipper for overs. **Lookahead** (Signalsmith `limiter.h`, latency = attack time) needs an ADR: CLAP latency reporting (changes restart the plugin), compensated CLI renders, delayed audio in scorers | 50–100 | Zero latency keeps ADR-0006; clippers are a bass-music mastering staple (anecdotal) | **Signalsmith Basics `limiter.h` (MIT)**, blog *Designing a straightforward limiter* (2022); DaisySP `limiter.cpp` (MIT) | S |
| **Transient shaper** | Fast and slow envelope followers; gain from their ratio, in ratio space (no transcendentals) or in dB (`libm`) | 150–300 | Exact | Textbook; Rings `follower.h` (MIT) for followers | S |
| **Trance gate** | 8–32-step amplitude pattern with attack/release ramps; per voice (retriggered) or global | ~10 | Wants tempo (a rate in Hz until then) | Plaits `engine2/arpeggiator.h` (MIT) for step logic | S |

**Sidechain.** An external sidechain (a kick ducking enth) needs a CLAP audio input and an input stream in `render()`: scores would depend on audio enth does not own. The deterministic alternative is a tempo-synced "pump" (MSEG or gate on amplitude), how most producers get the effect (anecdotal). Internal followers fit the segment model, with a one-segment lag when they read a later stage.

## 5. Filters beyond the SVF

| Module | Algorithm and aliasing | Cost per voice-sample, state | Determinism | Sources | Effort |
| --- | --- | --- | --- | --- | --- |
| **Moog ladder** | Zavalishin TPT/ZDF: four TPT one-poles, linear feedback solved exactly, one `tanh_approx` at the input. "Full" model: a nonlinearity per stage (Huovilainen 2004; D'Angelo & Välimäki 2013) at 2× | 54 (measured); ~150–250 full at 2× (est.). 4 floats | No iteration, or a fixed count; flush | **MoogLadders `ImprovedModel.h` (ISC, © 2012 S. D'Angelo) and `HyperionModel.h` (TPT, public domain, 2025–26)**; its `HuovilainenModel.h` is **LGPL**. **Faust `moogLadder` (STK-4.3), `lowpassLadder4` (MIT)**; DaisySP `ladder.cpp` (MIT, Huovilainen, 4× OS) · sst-filters, chowdsp_filters (GPL-3) | M |
| **Diode ladder** | TPT coupled stages, cubic nonlinearity | ~70–90 (est.) | As above | **Faust `diodeLadder` (E. Tarr, STK-4.3)**; Zavalishin | M |
| **Comb (FF/FB), tuned to pitch** | Delay `fs/f` from the voice's pitch; damping one-pole in the feedback; Thiran allpass or 4-point Lagrange for the fraction. Rings sized for the lowest pitch: 20 Hz → 2,400 samples × 32 voices × 4 B = 300 KB | 40–70 | Flush the feedback write | Faust `ff_comb`, `fb_comb`, `fb_fcomb` (STK-4.3); JOS *PASP* | S |
| **Formant / vowel** | 3–5 parallel TPT band-passes (enth's SVF core), with frequency, bandwidth and gain per vowel; morph by interpolating log-frequency; coefficients once per segment | ≈ 105 (≈ 1,700 per frame at 16 voices). 10 floats | Exact | **Plaits `speech/naive_speech_synth.cc` (MIT): 5 vowels × 5 registers × 5 formants**; primary data: Peterson & Barney, JASA 1952 · Faust `formantFilterbank` and its Csound table (file-level **LGPL**) | S |
| **Frequency shifter** | Hilbert transform to an analytic signal, times a complex oscillator, real part. Signalsmith: 12 complex one-poles, 20 kHz band edge | ~180 per channel: global 360–400 per frame; per voice ≈ 2,900 per frame (too heavy) | Replace `std::polar` with a rotating phasor; discretise poles at `prepare` with `libm` | **Signalsmith `hilbert-iir` (0BSD) + `freq-shifter.h` (MIT)**; **Warps `quadrature_transform.h` (MIT)**: real allpass pairs, cheaper | S |
| **Resonator bank** | Parallel high-Q band-passes or modal two-poles on the partials of the pitch | ~20 per mode | Exact | Rings `resonator.cc` (MIT); DaisySP `resonator.cpp`, `modalvoice.cpp` (MIT) | M |

## 6. Modulators

| Module | Algorithm | Determinism / contracts | Sources | Effort |
| --- | --- | --- | --- | --- |
| **MSEG / drawable LFO** | `(t, v, curve)` breakpoints; segment curve `x/(x + k(1−x))` (no per-sample `pow`; `k` from `libm` once); loop points; one-shot or looping; per voice or global. Evaluated once per segment: negligible cost | Exact. **Contracts:** breakpoint lists are variable-length data that the catalogue, patch v1 and `propose-layout/1` (102 fixed values) cannot hold. Fix a maximum (e.g. 8 points as scalar params) or offer a parametric shape set | Signalsmith `curves.h` (MIT: `CubicSegmentCurve`, `Reciprocal`) · Vital, Serum (ideas) | M |
| **S&H, smooth random** | Already in `lfo.rs` (seeded S&H, smoothstep random). Add a **per-note random** (one draw at note-on from the voice seed) and a global random from the global seed | Keyed seeds (design.md §5) | own | S |
| **Envelope follower** | Rectify, then an attack/release one-pole (as in `compressor.rs`): 5–10 per sample per source. The first modulator that reads audio: `VoiceModCx`/`GlobalModCx` gain an audio input | One-segment lag when it reads a later stage | own; Rings `follower.h` (MIT, 3-band) | S, + M for the role |
| **Slew / lag** | Linear-rate or one-pole limiter, per route or as a module | Exact | own | S |
| **Step sequencer** | 8–32 values, glide per step, rate in Hz or synced | A fixed step count fits the catalogue; sync needs tempo | Plaits `arpeggiator.h` (MIT) | S |
| **Arpeggiator** | Generates notes before the voice manager, quantised to the 32-frame grid | Changes the `Performance` contract (CLI = plugin) and what the model must explain; defer (DAW MIDI tools exist) | Plaits `engine2/arpeggiator.h` (MIT) | M–L |
| **MPE** | Per-note bend, pressure, timbre (CC74) as per-voice sources; CLAP note expressions map directly | `Performance` must carry per-note expression curves; poly-mod offsets already exist | — | M |

## 7. Oversampling and SIMD

**Oversampling belongs inside the module that aliases, never on the whole voice.** The oscillators are already band-limited (polyBLEP; tables below −85 dB, `docs/dsp/osc.md`), so a 2–4× voice buys nothing. Candidates: the distortion insert (2×, 4× global), a high-index FM pair, the full ladder, phase distortion (2× each). One shared `enth-dsp::resample` with a **two-path polyphase allpass half-band** (Valenzuela & Constantinides 1983; HIIR's structure) serves all: 59 instructions per base-rate sample for 2× up and down with two second-order sections per path and two shapers (measured). Coefficients are computed offline and committed with a test. Not linear-phase, so nothing to report; a linear-phase FIR (Signalsmith `rates.h`, MIT) adds latency and is excluded by ADR-0006.

- `halfband` 0.3.0 (MIT, 2026-09-12, one maintainer, 3.5 k downloads) ports HIIR's structure but designs FIR taps with `std` `sin`/`powf` and pulls three crates: **port its ~100-line IIR, don't depend on it.** HIIR (v1.50) says "CC0 / WTFPL unless explicitly specified" on its site (archive not read; WTFPL is not on ADR-0004's list): design coefficients from the paper. `pulp` is excluded below.

**SIMD and bit-exactness:**

| Option | Status (2026-10-25) | Bit-exact on x86-64, aarch64, wasm32? |
| --- | --- | --- |
| `std::simd` | Nightly-only | n/a on stable 1.99.1 |
| **`wide` 1.7.1** (2026-09-14; Zlib/Apache/MIT; 86 M downloads) | Compile-time width, with a scalar-array fallback | **Yes for `+ − × ÷ sqrt min max`, compares and blends.** The Phase 0 spike measured scalar = f32x4 = f32x8 hashes across x86-64, Windows, wasm32 ± simd128 and `+fma,+avx2`. **No for:** the `mul_add` family (banned); **`recip`/`recip_sqrt`** (SSE `_mm_rcp_ps`/`_mm_rsqrt_ps` approximations vs exact division on wasm/NEON: `wide`/`safe_arch` source); **`exp`, `exp2`, `ln`, `sin_cos`, `pow`, `exp_m1`, `ln_1p`** (call `mul_add` internally). **Add these to `clippy.toml`** |
| `pulp` 0.22.3 (2026-06-20), `fearless_simd` | Runtime ISA dispatch | Only if every per-ISA kernel is numerically identical; AGENTS.md forbids dispatch between numerically different kernels. Avoid in L0–L3 |
| Autovectorisation | Unreliable (2026 survey, anecdotal) | Exact when it happens (Rust never contracts FMA), but not a plan |

**Where SIMD pays:** unison copies (identical code, different phases) first, then voices across the SoA arrays. The spike ran 16 voices in 122.4 M instructions scalar against 62.9 M with f32x8 per 2 s (1.95×). Branchy code (BLEP insertion, ADAA fallbacks) becomes blend masks, which stay exact. Effort L; best done after unison exists.

## 8. Library scan (verified 2026-10-25)

| Name | Licence, verified how | Last activity | Fit |
| --- | --- | --- | --- |
| Signalsmith DSP | MIT (`LICENSE.txt`) | `ca390d2` 2026-10-13; tag v1.7.3 | **Port**: delay interpolators, `curves.h`, `envelopes.h` (already used for the reverb) |
| Signalsmith Basics | MIT (`LICENSE.txt`, © 2022–2025) | `369e906` 2026-09-30; tag 1.2.1 | **Port** `chorus.h`, `freq-shifter.h`, `limiter.h` (lookahead needs an ADR); `crunch.h` for ideas |
| Signalsmith `hilbert-iir` | **0BSD** (`LICENSE.txt` + header; allowed for ported source by ADR-0004) | 2026-01-20 | **Port** (frequency shifter) |
| MI eurorack | README: STM32 code MIT, AVR code GPL-3; **per-file MIT headers checked** in Plaits `fm/`, `engine2/`, `fx/`, `speech/naive*`, `oscillator/`, Warps, Rings | `f4cfa76` 2026-10-12 | **Port**: six-op FM, PD, SRR, ensemble, overdrive, vowel table, ring mods, Hilbert pair, follower; check each file |
| DaisySP | MIT (`LICENSE`; most files have no header); **LGPL submodule `DaisySP-LGPL`** excluded | `2c72eaf` 2026-09-28; release V1.0.0 2024-01-08 | **Port with care**: chorus, flanger, phaser, wavefolder, decimator, ladder (Teensy MIT header). Global `rand()` in `maytrig.h`, `dust.h`, `analogsnaredrum.cpp` |
| Faust libraries | **Per function.** `aanl.lib`: MIT. `vaeffects.lib` ladders: STK-4.3 or MIT. `phaflangers.lib`: STK-4.3 (file). `misceffects.lib`: LGPL-2.1 + exception (file) except functions declared STK-4.3 or MIT. `physmodels.lib` formants: LGPL (file). `fexp`/`fsin`/…: LGPL. `processArray`: AGPL-3 | `42fab48` 2026-10-24 | **Hand-port declared MIT/STK-4.3 functions only**; never compiler output |
| Airwindows | MIT (`LICENSE`, © 2018 Chris Johnson) | `dfa5401` 2026-10-24 | Ideas and simple saturators; replace the global-`rand()` dither seeding (ADR-0006) |
| chowdsp_utils | **Per module** (`README.md`, `LICENSE.md`): core, math, simd BSD; `waveshapers`, `filters`, `eq`, `dsp_utils`, `compressor`, `reverb`, `sources` **GPL-3** | v2.4.0 2025-11-29; pushed 2026-10-22 | Study only for DSP; `chowdsp_math` (BSD) portable |
| jatinchowdhury18/ADAA | BSD-3 (`LICENSE`) | 2021-11-29 (stale) | Reference |
| Dexed MSFA | **Apache-2.0 per file** (`source/msfa/*`); repo GPL-3; `tuning.*` no header. The original, google/music-synthesizer-for-android (Apache-2.0), is archived (2021-11-23) | `fea2f0f` 2026-10-14 | **Port** if Plaits' FM is not enough (Apache notice duties) |
| ddiakopoulos/MoogLadders | **Per file**: ISC (Improved); public domain (Hyperion, Krajeski, Stilson, Microtracker); **LGPL** (Huovilainen); unclear (MusicDSP) | `a968ec6` 2026-06-13 | **Port** Improved or Hyperion |
| fundsp | MIT OR Apache-2.0 | 0.23.0 2026-01-07; no commit since 2026-03-03 | A/B reference only (ADR-0006) |
| `wide` | Zlib OR Apache-2.0 OR MIT | 1.7.1 2026-09-14 | **Dependency** for SIMD, with the extra bans |
| `biquad`, `dasp`, `rubato`, `idsp` | MIT OR Apache-2.0 | 0.6.0 2026-03-22; 0.11.0 2020-05-29; 5.0.1 2026-10-01; 0.23.1 2026-10-17 | No for the audio path (`rubato` for offline import resampling) |
| `synfx-dsp`, sst-basic-blocks/filters/waveshapers, Vital; `valib`'s repo | **GPL-3** (crates.io, GitHub API; `valib` 0.2.6 is MIT on crates.io but its repo mixes GPL plugins) | 2024-01-04; 2026-10-23; 2026-10-13; 2026-02-25; 2023-05-25 | Study only |

**Depend or port?** Only `wide` passes the gates as an audio-path dependency. Everything else is ported into `enth-dsp` per ADR-0006, with provenance in `THIRD_PARTY_NOTICES.md` and `docs/dsp/<module>.md`.

## 9. Ranked build list

Ordered by sound-design breadth per engineering hour, weighted toward dubstep per the brief. Effort: S ≤ 3 days, M 1–2 weeks, L > 2 weeks; each includes ParamSpecs, goldens, docs and a lesson. CPU is at 16 voices, or per stereo frame for global FX.

| # | Module | Effort | CPU | Architecture change |
| --- | --- | --- | --- | --- |
| 1 | **Distortion** (soft, hard, fold, asym, bitcrush, SRR; ADAA-1; optional 2× IIR), as voice FX and global FX: growls and screeches are distorted after the filter | S–M | 320–480 per voice insert; 120–500 global | `enth-dsp::resample` only |
| 2 | **PM/FM, AM, RM between generators** + self-feedback: FM growls, metallic and riddim tones | M | +10–40 per voice-sample per carrier | **Per-voice audio-rate routing** (per-generator buffers, DAG order) |
| 3 | **Unison**, mono first, then spread: reese, supersaw | M / L | 7 × saw ≈ 4,700 | **Stereo voice bus**; motivates SIMD |
| 4 | **Modulated-delay core + chorus/ensemble, flanger, phaser** | S core + S each | 100–450 each | None (TZF: fixed dry delay) |
| 5 | **EQ** (SVF bells/shelves) | S | ~200 | None |
| 6 | **Comb + formant filters** (keytracked comb; vowel morph): "yoi" and talking basses | S each | 40–70; ~105 per voice-sample | None |
| 7 | **Delay** (stereo, ping-pong, filtered feedback) | S + M | 80–200 | **Tempo as a render input** |
| 8 | **Wavetable warps + phase distortion** | M | +10–45 per voice-sample | Slope-aware mip selection |
| 9 | **Multiband split + OTT** | M | 700–1,000 | None (reuses the compressor) |
| 10 | **Ladder (Moog TPT) + diode ladder** | M | 54–250 per voice-sample | Optional internal 2× |
| 11 | **MSEG + step sequencer + slew + per-note random**: exact wobble shapes | M | negligible | **Catalogue/patch/model layout** (fixed point count); tempo |
| 12 | **Zero-latency limiter / clipper** | S | 50–100 | None (lookahead would need an ADR) |
| 13 | **Frequency shifter** (global) | S | 360–400 | None |
| 14 | **Envelope follower** | S + M | 5–10 per sample | **Modulators that read audio** |
| 15 | **Transient shaper, trance gate** | S each | 150–300; ~10 | The gate wants tempo |
| 16 | **SIMD voices / unison lanes** (`wide`) | L | −40–50 % on vectorised modules | Kernel structure; new clippy bans |
| 17 | **MPE** | M | negligible | **Per-note expressions in `Performance`** |
| 18 | **Resonator bank; arpeggiator** | M; M–L | ~20 per mode | Arp: event generation before the voices |

## Gaps

- HIIR's licence file was not read (downloading its archive needs the owner's OK); design the half-band coefficients from Valenzuela–Constantinides instead.
- No aarch64 run of the micro-benchmark or of `wide`'s exact subset; the Phase 0 spike covered x86-64, Windows and wasm32 (design.md §5). Run the spike's hash test on an arm runner before SIMD lands.
- Kernel costs come from a scalar micro-benchmark (±30 %), not gungraun budgets in the engine. No aliasing was measured for the ADAA, FM or warp options; the next step is `enth describe` sweeps, as `docs/dsp/osc.md` did for the oscillators.
- Serum's and Vital's warp definitions come from public descriptions only (Serum manual not read; Vital is GPL); the formulas are first-principles proposals.
- Szabo's thesis was read only through secondary summaries; the IEEE SPL 2017 DOI and pages are unconfirmed (cited via a thesis listing); Zavalishin rev. 2.1.2 was not found on NI's site (2.1.0 was).
- No listening tests. The claims about clippers and pumping are anecdotal and belong to report 04.

## Sources

**Repos** (licences read from files in `~/code/enth-scratch/2026-10-25-s2ft/05/src/`): [Signalsmith DSP](https://github.com/Signalsmith-Audio/dsp) · [Signalsmith Basics](https://github.com/Signalsmith-Audio/basics) · [hilbert-iir](https://github.com/Signalsmith-Audio/hilbert-iir) · [MI eurorack](https://github.com/pichenettes/eurorack) · [DaisySP](https://github.com/daisyaudio/DaisySP) · [Faust libraries](https://github.com/grame-cncm/faustlibraries) · [Airwindows](https://github.com/airwindows/airwindows) · [chowdsp_utils](https://github.com/Chowdhury-DSP/chowdsp_utils) · [ADAA](https://github.com/jatinchowdhury18/ADAA) · [Dexed](https://github.com/asb2m10/dexed) · [music-synthesizer-for-android](https://github.com/google/music-synthesizer-for-android) · [MoogLadders](https://github.com/ddiakopoulos/MoogLadders) · [halfband](https://github.com/Sin-tel/halfband) · [Vital](https://github.com/mtytel/vital) (GPL; API only)

**Crates:** crates.io API pages for each crate in section 8 (2026-10-25).

**Papers, books, pages:**
- Parker, Zavalishin, Le Bivic, DAFx-16 ([archive](https://dafx.de/paper-archive/details/vem_XXF5qBbfiWOH2RVVAA)); Bilbao, Esqueda, Parker, Välimäki, IEEE SPL 24(7), 2017 (via [Esqueda's thesis](https://aaltodoc.aalto.fi/items/a46ae786-3014-4c49-a409-957b4c330bcd)); Holters, DAFx-19 ([PDF](https://www.hsu-hh.de/ant/wp-content/uploads/sites/699/2020/10/DAFx2019_paper_4.pdf)) and *Appl. Sci.* 10(1):20 ([MDPI](https://www.mdpi.com/2076-3417/10/1/20)); Esqueda, Välimäki, Bilbao, *Rounding corners with BLAMP*, DAFx-16
- Esqueda, Pöntynen, Parker, Bilbao, wavefolders, *Appl. Sci.* 7(12):1328, 2017 ([MDPI](https://www.mdpi.com/2076-3417/7/12/1328))
- Kleimola, Lazzarini, Timoney, Välimäki, phaseshaping, SMC 2010 ([PDF](https://mural.maynoothuniversity.ie/4100/1/SMC2010.pdf)); vector phaseshaping, DAFx-11 ([PDF](https://mural.maynoothuniversity.ie/4096/1/vps_dafx11.pdf))
- Zavalishin, *The Art of VA Filter Design* rev. 2.1.0 ([NI PDF](https://www.native-instruments.com/fileadmin/ni_media/downloads/pdf/VAFilterDesign_2.1.0.pdf)); [Cytomic technical papers](https://cytomic.com/technical-papers/) (public domain); J. O. Smith III, *Physical Audio Signal Processing* (CCRMA online)
- Laakso, Välimäki, Karjalainen, Laine, *Splitting the unit delay*, IEEE SPM 1996; Huovilainen, DAFx-04; D'Angelo & Välimäki, ICASSP 2013; Välimäki & Huovilainen, CMJ 2006; Välimäki, Pekonen, Nam, JASA 2012
- Valenzuela & Constantinides, IEE Proc. G 1983 (polyphase allpass half-bands); Chowning, JAES 1973; Linkwitz, JAES 1976; Giannoulis, Massberg, Reiss, JAES 2012; Peterson & Barney, JASA 1952
- Szabo, *How to emulate the super saw* (2010), via [Synthtopia](https://www.synthtopia.com/content/2011/03/28/super-saw-synth-for-windows/) and [Scholl](https://schollz.com/til/221103) (anecdotal); HIIR v1.50 ([ldesoras.fr](https://ldesoras.fr/prod.html))
- Rust SIMD: 2026 survey ([summary](https://zeli.app/story/49844629), secondary); [Linebender](https://linebender.org/blog/towards-fearless-simd/); [`portable_simd` tracking issue](https://github.com/rust-lang/rust/issues/86656)
- Serum warps (anecdotal): [Xfer](https://www.xferrecords.com/serum); [Vital forum](https://forum.vital.audio/t/achieving-serums-remap-warp-mode/12239)
