# 03: Inference and integration in Rust (R3 Sound → patch)

- **Date:** 2026-10-21. Sub-researcher 03, axis 3 of [brief.md](brief.md).
- **Method:** crates.io and GitHub APIs (User-Agent `enth-research/0.1`), project docs, the ONNX backend scoreboard, the repo at `42e6d44`, and a small benchmark I built and ran in `~/code/enth-scratch/2026-10-21-r3/03/` (`bench/`, `notes.md`, `logs/`).
- **Labels:** **M** = measured here, **D** = documented by the source linked, **A** = anecdotal.
- **Bench conditions:** a 6-core/12-thread x86_64 laptop (AVX2 and FMA, no AVX-512), load average 15–20 from parallel build lanes, `CARGO_BUILD_JOBS=2`. Timings are noisy; the ratios between runtimes and the bit-exactness results are the solid part.

## Summary

1. **Inference is cheap at our scale.** A plausible audio-to-candidates CNN (0.8 M parameters, about 0.9 GFLOP) runs in 18–160 ms on one machine, whatever the runtime (**M**). That is under 0.3 % of a 60 s budget. So **runtime speed is not the deciding factor.** Size, build cost, determinism, wasm32 and the layer rules are.
2. **tract is not bit-exact across platforms.** On the same model, tract on native x86_64 and tract on wasm32 agreed on only 37–48 of 512 outputs, bit for bit (**M**). Our own naive scalar kernels (plain `+` and `×`, fixed order) gave **512 of 512 identical** on x86_64, on x86_64 built with `target-cpu=native`, and on wasm32 (**M**). rten's outputs even change with its thread count (**M**).
3. **tract is heavy.** Its inference path adds about **15.5 MB** to a binary (17.0 MB stripped against a 0.41 MB baseline; 15.9 MB even with fat LTO), against a 16 MB release plugin today. A clean release build costs about 17 CPU-minutes and 1.4 GB of RAM, it needs `cc` (assembly kernels), and `tract-onnx` **does not build for `wasm32-unknown-unknown`**: getrandom fails through `tract-onnx-opl` (**M**). `tract-nnef` alone does build for wasm32 (**M**).
4. **Recommendation:** ship inference as **our own small scalar kernel set inside `enth-solver`**, fed a versioned weights pack. Keep ONNX as the export format and the parity oracle: ONNX Runtime in `ml/`, tract or rten in a dev-only tool. Fallback if the model outgrows the kernel set: **`tract-nnef` only** (never `tract-onnx`), behind a feature. Reject ort (C++ native library) and candle for the product.
5. **Integration mostly exists.** `enth-solver` has an ask/tell `Search` with `cancel()`, a per-batch callback and `MetricTarget::from_audio`, and `enth match --target t.wav` already runs R3a's core. Missing: a K-candidate orchestrator, the plugin job and pool, decoders beyond WAV, the file drop (egui-baseview discards baseview's drop events) and the UI.

## 1. Runtimes compared (checked 2026-10-21)

