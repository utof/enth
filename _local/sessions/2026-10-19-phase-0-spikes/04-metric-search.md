# 04 — Metric and search

- Date: 2026-10-19
- Sub-researcher: 04 (metric and search)
- Questions answered: 1 (analysis crates), 2 (CMA-ES in Rust), 3 (audio distances), 4 (listening test for 30 pairs), 5 (parameter-space distance), 6 (Phase 0 spike and gate realism). Wrapped up early because the machine is about to reboot; what is unfinished is listed under Gaps.
- Labels: *measured* = I ran it on the owner's laptop today; *documented* = read in source code, docs, a paper or a registry API today; *anecdotal* = a blog post or a single unreplicated report.

## Answer in brief

- **Analysis: build on `rustfft` + `realfft` and write our own features. Use `ebur128` for loudness.** Mel, MFCC, centroid, the envelope and YIN come to a few hundred lines of code we own, which stays deterministic and wasm-friendly. The crates that do this for you are a poor fit. `mel_spec` pulls in `image` and is built for Whisper. `pyin` 1.2.0 has an unconditional `libc::malloc` C API and pulls in `ndarray-npy`/`zip`. The 2026 all-in-one crates have one author each and little use. *(documented)*
- **CMA-ES: write our own small ask–tell implementation. No crate does everything the design needs.** We need ask–tell, a caller-supplied batch evaluator, seeded determinism and wasm32. `cmaes` 0.2.2 has no ask/tell, depends on `rayon`, calls `Instant::now()` in its constructor (that panics on wasm32-unknown-unknown) and last released in 2024-12. `basin` 1.15.1 is good: seeded, wasm-friendly, with a bounded CMA-ES. But it has no ask/tell and no IPOP/BIPOP yet. My probe's CMA-ES with IPOP took about 300 lines. *(documented; measured for the line count)*
- **No single audio distance is best.** That holds in the 2025–2026 papers (Salimi et al. 2025/2026, Tian et al. ISMIR 2025). On human timbre ratings, MFCC distance is competitive, multi-scale spectrogram distance underperforms, and CLAP "style" embeddings come out best. *(documented)*
- **As a search objective in my toy probe, multi-resolution log-mel L1 (distance A) worked best. MFCC (C) was clearly worst. The composite (B) was in between.** MFCC reached the strict threshold on 0/12 targets, against 5/12 for A. *(measured, toy synth, not enth)*
- **Embedding distances (CLAP, EnCodec, DAC) cost about 5–10× a render per evaluation.** CLAP is about 55 ms per clip on an M4 Max CPU *(anecdotal)*. Two licence problems: MERT and Music2Latent weights are CC BY-NC 4.0, and the EnCodec weights licence is unclear. Keep embeddings out of Phase 0's inner loop and use them later as a final re-ranker. *(documented)*
- **Listening test: rate the 30 pairs 1–5 for similarity, but add 5 repeated pairs (test–retest) and 2+2 anchors.** Loudness-match every clip and randomise order and A/B side. Report Kendall τ-b and Spearman ρ with bootstrap CIs, plus triplet agreement. With n = 30, ρ = 0.6 has a 95 % CI of about [0.28, 0.80], so set the gate as a number. *(documented, my arithmetic)*
- **Measured cost per evaluation: render 2.3–5.6 ms plus features 5.5–10 ms** (toy, 1 voice, 2 s at 48 kHz, the box under load 6–26). Analysis costs as much as rendering or more. Design §7's "~20 ms per render" budget should be stated per evaluation, render plus analysis. *(measured)*
- **Evaluations to solve: about 400–3,700 per success, and 8,000 evaluations still left 2–6 of 12 targets unsolved with random starts** (10 parameters). One categorical parameter (the waveform) breaks plain CMA-ES: 3/12 runs ended on the wrong waveform. Cycling the waveform per restart fixed that (11–12/12). *(measured)*
- **Gate realism: "under a minute" is realistic for 10 parameters with an 8-worker evaluator** (8,000 evaluations × ~12 ms / 8 ≈ 12 s). **"Most targets sound the same" is borderline with random starts.** It needs the heuristic start, restarts that handle categorical parameters, and a 5–10 k evaluation budget, and "sounds the same" must be judged by an operational listening check. *(measured + inference)*

## Findings

### Q1 — Rust crates for STFT, mel, MFCC, chroma, onset/envelope, loudness and pitch

Registry data is from the crates.io API and repo data from `gh api`, both on 2026-10-19. Licences were checked against the repo's LICENSE files unless noted.

