# Summary: R3 Sound → patch research (2026-10-21)

Four reports, written in parallel: [01 prior art and models](01-prior-art-models.md), [02 data, augmentation and training](02-data-augmentation-training.md), [03 inference and integration in Rust](03-inference-integration.md), [04 evaluation, risks and the lane plan](04-evaluation-risks-plan.md). The coordinator wrote this summary. The decisions below are provisional until the owner steers; work proceeds on the defaults meanwhile.

## The answer

- **Nobody turns arbitrary real sounds into patches well,** and nobody has tested degraded uploads.
  - All published work trains on the synth's own renders, and out-of-domain error roughly doubles (Hayes 2025).
  - Only one small study compares model + search with search alone: a warm start matched a GA in 40 % of the time (Shier 2021, 7 parameters).
  - R3's robustness goal and GR3's equal-time test will be new evidence (01).
- **Today's search copies the damage** (04 §2, *measured*, one target).
  - `enth match` on degraded copies, scored against the clean target: clean 99 %, MP3 92 %, noise 10 %, reverb 25 %, clipping 64 %. CMA-ES turns knobs to reproduce noise, reverb and clipping.
  - Trimming the noisy tail alone raised 10 % to 56 %.
  - So **R3a needs a robust intake and a degradation-aware objective before it ships,** and GR3 must judge degraded uploads against the clean original.
- **R3a ships first, with no ML.** "Create from sound" in the CLI and the plugin:
  - an intake (trim, noise floor, bandwidth, clipping, inferred note and hold);
  - a robust objective under a new metric id (`sound/1` unchanged);
  - a portfolio of starts (the heuristic, templates, and k-NN retrieval over a pre-rendered patch bank);
  - K candidate branches refined by successive halving, on a cancellable pool of its own (01, 03, 04).
- **R3b, the first model, is narrow.**
  - A small CNN on `enth-analysis` log-mel features (about 1–4 M parameters) with **K = 8 winner-takes-all hypotheses** and a confidence head.
  - Categorical heads matter most (waveform, mode, LFO shape, enable flags, routes as a fixed source × destination grid), because that is where CMA-ES fails: level 5 solves 0 of 4.
  - The loss is masked by `observable()`, in JND units, with canonical slots.
  - Every hypothesis is rendered, ranked and refined; the heuristic start is always kept.
  - It must beat k-NN retrieval and search alone. Flow matching, discrete diffusion and render-reward fine-tuning come later, if needed (01 §7–§8).
- **Inference: our own small scalar kernels in `enth-solver`,** with weights in a versioned model file next to the plugin; ONNX stays the export and parity format (03).
  - Any runtime is fast enough (18–160 ms for a 0.8 M-parameter CNN against a 60 s budget).
  - tract adds about 15.5 MB to a 16 MB plugin and needs `cc`; `tract-onnx` doesn't build for wasm32; tract isn't bit-exact across platforms (37–48 of 512 outputs equal, native against wasm32).
  - Our naive kernels were 512 of 512 identical on x86_64, `target-cpu=native` and wasm32, for about 0.1 MB.
- **Data comes from our engine, through a new sampler `ds1`,** because generator v1 is unfit for training as it stands: level 1 has about 12 sounds, every phrase starts on C3, values are pulled towards the start (02 §2).
  - A dataset is a manifest (sampler version, seeds, augmentation config), with a cache of about 250 k clips and a rolling refresh. The CPU is the bottleneck: 10–28 ms per dry render, 130–175 ms with the reverb.
  - The augmentation chain (codecs through ffmpeg in `ml/`, rooms, device EQ, noise and hum, dynamics through our compressor, band limits, bit depth) is buildable today; held-out codecs, rooms and noises stay evaluation-only.
  - Shipped weights train only on our renders plus CC0, CC BY or Apache data.
- **Real audio without labels is a hypothesis, not a plan** (01 §3.4, 02 §6). In 2026 papers it helped through render rewards and barely helped as a passive input. The first use is search-distilled pseudo-labels, capped at about 30 %.
- **GR3, run honestly** (04 §3):
  - **Clause 1:** model + search against the *best tuned* search at equal wall-clock time, inference charged, on about 120 held-out targets (McNemar on pass rate, Wilcoxon on JND distance), plus one metric no arm optimises.
  - **Clause 2:** forced choice "which is closer to the reference?", level-matched, top candidates only, catch trials, 60 references (20 of them degraded and judged against the clean original), a cluster-bootstrap rule fixed in advance. Owner about 25 min; friends get a 30-reference kit.
  - **Clause 3:** a cap guarantees 5 minutes; gate instead on time to the first candidate (≤ 15 s) and on no xruns while a search runs in the DAW.