| | tract | rten | ort (ONNX Runtime) | candle | burn + burn-onnx | Own scalar kernels |
| --- | --- | --- | --- | --- | --- | --- |
| Version, date | 0.23.9, 2026-10-15 ([crates.io](https://crates.io/crates/tract-onnx)) | 0.27.0, 2026-10-02 ([crates.io](https://crates.io/crates/rten)) | 2.0.0-rc.15, 2026-10-19 (still a release candidate); ONNX Runtime 1.31.0 | 0.11.0, 2026-06-26 | burn 0.22.1 (2026-10-20), burn-onnx 0.22.0 (2026-10-06) | n/a |
| Licence (file checked) | MIT/Apache-2.0 (LICENSE-MIT and LICENSE-APACHE in the crates) | MIT/Apache-2.0 (`LICENSE-*.txt` in the repo, **missing from the .crate**) | crate MIT/Apache-2.0; ONNX Runtime MIT ([LICENSE](https://github.com/microsoft/onnxruntime/blob/main/LICENSE)) | MIT/Apache-2.0 (from crates.io metadata; file not checked) | MIT/Apache-2.0 (from the README; file not checked) | ours (MIT) |
| Pure Rust | Rust, plus assembly kernels built by `cc` (MSVC: `ml64.exe`) (**M**, build.rs) | yes, no `cc` (**M**, dependency list) | no: a C++ library, downloaded prebuilt from `cdn.pyke.io` at build time (**D**) | yes; heavy dependencies (`gemm`, `rayon`, `rand`, `zip`, `tokenizers`) (**M**, dependency list) | yes; build-time code generation from ONNX (**D**) | yes, no dependencies |
| wasm32-unknown-unknown | `tract-onnx` **fails** (getrandom 0.4.3); `tract-nnef` OK (**M**) | builds; pulls `rayon` and `wasm-bindgen` (**M**) | no prebuilt build ([dist.tsv](https://crates.io/crates/ort-sys)) | **D**: CPU and wasm ([ort backends page](https://ort.pyke.io/backends)) | **D**: wasm and `no_std` ([README](https://github.com/tracel-ai/burn-onnx)) | yes (**M**: wasm32-wasip1 run) |
| Added binary size | ~15.5 MB (nnef only), ~23.5 MB (onnx) (**M**) | ~4.3 MB (**M**) | static ONNX Runtime; archives are 11.4 MB (linux-x64) to 36.5 MB (osx-arm64) compressed ([v1.31.0 assets](https://github.com/microsoft/onnxruntime/releases/tag/v1.31.0)) | not measured | not measured | ~0.1 MB (**M**) |
| Clean release build | 1,040 CPU-s, 11:49 wall, 1.44 GB RSS (**M**) | 112 CPU-s, 1:02 wall, 0.65 GB (**M**; some dependencies were cached) | download only, plus libc++ on Linux (**D**) | not measured | not measured | seconds |
| Cross-platform bit-exact | **no** (**M**) | no; even the thread count changes outputs (**M**) | no (vendor kernels) | not measured | not measured | **yes** (**M**) |
| Threads | single-threaded by default (**M**) | its own rayon pool sized to the physical cores; `RunOptions::with_thread_pool`, `RTEN_NUM_THREADS` (**D**, source) | its own intra-op pool | global rayon pool | backend-dependent | ours: none |
| ONNX coverage | 57.11 %, 146 of 205 ops on the [ONNX backend scoreboard](https://onnx.ai/backend-scoreboard/) (run 2026-10-20). The README still says "about 85 %" | "most operators" ([issue #14](https://github.com/robertknight/rten/issues/14)); the scoreboard shows 0.00 % (likely a harness failure; no usable figure) | ONNX Runtime 1.31.0: 71.91 %, 166 of 205 (same scoreboard) | "limited" (**D**, [ort backends page](https://ort.pyke.io/backends)) | 1,185 of 1,765 node tests (**D**, README) | only what we write |
| Health | 3.1 k stars, 101 contributors, pushed 2026-10-20; every release published by one account; Sonos production use (**D**) | 347 stars, 10 contributors, one maintainer | 2.6 k stars, 78 contributors | 21 k stars, 284 contributors | 16 k stars, 350 contributors; pre-1.0, API churn | us |

Notes:

- **ort:** prebuilt x86-64 binaries need AVX2 (x86-64-v3), the Linux builds need libc++ ([prebuilt binaries](https://ort.pyke.io/misc/prebuilt-binaries), **D**), and `ort-sys` rc.15's `dist.tsv` has **no `x86_64-apple-darwin`** entry (**M**). Fine in `ml/` (Python `onnxruntime`) as the reference.
- **tract:** the `tract-linalg` 0.23.9 crate ships two 2.1 MB core dumps; 0.23 renamed `to_array_view` (**M**).
- **rten:** 0.27 rejected a valid ONNX file whose repeated `ints` attributes were *packed* (prost's default). The unpacked form, which is how PyTorch writes them, loaded fine; tract took both (**M**).

## 2. Benchmark (measured)

**Model.** Input: a `[1,1,128,256]` log-mel. Four 3×3 conv blocks with ReLU (2×2 max-pool after the first three, global average pool after the last), then Gemm → ReLU → Gemm to 512 outputs (8 candidates × 64 parameters). Three tiers, s/m/l: 0.26 M, 0.78 M and 2.6 M parameters; 1.0, 3.1 and 10.4 MB in f32; about 0.25, 0.9 and 3.7 GFLOP. Weights are seeded, so every runner gets the same model. The ONNX files were written from Rust (prost), since no Python ML stack was installed.

| Median time per inference, ms (load ~16) | s | m | l |
| --- | --- | --- | --- |
| tract 0.23.9, one thread | 13 | 62 | 167 |
| rten 0.27.0, one thread | 8 | 29 | 95 |
| rten 0.27.0, default pool (6 threads) | 6 | 18 | 38 |
| Own naive scalar code (one thread) | 69 | 159 | 985 (min 708) |
| tract on wasm32-wasip1 under Node 22 (no simd128) | 149 | 606 | – |
| Scalar code on wasm32-wasip1 (no simd128) | 1,122 | 4,002 | – |

Load and optimise times: tract 8–36 ms; rten 2–9 ms.

| Bit-for-bit agreement (512 outputs) | Result |
| --- | --- |
| Each runtime, run twice on one machine | identical (tract, rten, scalar) |
| tract native vs tract wasm32 | 48/512 (s), 37/512 (m); max abs diff 1.4e-6 |
| Scalar native vs scalar wasm32 | **512/512** (s, m) |
| Scalar default build vs `-C target-cpu=native` | **512/512** (Rust never fuses `a*b+c` without `mul_add`) |
| rten, 1 thread vs 6 threads | 512/512 (s, m); **191/512 (l)** |
| tract vs rten vs scalar | 14–44/512 bit-equal; max abs diff ≤ 3.3e-6 |
| tract ONNX path vs ONNX → NNEF → `tract-nnef` | 512/512 (s, m, l) |

The scalar code is deliberately naive: no blocking or im2col, but the loop over outputs auto-vectorises, which does not change the per-element order. A blocked version with a fixed accumulation order would likely be 2–3× faster and stay bit-exact. That is an estimate, not measured.

## 3. Fit with enth's rules

- **Layers.** design.md §8 puts inference in `enth-solver` (L5, a core crate: no threads, no I/O, wasm32, "No crates that wrap C/C++ in L0–L5").
  - rten spawns a rayon pool, so it can only live in a shell-side crate.
  - tract needs `cc` for assembly, and AGENTS.md asks for the owner's OK before adding a dependency that pulls in a C toolchain. Its default `tract-onnx` breaks `cargo xtask wasm-check`.
  - ort is excluded outright.
  - Own kernels fit L5 with no exceptions: weights come in as `&[u8]`, and transcendentals go through the pinned `libm` (GELU's `erff`, `tanhf`, `expf` for softmax), as the DSP rules already require.
- **Licences.** All candidates are MIT/Apache-2.0. The weights are ours, but axis 2 must confirm that no licence of an augmentation dataset (noise, impulse responses) restricts trained models.
- **Precedent.** The repo already owns small numeric kernels for determinism: the scalar FFT path, and its own CMA-ES (design.md §8).

## 4. Must inference be deterministic?

What the product needs:

| Level | Needed? | How |
| --- | --- | --- |
| Same binary, same input, same machine → same proposals | **Yes** (reproducible bugs, tests, "run it again") | Any runtime run single-threaded. rten must be pinned to a fixed thread count (**M**: its outputs follow the pool size, which defaults to the number of physical cores, so they differ between a 4-core and an 8-core machine) |
| Bit-exact across platforms (Linux, Windows, macOS ARM/Intel, wasm) | Strongly preferred, not strictly required | Only own scalar kernels give it (**M**). Without it, a 1e-6 drift in a start can send CMA-ES elsewhere, so the same sound gives different results on two OSes, and goldens or GR3 runs are not portable. Quantising the outputs does not fix this: with roughly 3e-6 drift, a 1/1024 grid and 512 outputs, about three quarters of runs would still have at least one value cross a rounding boundary (estimate) |
| Search deterministic given its starts | Yes; this exists | `Search` with a seed and order-stable evaluation (`crates/enth-solver/src/search.rs`) |

Whatever runtime ships:

- record the K proposals (as patches) in the job result and in the CLI JSON, so a run can be replayed from its starts;
- hash the input features in the result.

A file decoded from MP3 or AAC may itself differ in the last bits across CPUs if the decoder takes SIMD paths. Symphonia's `opt-simd-*` features are on by default and can be turned off; that this makes decoding scalar and identical everywhere is unverified (see Gaps).

## 5. Model size and latency budgets; bundled or a pack

**Budgets (recommended; for axes 1 and 2):**

- **Parameters:** at most about 4 M, stored as f16 in the pack (8 MB at most). f16 → f32 conversion is exact, so it does not affect determinism. int8 only later, with integer kernels written to be bit-exact.
- **Compute:** at most about 2 GFLOP per inference. That keeps naive scalar inference near 1 s natively (**M**: 3.7 GFLOP took 0.7–1.0 s). Run the audio encoder **once**; a generative head (flow matching, diffusion, or K slots) runs per candidate and per sampling step on the pooled embedding only, which costs megaflops, not gigaflops.
- **Ops:** restrict the architecture to a small set the kernels implement: Conv1d/Conv2d (stride, padding, dilation, groups), Linear, BatchNorm folded at export, LayerNorm, ReLU/GELU/SiLU/tanh/sigmoid, max/average/global pooling, softmax, residual add and concat, and multi-head attention (Linear + matmul + softmax). Axes 1 and 2 should confirm their model fits this set.
- **Front end:** decode, trim and features at most 0.2 s; inference at most 1 s; the remaining ~55 s of the 60 s target goes to search.

**Distribution:**

- The plugin is ~16 MB per binary and the Linux zip is 11 MB ([distribution.md](../../../docs/distribution.md)). `include_bytes!` would copy the weights into **both** the CLAP and the VST3 binary, and float weights compress poorly (ratio not measured).
- **Recommendation:** a separate, versioned **model pack** (`enth-model-<id>.bin`), with a fixture test.
  - Header: format version, model id, template id and version, the ordered parameter addresses and their normalisation, feature config, engine version, sha256.
  - Shipped in the friend zip; the plugin looks next to its binary, then in `<Documents>/enth/Models/`.
- **No in-plugin downloading:** it would add rustls (ring or aws-lc: C/assembly) and a network and privacy surface. Updates are plain file downloads, like content packs.
- **No pack, or a version mismatch:** "Create from sound" runs R3a (heuristic + CMA-ES) and says so.
- Embed the weights only if they stay at 2 MB or less.

## 6. Threading on the plugin worker, beside CMA-ES

**Today.** The plugin has one lazily spawned job thread, `enth-game`, with queues behind short `parking_lot` locks (never the session lock), and posts results with `request_repaint` ([plugin.md](../../../docs/plugin.md)). Hints use `enth_solver::Serial`; a parallel hint evaluator is in progress (lane HINT). There is no rayon in the plugin. Phase 0: one evaluation costs about 7.5 ms on one thread on the toy synth, and 8 workers give 4.3–4.7× on 6 cores ([brief.md](brief.md)).

**Proposed:**

1. **An `enth-create` orchestrator thread,** spawned at the first job and joined on drop, like `enth-game`. It decodes, computes features, runs inference on one thread (milliseconds) and drives the searches. It never runs on the audio thread and never holds the session lock while computing; results are posted the way the game worker posts them.
2. **A per-job `rayon::ThreadPool`** (never the global one), named `enth-search-N` and dropped when the job ends, so no threads exist between jobs.
   - Default workers: `min(available_parallelism()/2, 8)`, at normal priority, never real-time.
   - Isolated benchmarks mislead under DAW load ([DAFx-2026](https://dafx.de/paper-archive/details/rBxXewYH3YxRTiGXqSabwQ), **D**). So **gate** the default with `enth-test-host process-stats --gate g1` while a create job runs, as lane SKR did for the A/B.
3. **K searches on one pool.** Each candidate is its own `Search` (it already takes one start). Each round merges every live search's `ask()` batch into one pool batch and splits the distances back to each `tell()`.
   - The outcome does not depend on the worker count.
   - 8 × λ 16 = 128 jobs per round keep the pool busy.
4. **Successive halving across candidates** (deterministic): all K get about 10 % of the evaluations, the best half the next 30 %, the best two the rest.
   - At the Phase 0 cost, 8 workers give about 600 evaluations/s, so about 36,000 in 60 s (K = 8 × about 4,500).
   - R1's engine with effects will cost more per evaluation; not measured.
5. **Cancel:** an `AtomicBool` checked per job in the pool and after each round (`Search::cancel()`). Latency is about one evaluation (~10 ms, estimate). The wall-clock cap lives in the shell's `search_with` callback, since the solver reads no clock.

## 7. Audio input path

**Decoders** (licence files checked; dates from crates.io):

| Format | Option | Licence | Status |
| --- | --- | --- | --- |
| WAV | `hound` 3.5.1 (already in the workspace; the CLI reads 8–32-bit int and f32) | Apache-2.0 | last release 2023-09-25 |
| FLAC | `claxon` 0.4.3 | Apache-2.0 | last release 2020-08-09 |
| Ogg Vorbis | `lewton` 0.10.2 | MIT/Apache-2.0 | last release 2021-01-20 |
| MP3 | `nanomp3` 0.2.0: a pure-Rust, safe port of minimp3 (which is CC0) | MIT/Apache-2.0 (per its repo) | 2026-09-30; one maintainer, 23 stars (**D**) |
| AIFF | a small in-house reader (the chunk format is simple), or symphonia | – | – |
| Everything above, plus M4A/AAC-LC and ALAC | `symphonia` 0.6.1 (2026-08-13; 51 contributors; "100% safe Rust" (**D**)) | **MPL-2.0** ([LICENSE](https://github.com/pdeljanov/Symphonia/blob/master/LICENSE)), not in `deny.toml`: **needs the owner's OK** | active |

- **Recommendation:** ask the owner for a scoped `deny.toml` exception allowing MPL-2.0 for the `symphonia*` crates only (file-level copyleft: unmodified crates need only a notice and a pointer to their source). Use `default-features = false` and only the formats needed. **If refused:** hound + claxon + lewton + nanomp3 + an in-house AIFF reader, and no M4A.
- **No resampler for the search.** Render and analysis run at the file's own rate (8–192 kHz), as `enth match` already does.
  - For the model, compute the log-mel with `enth-analysis` (its scalar FFT path is identical everywhere) **at the native rate**: bands in Hz, window and hop in seconds. A low-rate file then just has empty top bands, which axis 2's resampling augmentation covers.
  - Training uses the **same Rust code** through `enth-py`, so features cannot drift between training and inference.
  - If a resampler is ever needed, `rubato` 5.0.1 (MIT/Apache-2.0) picks SIMD at runtime, so it is not bit-exact across CPUs (**D**, [README](https://github.com/HEnquist/rubato)).
- **Conditioning:**
  - mono = mean of channels;
  - strip leading silence (about 60 dB below the peak, a few ms of pre-roll);
  - cap the length at 4–6 s (evaluation cost grows with it);
  - note-off and pitch from the existing `infer_note_off` and YIN;
  - warn when several onsets suggest a phrase;
  - **never reject short sounds**.
- **Inputs:** the CLI already takes `--target t.wav`; add the other formats there first.
- **File drop:** **baseview 0.4.1 (vendored) delivers file drops on X11 (XDND), Windows and macOS, but egui-baseview 0.9.0 ignores `DragEntered`, `DragMoved` and `DragDropped`**: they fall into `_ => do_repaint = false`, and it never returns `EventStatus::AcceptDrop` (**M**, `vendor/egui-baseview/src/window.rs`). It needs a small lane-FK patch that maps them to egui 0.37's `RawInput::hovered_files` and `dropped_files` and returns `AcceptDrop(Copy)`.
- **File dialog:** a "Choose file…" button would need `rfd` (on Linux a portal or GTK backend, which needs the owner's OK). Ship the drop first.

## 8. UI: candidates as branches, progress, cancel

- **A `create` view in `enth-ui`,** behind a feature like `recreate`:
  - a drop zone showing the file's name, length and detected pitch;
  - **K candidate tiles**, each with its distance and state (proposed → searching → done or pruned);
  - one job progress bar (renders against the budget, time left) and Cancel;
  - per tile: ▶ audition and A/B against the target (reusing Recreate's `Mailbox` and `Player`, level-matched), and "Use this".
- "Use this" is **one undo step** (`ops_between`, as preset loads do).
- **Branches**, Synplant-style: the proposal and the refined patch are both kept and selectable, so you can hear what the model gave and what the search added. That is also GR3 evidence.
- **Repaint** only on a worker post (once per round) while the view is visible.
- **The session side:** a pure `create` module in `enth-session`, like `game`: `Run` and `Cancel` go out to the shell, results come back.

## 9. Code structure (sketch)

```
crates/enth-solver/src/propose/   # L5, pure: no I/O, threads or clocks; wasm32
  pack.rs      ModelPack::parse(&[u8]) -> Result<ModelPack, PackError>   (versioned header, sha256, fixture tests)
  ops.rs       conv1d/conv2d, linear, layer_norm, gelu/silu/softmax via libm, pools: fixed summation order, no mul_add
  net.rs       Net::run(&self, input: &Tensor) -> Tensor                 (layers listed in the pack)
  features.rs  model input from enth-analysis log-mel at the native rate (scalar FFT path)
  decode.rs    outputs -> Vec<Patch>: normalised -> plain through ParamSpec, onto the template's FreeSet
crates/enth-solver/src/create.rs  # CreateJob: K Searches, a merged ask/tell, successive halving, Progress; pure
crates/enth-session/src/create.rs # the view's state machine, shell requests, "use candidate" as one undo step
crates/enth-plugin/src/create.rs  # enth-create thread, per-job rayon pool, decode (I/O), pack lookup, posts
crates/enth-cli: `enth create --target x.wav --candidates 8 --seconds 60 [--model pack.bin]` (schema enth.create/1)
tools/model-check (exempt)        # parity: our kernels vs tract-nnef/rten on the exported ONNX (tolerance 1e-5),
                                  # plus exact hashes of our kernels' outputs (identical on every platform)
ml/                               # PyTorch -> ONNX (interchange) -> our pack; onnxruntime parity in Python
```

## 10. Risks

| Risk | Mitigation |
| --- | --- |
| Bugs in our own kernels | Parity tests against ONNX Runtime and tract on exported fixtures; a golden output hash per pack on every CI platform |
| The model outgrows the op set | The budget in §5; the `tract-nnef` fallback |
| CPU contention in a loaded DAW | The process-stats gate; fewer default workers; cancel |
| R1's engine costs more than 7.5 ms per evaluation | Measure before fixing K |
| A pack used with the wrong template | The pack header carries the parameter list and versions; refuse on mismatch |
| rten's crate omits its licence files | Notices point to the repository's licence files |

## Gaps

- No Python ML stack was installed, so the bench model was written directly as ONNX and no PyTorch-exported model was tried (packed vs unpacked attributes matter for rten).
- Not measured: candle, burn-onnx, ort (no native download made). ONNX Runtime's unpacked library size is not measured; the archives are 11.4–82.9 MB compressed.
- Bit-exactness was checked on x86_64 and wasm32 only. aarch64 (macOS) and MSVC are untested; for scalar code with no FMA, the IEEE argument says they should match. Run the bench's scalar path on the macOS CI leg.
- Bench timings were taken on a loaded machine (load average 15–20). Repeat them quiet before quoting absolute numbers.
- No measurement yet of a create job's effect on `process()` p99 in the plugin, or of per-evaluation cost on R1's engine with effects.
- Unknown: whether REAPER (Linux) drags media items into a plugin window as files; whether symphonia without `opt-simd` decodes identically across CPUs.
- rten's scoreboard row is 0.00 % (likely a harness failure); I did not verify its op coverage beyond the docs.

## Recommendations

1. **Decide the inference path (owner).** Recommended: our own scalar kernels in `enth-solver::propose`, weights in a versioned pack, ONNX only as the export and parity format. Fallback: `tract-nnef` (never `tract-onnx`) behind an `infer-tract` feature, with the model converted to NNEF in `ml/`. That needs the owner's OK for `cc` (assembly) and accepts about 15 MB more per plugin binary and no bit-exactness. Update design.md §8 ("ONNX + tract") either way.
2. **Constrain the model for axes 1 and 2:** at most ~4 M parameters (f16 in the pack), at most ~2 GFLOP, the op list in §5, the encoder run once, a light generative head per candidate.
3. **Make the determinism contract explicit in design.md §5/§8:** inference bit-exact across platforms with our kernels (goldens: exact hashes); the search deterministic given its starts; the K proposals recorded in every result.
4. **Features in Rust, shared with training:** log-mel from `enth-analysis` at the native rate, exposed to `ml/` through `enth-py`. No resampler in the product path.
5. **Decoders:** ask the owner for a scoped MPL-2.0 exception for `symphonia`. Otherwise hound + claxon + lewton + nanomp3 + an in-house AIFF reader.
6. **R3a first** (shippable alone): `enth create` over the existing `Search`, a `CreateJob` with K starts, the plugin job and pool, the egui-baseview drop patch (lane FK) and the `create` view. R3b then adds only `propose` and the pack.
7. **Plugin threading:** as §6. **Gate:** `process-stats --gate g1` with a create job running.
8. **Model distribution:** a separate pack file beside the plugin, no network code, the R3a fallback when it is missing.
9. **Local checks before deciding:**
   - rerun the bench on a quiet machine;
   - run the scalar path on the macOS and Windows CI legs and compare hashes;
   - measure per-evaluation cost on R1's engine;
   - run one PyTorch-exported model through tract-nnef, rten and our kernels.

## Sources

Besides the links inline:

- The crates.io API (2026-10-21) for every crate named here.
- GitHub repositories (API, 2026-10-21): [sonos/tract](https://github.com/sonos/tract) ([releases](https://github.com/sonos/tract/releases)), [robertknight/rten](https://github.com/robertknight/rten), [pykeio/ort](https://github.com/pykeio/ort), [huggingface/candle](https://github.com/huggingface/candle), [tracel-ai/burn](https://github.com/tracel-ai/burn), [robbie01/nanomp3](https://github.com/robbie01/nanomp3).
- The repo at `42e6d44`: `crates/enth-solver/src/search.rs`, `crates/enth-cli/src/{wav.rs,cmd/matching.rs}`, `vendor/{baseview/src/event.rs,egui-baseview/src/window.rs}`.
- Scratch: `~/code/enth-scratch/2026-10-21-r3/03/` (`bench/src/main.rs`, `logs/`, `notes.md`).
