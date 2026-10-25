# 02: Fast feedback in our own pipeline: where an iteration's hours go, and a minutes-to-one-hour loop

Sub-researcher 02, session 2026-10-25 (S2 Depth and R3b iteration speed). Final version.

Evidence labels: **measured** (a log, a file or a run I made today), **documented** (a repo doc or lane note reporting a measurement I did not redo), **estimate** (my arithmetic on measured numbers), **anecdotal**.

## TL;DR

- **Rendering is the critical path, not the GPU.** The 2026-10-22 iteration took about 4.3 h from the first render to an A/B decision with create numbers: rendering 2.07 M samples about 3.1 h on 16 threads; arm B on ready data **51 min**; arm A, beside the renderer, 131 min (82 training + 47 waiting for data). *(measured)*
- **Rendering beside training cost 40 % of GPU throughput:** A trained at a median 3,356 samples/s beside the renderer, B at 5,575 alone. On the CPU the GPU is 25× faster than rendering (5,575 against about 220 samples/s), so rendering on the fly can never feed it. *(measured)*
- **Recipe experiments don't need new renders.** A sample is a pure function of its seed, and 2.07 M samples (97 GB) already sit on the 2080 Ti machine. T0–T2 should reuse them; only a generator, layout or augmentation change needs new data (T3, overnight).
- **The dev set is not the noise; the training trajectory is.** Per-sample dev losses, recomputed from the checkpoints on the laptop CPU (2–3 min each), give A − B a paired SE of **0.0031**: the final gap **+0.0172 is 5.6 SE** (bootstrap 95 % CI +0.011 to +0.023). One run's eval-to-eval jitter is **0.016** at high LR and 0.003–0.008 as it decays. *(measured)*
- **When the A-versus-B ranking settled:** discrete metrics (head diversity, categorical oracle, presence recall) settled at **22–32 %** of training and kept the same sign to the end. Best-of-8 (a 1.2 % gap) settled only at **58 %**, the top-ranked loss at **80 %**. B led at 30 of 32 evaluations, but between 32 % and 55 % the gap was inside the trajectory noise. *(measured)*
- **Seed-to-seed noise is unmeasured.** A and B shared their initial weights and data order, so whether +0.017 survives a reseed is unknown. Every tier below includes a seed replicate for exactly this reason.
- **Proposed loop:**
  - **T0** (under 5 min): `train smoke`, which overfits one batch and round-trips the export;
  - **T1** (about 11 min per arm on the 2080 Ti): the full network, 12,500 steps, a fixed 427 k-sample slice, the schedule compressed so it ends at a low learning rate, and a paired per-sample dev test;
  - **T2** (under 2 h): the full 16 M-sample schedule on the existing data, export, and create at 500 renders on the 30 dev targets (about 2 min per arm instead of 11–22);
  - **T3** (overnight): new data, augmentation, create at 4,000 renders, and the GR3 test half.
- **Warm start for layout 2 is cheap in code:** 97 % of the 3.8 M parameters (trunk, embedding, hidden head layers, confidence) don't depend on the layout. Only `hyp_w` and `hyp_b` (105 k parameters) change width, and their rows map by entry name. The bigger saving is probably fewer *new* renders, not GPU time *(estimate)*.
- **The proxy experiment did not run:** both GPUs were busy with other work all session, and the coordinator's rule is to skip in that case. The exact protocol is under "The experiment", ready to run.

## 1. The iteration's time budget, measured

One full R3b iteration as run on 2026-10-22: lanes TR2 (data and training) and EVM (choice, export and create). Times are wall clock.