- **Hygiene:** freeze and hash the held-out sets before any training; never fit priors on presets that are also test targets.

## Decisions (provisional)

| Area | Decision | Evidence | Status |
| --- | --- | --- | --- |
| Phasing | R3a (intake, robust objective, portfolio + k-NN, K branches, CLI + plugin) ships alone; R3b adds the model | 01, 03, 04 | Adopted |
| Robust intake and objective | `enth-analysis` intake modules (pure, wasm32); a degradation-aware objective under a new id; `sound/1` unchanged | 04 §2 | Adopted |
| Degradation policy | Codec damage, hum, noise and device EQ count as damage; for room reverb and compression the K candidates include a dry patch and one using the patch's own reverb or compressor | 02 §4 | **Owner decides**; proceed on this |
| First model | CNN, K = 8 WTA + confidence, masked JND loss, canonical slots, routes as a grid; must beat k-NN and search alone | 01 §7 | Adopted |
| Inference | Our own scalar kernels in `enth-solver::propose` (bit-exact on every platform); ONNX for export and parity; fallback `tract-nnef` only | 03 | **Owner decides**; proceed on own kernels |
| Model budget | ≤ ~4 M parameters (f16 in the file), ≤ ~2 GFLOP, a fixed op set; the encoder runs once | 03 §5 | Adopted |
| Model delivery | A separate versioned model file beside the plugin (or `<Documents>/enth/Models/`); embed only if ≤ 2 MB; no network code; R3a runs when it's missing | 03 §5 | Adopted |
| Determinism | Inference bit-exact (own kernels); search deterministic given its starts; the K proposals recorded in every result | 03 §4, 04 R8 | Adopted |
| Training data | `ds1` sampler beside generator v1 (its keyed streams and priors at `amounts` 100, a phrase sampler, disjoint `r3/train/dev/test` streams); preset candidates split into halves (anchors, test) | 02 §2, 04 R6 | Adopted |
| Augmentation licences | LAME, ffmpeg and other LGPL/GPL tools only in `ml/` as subprocesses; no pedalboard, gpuRIR or fdk-aac; shipped weights only on our renders + CC0/CC BY/Apache data | 02 §5 | Adopted |
| Decoders | WAV (hound), FLAC (claxon), Ogg (lewton), MP3 (nanomp3), an in-house AIFF reader; `symphonia` (MPL-2.0, adds M4A) only with the owner's OK | 03 §7 | **Owner decides**; proceed without MPL |
| File input | A fork patch so egui-baseview passes file drops to egui; an in-editor folder browser as the fallback (no `rfd`) | 03 §7, 04 | Adopted |
| Plugin threading | An `enth-create` thread plus a per-job pool, cancellable within one batch, workers `min(cores/2, 8)`, gated by `process-stats --gate g1` while a job runs | 03 §6, 04 R9 | Adopted (rayon in the plugin is a coordinator pin) |
| Compute | The laptop GPU overnight, attended the first time, never beside heavy builds; a rented GPU only if a run would exceed ~24 h, and only with the owner's account | 02 §7 | **Owner decides** on renting |
| Neural proxy | A learned embedding proxy in `ml/` is close to the banned differentiable copy | 02 §6, 04 R13 | **Owner decides**; default no |
| Framework | PyTorch with uv in `ml/`; features computed in Rust (`enth-analysis` through `enth-py`), never re-implemented in Python | 01, 02, 03 | Adopted |

## Corrections to sub-reports

- **Runtime.** 01 assumes "fast with `tract`" and 04 plans a "tract-supported opset" and a `propose/onnx.rs`. 03's measurements supersede both: our own kernels, with the model contract (F3s) defined as our kernel op set, and the files `propose/{pack,ops,net,features,decode}.rs`. 04's snapping of proposals stays as a cheap guard, but bit-exact kernels make it optional.
- **Training data.** 01 trains on generator v1 renders; 02 shows v1 is unfit as it stands. `ds1` replaces it. 02 draws 25 % of samples from perturbed preset candidates while 04 forbids fitting priors on test presets; both hold if the candidates are split into disjoint halves.
- **File input.** 04's cut list drops drag-and-drop and keeps "the folder browser"; 03 found the drop needs only a small fork patch. Both are planned; the browser is the fallback.
- **Model delivery.** 04's default ("bundled if ≤ 10 MB") conflicts with 03 (bundling copies the weights into both the CLAP and the VST3 binary). 03's separate file wins; the R2 pack format may carry it later.
- **Personal details.** The exact GPU model, CPU model and driver version in reports 02 and 04 were redacted by the coordinator before commit (the repository is public). Nothing else in the reports was changed.