| Crate | Version (date) | Licence (verified) | Health (2026-10-19) | Dependencies (`cargo tree -e normal`, measured) | Fit |
| --- | --- | --- | --- | --- | --- |
| [rustfft](https://github.com/ejmahler/RustFFT) | 6.4.1 (crate updated 2025-09-18) | MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE) | pushed 2026-10-17, 949★, 32.6 M downloads | small (14 for `pitch-detection`, which only adds rustfft) | The base. It has an opt-in `wasm_simd` feature (`src/lib.rs`, documented). Allocates only when planning, so plan once and reuse scratch buffers |
| [realfft](https://github.com/HEnquist/realfft) | 3.5.0 (2025-06-12) | `license = "MIT"` in Cargo.toml; **the repo has no LICENSE file** (checked the repo root) | pushed 2026-03-12, 18.7 M downloads | wraps rustfft | Real-input FFT at half the cost. A missing licence text is a minor compliance chore: ask upstream or copy the MIT text into our NOTICE |
| [ebur128](https://github.com/sdroege/ebur128) | 0.1.10 (2024-10-26) | MIT (LICENSE) | pushed 2025-12-13, 1.05 M downloads, 7 open issues | **5** crates (bitflags, dasp_frame, dasp_sample, smallvec) | The loudness pick. `rand` appears only in tests (source). Caveat: BS.1770 integrated loudness gates on 400 ms blocks, so it is fine for 1–2 s renders but loose for very short percussive clips. Use short-term/momentary values there |
| [bs1770](https://github.com/ruuda/bs1770) | 1.0.0 (2020-09-02) | Apache-2.0 | pushed 2025-12-03 | small | Fallback for the loudness pick |
| [spectrum-analyzer](https://github.com/phip1611/spectrum-analyzer) | 2.0.0 (2026-09-16) | MIT | pushed 2026-10-01, 175★ | 13, `#![no_std]`, uses microfft | Good for a UI spectrum display. Not built for STFT feature pipelines |
| [mel_spec](https://github.com/wavey-ai/mel-spec) | 0.5.0 (2026-10-02) | MIT | active, 1.68 M downloads | **69** (`image`, `ndarray`), plus `wgpu`/VAD modules that use `Instant` | Built to match Whisper. Too heavy and too opinionated for L3 |
| [spectrograms](https://github.com/jmg049/Spectrograms) | 2.1.3 (2026-07-08) | MIT | created 2026-01-23, 19★, 6 k downloads | 40 (ndarray, regex) | Young, single author |
| [pyin](https://github.com/Sytronik/pyin-rs) | 1.2.0 (2024-07-21) | MIT (LICENSE) | last push 2024-07-21 | **81** (ndarray 0.15, ndarray-npy, zip, statrs, libc) | `src/lib.rs` imports `libc` unconditionally and calls `libc::malloc` for its C API (source). I expect it to fail on wasm32-unknown-unknown, but that is not compiled — see Gaps. Use it as a reference implementation and write our own YIN/pYIN |
| [pitch-detection](https://github.com/alesgenova/pitch-detection) | 0.3.0 (2022-06-30) | MIT/Apache-2.0 | last push 2023-01-08, 260★ | 14 | McLeod/autocorrelation. Usable but stale |
| [cochlea-features](https://github.com/richer-richard/cochlea) | 0.7.0 (2026-08-11) | MIT OR Apache-2.0 | created 2026-07-10, 15★ | not checked | All-in-one (LUFS, onsets, pitch, chroma). Too new, single author |
| [dasp-rs](https://github.com/dasp-rs/dasp-rs), [resonant-analysis](https://github.com/TheKoren/resonant), [pitch-core](https://github.com/gzivdo/pitch-core), [cqt-rs](https://github.com/F0rty-Tw0/cqt-rs) | 2025–2026 | MIT / MIT-or-Apache (registry metadata) | 0–15★, low downloads | not checked | Watch list only |
| sonogram, aubio-rs, bliss-audio | — | **GPL-3.0** (registry) | — | — | Reference only |

- **No chroma crate of note** turned up (crates.io search for "chroma"/"chromagram" on 2026-10-19 *(documented)*). Chroma is not needed for Task 6. When it is, fold mel or CQT bins into 12 pitch classes (about 30 lines).
- **Onset and envelope following:** no maintained, permissive, focused crate. RMS per 10 ms frame and spectral flux are about 20 lines each.
- **Real-time or offline:** all scoring runs offline on a worker thread, so the audio-thread rules don't apply. But L3 must build for wasm32 and must be deterministic, which argues for our own code on `realfft`: no hidden RNG, no clocks, no `std::time`.

**Recommendation for `enth-analysis`:** depend on `rustfft`/`realfft` and `ebur128`. Own the STFT, mel filterbank, DCT (for MFCC), centroid/tilt, envelope, spectral flux and YIN, cross-checked against librosa in a Python fixture test. The probe's version of all of this (except YIN) is about 250 lines *(measured)*.

### Q2 — CMA-ES in Rust

| Crate | Version (date) | Licence | Health (2026-10-19) | Ask/tell | Batch through caller's evaluator | Bounds | Restarts | Seeded | wasm32 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| [cmaes](https://github.com/pengowen123/cmaes) | 0.2.2 (2024-12-12) | MIT OR Apache-2.0 (both files) | last push 2025-05-19, 40★ | **No.** Only `next()`/`next_parallel()` ([docs.rs](https://docs.rs/cmaes/latest/cmaes/struct.CMAES.html)) | **No.** Parallelism is internal `rayon` (hard dependency) | No | IPOP, BIPOP and local (`src/restart/`) | Yes (`seed`; ChaCha12) | **Problem:** `Instant::now()` in `CMAES::new` (`src/lib.rs:279`), which panics at run time on wasm32-unknown-unknown. 50 crates incl. nalgebra 0.33 and statrs. *(source read)* |
| [basin](https://github.com/jolars/basin) | 1.15.1 (2026-09-29) | MIT OR Apache-2.0 (both files) | created 2026-04-30, 32 releases, pushed 2026-10-18, 124★, 100 k downloads | **No.** The solver evaluates inside `next_iter` | Batch only via its own `rayon` `parallel` feature (`Problem::cost_batch`, results bit-identical to serial) | `BoundedCmaEs` (adaptive quadratic penalty, pycma-style) | **Not yet.** BIPOP is "reserved for restart machinery" (source comment) | Yes (ChaCha8; same trajectory "on every platform … including wasm32") | "Default build is wasm-friendly and single-threaded", uses `web-time` ([docs.rs](https://docs.rs/basin/latest/basin/)). 19 crates |
| [fcmaes-core](https://github.com/dietmarwo/fcmaes-rust) | 0.1.5 (2026-08-15) | MIT (LICENSE) | created 2026-07-24, 3★ (author of the Python fcmaes) | Says "ask/tell or equivalent batch boundary" ([docs.rs](https://docs.rs/fcmaes-core/latest/fcmaes_core/)) | Yes (claimed) | Yes | Retry module, no IPOP/BIPOP | partial | `rayon`; wasm not documented |
| [haru_cmaes](https://github.com/maulberto3/haru_cmaes) | 2.0.1 (2026-05-31) | registry says MIT OR Apache-2.0; **the repo has no detected licence** | 2★ | — | — | — | — | — | — |
| [argmin](https://github.com/argmin-rs/argmin) | 0.11.0 (2025-09-28) | MIT OR Apache-2.0 | 1,278★, 83 open issues | no CMA-ES in its solver list (not exhaustively checked) | — | — | — | — | — |
| genoxide, blackopt, heuropt, optimizer, cmaes-lbfgsb | 2025–2026 | MIT / MIT-or-Apache (registry) | ≤ 2.4 k downloads, months old | not checked | | | | | |
| mahf | 0.1.0 | **GPL-3.0** | — | reference only | | | | | |

Reference implementations to port or test against:

- [CyberAgentAILab/cmaes](https://github.com/CyberAgentAILab/cmaes): MIT, v0.13.1 on 2026-08-21. Ask–tell, with **CatCMA with Margin** for mixed continuous, integer and categorical variables (README).
- [pycma](https://github.com/CMA-ES/pycma): BSD-3-Clause (LICENSE checked), r4.5.0 on 2026-09-13.
- [CMA-ES with Margin, GECCO 2022](https://arxiv.org/abs/2205.13482): the paper behind the margin method.

*(documented)*

**Alternatives.** Differential evolution (`basin::De`, fcmaes) suits mixed or plateau-heavy landscapes but is usually slower than CMA-ES on continuous ones. Bayesian optimisation (`egobox` 1.0.0 on 2026-10-12, Apache-2.0; `friedrich`) pays off when an evaluation costs seconds. Ours costs about 10 ms, so the BO overhead buys nothing in the inner loop. It could suit a later "few expensive embedding evaluations" re-rank. *(documented / judgement)*

**Write our own?** Yes. The design wants the solver "single-threaded and pure; the caller supplies the parallel evaluator" (design §7, architecture L5), which means ask/tell. No crate offers ask/tell with seeded determinism and wasm safety. In the probe, a Hansen-2016 CMA-ES with positive weights, a Jacobi eigensolver for n ≤ 30, IPOP and bound handling by clamping plus a penalty took about 300 lines, with no dependencies beyond std *(measured)*. Test it against pycma or `basin` on sphere, Rosenbrock and Rastrigin with fixed seeds, keeping `basin` as a dev-dependency oracle. Port CatCMAwM (MIT) once templates have several categorical parameters.

### Q3 — Audio distances for synth sound matching (2025–2026)

| Work (date) | What was compared | Agreement with listening | Weight of evidence |
| --- | --- | --- | --- |
| [Salimi, Hindle, Zaïane, arXiv 2506.22628](https://arxiv.org/html/2506.22628v2) (v2 2025-10-08) | L1 spectrogram, scale-invariant MSE spectrogram, JTFS, soft-DTW on envelope; 4 differentiable synths with **2 parameters each**; 300 trials per pair | The best loss depends on the synth (SIMSE for filtered noise, JTFS for additive, DTW-envelope for AM). P-loss and MSS each disagreed with the listening ranking in 3 of 16 cells. Listening was done by **2 listeners, both authors**, inter-rater ρ = 0.86 | Weak: tiny parameter spaces, author-raters *(documented)* |
| [Salimi et al., arXiv 2608.27698](https://arxiv.org/abs/2608.27698) (2026-08-27) | Same 4 losses, out-of-domain, 7 scenarios; "partial parameter distance" | PPD agreed with blind listening on the top loss in **5 of 7** scenarios | Abstract only read *(documented)* |
| [Tian, Lattner, Saitis, ISMIR 2025, arXiv 2507.07764](https://arxiv.org/html/2507.07764v1) (2025-07-10) | MFCC, MSS, JTFS, 3 CLAP, 2 CDPAM, 2 EnCodec, DAC, Music2Latent, CAE, plus style (Gram) embeddings; 2,614 human ratings on 334 sounds | **MFCC competitive and beats many trained models. MSS underperforms. JTFS moderate on rank, poor on MAE. CDPAM transfers poorly. CLAP "Huang" style embedding best.** Metrics: MAE, Kendall, Spearman, NDCG, triplet agreement | Medium. The ratings come from classic timbre studies with pitch, loudness and duration equalised, so envelope and pitch differences, which matter in Recreate, are mostly absent *(documented)* |
| [INSTRUMENTAL, Bogdan, arXiv 2603.15905](https://arxiv.org/html/2603.15905v1) (2026-03-16) | 28-param subtractive + CMA-ES (λ = 40, σ₀ = 0.15 on [0,1], 100 k evaluations); loss = mel MSS + 0.1·centroid + 0.05·MFCC | **No listening test.** One real target. "90 % of the improvement in the first 10 k evaluations". 553 evaluations/s on a 10-core M4. The paper has internal inconsistencies (8 hypotheses claimed, 6 reported) | Weak *(documented)* |
| [Han, Lostanlen, Lagrange, arXiv 2311.14213](https://arxiv.org/abs/2311.14213) (rev. 2024-05) | PNP: a JTFS-based perceptual loss linearised around each parameter point | Builds on studies where scattering distance predicts dissimilarity judgements | Method paper *(documented)* |
| Vahidi et al. 2021 (QMUL DMRN workshop; via search summary) | MFCC, OpenL3, JTFS triplet agreement on 17 timbre datasets | Triplet agreement varied from about 0.41–0.48 (Grey 1978) to 0.71–0.88 by dataset | Secondary *(anecdotal: search snippet, PDF not opened)* |

Cost per evaluation, on top of the render:

- **Multi-resolution log-mel plus envelope, centroid and MFCC:** 5.5–10 ms for 2 s at 48 kHz, single thread, i7-9750H under load 6–26 *(measured)*. The distance itself costs about 0.1 ms once the target's features are cached *(measured)*.
- **JTFS:** no Rust implementation found. Kymatio (Python) only. Expect it to cost much more than an STFT. Not for Phase 0.
- **CLAP (HTSAT) through ONNX Runtime on CPU:** about 55 ms per 10 s input, whole pipeline, on an M4 Max ([dev.to, 2026](https://dev.to/kiarina/classifying-environmental-sounds-with-clap-and-onnx-runtime-a-1-nn-evaluation-on-esc-50-hoi)) *(anecdotal)*. Expect 1.5–3× that on the i7-9750H, and more under `tract`. That is 5–15× our per-evaluation cost.
- **Embedding weight licences:**
  - LAION CLAP: Apache-2.0 on HF, repo CC0-1.0.
  - DAC: MIT repo.
  - EnCodec: the README licenses only the code (MIT). Weights unclear.
  - MERT-v1-95M: **CC BY-NC 4.0** (HF tag).
  - Music2Latent: **CC BY-NC 4.0** (LICENSE).

  *(documented 2026-10-19)*
- **Runtimes:** `tract-onnx` 0.21.19 (2026-10-15, MIT OR Apache-2.0, pure Rust). `ort` 2.0.0-rc.14 (2026-10-12, MIT OR Apache-2.0, wraps the C++ ONNX Runtime, so it can't serve wasm32-unknown-unknown). *(documented)*

**Toy-probe comparison of the three Task 6 distances as search objectives** *(measured; my own toy 10-param subtractive synth; see Q6 for the setup)*:

| Distance | Strict threshold T1 reached (2,400 evaluations, waveform cycling) | Looser T3 reached | Notes |
| --- | --- | --- | --- |
| A: multi-resolution log-mel L1 (FFT 2048/1024/512, 64 mel bands) | 5/12 | 9/12 | best. With 8,000 evaluations: T1 6/12, T3 10/12, waveform right 12/12 |
| B: A + 10 ms RMS-dB envelope + energy-weighted log-centroid track (each scaled by its median over random pairs) | 1/12 | 9/12 | Landscape harder at the strict level |
| C: MFCC c0–c12 MSE | 0/12 (plain run, 2,000 evaluations) | — | clearly worst. Final parameter errors largest (median mean-abs 0.257 vs 0.187 for A) |

T1/T3 are proxies, not listening results: T1 is the median distance from the target to 16 renders with every continuous parameter jittered by ±1 % of its range (T3: ±3 %). "Not reaching T1" does not mean "sounds different".

### Q4 — A listening test where one listener judges 30 pairs

Method:

- **Use pairwise similarity ratings, as Task 6 plans: "how similar are A and B?", 1 = very different … 5 = indistinguishable.** That is the DCR-style protocol used by Salimi et al. (5-point) and by the timbre-dissimilarity literature. MUSHRA (hidden reference plus anchors, 0–100) answers a different question, quality against a reference, and needs several conditions per trial. It fits judging CMA-ES results against a target, not similarity pairs.
- **Add triplets as a cross-check where cheap.** "Which of B or C is closer to A?" is easier for a single listener and maps straight onto triplet agreement (the metric Tian et al. 2025 report). Triplets recover individual preferences better than ratings in a face-similarity study (search summary, *anecdotal*).
- **Use ABX/same–different for "sounds the same".** When the gate says "sounds the same to me", judge each CMA-ES result as target vs result in a forced "same / different" call with a hidden identical-pair control. A 1–5 rating is a poor operational definition of "same".

Design for 30 pairs:

1. **Stimuli:** 30 target–candidate pairs spread over distance quantiles (for example 6 bins × 5, binned by distance A so all three distances see a range). Same note(s) and performance as the scorer, 2 s each.
2. **Level matching:** normalise every clip to the same integrated loudness (ITU-R BS.1770 / EBU R128 via `ebur128`, e.g. −23 LUFS). For clips under about 1 s, match short-term loudness instead. Use the same normalisation the scorer uses, otherwise "louder sounds closer" leaks in. Add a 10 ms fade in and out, and a 0.5 s gap between A and B.
3. **Randomisation:** shuffle pair order with a recorded seed and randomise which sound plays first. Keep the metric hidden (filenames are random IDs). Allow replays but log the count.
4. **Reliability:** add 5 repeated pairs (test–retest: your own consistency is the ceiling any metric can reach), 2 identical pairs (should get 5) and 2 obviously different pairs (should get 1). That makes about 39 trials, roughly 15 minutes. Use one session with headphones at a fixed volume.
5. **Analysis per distance:**
   - **Kendall τ-b** (handles the many ties of a 5-point scale) and **Spearman ρ**, between the negated distance and your rating.
   - **95 % CIs by bootstrap** over pairs (10 k resamples).
   - Triplet agreement over every pair of pairs that you rated differently.
   - **Your test–retest ρ** as the ceiling.
   - Compare distances with a paired bootstrap on the τ difference.
6. **What n = 30 can show** (Fisher-z arithmetic, mine): critical Spearman ρ at α = 0.05 two-sided is about 0.36. ρ = 0.6 gives a 95 % CI of about [0.28, 0.80]; ρ = 0.8 gives about [0.61, 0.90]. So "agrees on most pairs" should become a number, e.g. **ρ ≥ 0.7 with the CI lower bound ≥ 0.4, and τ-b within 0.15 of your test–retest τ-b**. Telling A from B apart needs larger differences than 30 pairs can resolve, so prefer the simplest metric unless another is clearly better.

Tools:

- **The simplest is our own.** A tiny Rust or Python script writes randomised WAVs and a CSV, plays them, and records keypresses. Or a static local HTML page with `<audio>` elements that downloads a JSON of answers; that is a dev tool, not product UI.
- **[webMUSHRA](https://github.com/audiolabs/webMUSHRA):** pushed 2026-05-20. Supports MUSHRA, BS.1116, paired comparison and Likert. Its LICENSE.txt is a **custom Fraunhofer licence**: source must be shared with binaries, there is no patent licence, and modified versions must be renamed. Fine to *run* locally, but don't vendor it into the MIT repo.
- **[BeaqleJS](https://github.com/HSU-ANT/beaqlejs):** GPL-3.0, last push 2019. Avoid it.

*(licences documented 2026-10-19)*

### Q5 — Parameter-space distance and "how far were your knobs"

research.md §2 checks out against the sources:

- [Hayes, Saitis, Fazekas, ISMIR 2025 (best student paper)](https://arxiv.org/abs/2506.07199): symmetries make inversion ill-posed, and an equivariant flow beats regression on Surge XT.
- [Salimi 2026](https://arxiv.org/abs/2608.27698): partial parameter distance agreed with listening in 5 of 7 scenarios.
- [Han et al. 2023](https://arxiv.org/abs/2301.02886): weight parameter errors by perceptual effect.

*(documented)*

Additions and sharpening:

- **PNP gives §7 layer 1 its formula.** The PNP line ([arXiv 2311.14213](https://arxiv.org/abs/2311.14213)) uses the local metric M(θ) = J(θ)ᵀJ(θ), where J is the Jacobian of the auditory features with respect to the parameters. The parameter distance then becomes Δθᵀ M Δθ, "the best quadratic approximation of the spectral loss". We can get J by finite differences, the **same renders** that power `suggest` and the coach hints (§7 layer 2). One mechanism serves layers 1 and 2. For the 10-param template that is 20 extra renders at about 10 ms each, so about 0.2 s single-threaded. *(documented + arithmetic)*
- **Insensitive parameters.** In the probe, final parameter errors stayed large (median mean-abs 0.15–0.19 of range) even on solves that met the strict audio threshold. Resonance with the cutoff wide open, filter-env amount with a short filter decay, and release beyond the render window are near-invisible to the audio. *(measured)* This is direct evidence that raw parameter distance would punish correct answers. Use the sensitivity weighting (M(θ)), or partial/critical-parameter distance, as §7 already says.
- **Categorical parameters need care in both the distance and the search.** Waveform mismatch is discrete. Score it as a separate component ("wrong waveform") rather than inside a continuous distance. In search, use waveform-cycling restarts (measured fix) or CatCMAwM ([CyberAgent cmaes](https://github.com/CyberAgentAILab/cmaes), MIT).
- **Symmetries.** Canonicalise before comparing (sort identical oscillators by a key, e.g. pitch then level). Where structure differs, fall back to layer 3 (nearest passing solution). That matches §7. I found no newer (2026) work that changes this; search on 2026-10-19 found none beyond the above.

### Q6 — A concrete Phase 0 metric spike, and whether the gate is realistic

**What I measured today** (scratch crate `/tmp/enth-research/04/metric-probe`, Rust 1.95.0, release build, `realfft` 3.5.0 only; i7-9750H, 6C/12T; **load average 6–26 from six parallel researchers, so treat times as upper bounds**):

- **Toy template:** 10 params (waveform saw/square/triangle; detune 0–50 cents; cutoff 50 Hz–16 kHz; resonance Q 0.5–10; filter-env amount 0–6 octaves; filter-env decay; amp A/D/S/R). Two polyBLEP oscillators into a TPT SVF lowpass, 1 voice, A3, note-off at 1.5 s, 2 s at 48 kHz.
- **Per evaluation:** render 2.3–5.6 ms and features 5.5–10.0 ms. Whole evaluation **9.7–20 ms** depending on load.
- **CMA-ES:** own ask/tell, λ = 10 (= 4 + ⌊3 ln n⌋), σ₀ = 0.3 on [0,1], random start, clamp plus quadratic penalty. IPOP doubles λ after every three restarts. Restart when there has been no 1 % improvement for 15 generations, or σ·max(D) < 1e-4.

| Run | Budget | Result (12 targets) |
| --- | --- | --- |
| Plain CMA-ES (waveform as a continuous param floored to 3 classes), slow restart rule | 2,000 | A: T1 5/12 (median 579 evaluations); B: 1/12; C: 0/12; **waveform wrong in 3/12 (A)** |
| Waveform fixed per restart, cycling; slow restart rule | 2,400 | Restarts rarely fired, so the cycle didn't help (waveform right 8/12). **The restart rule matters** |
| Waveform cycling + 1 %-in-15-generations restart rule | 2,400 | A: T1 5/12, T3 9/12 (median 534 evaluations), waveform right 11/12; B: T1 1/12, T3 9/12 |
| Same, distance A only | 8,000 | **T1 6/12 (median 3,692 evaluations), T3 10/12 (median 3,708), waveform right 12/12.** About 11 ms per evaluation |

**Recommended spike** (refines Task 6 rather than replacing it):

1. **Template:** keep the 10 parameters. Mark the waveform as categorical in the schema.
2. **Features:** multi-resolution log-mel (2048/1024/512, hop n/4, 64 mel bands, 30 Hz–16 kHz, log(mag + 1e-5)), 10 ms RMS-dB envelope, energy-weighted log-centroid track, f0 (YIN). Normalise loudness first with `ebur128`, the same as the listening clips.
3. **Distances:**
   - (A) multi-resolution log-mel L1.
   - (B) A + envelope + centroid, each scaled by its median over random pairs.
   - (C) MFCC, kept as the control because Tian 2025 found it competitive on human timbre ratings. Expect it to lose as a search objective.
   - Optional (D): A + spectral convergence (the auraloss/INSTRUMENTAL form), which costs nothing extra.
4. **Listening:** 30 pairs + 5 repeats + 4 anchors, as in Q4. Kendall τ-b, Spearman and bootstrap CIs per distance.
5. **CMA-ES:**
   - Own ask/tell in `spikes/metric`, with a caller-side evaluator over `std::thread::scope`, 8 workers.
   - λ = 10, σ₀ = 0.3, IPOP, the relative-stall restart rule, waveform cycling across restarts.
   - Heuristic start (§8.1) vs random start as an A/B.
   - Budgets of 2,000, 5,000 and 10,000 evaluations; 20 targets × 3 distances.
   - Record evaluations-to-threshold and wall time with 1 and 8 workers.
6. **"Sounds the same":** an operational ABX/same–different listen of target vs result on all 20 results of the winning distance, at its 5 k budget.

**Realistic time per solve** for 10 parameters with 8 workers, at 10–20 ms per evaluation (scaling by worker count is inferred, not measured):

| Budget | Wall time |
| --- | --- |
| 2,000 evaluations | 3–5 s |
| 5,000 evaluations | 6–13 s |
| 10,000 evaluations | 13–25 s |

That needs ≥ 100× realtime per core at 16 voices, or the solver uses 1-voice renders.

**Is the Task 6 gate realistic?**

- **"Under a minute":** yes, comfortably for 10 parameters on this laptop with 8 workers.
- **"Most targets sound the same":** not proven. With random starts and 8,000 evaluations, the probe met the strict proxy on 6/12 and the looser one on 10/12. Whether the looser one is "the same to your ears" is exactly what the ABX check must settle. Expect "most" to need the heuristic start and maybe 2–3 independent restarts in parallel. Write the gate with numbers (below) so a near-miss is visible rather than argued.

## Recommendation for Phase 0

1. **`enth-analysis` (L3):** own STFT, mel, MFCC, centroid, envelope, flux and YIN on `realfft`/`rustfft`, plus `ebur128`. Add a Python fixture test against librosa. Don't depend on `mel_spec`, `pyin` or `spectrograms`.
2. **`enth-solver` (L5):** own ask–tell CMA-ES (Hansen 2016 defaults, IPOP, bounds by clamp plus penalty, categorical parameters by restart cycling now and CatCMAwM later), seeded with our own RNG, no clocks or threads. Validate against pycma or `basin` on standard test functions. Budget it as **evaluations**, not renders, with a time cap the caller enforces.
3. **Metric v1 candidate:** distance A (multi-resolution log-mel L1 after loudness normalisation) as the search objective. Report components (envelope, centroid, f0, waveform) as separate coaching outputs, as §6 already says. Choose between A and B by the listening data, not by the probe.
4. **Task 6 additions:** level matching, repeats and anchors, Kendall τ-b + Spearman + bootstrap CI, an ABX check for "sounds the same", evaluations-to-threshold and wall time at 1 and 8 workers, heuristic vs random start.
5. **Embeddings (CLAP etc.):** not in Phase 0. Revisit in R-milestones as a final re-ranker of the K best CMA-ES candidates (a few hundred embedding calls, seconds), only with Apache, MIT or CC0 weights (LAION-CLAP, DAC). Exclude MERT and Music2Latent (CC BY-NC).

## Plan check — what the evidence says should change

- **design.md §6 (`sound` scorer):** keep the structure. Add that loudness normalisation uses BS.1770/EBU R128 (`ebur128`), with a short-clip rule (short-term loudness under about 1 s). Add that categorical mismatches (e.g. waveform) are reported as a separate component, not hidden in the spectral distance. Reason: the probe showed waveform errors dominate failures.
- **design.md §7 budget check: change it.**
  - (a) State the budget per **evaluation**, render plus analysis: analysis measured at 5.5–10 ms, the same as or more than a 1-voice render.
  - (b) "A 1,000-render search" is optimistic. The probe needed a median of about 600 evaluations (strict, when it succeeded) up to about 3,700 (8 k-budget run), and some targets were unsolved at 8,000. Plan **5,000–10,000 evaluations per solve**: about 6–25 s with 8 workers, still inside a minute.
  - (c) Layer 1's "weight each by its audio sensitivity" should name the concrete form, Δθᵀ JᵀJ Δθ, with J from the same finite-difference renders as layer 2. That unifies layers 1 and 2.
- **design.md §8 (solver pipeline):** add "categorical parameters: restart cycling or a mixed-variable CMA-ES (CatCMAwM)". Keep the heuristic start; the probe shows random starts leave about half the targets short of the strict proxy.
- **architecture.md L5:** no change. "The solver spawns no threads, caller-supplied evaluator" is right, and it is the reason to write our own CMA-ES: no crate fits.
- **phase-0.md Task 6: change it** as in "Recommended spike": level matching, repeats and anchors, Kendall τ-b + bootstrap CI, ABX for "sounds the same", evaluations and wall time at 1 and 8 workers, heuristic vs random start, 5 k/10 k budgets.
- **roadmap.md Gate G0: make it numeric.** For example:
  - "Best distance: Spearman ρ ≥ 0.7 against your 30 ratings, lower 95 % CI ≥ 0.4, and within 0.15 τ-b of your own test–retest."
  - "With 8 workers and ≤ 60 s wall per target, ≥ 14 of 20 targets judged *same* in an ABX check."

  As written ("most pairs", "most targets") the gate can't fail clearly.
- Rust and native UI are unaffected. Nothing here touches the shell or UI choices.

## Gaps — what I couldn't verify, and the local check that would settle it

| Gap | Local check after the reboot |
| --- | --- |
| **wasm32 builds not checked** (no wasm32 target installed): `pyin` (libc C API), `cmaes` (`Instant::now` panics at run time, a compile is not enough), `basin`, `rustfft`, `realfft`, `ebur128` | `rustup target add wasm32-unknown-unknown`, then in a scratch crate with those deps: `CARGO_BUILD_JOBS=2 cargo check --target wasm32-unknown-unknown`. For `cmaes`, run a smoke test under `wasm32-wasip1` + `wasmtime` too (WASI has clocks; unknown-unknown doesn't) |
| **Probe timings taken under load 6–26**, and the probe sits in `/tmp`, which Mint may clear at boot | If `/tmp/enth-research/04/metric-probe` survives: `./target/release/metric-probe 8000 12 8 0.01 cycle 3` on an idle machine (args: budget, targets, threads, jitter, `cycle`, number of distances). If it is gone, Task 6 re-implements it. The design is described in Q6 |
| **Per-worker scaling** (8 workers) inferred, not measured | Same command with threads = 1 vs 8 on an idle machine; compare `per-eval` and wall time |
| **T1/T3 proxies not validated by ears** | Task 6's ABX check on solved and unsolved results |
| **Heuristic start vs random start** not tested | Add §8.1 heuristics to the probe or spike and rerun at the same seeds |
| **CLAP/EnCodec/DAC cost on this CPU** (only an M4 Max anecdote) | Export the LAION-CLAP audio encoder to ONNX; time 100 clips with `ort` and with `tract-onnx` 0.21.19 on the i7-9750H (`hyperfine` or Criterion) |
| **EnCodec weights licence** unclear | Read the HF model card and Meta's audiocraft weight terms before any use |
| **Salimi 2026 listening details** (listeners, scale): abstract only | Read the full text of arXiv 2608.27698 |
| **Tian 2025 numbers**: only plots, no table read | Pull figures/data from [timbremetrics](https://github.com/tiianhk/timbremetrics) (repo has no detected licence, so reference only) |
| **argmin has no CMA-ES**: not exhaustively verified | `gh api repos/argmin-rs/argmin/contents/crates/argmin/src/solver` |
| **fcmaes-core ask/tell signatures** not inspected | Download the crate source and read `Cmaes` |
| **realfft has no LICENSE file in the repo** | Open an upstream issue, or add the MIT text to our third-party notices; `cargo deny check licenses` will show the declared licence only |
| **Spearman CI figures** are my Fisher-z/Bonett–Wright arithmetic, not a library | Recompute with scipy or a Rust bootstrap in the spike |