| Stage | Wall clock | Where, how fast | Label (source) |
| --- | --- | --- | --- |
| `ds1` render, 2.07 M samples (34 parts of 65,536 candidates, the f16 cache included) | **about 3.1 h** (about 300 s a part alone, 336–355 s beside training) | 16 threads; 215–220 candidates/s alone, 184–195 beside training; 3.05 GB a part (50 KB a sample) | measured (`produce.jsonl`) |
| f16 conversion (inside `produce`) | 5–10 s a part | the same | measured (`produce.jsonl`) |
| The dev set (10 k candidates → 9,324) | 57 s on 12 threads; 304 s on 4 workers of the loaded laptop (load 17) |  | documented / measured (TR notes; my re-render) |
| Augmentation, 10 % of 2 M (built, never run at scale) | about 52 CPU-hours, so about 3.2 h on 16 threads alone | 0.93 s of CPU a clip (laptop), 2.3 s a clip a worker beside training | estimate (r3.md TR2; TR2 notes) |
| Training A (16 M samples seen, beside the producer) | **131 min**: 82.0 training + 46.6 waiting for parts | 2080 Ti, AMP, batch 256: median 3,356 samples/s, GPU 43 % (median) | measured (`full-v0-a/`) |
| Training B (the same schedule, data ready, replaying A's parts) | **51 min** (49.8 training) | median 5,575 samples/s, 45.9 ms a step, GPU 73 %, loader stall 4 % | measured (`full-v0-b/`) |
| The pilot (1660 Ti, FP32, 186 k samples) | 150 min budget; best at 67 min | 1,920 samples/s | documented (TR notes, r3.md) |
| One dev evaluation (9,324 samples) | 0.8–1.2 s (2080 Ti); 2 s (1660 Ti); 32 per run, so about 40 s a run |  | measured (`eval_seconds`) |
| Per-sample dev losses on the laptop CPU | 110–200 s a checkpoint (4 threads, loaded) |  | measured (this report) |
| Export + PyTorch parity (256 inputs, decode check) | about 4 min (B ended 15:36 UTC; READY about 15:40) | laptop | documented (EVM notes) |
| Parity on the whole dev set | about 7 min on 12 processes | 0.53–0.56 s a sample a process | documented (r3.md, EXP) |
| Create eval: 30 GR3 dev targets × 4,000 renders, 6 workers | **11–22 min an arm** (7 runs: 18.0, 19.1, 15.5, 14.6, 11.3, 15.8, 21.9) | the loaded laptop (load 13 at the start) | measured (`gr3-run` logs in `r23/{ms,evm}/dev/`) |
| Create eval read at 250 and 500 renders only (`--budget 500`) | about 1.5–3 min an arm | rungs don't depend on the budget, so this is the same 250/500 reading | estimate: 12.5 % of the renders (r3.md, EVM) |
| GR3 test half, arm A, 40 core targets × 8,000 renders, 8 workers | 54.6 min an arm | laptop | measured (`content/gr3/runs.jsonl`) |

**The critical path as it ran** (UTC, 2026-10-22; the producer's start is my estimate from 34 parts at about 5.5 min each):

```text
~11:35  producer starts (parts 1-7 rendered to the HDD, then copied to WSL's disk)
12:33   A starts on 11 parts (0.67 M samples)
13:14-14:38  A waits 6 times, 47 min in all (pacing: seen <= 8 x distinct)
14:44   A ends (62,500 steps) and the producer ends (part 34) in the same minute
14:45-15:36  B (replays A's parts; nothing else on the CPU)
15:22-15:38  A's create eval on the laptop, overlapping B
15:36-15:40  B export + parity;  15:41-15:55  B's create eval  -> decision
(+ cap runs to 16:44; a GR3 test pair would add about 2 x 55 min)
```

So about **4.3 h** from the first render to the A/B decision with create numbers, about 6 h with the GR3 test arms, and about 3.2 h more of 16-thread CPU if a 10 % augmented share had been made. That matches the brief's 5–8 h *(estimate from measured stages)*.

**What ran in parallel, and what it cost:**

- Rendering beside training (`produce` plus `max_repeat`) overlapped the two longest stages at the GPU's expense: A ran 40 % slower than B (3,356 against 5,575 samples/s) and still waited 47 min, while the producer lost only 12–16 %. The loader's workers and Python's main thread compete with 14–16 render workers for the CPU. *(measured)*
- The arms ran one after the other. B's GPU sat at 73 % at batch 256; TR2's probe ran 5,983 samples/s at batch 512 against 3,942 at 256 *(documented)*, but a larger batch changes the recipe (the LR needs retuning).
- Create evaluation overlapped the next training run (EVM). Export and parity take minutes, so they're never critical.

**The loop's own overheads** *(documented)*: `resume` to the first step 10–60 s; a checkpoint 1–4 s; building `enth-datagen` or `enth-model-check` about 1.5 min; the ML suite 228–337 s on the laptop CPU; a lane gate 15–50 min.

## 2. What the existing logs already say

### 2.1 A against B, evaluation by evaluation

A and B share their initial weights (seed 1), their data and their data order (B replays A's parts), the machine and the schedule; they differ in one key, `select_discrete_boost` (3 against 1). There were 32 dev evaluations each, every 2,000 steps. Below, Δ = A − B (positive means B is better on a loss; for the discrete metrics, positive means A has more).

| Step (fraction) | Δ best-of-8 | Δ top-ranked | Δ distinct choices of 8 | Δ categorical oracle | B best-of-8 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 4,000 (6 %) | +0.267 | +0.037 | −0.06 | −0.010 | 2.387 |
| 8,000 (13 %) | +0.201 | −0.078 | −0.55 | −0.023 | 2.028 |
| 14,000 (22 %) | +0.014 | −0.024 | +0.12 | +0.004 | 1.832 |
| 20,000 (32 %) | +0.004 | +0.029 | +0.97 | +0.040 | 1.761 |
| 32,000 (51 %) | **−0.007** | +0.002 | +1.15 | +0.039 | 1.651 |
| 34,000 (54 %) | **−0.013** | −0.008 | +1.26 | +0.047 | 1.642 |
| 36,000 (58 %) | +0.053 | +0.085 | +1.50 | +0.055 | 1.569 |
| 50,000 (80 %) | +0.0003 | +0.008 | +1.46 | +0.046 | 1.446 |
| 62,500 (100 %) | **+0.018** | +0.028 | +1.63 | +0.049 | 1.371 |

**When each ranking settled for good** (the first evaluation after which the sign never changed):

| Metric | Settled at | Final gap |
| --- | --- | --- |
| Distinct joint discrete choices (A more diverse) | step 14,000, **22 %** | +1.63 of 8 |
| Categorical oracle (A higher) | 14,000, **22 %**; at its final size (about 0.045) by 20,000, 32 % | +0.049 |
| Presence recall of the best (A higher) | 14,000–16,000, **22–26 %** | +0.047 |
| Best-of-8 loss (B lower) | 36,000, **58 %** | 0.017 (1.2 %) |
| Top-ranked loss (B lower) | 50,000, **80 %** | 0.028 |

*(measured from `metrics.jsonl` of both runs)*

**Reading:**

- **Large effects settle early.** The boost's intended effect, heads that disagree on discrete choices, showed its final sign at 22 % and its final size at about 32 %, right after the WTA temperature finishes annealing (`anneal_frac` 0.3).
- **Small effects don't settle at high LR.** Best-of-8 was 10× its final gap early on (+0.27 at 6 %), for another reason: the boosted selection slows the anneal phase. It then sat inside the noise from 32 % to 55 % and flipped twice.
- **The confidence metric is the noisiest.** Its per-sample SD is 1.06, against 0.30 for best-of-8.

### 2.2 How noisy one evaluation is

There are three noise sources, and the logs can measure two of them:

| Source | Size | How measured | Label |
| --- | --- | --- | --- |
| **Dev-sample noise** (which 9,324 samples) | Paired A − B: SE **0.0031** (per-sample SD of the difference 0.297; correlation of A's and B's per-sample losses 0.953). One model's mean alone: SE 0.0100 | per-sample losses recomputed today from the checkpoints (below) | measured |
| **Trajectory jitter** (where SGD happens to be at an eval) | **0.016** per eval while LR ≥ about 0.8× its peak (A and B from 20 k steps; the pilot, epochs 5–40); 0.007–0.008 at about 0.3×; **0.003–0.004** at ≤ 0.1× (the last 20 %) | Rice's second-difference estimator on the dev curves, σ = sd(Δ²)/√6 | measured |
| **Seed-to-seed noise** (initial weights, data order) | **unknown**: no run has a replicate | — | gap |

- **The 1.389 against 1.372 gap is real with respect to the dev set:** t = 5.6, bootstrap CI +0.011 to +0.023, Wilcoxon z = 6.5. It is spread across sources (generator v1 +0.016 ± 0.004, preset +0.015 ± 0.004, uniform +0.025 ± 0.010) and is not a tail effect (without the 1 % largest |Δ| it is +0.016).
- **It is also stable at the end of training.** The last 7 evaluations (from 52 k steps) read +0.018 to +0.029, and the same run's checkpoints at 60 k, 62 k and 62.5 k move by only 0.001–0.004 (paired SE 0.001).
- **What it is not, yet:** shown to survive a different seed. One indirect bound: the A − B difference series jitters about as much as each curve (0.020 against 0.016 per curve from 20 k steps; independent noise would give √2 × 0.016 = 0.023), although the runs share init and data order. So by mid-run the two trajectories have decorrelated, and the jitter is run-specific rather than driven by the data order. A seed-to-seed SD of the final loss is then at least the end-of-run jitter (about 0.0035), which caps the gap at about 3.4 SD of a two-run difference (0.017 / (√2 × 0.0035)). It could be far less if runs also differ by persistent offsets. Literature priors belong to sub-report 01.
- **The rule in use** (dev best-of-8, threshold 0.002) suits "is this checkpoint better" within a run, not "is this recipe better" across runs.

**Per-sample dev losses: possible, cheap, and the right test.**

- The trainer logs only sums (`DevMetrics.update`), but `ProposeLoss.hypothesis_losses` gives per-sample losses directly. I re-rendered `r3/dev` seed 1 on the laptop (same 9,324 samples and skips; B's `best.pt` gives 1.371723 against the logged 1.371715), copied four checkpoints (46 MB each, checksums match), and `scripts/persample.py` evaluated each in 2–3 min of CPU.
- How many dev samples a paired test needs, from the measured SD of 0.297:

| To detect a gap of | at 2 SE | at 3 SE |
| --- | ---: | ---: |
| 0.017 (A/B, 1.2 %) | 1,220 samples | 2,745 |
| 0.035 (2.5 %) | 290 | 650 |
| 0.07 (5 %) | 72 | 160 |

- A 2,000-sample subset has a paired SE of 0.0066. On a GPU the full dev set costs a second, so subsets only matter for CPU-side checks.

### 2.3 The pilot

- **Curve:** 90 epochs over 186 k samples, evaluated after each epoch. Dev best-of-8 reached within 0.02 of its minimum by epoch 21 (37 min of 150), hit its minimum at epoch 39 (67 min), then rose to 2.02 (overfitting).
- **Jitter:** 0.016 during the plateau, 0.004–0.006 in the last third, where the learning rate decays (on the wall clock).
- **What it tells us:** a data-bound run shows its best within about 25 % of its budget; the rest says nothing about the recipe. It can't be ranked against A or B (11× less data, other loss weights).

## 3. A tiered loop for us (a lane spec)

**The principles, from the measurements above:**

1. **Never render in the inner loop:** T0–T2 run on data that already exists.
2. **Compare at a low learning rate:** a compressed schedule, or a cooldown branch.
3. **Pair everything:** the same seed, data order and dev samples, compared per sample.
4. **One seed replicate per decision**, until seed noise is measured.
5. **Let the large-effect metrics decide early** (the discrete ones); small-effect losses go up a tier.

| Tier | Budget | Data | Network | Steps | Decides on | Machine |
| --- | --- | --- | --- | --- | --- | --- |
| **T0 smoke** | ≤ 5 min | 256 dev samples (one batch) | the config's | 300 on one batch, then export | loss falls ≥ 70 % (overfit); export reads back exactly; our kernels match PyTorch within 1e-4 on 8 inputs; budgets (2²² parameters, 2³¹ FLOPs) | laptop CPU or any GPU |
| **T1 proxy** | about 11 min an arm on the 2080 Ti (about 30 on the 1660 Ti); 3 runs about 35 min | a fixed slice of the existing parts: 7 parts, 427 k samples (about 7.5 repeats at 3.2 M seen, under the full run's cap of 8) | full width (no width-transfer assumption) | 12,500 at batch 256 (3.2 M seen, 1/5 of the full run), warmup 500, decay to 2 %, anneal over 30 % of the proxy | **the paired per-sample dev test** on best-of-8 (all 9,324) and the discrete metrics; a change counts if its gain is over max(3 × paired SE ≈ 0.01, 2 × the proxy's seed SD) | the 2080 Ti; the 1660 Ti only with a slice of 150–240 k samples in the host's file cache or on WSL's disk (13–20 repeats at 3.2 M seen, which biases a proxy towards recipes that regularise) |
| **T2 confirm** | ≤ 2 h | all 2.07 M existing samples | full | 62,500 (51 min) or a half schedule (26 min) | paired dev test; create at **250/500 renders** on the 30 GR3 dev targets (McNemar and Wilcoxon against the incumbent) | the 2080 Ti + laptop |
| **T3 ship** | overnight | new renders (about 3.1 h per 2 M on 16 threads) + augmentation (about 3.2 h per 10 %) | full (or bigger) | 16–32 M seen | create at 4,000 on dev; GR3 test half arms A+B (about 55 min each); full-dev parity; model card | both, plus laptop |

**What T1 can and cannot resolve** *(estimate)*:

- At the end of a decayed schedule the trajectory jitter is about 0.004 and the paired dev SE 0.003. If the proxy's seed SD is of the same order, T1 resolves gaps of about 0.015–0.02 (1–1.5 %), just enough for an A/B-sized effect, and anything of 3 % or more clearly.
- Discrete-metric effects like the boost's resolve easily (gap 1.6 against a jitter of about 0.1).
- Data-size effects probably don't: at 3.2 M seen, the pilot's 186 k-sample run was still improving. Measure those at T2 or T3.

### 3.1 The changes, file by file

1. **`ml/enth_ml/train/__main__.py` + a new `smoke.py` (T0).** `python -m enth_ml.train smoke --config C --dev DIR [--model-check BIN] [--device cpu]` does four things:
   - builds the model and checks the contract's budgets (parameters ≤ 2²², FLOPs ≤ 2³¹, as `pack.rs` counts them);
   - overfits 256 dev samples for 300 steps and asserts that best-of-K falls ≥ 70 %, that the result is finite and that every head wins at least once;
   - exports through `export/propose.py`'s `as_stored` path to a temporary file, which already reads it back and compares exactly;
   - with `--model-check`, runs `enth-model-check run --tolerance 1e-4 --decode` on 8 inputs.

   It writes `smoke.json`. Every piece exists (the `test_train_*` fixtures, `check_round_trip`, the tiny fixture); this is wiring.
2. **`ml/enth_ml/train/proxy-v0.toml` (T1)**, a copy of `full-v0-b.toml` with:
   - `schedule_steps = 12500`, `warmup_steps = 500`, `eval_every_steps = 1000`;
   - `early_stop_patience = 0`, `max_repeat = 0`, `train_glob = ""`, `min_train_samples = 0`, `ckpt_every_minutes = 10`;
   - `train_dirs` set on the command line to the 7 parts.

   Every key exists already. Optional: a `base = "full-v0-b.toml"` key in `TrainConfig.load` (about 10 lines).
3. **`ml/enth_ml/train/loop.py`: per-sample dev losses.** A new key `persample_evals = "final"` (or `"all"`) writes `dev-persample-<step>.npz`: best, top and mean-K per sample, the winner, and `source` (9,324 × 4 B a field, about 37 KB). `DevMetrics.update` already computes `best_l` and `losses[ar, top]`; keep them instead of summing. Also fix the end-of-run quirk EVM found: a final evaluation that improves should rewrite `best.pt`.
4. **`ml/enth_ml/train/compare.py` (new): `python -m enth_ml.train compare RUN_A RUN_B [--step N]`.** It prints:
   - the paired mean difference, SE, bootstrap CI and Wilcoxon for best-of-8, top and mean-K;
   - the discrete-metric differences, each with its band from the runs' own second-difference jitter;
   - the per-source breakdown;
   - a verdict: "A better", "B better" or "no decision at this tier".

   My scratch `scripts/paired.py` is the prototype.
5. **`ml/enth_ml/train/loop.py` + `loss.py`: a schedule with cheap cooldowns.**
   - `lr_schedule = "cosine" | "wsd"` (warmup, then a constant LR, then a decay over the last `decay_frac`, default 0.2), and `anneal_steps` so the WTA temperature follows absolute steps rather than progress.
   - Then `python -m enth_ml.train cooldown RUN_DIR --from-step N --steps M --out OUT` branches a decayed copy from any stable-phase checkpoint, using the stop/resume machinery TR2 built.
   - Uses: low-noise comparison points at 20, 40 and 60 % of one long run for the price of short cooldowns, and extending a run without restarting. The schedule keys are protected by `identity_diff`, hence the separate command.
6. **A RAM-resident slice for proxies:** `train_in_ram = true` loads the slice's shards into one array at start, as `DevSet` already does for dev (427 k × 50 KB = 21 GB: too much for the 2080 Ti machine's 20 GB WSL, so 240 k samples, 12 GB, there and on the 1660 Ti machine; or memory-map the parts). It removes loader workers and disk reads from T1. On the 1660 Ti machine, whose HDD streams 500–950 samples/s, it decides whether T1 is possible at all.
7. **A fast create eval (T2), as one command.**
   - Today the proposals file for `cargo xtask gr3-run --arm B --proposals F` comes from a scratch tool (`r23/ms/devprop`, reused by EVM).
   - Make it a subcommand: `enth-datagen gr3 proposals --model M --split dev --out F`.
   - Run the arms at `--budget 500` and read them with `gr3-report --at 250` and `--at 500`. That is about 2 min an arm instead of 11–22, and it is the budget where models differed (B: p = 0.004 at 250, level at 4,000). Search alone's run is reusable while the build is unchanged.
   - Optional, for power: 200 extra create targets drawn from `r3/dev` (250 renders each, about 5 min on 6 workers *(estimate)*); in-distribution only, so an addition to the 30 GR3 dev targets.
8. **The runbook.** A one-line wrapper per machine (`tr2`'s pattern) for `smoke`, `proxy ARM SEED` and `compare`, writing only under an `exp-fast/` directory.

### 3.2 Making data generation stop being the bottleneck

1. **Reuse, don't re-render.** Recipe changes (loss weights, WTA, schedule, network) never need new data; the 2.07 M samples (97 GB on the 2080 Ti machine) serve T0–T2, and pairing needs the same data anyway.
2. **When new data is needed, render before training, not beside it.** Beside, rendering cost 40 % of training throughput. Before, B-style training takes 51 min after a 3.1 h render. Render overnight (T3) and train in the morning, or cap the producer at 8 threads when the two must overlap. *(measured trade-off; the 8-thread cap is an estimate)*
3. **On-the-fly rendering is not worth building.** 16 threads render about 220 samples/s against the GPU's 5,575, so each sample would be reused about 25× anyway, which the rolling cache plus `max_repeat` already does.
4. **Make each render cheaper (a `ds1` v2 question).** A candidate costs about 54–62 ms of CPU: render 31, intake 13, features 8 (laptop). The window rendered is 3–4 s, but the model reads 2 s from the onset; rendering about 2.5 s would save roughly 25–30 % of the render share *(estimate; it changes what the intake sees and must be checked against the release masks)*.
5. **Pre-bake augmentation once** (it is 15× a clean render) and reuse the parts across recipes. Never in T0–T2.
6. **Mels, not audio, are already the cache** (f16, 50 KB a sample, rounding cost under 1e-4 on dev). Enough at 2 M samples.

## 4. The experiment

**Not run.**

- **The 1660 Ti machine:** at 16:03, 16:25, 16:34 and 16:38 (laptop time), `nvidia-smi` showed it at 99–100 % utilisation with 3–4 GB used by another project's job.
- **The 2080 Ti machine:** the coordinator allowed it if idle, but it showed 97–100 % utilisation from other processes at 16:04, 16:25 and 16:38 (and did not answer ssh at 16:34).
- I touched neither; per the coordinator, the experiment becomes a protocol. What ran instead (laptop CPU) is the paired per-sample re-evaluation of A and B (section 2.2), the dev-noise half of the question.

**The pair:** A against B, the only pair that differs in one key with identical data, init and machine. Its full-run orderings (A − B) are one small effect, best-of-8 +0.017 (B better; 5.6 dev SE, seed noise unknown), and three large ones: distinct choices +1.63, categorical oracle +0.049, presence recall +0.047. The pilot against the full run is confounded (data 11×, loss weights, AMP, confidence targets), so it can't validate a proxy.

**The protocol (the 2080 Ti machine; everything under `~/enth-train/exp-fast/`; existing parts read-only):**

```bash
# once: the dev set into exp-fast (about 1 min on 16 threads; same samples as r3/dev seed 1; manifest sha256 should be 759bd22b...444c)
source ~/enth-train/env.sh; E=~/enth-train/exp-fast; mkdir -p $E/runs
~/enth-train/enth-datagen-3a573e1 ds1 gen --count 10000 --seed 1 --stream r3/dev --out $E/dev-s1 \
  --workers 16 --shard-size 256 --gr3 ~/enth-train/gr3-3a573e1
# four proxy runs, about 11 min each, one at a time (nvidia-smi idle first); seeds 1 and 2 per arm
cd ~/enth-tr2/ml; PY=~/enth-train/venv/bin/python
PARTS=$(printf '"%s",' ~/enth-train/data/train/train-s10{1..7}-f16); PARTS="[${PARTS%,}]"
for seed in 1 2; do for arm in b a; do
  $PY -m enth_ml.train run --config enth_ml/train/full-v0-$arm.toml --no-resume \
    --set "train_dirs=$PARTS" --set 'train_glob=""' --set 'replay_from=""' --set max_repeat=0 --set min_train_samples=0 \
    --set "dev_dirs=[\"$E/dev-s1\"]" --set schedule_steps=12500 --set warmup_steps=500 --set eval_every_steps=1000 \
    --set early_stop_patience=0 --set ckpt_every_minutes=10 --set seed=$seed --set "out=\"$E/runs/proxy-$arm-s$seed\""
done; done
# per-sample losses of each final checkpoint, then the paired comparison (scripts/persample.py, paired.py from my scratch)
```

- Detach each run with `nohup setsid … &` and a 2 s sleep (TR2's `launch`); stop with `python -m enth_ml.train stop RUN`. About 45 min of GPU for four runs.
- **On the 1660 Ti instead:**
  - use the newest 8 of the 22 parts already on D: (30.4 k samples each, so 244 k, held in the Windows file cache), or 5 parts (152 k, 7.4 GB) copied into a cache of 8 GB or less on WSL's own disk, deleted afterwards;
  - add `--set amp=false` (FP16 is 2.4× slower there);
  - use `schedule_steps=8000` (2 M seen, about 19 min a run at 1,800 samples/s): about 80 min in all.

**Reading it:**

- **Seed SD:** from |B s1 − B s2| and |A s1 − A s2|.
- **The proxy reproduces the discrete ordering** if Δ distinct, Δ categorical oracle and Δ presence recall are > 0 in both seeds.
- **It reproduces best-of-8** if Δ > 0 in both seeds and the mean Δ is over 2 × the seed SD.
- **If only the discrete part reproduces,** T1 ranks large effects only, and best-of-8 changes of about 1 % go to T2.
- **Optional:** the same four runs at half width (`model.widths=[16,32,64,128] model.stem=12 model.embed=192`) test whether a cheaper network keeps the ordering. Its throughput is unmeasured: with B's GPU at 73 % at full width, the loader may limit it.

## 5. When the synth grows: warm-starting layout 2 from v0-b

**What depends on the layout today:**

- **In the network (`model/net.py`):** only `hyp_w` `[K, 128, W]` and `hyp_b` `[K, W]`, 105,264 of 3.8 M parameters (2.8 %). The stem, the stages, `proj`, `mix`, `hyp_in` (embedding → K × 128) and the confidence head don't depend on W. The input side (feature set 1) is unchanged by new modules.
- **In Python:** `model/layout.py` pins layout 1 as constants (`WIDTH = 102`, the segment offsets, `GROUPS`, `CONTINUOUS_NAMES`, the LFO swap permutation, the layout's SHA-256). They are read in `loss.py` (11 uses), `metrics.py` (14), `net.py` (4), `shards.py` (3), `export/propose.py` (3) and `loop.py` (1).

**What warm-starting needs:**

1. **The layout comes from the data.** `layout.py` builds segments, groups, names and the swap permutation from the shard manifest's `layout` block (it already carries the entry names and hash); loss, metrics, export and `ModelConfig.width` read that object. A refactor with no behaviour change for layout 1 (the existing tests prove it).
2. **`--init-from CKPT` in `train run`** with a name-based mapper:
   - copy every tensor whose name and shape match (trunk, embedding, `hyp_in`, confidence);
   - for `hyp_w`/`hyp_b`, copy the column of every entry whose name, kind, range and scale are unchanged in layout 2 (from both layouts' canonical texts); give new columns a small init (zero weight, the prior's mean as the bias) and reinitialise moved ones;
   - reuse the standardisation constants (same input);
   - start a fresh AdamW with a short warmup, and begin the WTA temperature low (for example 0.1 → 0.01), so the eight heads keep their specialisation;
   - log what was copied (`run.json` `init_from`; an export info key).
3. **Unchanged:** export, ENTHMODL format 1 and op set 1 (a new layout id and hash are already header fields). The Rust side is the contract's own work, needed with or without a warm start: `OutputLayout` for layout 2, `layout-v2.txt`, decode, and `ds1` v2 (or `ds2`) shards with `[n, W2]` labels and masks.

**The saving** *(estimate; to test at T2: warm against scratch on the same new data, dev at 25/50/100 % of the schedule)*:

- **GPU:** B passed 1.75 at 35 % of the schedule and spent the remaining 65 % going to 1.37. The early phase is mostly generic spectro-temporal features and the shared parameters. A warm start would likely skip most of that 35 % for the entries layout 1 already has, so about 20–40 % of 51 min, roughly 10–20 min. Small.
- **Data, the real saving:** if a warm-started model reaches the scratch model's dev loss on fewer distinct new-layout samples (for example 0.5 M instead of 2 M), that saves about 2.3 h of rendering at today's cost, and more once FM, unison and effects make each render costlier. The open question is the boundary: whether layout 2 is a superset of layout 1's 102 entries (then most columns transfer) or a re-templated space (then only the trunk does).

## Gaps

- **The proxy experiment did not run** (both GPUs were busy); its protocol is in section 4.
- **Seed-to-seed noise** of the dev metric is unmeasured at any scale.
- **The proxy's throughput** at half width and the 2080 Ti's at batch 512 *with data loading* are unmeasured (only the model-alone probes exist).
- **Transfer speed between the 2080 Ti machine and the 1660 Ti machine** is unmeasured; from the laptop to the 2080 Ti machine, scp ran at about 0.8 MB/s (TR2 notes). That decides whether the 1660 Ti machine could render for the 2080 Ti.
- **The producer's start time** on 2026-10-22 is estimated (34 parts × about 5.5 min); parts 1–7 were rendered before the producer log begins.
- **The create eval at `--budget 500`** is an estimate (12.5 % of the 4,000-render runs' time); not run.
- **No GR3 test run with a model** (EVb) exists yet, so the T3 cost of an arm B is extrapolated from arm A's 54.6 min.

## Sources

- Repo: `docs/r3.md` (R3b: DG, DEG/AUG, the TR model card, EXP, TR2, EVM, MS, the model contract, Training); `ml/README.md` (TR, TR2 stop/resume, the runbook, EXP); `ml/enth_ml/train/{config,loop,loss,metrics,data,__main__}.py`, `full-v0-a.toml`, `full-v0-b.toml`, `pilot-v0.toml`; `ml/enth_ml/model/{net,layout}.py`; `ml/enth_ml/export/propose.py`; `content/gr3/runs.jsonl`.
- Lane notes: `~/code/enth-scratch/r23/tr/notes.md`, `tr/pilot/metrics.jsonl`, `r23/tr2/notes.md`, `r23/evm/notes.md`, `r23/ms/notes.md`, `gr3-run` logs in `r23/ms/dev/` and `r23/evm/dev/`.
- Copied from the 2080 Ti machine (read-only): `full-v0-{a,b}/{metrics.jsonl,summary.json,gpu.csv,train.log,run.json,config.json}`, the producer's `produce.jsonl`, and four checkpoints (A 60 k and 62.5 k, B 62 k and 62.5 k; SHA-256 checked). Copies and scripts are in `~/code/enth-scratch/2026-10-25-s2ft/02/` (`remote/`, `ckpt/`, `persample/`, `scripts/persample.py`, `scripts/paired.py`, `notes.md`).