## Open checks

| Check | Owner | When |
| --- | --- | --- |
| The degraded-match result on 40 targets × 4 degradations (not 1) | OBJ | Wave 2 |
| Per-evaluation cost on R1's engine with effects (sizes K and the budget) | TP | Wave 2 |
| Our kernels bit-exact on macOS arm64 and MSVC (run the bench's scalar path on CI) | ONX | Wave 6 |
| A create job's effect on `process()` p99 in the plugin | SK3 | F2s |
| GPU training throughput on the laptop; the first PyTorch-exported model through our kernels | TR | Wave 6 |
| Whether REAPER drags media items into a plugin window as files | SK3 or the drop patch | Wave 3 |
| Licences of AMR, GSM, WHAM!, OpenSLR 28's sources; Freesound's terms on training | DS, AUG | Waves 1, 6 |

## The lane plan (from 04 §5, adjusted)

- **Freezes:**
  - F0s: S0s stubs, the `ml/` skeleton, and DS's held-out sets with their hashes;
  - F1s: the intake types, `Candidate` and the `create` API, judge-reviewed;
  - F2s: SK3, create end to end in the CLI and plugin;
  - F3s: the model contract on our op set, judge-reviewed before long training.
- **R3 holds one heavy build slot,** and R2 the other.
- **Waves:**
  - 1: S0s (heavy), DS, DEG, LT, and DOCS3 (shared with R2);
  - 2: FE, OBJ, TP (to F1s), EVK;
  - 3: SK3 (heavy), CLC, the drop patch;
  - 4: PUI (heavy) and the arm-A baseline overnight;
  - 5: INTa, then the owner's R3a check, and **R3a ships**;
  - 6–9: ONX (our kernels, heavy), F3s, DG, AUG, a pilot training run with a 10-sound pilot listen, then TR, MS, EVb, PUIb, INTb and the GR3 listening.
- **Size:** about 14–18 agent-hours for R3a and 11–16 for R3b, plus 1–3 days of training runs. R3a's critical path (S0s → TP → SK3 → PUI → INTa) is about 8–11 h of wall time.
- **Interleaving with R2:**
  - wave 1 of both starts once fu1-hint and fu1-fk merge;
  - R2's S0l and R3's S0s both touch the root manifests, so they run one after the other, coordinator pins first.

## The owner's part

| What | Time | Default if no answer |
| --- | --- | --- |
| Read this summary | 20 min | — |
| Decide: the degradation policy; own kernels vs `tract-nnef`; MPL-2.0 for `symphonia`; the neural proxy; renting a GPU | 10 min | The policy above; own kernels; no MPL (no M4A); no proxy; laptop only |
| Real picks: 10 sounds you care about; OK the agent's CC0 picks | 30–45 min | 40 CC0 picks by an agent blind to results |
| About 30 phone re-recordings for the degraded set | 30 min | Synthetic phone EQ only |
| The first laptop GPU run, attended | 30 min | CPU, or wait |
| R3a check: create from 5 of your sounds in the plugin | 20–30 min | Ship on the kit's numbers |
| GR3 listening (66 trials, resumable); friend kits | 25 min + 10 min | GR3 waits; owner-only |

## Docs to change (lane DOCS3, with R2's rows)

- `docs/design.md`:
  - §8: R3a and R3b, own kernels instead of "ONNX + `tract`", K-hypothesis proposals, k-NN retrieval;
  - §5: the determinism contract for inference and proposals.
- `docs/roadmap.md` (R3): real audio as a hypothesis; the degradation policy; GR3's clauses as above.
- `docs/architecture.md`: `enth-solver::propose` and `create`; `enth-session::create`; `ml/` and `tools/datagen`.
- `docs/phase-3.md`: the R3 lanes beside R2's.
- `docs/r3.md` (new): intake, objective, candidates, model file, GR3.
- `docs/README.md`: the new rows.
