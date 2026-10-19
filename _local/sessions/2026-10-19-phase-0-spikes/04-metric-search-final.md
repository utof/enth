# 04 — Metric and search (final)

- Date: 2026-10-19 (first report written ~02:20; this final report after two reboots, local work 02:30–06:00)
- Sub-researcher: 04 (metric and search)
- Questions answered: 1 (analysis crates), 2 (CMA-ES in Rust), 3 (audio distances), 4 (listening test for 30 pairs), 5 (parameter-space distance), 6 (Phase 0 spike, gate realism), plus every item left under Gaps in `04-metric-search.md`.
- Relation to the first report: `04-metric-search.md` stays as written. This report replaces it and stands on its own. Anything new or different is marked **[changed]** or **[new]**; everything else carries over and was re-checked today.
- Labels: *measured* = I ran it on the owner's laptop today; *documented* = read in source, docs, a paper or a registry/API today; *anecdotal* = a blog post or other single unreplicated report.
- Machine caveat for every timing: i7-9750H (6 cores / 12 threads, governor `powersave`, max 4.5 GHz), rustc 1.95.0. Five other researchers were running, and processes outside the shared lock kept the machine busy (a Cinnamon process at about 100 %, an NTFS mount helper, another researcher's Python at 100–800 %). Load averages during my timings were 2.6–25, recorded next to each number. Even with an exclusive lock, timings are **upper bounds**. Evaluation counts and success rates are deterministic, so load doesn't affect them.
- Scratch code (not in the repo; persistent): `~/code/enth-scratch/2026-10-19-phase-0-spikes/04/` — `probe2/` (toy synth, features, distances, CMA-ES, heuristic start, parallel evaluator), `wasm-probe/`, `wasm-run/`, `wasm-check/`, `clap-bench/`, run logs `probe2/r*.txt`, `notes.md`.

## Answer in brief

- **Analysis: build on `rustfft` 6.4.1 + `realfft` 3.5.0 and write our own mel/MFCC/centroid/envelope/YIN; use `ebur128` 0.1.10 for loudness.** All three pass `cargo check` for wasm32-unknown-unknown *(measured)*. `pyin` 1.2.0 **fails** on wasm32 with 11 libc errors (`malloc`, `memcpy`, `EINVAL`…) *(measured, [changed]: was inferred)*. `realfft` declares MIT but ships no licence text, in the repo or in the crate tarball; upstream issue #45 is open *(documented)*.
- **CMA-ES: write our own small ask–tell implementation.** Measured on wasm32:
  - `cmaes` 0.2.2 needs getrandom's `js` feature to build, then **traps at run time** with "time not implemented on this platform" *(measured, [changed])*.
  - `basin` 1.15.1 runs, but has no ask/tell and no IPOP/BIPOP.
  - **[new]** `fcmaes-core` 0.1.5 does have ask/tell, a seed and bounds, and runs on wasm32 *(measured)*. But it is 3 months old, has 3 stars and a single author, and pulls in nalgebra + rayon. Keep it as a fallback, not a dependency.
- **The heuristic start pays off [new, measured].** Distance A, 20 targets, 5,000 evaluations, 8 workers:
  - Strict threshold (T1): 11 → 14 solved at λ = 10; 14 → 15 at λ = 16.
  - Median evaluations to the looser threshold (T3): 1,318 → 645 at λ = 10; 1,124 → 231 at λ = 16.
  - Distance B: 8 → 13 solved at T1. Distance C: 6 → 9.
- **Population size and parallel restarts [new, measured].** λ = 16 beats the default λ = 10 with 8 workers: random-start T1 rose 11 → 14. Running heuristic and random restarts side by side solves 18/20 at T1. **Every** target was solved by at least one of the four distance-A configurations, so the failures are search luck, not unreachable targets.
- **Measured cost per evaluation [changed]:** render 2.2–2.5 ms plus features 5.1 ms (hop n/4) or 2.5 ms (hop n/2). That is about 7.9 ms single-threaded, 127 evaluations/s (load 8). The first report's 10–20 ms came from a more heavily loaded machine.
- **8-worker speed-up is 4.4–5.4×, not 8× [changed: was inferred].** Per solve it measured 3.97× (29.1 s → 7.3 s). Results are **bit-identical** for 1 and 8 workers (same checksum) *(measured)*. A contiguous-chunk evaluator used only 5 of 8 threads at λ = 10 (2.3×), so use a work queue and set λ ≥ workers.
- **Time per solve, measured [changed]:** heuristic start, λ = 16, 8 workers, 5,000-evaluation cap. Per solve: median 2.6 s, maximum 10.6 s. Median time to reach the strict threshold: 1.5 s. Load was 10 during the exclusive run. **"Under a minute" holds with a wide margin.**
- **Which distance [changed, 20 targets]:** multi-resolution log-mel L1 (A) remains the best single objective at the strict threshold (heuristic start: A 14–15/20, B 13/20, C 9/20). B matches A at the loose threshold (19–20/20). MFCC (C) is still worst. The first report's "C 0/12" came from a weaker setup.
- **CLAP embedding cost on this CPU, measured [changed: was 5–15× a render, from an anecdote]:** the LAION CLAP-HTSAT audio encoder through `tract-onnx` 0.23.9 takes 0.61–0.70 s per call, single-threaded, whatever the input length (load 17–18). That is **about 60–90× our per-evaluation cost**. Fine for re-ranking a handful of candidates; never inside the search loop.
- **Listening test [changed]:** rate all 30 pairs **twice**, in two reshuffled sessions, instead of the first report's 5 repeats; 5 repeats can't give a test–retest correlation. Loudness-match every clip, add anchors, and report Kendall τ-b + Spearman with bootstrap CIs, plus triplet agreement. With n = 30, ρ = 0.7 has a 95 % CI of about [0.42, 0.86] *(computed)*.
- **Gate realism:** "under a minute" is comfortably met. "Most targets sound the same" is plausible on the toy synth: 15/20 strict and 17/20 loose with the heuristic start, 18/20 with the side-by-side restarts. Until an ABX listen confirms that the loose proxy means "same" to your ears, it stays a proxy. Make the gate numeric.

## What changed since the first report

| Topic | First report | Now (evidence) |
| --- | --- | --- |
| `cmaes` on wasm32 | Inferred from `Instant::now()` in the source | Build needs getrandom `js`; run in node 22 traps: "time not implemented on this platform" *(measured)* |
| `pyin` on wasm32 | Expected to fail | Fails: 11 libc errors, even with getrandom `js` *(measured)* |
| `fcmaes-core` | "Ask/tell claimed" | `ask() -> Vec<Vec<f64>>`, `tell(&[f64]) -> i32`, seed, bounds; runs on wasm32 *(measured)* |
| `tract-onnx` version | "0.21.19 (2026-10-15)" — wrong pick from the registry API | Latest is **0.23.9** (2026-10-15, rust-version 1.91). 0.21.19 is a backport published the same day. wasm32 needs getrandom 0.4 `wasm_js`; the MSRV-aware resolver is needed because `kstring` 2.0.5 wants rustc 1.96 *(measured)* |
| Cost per evaluation | 9.7–20 ms (load 6–26) | 7.9 ms single-thread at load 8; features at hop n/2 halve analysis cost *(measured)* |
| 8-worker speed-up | Inferred 8× | 4.4–5.4× throughput, 3.97× per solve, results bit-identical *(measured)* |
| Heuristic vs random start | Not tested | Heuristic better on every distance (see Q6) *(measured)* |
| λ for 8 workers | Default 10 | 16 is better (success and parallel efficiency) *(measured)* |
| Distances (12 → 20 targets) | A > B > C, C 0/12 | A ≥ B > C; C 6–9/20 with restart cycling *(measured)* |
| CLAP cost | 5–15× a render (M4 Max anecdote) | 0.61–0.70 s per call via tract on this CPU, about 60–90× *(measured)* |
| Salimi 2026 | Abstract only | Full paper read: 4 author-raters, ICC3k 0.86, 2–3 params, gradient-only optimiser *(documented)* |
| Listening design | 5 repeated pairs | Two full sessions (test–retest over all 30) |
| wasm determinism | — | Native vs wasm distances differ by ≤ 2.7e-6 relative; wasm is 2.1–3.3× slower *(measured)* |
| Weights licences | EnCodec unclear | DAC weights MIT (README); original EnCodec weights still unclear; AudioCraft weights CC BY-NC 4.0 *(documented)* |

## Findings

### Q1 — Rust crates for STFT, mel, MFCC, chroma, onset/envelope, loudness and pitch

Sources and checks:

- Registry data: crates.io API, 2026-10-19, read as the `max_stable_version` record (fixing the first report's tract mistake).
- Repo data: `gh api`, 2026-10-19.
- Licences: checked in the repo's LICENSE files.
- wasm32 column: `cargo check --target wasm32-unknown-unknown`, rustc 1.95.0, 2026-10-19 *(measured)*.

| Crate | Version (date) | Licence (verified) | Health | Dependencies (normal) | wasm32 check | Fit |
| --- | --- | --- | --- | --- | --- | --- |
| [rustfft](https://github.com/ejmahler/RustFFT) | 6.4.1 (2025-09-18), rust-version 1.61 | MIT OR Apache-2.0 (both files) | pushed 2026-10-17, 949★, 32.6 M downloads | small | **OK** | The base. Optional `wasm_simd` feature: our probe ran 1.6× faster with it on wasm (below) |
| [realfft](https://github.com/HEnquist/realfft) | 3.5.0 (2025-06-12) | Cargo.toml `license = "MIT"`; **no LICENSE file in the repo or the crate tarball**; [issue #45](https://github.com/HEnquist/realfft/issues/45) "Switch to MIT + Apache dual license" open | pushed 2026-03-12, 18.7 M downloads | wraps rustfft | **OK** | Real-input FFT. Licence text: add the standard MIT text to our third-party notices, or follow up on #45 |
| [ebur128](https://github.com/sdroege/ebur128) | 0.1.10 (2024-10-26) | MIT | pushed 2025-12-13, 1.05 M downloads | **5** crates | **OK** | The loudness pick. API has `loudness_global`, `loudness_momentary`, `loudness_shortterm`, `loudness_window(ms)`, `loudness_range`, `true_peak` (source). `rand` only in tests |
| [bs1770](https://github.com/ruuda/bs1770) | 1.0.0 (2020-09-02) | Apache-2.0 | pushed 2025-12-03 | small | not checked | Fallback |
| [spectrum-analyzer](https://github.com/phip1611/spectrum-analyzer) | 2.0.0 (2026-09-16) | MIT | pushed 2026-10-01, 175★ | 13, `no_std` | **OK** | UI spectrum display |
| [mel_spec](https://github.com/wavey-ai/mel-spec) | 0.5.0 (2026-10-02) | MIT | active, 1.68 M downloads | **69** (`image`, `ndarray`) | **OK** | Built for Whisper. Too heavy for L3 |
| [spectrograms](https://github.com/jmg049/Spectrograms) | 2.1.3 (2026-07-08) | MIT | created 2026-01, 19★ | 40 | **OK** (49 s check) | Young, single author |
| [pyin](https://github.com/Sytronik/pyin-rs) | 1.2.0 (2024-07-21) | MIT | last push 2024-07-21 | **81** | **FAILS**: getrandom, then with getrandom `js` 11 errors (`libc::malloc`, `memcpy`, `EINVAL`, `c_double`, `c_uint`) | Reference only. Write our own YIN/pYIN |
| [pitch-detection](https://github.com/alesgenova/pitch-detection) | 0.3.0 (2022-06-30) | MIT/Apache-2.0 | last push 2023-01 | 14 | **OK** | Stale |
| cochlea-features 0.7.0, dasp-rs 0.5.2, resonant-analysis 0.4.0, pitch-core 0.1.1, cqt-rs 0.2.1 | 2025–2026 | MIT / MIT-or-Apache (registry) | 0–15★ | — | not checked | Watch list |
| sonogram, aubio-rs, bliss-audio | — | **GPL-3.0** | — | — | — | Reference only |

- **Chroma:** no crate of note. Fold mel or CQT bins into 12 pitch classes when needed (not needed for Task 6).
- **Onset/envelope:** no maintained crate. RMS-per-frame and spectral flux are about 20 lines each. **[new]** Detuned oscillators make the 10 ms RMS beat at the detune rate: in target 0 the RMS swings from 0.11 to 1.5 within about 150 ms *(measured)*. Envelope features and heuristics must smooth over beats (crest envelope over about one beat period). That beating also makes an envelope-distance term sensitive to beat phase.
- **Loudness on short clips:** BS.1770 integrated loudness gates on 400 ms blocks. Use `loudness_shortterm`/`loudness_window` for clips under about 1 s.
- **Cross-platform determinism of our own features [new, measured]:** the probe core compiled for wasm32 and run in node 22, against native x86-64, over 8 render pairs.

  | Quantity | Max relative difference |
  | --- | --- |
  | Distance A | 2.7e-6 |
  | Distance B | 1.2e-6 |
  | Distance C | 2.5e-6 |
  | Log-mel sum | 5.5e-8 |
  | Raw audio sum | 2.5e-5 (one near-zero sum) |

  Cause: `exp`/`powf`/`tan` come from glibc natively and from Rust's own libm on wasm. A SIMD wasm build differs from a scalar wasm build in the last bits too. Feeds design §5's "equal within a tolerance": about 1e-5 relative on distances is a safe tolerance.
- **wasm speed [new, measured]:** one evaluation (render + features + distance) took 40.4 ms in node 22 (scalar wasm) and 25.8 ms with `+simd128` and rustfft `wasm_simd`. Native at the same moment took 12.4 ms (load 10–15), so wasm is 2.1–3.3× slower than native.

**Recommendation for `enth-analysis` (unchanged):** depend on `rustfft`/`realfft` and `ebur128`. Own STFT, mel, DCT/MFCC, centroid/tilt, envelope (beat-aware), flux and YIN, cross-checked against librosa in a fixture test. The probe's version, minus YIN, is about 250 lines.

### Q2 — CMA-ES in Rust

| Crate | Version (date), MSRV | Licence | Health (2026-10-19) | Ask/tell | Caller-side batch | Bounds | Restarts | Seeded | wasm32 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| [cmaes](https://github.com/pengowen123/cmaes) | 0.2.2 (2024-12-12) | MIT OR Apache-2.0 (both files) | last push 2025-05-19, 40★ | No (`next`, `next_parallel`) | No (internal rayon) | No | IPOP, BIPOP, local | Yes | **Build fails** without getrandom `js`. **Run traps:** "time not implemented on this platform" (`Instant::now` in `CMAES::new`) *(measured)* |
| [basin](https://github.com/jolars/basin) | 1.15.1 (2026-09-29), 1.87 | MIT OR Apache-2.0 | created 2026-04-30, 32 releases, 124★ | No (solver evaluates inside `next_iter`) | Only via its own rayon `parallel` feature (bit-identical to serial) | `BoundedCmaEs` (adaptive penalty) | None yet ("reserved") | Yes (ChaCha8) | **Runs**: 5-D sphere, 200 iterations → 1.4e-20 in node *(measured)*. Uses `web-time` |
| [fcmaes-core](https://github.com/dietmarwo/fcmaes-rust) | 0.1.5 (2026-08-15), 1.88 | MIT | created 2026-07-24, 3★, single author (author of Python fcmaes) | **Yes**: `ask() -> Vec<Vec<f64>>`, `tell(&[f64]) -> i32`, `tell_x` (source) | Yes, via ask/tell | `Fitness::bounded` | Retry module, no IPOP/BIPOP | Yes (`CmaesParams.seed`, PCG) | **Runs** ask/tell: 5-D sphere → 7.1e-16 in node, with getrandom `js` in the tree *(measured)*. Deps nalgebra 0.33, rand 0.8, rayon; `Instant` only in `retry`/`moretry` |
| [argmin](https://github.com/argmin-rs/argmin) | 0.11.0 (2025-09-28) | MIT OR Apache-2.0 | 1,278★ | — | — | — | — | — | **No CMA-ES**: its solver directory lists brent, CG, Gauss–Newton, golden section, GD, Landweber, line search, Nelder–Mead, Newton, PSO, quasi-Newton, SA, trust region *(documented, [changed]: now verified)* |
| haru_cmaes 2.0.1; genoxide, blackopt, heuropt, optimizer, cmaes-lbfgsb | 2025–2026 | MIT / MIT-or-Apache (registry); haru repo has no detected licence | ≤ 2.4 k downloads | not checked | | | | | |
| mahf 0.1.0 | — | **GPL-3.0** | — | reference only | | | | | |

Reference implementations to port or test against: [CyberAgentAILab/cmaes](https://github.com/CyberAgentAILab/cmaes) (MIT, v0.13.1 on 2026-08-21; ask–tell; CatCMA with Margin for mixed continuous, integer and categorical variables) and [pycma](https://github.com/CMA-ES/pycma) (BSD-3-Clause, r4.5.0 on 2026-09-13) *(documented)*.

**Alternatives:**

- Differential evolution: `basin::De`, fcmaes.
- Bayesian optimisation: [egobox](https://github.com/relf/egobox) 1.0.0 (2026-10-12, Apache-2.0). BO pays off only when evaluations are expensive, e.g. a final CLAP-scored re-rank at about 0.6 s per call.

**Decision: our own ask–tell CMA-ES in `enth-solver` (unchanged), with fcmaes-core as the fallback if ours stalls.** Reasons:

- The design wants a pure, single-threaded solver with a caller-supplied evaluator; only fcmaes-core fits, and it is young.
- Ours is about 300 lines with no dependencies, and in the probe it gave **bit-identical results for 1 and 8 workers** *(measured)*.
- Validate it against pycma or `basin` on sphere, Rosenbrock and Rastrigin with fixed seeds.

### Q3 — Audio distances for synth sound matching (2025–2026)

| Work (date) | What was compared | Agreement with listening | Weight of evidence |
| --- | --- | --- | --- |
| [Salimi, Hindle, Zaïane, arXiv 2506.22628](https://arxiv.org/html/2506.22628v2) (v2 2025-10-08) | L1 spectrogram, scale-invariant MSE, JTFS, soft-DTW envelope; 4 synths with **2 params each** | Best loss depends on the synth. P-loss and MSS each disagreed with the listening ranking in 3/16 cells. Listening: 2 author-raters, inter-rater ρ = 0.86 | Weak *(documented)* |
| [Salimi et al., arXiv 2608.27698](https://arxiv.org/html/2608.27698v1) (2026-08-27) — **[changed] full paper read** | Same 4 losses. 7 scenarios: band-passed saw vs band-passed noise, 3 AM variants, 3 chirplet-pitch variants; 2–3 searched params. STFT 512, hop 100; JTFS J = 10, Q = 2; soft-DTW γ = 1; 1 s at 48 kHz; RMSProp lr 0.045, 200 steps, ≥ 300 trials per loss per scenario; JAX + DawDreamer/Faust | Listening: **4 authors** (3 not involved in the synths), blind, 5-point "are the two sounds similar?", 40 pairs per loss per scenario, ICC3k = 0.86, top loss stable under leave-one-rater-out (mean ρ 0.98). PPD top loss = listening top loss in **5/7**; both disagreements are overlapping-carrier AM, where listeners preferred spectrogram losses. Winners: SIMSE for filter cutoffs, DTW-envelope for AM rate, JTFS for pitch glides. No correlation coefficients, no compute costs | Medium-weak. Authors' own caveats: author panel, gradient-only optimiser ("rankings may change under … evolutionary methods"), unclear scaling to more parameters or real targets *(documented)* |
| [Tian, Lattner, Saitis, ISMIR 2025](https://arxiv.org/html/2507.07764v1) | MFCC, MSS, JTFS, 3 CLAP, 2 CDPAM, 2 EnCodec, DAC, Music2Latent, CAE, style embeddings; 2,614 human ratings on 334 sounds | **MFCC competitive. MSS underperforms. JTFS moderate on rank, poor on MAE. CDPAM transfers poorly. CLAP "Huang" style embedding best.** Results shown only in figures | Medium; the ratings are pitch/loudness/duration-equalised timbre studies, so envelope and pitch differences are absent *(documented)* |
| [INSTRUMENTAL, arXiv 2603.15905](https://arxiv.org/html/2603.15905v1) (2026-03-16) | 28-param subtractive + CMA-ES (λ = 40, σ₀ = 0.15, 100 k evaluations); loss = mel MSS + 0.1·centroid + 0.05·MFCC | **No listening test.** 90 % of the gain in the first 10 k evaluations; 553 evaluations/s on a 10-core M4; internal inconsistencies | Weak *(documented)* |
| [Han, Lostanlen, Lagrange, arXiv 2311.14213](https://arxiv.org/abs/2311.14213) | PNP: JTFS loss linearised around each parameter point | Builds on scattering-dissimilarity studies | Method *(documented)* |
| Synth-specific human similarity data | Search 2026-10-19 | None found. The closest is a [TISMIR study](https://transactions.ismir.net/articles/10.5334/tismir.76) with 80 synthetic sounds rated on perceptual scales (not pairs) | *(documented)* |

Cost per evaluation on this CPU:

- **Render (toy, 1 voice, 2 s at 48 kHz):** 2.2–2.5 ms *(measured, load 8–14)*.
- **Features** (3-resolution log-mel, 10 ms envelope, centroid track, 13 MFCC): 5.1 ms at hop n/4, **2.5 ms at hop n/2** *(measured)*.
- **Distance on cached target features:** A 0.13 ms, B 0.13 ms, C 0.008 ms *(measured)*.
- **[changed] CLAP (LAION CLAP-HTSAT unfused) audio encoder through `tract-onnx` 0.23.9, single call, single thread:** median 701 ms (10 runs, load 17) and 609–612 ms (load 18). Inputs of 201 frames (2 s) and 1,001 frames (10 s) cost the same, because HTSAT resizes to a fixed image. Model load 0.44 s, optimisation 1.48 s, peak RSS 268 MB *(measured)*. Model: `onnx/audio_model.onnx` from [Xenova/clap-htsat-unfused](https://huggingface.co/Xenova/clap-htsat-unfused) (117,528,416 bytes; SHA-256 prefix matches the LFS id), base model [laion/clap-htsat-unfused](https://huggingface.co/laion/clap-htsat-unfused) Apache-2.0. Not measured: ONNX Runtime (`ort`), tract's `multithread-mm` feature, the quantised model, mel preprocessing. For comparison, about 55 ms on an M4 Max with ONNX Runtime ([dev.to, 2026](https://dev.to/kiarina/classifying-environmental-sounds-with-clap-and-onnx-runtime-a-1-nn-evaluation-on-esc-50-hoi)) *(anecdotal)*.
- **JTFS:** no Rust implementation.

Embedding weight licences:

| Model | Weights licence |
| --- | --- |
| LAION-CLAP | Apache-2.0 (HF) / CC0-1.0 (repo) |
| DAC | MIT ("Weights are released as part of this repo under MIT license", [README](https://github.com/descriptinc/descript-audio-codec)) |
| Original EnCodec | **Unclear**: the README licenses only the code (MIT); the HF card names no licence |
| AudioCraft (incl. its EnCodec) | **CC BY-NC 4.0** (LICENSE_weights) |
| MERT-v1-95M | **CC BY-NC 4.0** |
| Music2Latent | **CC BY-NC 4.0** |

*(documented 2026-10-19)*

Runtimes: `tract-onnx` 0.23.9 (2026-10-15, MIT OR Apache-2.0, rust-version 1.91, pure Rust; builds for wasm32 once getrandom 0.4 `wasm_js` is enabled *(measured)*). `ort` 2.0.0-rc.14 (2026-10-12, rust-version 1.92, wraps the C++ ONNX Runtime).

**The three Task 6 distances as search objectives [changed: 20 targets, 5,000 evaluations, 8 workers, waveform cycled per restart; measured]:**

| Distance | Random start: T1 / T3 (median evaluations to T1, T3) | Heuristic start: T1 / T3 (median evaluations) | Waveform right (random / heuristic) |
| --- | --- | --- | --- |
| A: multi-resolution log-mel L1 | 11 / 15 (1,430, 1,318) | **14 / 18** (1,024, 645) | 19 / 19 |
| A with λ = 16 | 14 / 16 (1,688, 1,124) | **15 / 17** (660, 231) | 19 / 20 |
| B: A + envelope + centroid track | 8 / **20** (1,162, 635) | 13 / 19 (625, 203) | 17 / 17 |
| C: MFCC c0–c12 MSE | 6 / 10 (2,360, 1,905) | 9 / 14 (2,135, 649) | 18 / 18 |

- **T1 and T3 are per-distance proxies, not listening results.** Each is the median distance from the target to 16 renders with every continuous parameter jittered by ±1 % (T1) or ±3 % (T3) of its range.
- Comparing distances is imperfect. B's T3 is probably loose because a ±3 % jitter moves the beat phase, so small parameter changes look large under B's envelope term. I did not judge every result with one common metric (Gaps).

### Q4 — A listening test where one listener judges 30 pairs

Method:

- **Use 1–5 similarity ratings, as Task 6 plans: "how similar are A and B?", 1 = very different … 5 = indistinguishable.** That is the protocol Salimi et al. used (5-point, blind), and closest to the timbre-dissimilarity literature. Don't use MUSHRA for similarity pairs: it rates quality against a reference.
- **Use triplet agreement as an analysis.** Over every pair of pairs you rated differently, check which one the metric ranks as closer. That is what Tian et al. 2025 report.
- **Use ABX/same–different for "sounds the same".** Judge each CMA-ES result against its target, with hidden identical-pair controls.

Design for 30 pairs **[changed]**:

1. **Stimuli:** 30 target–candidate pairs spread across distance quantiles (6 bins × 5, binned by distance A), with the scorer's own note and performance, 2 s each.
2. **Level matching:** match loudness with `ebur128` to the same LUFS as the scorer (short-term loudness for clips under about 1 s). Add a 10 ms fade in and out and a 0.5 s gap between A and B.
3. **Randomisation:** seeded shuffle of the pairs, random A/B order, random IDs as filenames, replays allowed and logged.
4. **Anchors:** 2 identical pairs (should get 5) and 2 obviously different pairs (should get 1) per session.
5. **Reliability [changed]:** rate **all 30 pairs in two sessions**, reshuffled, ideally on different days (about 12 minutes each, headphones at a fixed volume). Session 1 vs session 2 gives a test–retest ρ over n = 30, the ceiling any metric can reach. The mean of the two sessions is the rating the metrics are compared against. The first report's 5 repeats can't support a correlation.
6. **Analysis per distance:**
   - Kendall τ-b (handles the ties of a 5-point scale) and Spearman ρ, between the negated distance and the mean rating.
   - 95 % bootstrap CIs over pairs.
   - Triplet agreement.
   - Compare distances with a paired bootstrap of the τ difference; Williams' test is an alternative for two dependent correlations.
7. **What n = 30 can resolve** *(computed with Fisher-z, Bonett–Wright and Fieller formulas)*:
   - Critical values: Spearman 0.361, Kendall 0.253.
   - Spearman 95 % CIs: ρ = 0.6 → [0.28, 0.80], 0.7 → [0.42, 0.86], 0.8 → [0.58, 0.91].
   - Kendall τ-b CIs: 0.5 → [0.29, 0.67], 0.6 → [0.41, 0.74].

   So set gates such as **ρ ≥ 0.7 with the lower CI bound ≥ 0.4, and within 0.15 τ-b of your own test–retest τ-b**. Expect 30 pairs to be unable to separate A from B unless they differ a lot; then prefer the simpler one.

Tools *(documented 2026-10-19)*:

- **Simplest:** our own script (Rust or Python) that writes randomised WAVs and a CSV and records keypresses, or a static local HTML page that downloads a JSON of answers (a dev tool, not product UI).
- [webMUSHRA](https://github.com/audiolabs/webMUSHRA): custom Fraunhofer licence (source must be shared with binaries, no patent licence). Run it locally only; don't vendor it.
- [Web Audio Evaluation Toolbox](https://github.com/BrechtDeMan/WebAudioEvaluationTool): GPL-3.0, last push 2021-07-26.
- [BeaqleJS](https://github.com/HSU-ANT/beaqlejs): GPL-3.0, last push 2019.
- [microsoft/P.808](https://github.com/microsoft/P.808): MIT, but built for crowdsourced speech ACR. Not a fit.

### Q5 — Parameter-space distance and "how far were your knobs"

research.md §2 checks out against the sources:

- [Hayes et al., ISMIR 2025](https://arxiv.org/abs/2506.07199): symmetries make inversion ill-posed.
- [Salimi 2026](https://arxiv.org/html/2608.27698v1): PPD agreed with listening in 5/7 scenarios. The full paper adds: 2–3 critical parameters per scenario, "selection of critical parameters also carries subjectivity", gradient-only.
- [Han et al. 2023](https://arxiv.org/abs/2301.02886).

*(documented)*

Additions:

- **The concrete formula for §7 layer 1:** sensitivity-weighted distance Δθᵀ M(θ) Δθ with M = JᵀJ, where J is the Jacobian of the features with respect to the parameters, from finite differences (the PNP idea, [arXiv 2311.14213](https://arxiv.org/abs/2311.14213)).
  - The same ± renders power layer 2's hints and `suggest`.
  - Cost for 10 parameters: 20 renders. At the measured 7.9 ms each, that is about 0.16 s single-threaded or about 0.04 s on 8 workers *(arithmetic on measured cost)*.
- **Raw parameter distance punishes correct answers:**
  - Solves that met the strict audio threshold still had a median mean-absolute parameter error of 0.16–0.18 of range (heuristic, λ = 16: 0.178) *(measured)*.
  - Resonance with the cutoff wide open, filter-env amount with a very short filter decay, and release beyond the render window barely change the audio.
  - **[new]** The heuristic confirms it: from the audio alone it recovered detune (median error 0.011, from beat rate), release (0.023), cutoff (0.125), attack (0.147) and sustain (0.184), but resonance and filter-env decay hardly better than chance (0.33/0.30) *(measured)*.
- **Categorical parameters:** score the waveform as its own component ("wrong waveform"). Search it by restart cycling: the waveform was right in 19–20/20 runs with the heuristic ordering the cycle. The heuristic's first guess was right on 16/20 targets *(measured)*. Port CatCMAwM later for templates with several categorical parameters.
- **Symmetries:** canonicalise before comparing. Fall back to layer 3 (nearest passing solution) across structures. No 2026 work changes this (search 2026-10-19).

### Q6 — Phase 0 metric spike, and whether the gate is realistic

**Probe setup** (unchanged from the first report except where marked):

- **Toy template, 10 params:** waveform saw/square/triangle; detune 0–50 cents; cutoff 50 Hz–16 kHz; resonance Q 0.5–10; filter-env amount 0–6 octaves; filter-env decay; amp A/D/S/R.
- **Signal path:** two polyBLEP oscillators into a TPT SVF lowpass. 1 voice, A3, note-off at 1.5 s, 2 s at 48 kHz.
- **CMA-ES:** own ask/tell. Waveform fixed per restart, cycling (in the heuristic's order when available). σ₀ = 0.3 for random restarts, 0.15 around the heuristic mean for the first restart per waveform. Restart on no 1 % improvement in 15 generations, σ·max(D) < 1e-4, or condition > 1e14. IPOP doubles λ after each 3-restart cycle.
- **[new]** Evaluation runs through a caller-side **work-queue** evaluator (`std::thread::scope`), with results written back by index.
- **[new] Heuristic start:**
  - Beat-rate autocorrelation of the 10 ms RMS gives detune.
  - A crest envelope gives A/D/S/R.
  - Harmonic roll-off, relative to the guessed waveform's ideal spectrum, gives cutoff (early vs steady state) and filter-env amount and decay.
  - Even/odd harmonic balance and H3/H1 guess the waveform.

**Cost and scaling [changed; measured; exclusive lock]:**

| Measure | Value | When / load |
| --- | --- | --- |
| Render, 1 voice, 2 s | 2.18–2.41 ms median (300 reps) | 05:35, load 8.5 |
| Features, hop n/4 / n/2 | 5.11 / 2.45 ms | same |
| One evaluation, 1 worker | 126.9 evaluations/s = 7.9 ms | 05:36, load 8.1 |
| Throughput at 2/4/6/8/10/12 workers (batches of 40) | 1.91 / 3.17 / 3.74 / **4.36** / 4.64 / 4.77× | 05:36, load 8.1 (an earlier run at load 4.5–6.8 gave 8 workers **5.42×**, 12 workers 6.08×) |
| 8 workers, batch of 10 / 16 / 24 | 3.59 / 4.11 / 4.05× (earlier run 4.74 / 5.10 / 5.27×) | same |
| Contiguous-chunk evaluator, 8 workers, batch 10 | 2.34× (only 5 threads busy) | 03:00, load 14 |
| Per solve, distance A, random, 6 targets, 3,000 evaluations | 29.10 s median (1 worker) → 7.33 s (8 workers) = **3.97×**; **identical checksum** 13201.546978885 | 03:09 / 03:51, load 8.5 / 10.6 |

**Solve results, 20 targets, 5,000-evaluation cap, 8 workers** (full table in Q3; per-target logs in `probe2/r*.txt`):

| Configuration | T1 (strict) | T3 (loose) | Per solve: median / max | Median time to T1 | Lock / load |
| --- | --- | --- | --- | --- | --- |
| A, random, λ 10 | 11/20 | 15/20 | 11.8 s / 30.1 s | 3.9 s | exclusive, 17.7–23 |
| A, heuristic, λ 10 | 14/20 | 18/20 | 11.5 s / 30.6 s | 6.9 s | shared, 16.5–25.7 |
| A, random, λ 16 | 14/20 | 16/20 | 5.0 s / 13.8 s | 3.9 s | shared, 12.5–17.5 |
| **A, heuristic, λ 16** | **15/20** | **17/20** | **2.6 s / 10.6 s** | **1.5 s** | **exclusive, 10.3–10.6** (shared run at load 2.6–10.6: 2.66 s / 17.3 s, same checksum) |
| A, heuristic + random side by side (λ 16) | **18/20** | 18/20 | about 2× the evaluations | — | union of the two runs |
| Any of the four A configurations | 20/20 | — | — | — | union |

**Recommended spike** (refines Task 6):

1. **Template:** keep the 10 parameters. Mark the waveform as categorical in the schema.
2. **Features:** multi-resolution log-mel (2048/1024/512, hop n/2 is enough and halves the cost, 64 mel bands, 30 Hz–16 kHz), a beat-aware 10 ms envelope, an energy-weighted centroid track, YIN f0. Normalise loudness first with `ebur128`, the same as the listening clips.
3. **Distances:**
   - A: multi-resolution log-mel L1 (the expected v1).
   - B: A + envelope + centroid, components scaled by their medians over random pairs; use a crest envelope to reduce beat sensitivity.
   - C: MFCC, kept as the control.
   - Optional D: A + spectral convergence.
4. **Listening:** 30 pairs × 2 sessions + anchors, as in Q4.
5. **CMA-ES:**
   - Own ask/tell with a work-queue evaluator, 8 workers, **λ = 16**.
   - σ₀ = 0.15 around the heuristic, 0.3 for random restarts; IPOP; relative-stall restart rule; waveform cycling.
   - **Run heuristic-start and random-start searches side by side** and keep the best.
   - Budget 5,000 evaluations per search, with a wall-clock cap the caller enforces (e.g. 30 s).
6. **"Sounds the same":** ABX on all 20 results of the winning distance.
7. **Report:** evaluations to threshold and wall time at 1 and 8 workers, with `uptime`.

**Is the Task 6 / G0 gate realistic?**

- **"Under a minute": yes.** Median 2.6 s and maximum 10.6 s per solve on 8 workers at a 5,000-evaluation cap, on a loaded laptop *(measured)*.
- **"Most targets sound the same": likely on a 10-parameter template.** Heuristic + λ 16 reached 15/20 strict and 17/20 loose; side by side, 18/20 strict. Whether "loose" = "same" for your ears is exactly what the ABX check must settle.
- Caveat: these numbers come from a toy synth with one note. Real templates with modulation, effects or several notes make each evaluation dearer and the landscape harder.

## Recommendation for Phase 0

1. **`enth-analysis` (L3):** own features on `realfft`/`rustfft`, plus `ebur128`. All three pass `cargo check` for wasm32 *(measured)*. Set the cross-platform tolerance on distances at about 1e-5 relative (measured worst case 2.7e-6, native vs wasm). Add the MIT text for `realfft` to third-party notices.
2. **`enth-solver` (L5):**
   - Own ask–tell CMA-ES: Hansen 2016 defaults, IPOP, clamp + penalty bounds, categorical parameters by restart cycling now and CatCMAwM later.
   - Seeded with our own RNG; no clocks, threads or `std::time`.
   - The caller-side evaluator must be a **work queue** that returns results in input order (determinism across worker counts measured).
   - Default λ = max(4 + ⌊3 ln n⌋, number of workers rounded up to a multiple of it), e.g. 16 for 8 workers.
   - Run a heuristic-start search and a random-start search side by side.
   - Budget by evaluations plus a caller-enforced time cap.
3. **Metric v1 candidate:** distance A on loudness-normalised audio, components reported separately for coaching (§6). The listening data decides between A and B.
4. **Embeddings:** not in Phase 0. At about 0.6 s per CLAP call via tract on this CPU, use them only to re-rank a few final candidates (8 candidates ≈ 5 s on one core). Use Apache/MIT/CC0 weights only (LAION-CLAP, DAC). If ever needed, check `ort` and quantised models.
5. **Workspace (for sub-researcher 05 and the summary):** `tract-onnx` 0.23.9 pulls `kstring` 2.0.5, which needs rustc 1.96. Use edition 2024 / resolver 3 with `rust-version` set so Cargo picks compatible versions. Any wasm build of a crate that pulls `rand` needs the matching getrandom backend feature (`js` for 0.2, `wasm_js` for 0.4).

## Plan check — what the evidence says should change

- **design.md §5 (determinism):** add a measured number. Native x86-64 and wasm32 differ by up to 2.7e-6 relative on the distances, because of libm differences (glibc vs Rust's libm) and SIMD vs scalar paths. Bit-exact holds only within the same build and platform, as §5 already says; a tolerance of about 1e-5 on distance values is safe. **Change: add the tolerance.**
- **design.md §6 (`sound` scorer):** keep the structure. Add:
  - Loudness via BS.1770/EBU R128 (`ebur128`), with short-term loudness under about 1 s.
  - Categorical mismatches (waveform) reported as a separate component.
  - Envelope features are beat-aware (crest envelope), because detuned oscillators make raw RMS beat.
- **design.md §7 (budget check): change it.**
  - (a) Budget per **evaluation** (render + analysis). Analysis costs as much as a 1-voice render or more: 2.5–5.1 ms vs 2.2–2.5 ms.
  - (b) "1,000 renders, ~3 s on 8 cores" assumed 8× scaling. Measured 8-worker scaling on this 6-core laptop is **4.4–5.4×**, about 4× per solve.
  - (c) Plan **5,000 evaluations per search**: measured median 2.6 s, maximum 10.6 s per solve on 8 workers with the heuristic start.
  - (d) Layer 1's sensitivity weighting = Δθᵀ JᵀJ Δθ, with J from the same finite-difference renders as layer 2.
- **design.md §8 (solver pipeline):** add:
  - Categorical parameters by restart cycling (later CatCMAwM).
  - **A portfolio of a heuristic start and random restarts run side by side.** Measured: 18/20 vs 15/20 for the heuristic alone and 14/20 random.
  - λ sized to the worker count.

  The heuristic start is confirmed useful: median evaluations to the loose threshold fell 2–5×.
- **architecture.md L5:** no change to "solver spawns no threads, caller-supplied evaluator". Add a note that the evaluator must be order-stable (a work queue writing results by index); that is what makes results identical across worker counts.
- **phase-0.md Task 6: change it.**
  - Listening: 2 sessions × 30 pairs + anchors, Kendall τ-b + bootstrap CI, ABX for "sounds the same".
  - Search: λ = 16 with 8 workers, heuristic + random restarts side by side, 5,000-evaluation budget.
  - Reporting: evaluations to threshold and wall time at 1 and 8 workers with `uptime`.
  - Features at hop n/2.
- **roadmap.md Gate G0: make it numeric.** For example:
  - "Best distance: Spearman ρ ≥ 0.7 against your mean rating of 30 pairs, lower 95 % CI ≥ 0.4, and within 0.15 τ-b of your session-to-session τ-b."
  - "With 8 workers and ≤ 60 s wall per target, ≥ 14 of 20 targets judged *same* in ABX."

  The measured toy numbers suggest this is reachable.
- Rust and native UI are unaffected.

## Gaps — what is still open, and the check that would settle it

| Gap | Check |
| --- | --- |
| **No truly quiet machine.** During exclusive holds, processes outside the lock kept load at 8–10 (Cinnamon 90–100 %, another researcher's Python). All times are upper bounds | Re-run `probe2 bench 300`, `probe2 scale 1200` and `probe2 solve A heur 8 5000 20 0.15 16` (in `~/code/enth-scratch/2026-10-19-phase-0-spikes/04/probe2`) with nothing else running and the `performance` governor if the owner allows; compare to the tables above. The checksum must stay 42934.673045141 |
| **T1/T3 proxies not validated by ears** | Task 6 ABX on solved and unsolved results |
| **No common-judge comparison of distances** (each judged by its own thresholds) | Re-score every final result with all three distances and the listening ratings in Task 6 |
| **CLAP:** ONNX Runtime, tract `multithread-mm`, the quantised model and mel preprocessing not measured | `clap-bench` with `tract-onnx/multithread-mm`; an `ort` variant (downloads the ONNX Runtime binary, so ask first); `audio_model_quantized.onnx` (34 MB) |
| **16-voice / multi-note evaluation cost** not measured (engine-dependent, sub-researcher 03's area) | Time render + features on the real `spike-engine` once Task 5 exists |
| **Heuristic quality** for resonance and filter-env decay is near chance; the real enth heuristics need their own design | Task 6: per-parameter heuristic error as in `probe2 heurcheck` |
| **Tian 2025 numbers** exist only as figures | Read the plots, or rerun with [timbremetrics](https://github.com/tiianhk/timbremetrics) (repo has no detected licence, so reference only) |
| **EnCodec (original) weights licence** unclear | Ask upstream, or avoid it (DAC is MIT) |
| **fcmaes-core long-term maintenance** unknown (3 months old) | Re-check activity before any adoption |
| **`realfft` licence text** missing upstream | Follow [issue #45](https://github.com/HEnquist/realfft/issues/45); meanwhile record the MIT declaration in our notices and in `cargo deny` |
