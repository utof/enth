# 01: Fast feedback loops for training: literature and practice

Sub-researcher 01, session 2026-10-25. The questions:

- which small proxies predict big runs;
- what of that fits R3b (a 3.8 M-parameter CNN regressing synth parameters from log-mels, trained on our own deterministic renders, data-limited);
- how sound-matching groups iterate;
- how to grow the model when the synth grows;
- the statistics of deciding A vs B;
- a tiered recipe ladder.

Each claim carries a label:

- **measured:** our numbers (docs/r3.md, ml/README.md, `full-v0-b.toml`) or arithmetic done here;
- **documented:** a paper says it (the authors' own result unless a replication is named);
- **anecdotal:** a blog or third-party summary page.

Works are named by title or acronym in the text; links are in Sources.

## TL;DR

1. **Ranking at one small, fixed scale is a strong baseline; fancier extrapolation often does not beat it.**
   - DataDecide: ranking 25 data recipes at 150 M parameters picks the 1 B winner in about 80 % of pairwise comparisons, and none of 8 scaling-law fits beat that (documented).
   - EcoNAS: for equal iterations, **all the data with fewer epochs ranks better than fewer samples with more epochs**, and **cutting width is more reliable than cutting input resolution** (documented).
2. **A short run must finish its schedule.** In NAS-Bench-201, a cosine compressed into 12 epochs correlates 0.91 with full training, while the 12-epoch checkpoint of a 200-epoch cosine correlates 0.75 and the 100-epoch one 0.79 (documented). Constant LR plus a 10–20 % cooldown (WSD) matches cosine, and the cooldown can branch from any checkpoint (documented). **Rank cooled-down models, never mid-cosine checkpoints.**
3. **Proxies fail where a knob interacts with data or compute:**
   - augmentation and regularisation flip sign with data size and run length (AugReg);
   - the best data filter changes with compute;
   - short horizons favour learning rates that are too small;
   - warmup and optimiser tuning explain whole scaling-law disagreements (all documented).

   **For R3b, test regularisation-like knobs (augmentation, weight decay, data mix, curricula) at the full run's repetition ratio, samples seen ÷ distinct samples.**
4. **Our schedule ties three hidden knobs to the run length** (measured), so a shortened run silently changes all three:
   - the cosine;
   - the WTA temperature anneal (first 30 % of `schedule_steps`);
   - AdamW's decay timescale 1/(lr·wd) = 10,000 steps (16 % of the full run, 64 % of a 4 M-sample proxy).
5. **muP is not worth adopting now.** It transfers ResNet hyperparameters in its paper but is fragile in follow-ups (documented). At 3.8 M parameters and about 80 min per full training run, we can tune the real width; a half-width proxy pays only if it trains ≥ 2× faster (unmeasured).
6. **Learning-curve extrapolation and successive halving save 2–10× in HPO benchmarks**, but about 15 % of curves are ill-behaved and curves cross (documented). Use them to kill clear losers, not to crown winners.
7. **Sound-matching groups mostly train on precomputed sets for many epochs.**
   - Surge XT inversion, ISMIR 2025: 2 M renders, 128 M samples seen (documented).
   - Data comes on the fly only with GPU or differentiable synths (torchsynth: 16,200× realtime on one GPU).
   - Parameter losses are the efficient signal; spectral losses alone train poorly.

   **For us, reuse a fixed rendered pool instead of re-rendering.**
8. **Growing the model:**
   - model-growth papers save about 45 % of compute with warm starts (LiGO: 44.7 % of FLOPs);
   - LR re-warm, re-decay and replay match retraining on the union;
   - warm starts risk a generalisation gap, which shrink-and-perturb reduces (all documented).

   **For layout 2:** keep the trunk, add new output rows that start "absent", keep old-style patches in the sampler, re-warm the LR, check once against scratch.
9. **Statistics:**
   - paired per-sample dev differences: n ≈ 6.2·(σ_d/δ)² for one-sided α = 0.05 at 80 % power;
   - run-to-run (seed) noise is unmeasured and probably exceeds the evaluation noise of 9,324 dev samples;
   - McNemar on 30 create targets needs ≥ 5 discordant all one way, and has under 50 % power for subtle effects.
10. **The ladder.**

    | Tier | Budget | Role |
    | --- | --- | --- |
    | T0 | ≤ 5 min | Gates only |
    | T1 | ≤ 30 min | Ranks, with two cooled-down proxies: short on all data, and a ratio-matched subset |
    | T2 | ≤ 2 h | Confirms finalists at full size on a frozen pool |
    | T3 | Overnight | Renders new data and runs create |

    **Calibrate it first by replaying the A/B pair at T1 and measuring seed spread.**

## 1. Small proxies that predict big runs

### 1.1 Proxy families (documented unless marked)

| Family | Evidence | Saves | Known failure |
| --- | --- | --- | --- |
| **Rank at one small scale** | **DataDecide**: 25 corpora, 4 M–1 B parameters, 3 seeds. Ranking at 150 M gets about 80 % of 1 B pairwise decisions right; 8 scaling-law fits did not beat it; continuous likelihood metrics predict better than discrete ones. **EcoNAS** (CIFAR-10): Spearman between reduced and full settings is 0.6–0.7 for older NAS proxies and 0.85–0.95 for good ones | 10–1,000× | Rank flips (§1.2) |
| **Converged short schedule** | **NAS-Bench-201** Table 6, correlation with full training on CIFAR-10: 12/12 epochs **0.911**; 12/200 0.752; 100/200 0.794; 150/200 0.896. The benchmark uses the converged 12-epoch schedule for all non-weight-sharing NAS | 6 % of the compute ranks better than 50 % of a truncated run | Terms annealed with the schedule (our WTA, §2.3) |
| **Scaling-law fit** | Kaplan 2020 and Chinchilla 2022 laws; data-size power laws across domains, speech included (2017). A meta-study of 485 models: fit with intermediate checkpoints, final loss estimable from about ⅓ of training, similar sizes predict best, seed variance large. Judge fits by extrapolation error (2022) | Plans budgets | Warmup duration, last-layer cost and scale-dependent tuning flipped the published compute-optimal conclusion (2024) |
| **muP / muTransfer** | **Tensor Programs V**: ResNet on CIFAR-10 (SGD, 10 epochs, 15 seeds). Under the standard parameterisation, the best width-8 net is worse than width-4; under muP, wider is always better and the LR is stable. Training-time transfer holds "with a few caveats" | One small sweep | Surprising failures (2024 empirical study). **u-μP** ties the clean transfer partly to constant LR and many epochs. AdamW weight decay must be rescaled (§2.3). Other parameterisations transfer too with per-layer LRs (2024) |
| **Learning-curve extrapolation** | Freiburg extrapolation (IJCAI 2015): about 2× faster DNN HPO. Freeze-thaw BO (2014). **LC-PFN** (2023): over 10,000× faster than MCMC, 2–6× HPO speed-ups on 45 datasets | 2–6× | **LCDB 1.1**: about 15 % of curves significantly ill-behaved; crossing curves defeat successive halving and parametric fits |
| **Successive halving, Hyperband, ASHA** | Train all for one epoch, keep the top K: matched multi-fidelity HPO at about a tenth of the compute (2023) | About 10× | Crossing curves; bias toward fast starters |
| **Small proxies for data decisions** | DataComp scales; DsDm datamodels; RegMix (512 models of 1 M parameters pick a 1 B model's mixture, assuming rank invariance; a follow-up finds 70 M or < 6 k-step proxies weaker under a stricter metric, anecdotal) | Large | **Data filtering is not compute-agnostic** (CVPR 2024): a curated subset loses value with each repeat, so the best filter strength shifts with compute |
| **Early reads from schedules** | **WSD**: constant LR plus cooldown (below). LAWA (latest-checkpoint averaging); schedule-free AdamW | About 50 % of a scaling sweep | Loss before a cooldown is not a proper estimate |

### 1.2 Where proxies fail

1. **Regularisation × data × compute.** This is the failure that matters most for us.
   - AugReg (ViT): on ImageNet-1k any augmentation or regularisation helps. On ImageNet-21k at equal compute (30 epochs), "any kind of AugReg hurts performance for all but the largest models". At 300 epochs it helps more models but still hurts small ones. One knob gives three answers, depending on data size, run length and model size.
   - The data-filtering law says the same for curation.
   - EcoNAS links lost rank consistency to overfitting.
2. **The LR schedule.**
   - Truncated cosine checkpoints rank badly (NAS-Bench-201).
   - Short-horizon objectives pick LRs too small "by multiple orders of magnitude", even at 100 steps (ICLR 2018).
   - The WSD paper says a cooldown "is needed to properly estimate model performance".
3. **Warmup and optimiser details.** Warmup duration and scale-dependent tuning reconcile the Kaplan and Chinchilla laws; AdamW β₂ matters at small batch.
4. **Width without muP.** The optimal LR moves with width under the standard parameterisation (TP V).
5. **Ill-behaved and crossing curves** (LCDB 1.1).
6. **Seed noise.**
   - A 10⁴-seed CIFAR-10 scan: about 2 % between the best and worst seed (anecdotal, via summaries).
   - Data sampling, initialisation and order all matter; randomising more of them gives a better estimate at 51× less compute (MLSys 2021, documented).
7. **Metric mismatch.** B's best-of-8 is 29 % below the pilot's on the pilot's loss, yet in create B is level with the pilot except at 250 renders (measured). Dev loss ranks recipes; create gates.

### 1.3 What makes a proxy trustworthy

- **Calibrate on pairs with a known full-run answer** (DataDecide's pairwise accuracy is this exercise). Two or three pairs can expose a broken proxy but not certify a good one, so keep calibrating on every decision that reaches the full run anyway.
- **Every proxy model is a cooled-down one.**
- **Hold constant what the knob does not touch:** batch size, data distribution, warmup, decay timescale. Hold the repetition ratio for regularisation-like knobs.
- **Use a continuous metric** with per-sample values kept for paired tests.
- **Replicate.** At least 2 seeds near the noise floor. Common random numbers (the same init and data order in both arms, TR2's `replay_from`) cut the variance of the difference but condition the answer on one seed.
- **Pre-register the decision rule**, as lane EVM did.
- **Run two proxies that stress different axes; disagreement escalates.**

## 2. What applies to R3b

### 2.1 Our regime (measured)

| Item | Value |
| --- | --- |
| Network | CNN, 3,795,074 parameters, 0.477 GFLOP; K = 8 WTA hypotheses + confidence; 102 outputs per hypothesis |
| Schedule | AdamW, lr 2e-3, wd 0.05, β₂ 0.99, batch 256; 1,000 warmup steps, cosine to 2 % over 62,500 steps (16 M samples); WTA temperature 1 → 0.01 over the first 30 %; dev evaluation every 2,000 steps (9,324 samples, 2 s) |
| Throughput | 2080 Ti machine: 16 M samples in about 75–80 min (3.4–5.8 k/s, AMP). 1660 Ti machine: 1.9 k/s |
| Data | About 220 renders/s on 16 threads (2 M samples in about 3 h); augmentation about 50 CPU-h per 10 % of 2 M |
| Data limit | Pilot: 186 k distinct samples, best at 7.4 M seen, then overfit. A and B: about 2 M distinct, 16 M seen (about 8× repetition), still improving |
| A/B | B beat A by 1.2 % (1.3717 against 1.3890), **ahead at 30 of 32 evaluations** (same cosine, initial weights and data order) |

**Training is not the multi-hour part; data, export, parity and create are.** The 80-min run is already a "T2". What we need from the literature is which shortened runs rank recipes correctly, and a reason not to re-render for each experiment.

### 2.2 Fewer steps, or the same steps at a smaller width?

- **Fewer steps first.** EcoNAS says both work but that fewer samples hurts more than fewer epochs. NAS-Bench-201's converged short schedule correlates 0.91 with full training. The scaling meta-study predicts best from models of similar size. So: fewer steps, compressed and fully cooled down, at full width, on all the distinct data we have. It keeps the shipped architecture, needs no reparameterisation, and full width is cheap.
- **Width-halving only** for architecture questions, and only if it measures ≥ 2× faster. A 0.48-GFLOP CNN at batch 256 may be bound by memory, kernel launches or the loader; the pilot showed 95 % GPU utilisation on the 1660 Ti machine, so a speed-up is plausible but unmeasured.
- **Never reduce input resolution.** Feature set 1 is contract-fixed, and EcoNAS found resolution the least reliable reduction.

**Two T1 proxies, for two kinds of knob:**

| Proxy | Data | Repetition | For |
| --- | --- | --- | --- |
| **T1-short** | The whole ~2 M pool, 4 M samples seen, cooled down | About 2× (the full run is 8×) | Architecture, loss weights, heads, WTA rules, confidence |
| **T1-ratio** | A fixed 500 k subset, 4 M samples seen | About 8×, like the full run | Augmentation, weight decay, dropout, the 60/25/15 mixture, curricula, filtering |

The split follows AugReg and the data-filtering law: these knobs' value depends on how often each sample is seen. EcoNAS says subsets rank architectures worse, which is why architecture questions use T1-short.

### 2.3 Schedules that make truncation informative

- **WSD instead of cosine** (NeurIPS 2024, documented):
  - the cooldown's benefit plateaus at about 20 % of steps, and 5 % nearly matches cosine on long runs;
  - a `1 − sqrt` cooldown beats linear;
  - the constant LR that worked was about half of cosine's peak;
  - reusing runs halved their scaling-sweep compute.

  TR2's exact resume already holds the step, optimiser and data position, so a cooldown branch is "resume at step N with an M-step cooldown". A 16 M-sample constant-LR trunk with branches at 2, 4 and 8 M costs 17.5 % extra and yields a **curve of finished models** per recipe (measured arithmetic).
- **Decouple the WTA anneal from `schedule_steps`.** Today a 4 M-sample proxy anneals in 1.2 M samples against 4.8 M in the full run. Either anneal over a fixed sample count in every tier, or anneal inside each cooldown; which ranks correctly is an experiment. The A/B pair (a WTA-selection change) is the test.
- **Hold the AdamW timescale fixed.** PyTorch's AdamW shrinks weights by lr·wd per step: τ = 1/(lr·wd) = 10,000 steps = 2.56 M samples at peak LR (measured arithmetic). The optimal τ stays roughly constant **in epochs** across model and data size in multi-epoch image training, and muP's LR transfer fails for AdamW unless weight decay is rescaled (documented). For single-pass LLMs a follow-up normalises τ by total steps (documented). Keep τ the same in samples across tiers unless decay is the knob; whether it should scale with a bigger pool is a calibration item.
- **Weight averaging as a free monitor** during the constant phase. SWA helps but does not reach a cooldown (documented). Recompute BatchNorm statistics after averaging; never decide on it.
- **Warmup stays 1,000 steps everywhere** (it guards early instability), though it is 6 % of a 4 M-sample proxy.

### 2.4 Buy data with a fitted curve

Since bigger data kept helping, fit L(D) = E + B·D^−β on dev best-of-8 at 4 pool sizes (250 k, 500 k, 1 M, 2 M distinct), each at matched repetition and cooled down. Judge the fit by leave-one-out extrapolation error, then choose the next render budget by loss gain per CPU-hour. The pilot and B cannot serve: they differ in more than data.

## 3. Sound-matching literature

| Work | Synth | Data | On the fly? | Loss and notes |
| --- | --- | --- | --- | --- |
| InverSynth (TASLP 2019); II (ISMIR 2023) | Subtractive with FM oscillators | Rendered | No | Parameter classification with a strided CNN, depth matters; II adds a learned differentiable proxy and inference-time fine-tuning |
| DDSP (ICLR 2020); inverse-synthesis pitch (ICML-W 2020) | Differentiable harmonic + noise | Synth-generated | Yes | Spectral loss; parameter-regression pre-training sidesteps the non-convexity of frequency |
| torchsynth (DAFx 2021) | GPU modular | 1 billion 4 s sounds | **Yes, 16,200× realtime on one GPU** | Built to remove the render bottleneck |
| Masuda & Saito (2021; TASLP 2023) | Differentiable subtractive | Synthetic + out-of-domain audio | Synthetic part | Spectral alone "ineffective". The 2023 recipe: parameter loss on synthetic data, spectral loss on out-of-domain audio. Oscillator frequency and chorus delay are "poorly optimised by a spectral loss" (via the 2024 DDSP review) |
| Sound2Synth (IJCAI 2022) | Dexed | Rendered | No | Parameter loss |
| Perceptual-neural-physical (ICASSP 2023 + follow-up) | Drum model | Synthetic | Precomputed Jacobians | **Parameter loss weighted by the perceptual Jacobian**: as fast as a parameter loss; the follow-up reports about 100× faster than DDSP-style descent |
| AST matching (DAFx 2024) | Massive, 16 parameters | Random renders | Not stated | MSE; the transformer beat an MLP and a CNN at more data and compute |
| Symmetric flow matching (ISMIR 2025) | Surge XT | **2 × 2 M precomputed renders** | No | Flow matching in parameter space; 1 M steps × 128; ablations deferred |
| FM shared embeddings (DAFx 2026) | DX7, 32 routings | Rendered | No | **A GNN encoder that mirrors FM routing generalises to held-out routings** |
| Loss studies (2025, 2026) | Differentiable synths | Synthetic | Yes | The best loss depends on the synthesis method |

**For us:**

- **On-the-fly data.** Only GPU or differentiable synths generate on the fly. Our engine is deterministic CPU Rust, and r3.md rules out a differentiable copy. At about 220 renders/s against about 3.5 k samples/s of training, on-the-fly data would idle the GPU about 94 % of the time (measured arithmetic). So reuse a frozen pool for T1 and T2, and refresh it (TR2's rolling cache) only at T3. An 8× repetition is normal in the field: the ISMIR 2025 Surge XT work saw each render about 64 times (arithmetic from its numbers).
- **Parameter loss against audio loss.** Parameter losses are "relatively efficient", spectral gradients say little about frequency, and hybrids help out of domain (via the DDSP review). Our JND-unit Huber loss is a crude perceptual weighting. Render-reward fine-tuning (SynthRL, DDSynth-RL in r3.md) needs renders per step: a slow later stage, never part of the fast loop.
- **Curriculum over patch complexity.** No sound-matching paper measures one. In general, curricula help mainly under a limited budget or with noisy labels (ICLR 2021), which is exactly the short-proxy regime: **a curriculum may win at T1 and vanish at T3.** Route it through T1-ratio, then T2.
- **How these groups iterate.** No paper read describes a proxy protocol, and the ISMIR 2025 paper defers its ablations. There is no field practice to copy.

## 4. Growing the model when the synth grows

S2 turns layout 1 (102 values per hypothesis) into a layout 2 with more rows and a new catalogue hash. The options, with documented evidence:

- **From scratch per layout.** An overnight T3 each time; fine for rare major bumps.
- **Warm-start the trunk and add output rows (first choice).** Net2Net's function-preserving transforms are the principle, and the log-mel input is unchanged, so the trunk's features transfer.
- **Widen without changing the function** (Net2WiderNet, LiGO, GradMax), when capacity runs short. LiGO saves up to 50 %: 44.7 % of FLOPs for BERT-Base grown from half its size.
- **LR re-warm, re-decay and replay.** These match retraining on the union "using only a fraction of the compute" (405 M and 10 B LLMs, weak and strong shifts). Replay is free for us: the sampler can always draw old-style patches.
- **LwF, EWC and iCaRL exemplars** protect old tasks when old data is gone. Not needed here.
- **Shrink-and-perturb.** A warm start reaches the same training loss but generalises worse. Shrinking the weights and adding noise closes the gap; a replication partly confirms it and finds augmentation reduces it. A cheap hedge to test at T1.
- **Module-keyed heads or a graph decoder.** The GNN result above suggests module-structured models generalise to new structures. This is the longer-term way to make "add a module" a local change; our loss already masks absent modules.

**Savings:** about 40–50 % in model-growth papers (bert2BERT's similar figure was not verified), about 50 % from reusing runs (WSD), and "a fraction" for continual pre-training. **No sound-matching paper measures it.**

**A recipe for layout 2:**

1. **Map rows.**
   - Old modules keep their rows; transform a row if its range or normalisation changes, so its decoded value is unchanged.
   - New rows start at zero weight, with biases that make presence and structure logits strongly negative (absent).
   - The warm model then decodes every old patch as v0-b did.
2. **Data.** The layout-2 sampler, with old-only patches kept at about their old share (rehearsal). The masked loss trains new rows only where their module is present.
3. **Schedule.** Re-warm the LR (start at about half the original peak; tune at T1), use WSD, and anneal WTA by the fixed-sample rule.
4. **One check per major bump.** At T2, warm against scratch on the same pool: compute to reach scratch's final dev loss, and dev loss on old-module-only samples. Adopt warm starts if they reach it at ≤ 50 % of the compute without old-module regression beyond seed noise.
5. **If old-module loss regresses**, widen with a function-preserving transform instead of restarting.

## 5. Small-sample statistics for decisions

### 5.1 Dev loss: paired and continuous

- **The statistic.** d_i = loss_B(i) − loss_A(i), each sample's best-of-8 loss. Report:
  - mean(d) with a paired bootstrap CI (the losses are heavy-tailed);
  - a Wilcoxon signed-rank test;
  - the share of samples improved.

  Store the 9,324-value vector at every evaluation (2 s).
- **Sample size.** n = (z₁₋α + z₁₋β)²(σ_d/δ)², about 6.2·(σ_d/δ)² (one-sided α = 0.05, 80 % power; measured arithmetic):

  | σ_d/δ | 5 | 10 | 20 | 40 |
  | --- | ---: | ---: | ---: | ---: |
  | n | 155 | 618 | 2,473 | 9,892 |

- **Estimating σ_d.** Evaluate the A and B checkpoints once each on dev and take the SD of d_i. Without that, σ_d = σ·√(2(1 − ρ)), with σ the per-sample SD and ρ the correlation of the two models' per-sample losses. With assumed values σ = 1.5 and ρ = 0.9, σ_d = 0.67: detecting A–B's 0.017 needs about 9,600 samples, 0.035 about 2,300, and 0.07 about 570. So A–B's 1.2 % is about the limit of our dev set.
- **Two noises.** Var(Δ̂) is σ_d²/n **plus** run-to-run variance (seed, order, non-deterministic kernels); the paired test sees only the first.
  - Measure s_run with 3 seeds of the T1 baseline; for independent runs, s_Δ ≈ √2·s_run.
  - Decide only when |Δ| > 2·√(σ_d²/n + s_Δ²).
  - Common random numbers lower the noise but condition on one seed, so confirm finalists with a second seed pair.
- **Repeated looks.** Stopping when ahead inflates false positives, and consecutive evaluations are autocorrelated: "ahead at 30 of 32" shows a stable ordering, not 30 wins. Three remedies:
  1. decide at pre-registered points (the cooled-down ends);
  2. a group-sequential rule (strict early, near nominal at the end);
  3. always-valid p-values (the mixture SPRT of A/B testing).
- **Kill leniently and screen honestly.** Kill only at a gap of more than 3 s_Δ: a wrongly killed winner is never seen again. The best of k screened variants looks better than it is, so T1 claims nothing and T2 confirms with fresh seeds (Holm's correction if p-values are reported).
- **Never shrink dev for speed** (it costs 2 s). If σ_d/δ is large, grow dev with more `r3/dev` seeds.

### 5.2 Create's pass/fail: McNemar

Only discordant targets count: one-sided exact p = P(X ≥ b), X ~ Bin(n, ½) (measured arithmetic):

| Discordant n | Wins needed for p < 0.05 | Examples |
| ---: | ---: | --- |
| 4 | none possible | 4–0: 0.0625 |
| 5 | 5 | 5–0: 0.031 |
| 8 | 7 | 7–1: 0.035; 6–2: 0.145 |
| 10 | 9 | 9–1: 0.011; 8–2: 0.055 |
| 20 / 30 | 15 / 20 | |

**Power** (exact, one-sided α = 0.05). p_d is the chance a target is discordant; π is the chance a discordant target favours B (measured):

| Targets | p_d | π = 0.7 | 0.8 | 0.9 | 1.0 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 30 | 0.2 | 0.12 | 0.25 | 0.46 | 0.74 |
| 30 | 0.3 | 0.20 | 0.43 | 0.73 | 0.97 |
| 100 | 0.2 | 0.49 | 0.83 | 0.99 | 1.00 |
| 200 | 0.1 | 0.49 | 0.83 | 0.98 | 1.00 |

The 30 GR3 dev targets detect an effect like 250 renders' 0/8 split (power about 0.95) but not a subtle recipe change. Dev loss ranks recipes and create gates.

To make create more informative:

- add 100–200 **synthetic generator targets** (disjoint from GR3's test half) at 250, 500 and 1,000 renders. At the measured 186 renders/s with 6 workers, 500 renders take about 3 s, so 200 targets take about 10 min;
- read paired continuous measures (Wilcoxon on JND and on objective distance);
- read the area under each target's best-so-far curve;
- treat target families (wobble machines, risers) as clusters.

## 6. A recipe ladder

Throughput (measured): about 3.4 k samples/s on the 2080 Ti machine, 1.9 k on the 1660 Ti machine. **Both arms of a comparison run on the same machine.**

| Tier | Budget | Runs | Data | May decide | May not |
| --- | --- | --- | --- | --- | --- |
| **T0** | ≤ 5 min (about 0.5 M samples) | Overfit one batch; 200 real steps; mask and layout checks; throughput within ±10 % of the last run | Pool | Go or no-go (crash, NaN, a loss that does not fall, a slowdown) | Any ranking |
| **T1** | ≤ 30 min per arm (4 M samples: about 20 min on the 2080 Ti machine, 35 on the 1660 Ti) | WSD (1 k warmup, constant LR, 20 % `1 − sqrt` cooldown), **T1-short** or **T1-ratio**, common random numbers. With ≥ 4 variants, successive halving on cooldown branches at 1, 2 and 4 M | Frozen ~2.07 M `r3/train` pool; its 500 k subset | Kill changes worse by > 3 s_Δ; promote the top 1–3; pick a sweep value | Ship; claim a gain under 2 s_Δ |
| **T2** | ≤ 2 h per comparison | Full width; a 16 M WSD trunk with cooldowns at 4, 8 and 16 M (about 80–95 min on the 2080 Ti machine). The baseline's T2 run is reused while code and pool are unchanged, so a comparison costs one arm. Second seed for close calls | Frozen pool | The recipe for T3, by a pre-registered paired rule with seed noise; warm against scratch for a new layout | Ship |
| **T3** | Overnight | New renders and augmentation (`produce`, `augproduce`), the full or a longer run, export, parity, create on the 30 GR3 dev targets plus 200 synthetic targets; GR3's test half only for releases | Fresh | Ship | — |

**Validating the ladder:**

1. **Replay A vs B at T1, then at T2 with WSD.** Does B stay about 1 % ahead? A–B changes the WTA selection, so this also tests the anneal coupling.
2. **Measure seed noise:** 3 seeds at T1 (about 1–1.5 h) and 2 at T2.
3. **Forward-calibrate** the next 5 or more changes at both T1 and T2, whatever T1 says. T1 keeps its kill power only if its sign agrees with T2's at least 80 % of the time for |Δ| > 2 s_Δ (DataDecide's 80 % as a reference). Otherwise it kills only gross failures.
4. **Keep a decision log** (tier, knob, Δ, CI, verdict); re-check after layout 2 or any trunk change.
5. **On the next shipped model, check T2's ordering against T3** (fresh data, create).

## Recommendations for enth

1. **WSD with cooldown branches**, plus a command that branches a cooldown from any checkpoint using TR2's exact-resume state.
2. **Decouple the WTA anneal from `schedule_steps`**, and express weight decay as a timescale in samples (now 2.56 M at peak LR). Keep both fixed across tiers unless they are the knob.
3. **Freeze a proxy pool** (the ~2.07 M rendered samples and a fixed 500 k subset). Rendering is a T3 activity.
4. **Store per-sample dev losses** and add a `compare` report: mean Δ, bootstrap CI, Wilcoxon, share improved, and a seed-aware verdict.
5. **Measure seed noise and run the A/B calibration** before deciding anything near 1–2 %.
6. **Route knobs:** architecture, loss and heads to T1-short; augmentation, decay, mixture, curricula and filtering to T1-ratio, then T2.
7. **Add 100–200 synthetic create targets** at 250, 500 and 1,000 renders, with McNemar, Wilcoxon and best-so-far area. Keep the 30 GR3 dev targets as the gate.
8. **Fit L(D) at 4 pool sizes** before the next render budget.
9. **Warm-start layout 2** as in §4, with one warm-against-scratch check at T2.
10. **Skip muP** unless the trunk grows about 4× wider or the architecture changes family. Use curve extrapolation and successive halving only to kill.

## Gaps

- **R3b's seed and run-to-run noise is unmeasured**, and every threshold here depends on it.
- **Half-width throughput** on either GPU machine is unmeasured.
- **WSD with WTA and confidence heads** is untested anywhere I found.
- **No sound-matching paper** reports a proxy protocol, a warm-start saving, or a curriculum over patch complexity. The guidance transfers from image classification, NAS and LLMs.
- **muP for a small CNN regressor** with multi-hypothesis heads: no direct evidence (TP V's ResNets are SGD classifiers).
- **Read only through abstracts, pages or a review:** DataDecide, LCDB 1.1, the continual pre-training paper, LC-PFN, Masuda & Saito.
- **Known only from summary pages:** RegMix's correlations, the 10⁴-seed spread and bert2BERT's figure.
- **Not settled:** NAS-Bench-201's excerpt does not name its correlation type, and no DataComp cross-scale rank correlation was found.

## Sources

Proxies, scaling and schedules:
- DataDecide (ICML 2025): https://arxiv.org/abs/2504.11393
- EcoNAS (CVPR 2020): https://arxiv.org/abs/2001.01233
- NAS-Bench-201 (ICLR 2020), Table 6: https://arxiv.org/abs/2001.00326
- Scaling laws (2020): https://arxiv.org/abs/2001.08361
- Chinchilla (2022): https://arxiv.org/abs/2203.15556
- Scaling is predictable (2017): https://arxiv.org/abs/1712.00409
- Scaling-law estimation guide (ICML 2025): https://arxiv.org/abs/2410.11840
- Revisiting scaling laws (NeurIPS 2022): https://arxiv.org/abs/2209.06640
- Resolving compute-optimal discrepancies (NeurIPS 2024): https://arxiv.org/abs/2406.19146
- Tensor Programs V (2022): https://arxiv.org/abs/2203.03466
- μP LR transfer study (2024): https://arxiv.org/abs/2404.05728
- u-μP (2024): https://arxiv.org/abs/2407.17465
- Scaling exponents across parameterizations (ICML 2024): https://arxiv.org/abs/2407.05872
- AdamW weight decay scaling (ICML 2025): https://arxiv.org/abs/2405.13698
- Power lines (2025): https://arxiv.org/abs/2505.13738
- Learning-curve extrapolation (IJCAI 2015): https://www.ijcai.org/Abstract/15/487
- Freeze-thaw BO (2014): https://arxiv.org/abs/1406.3896
- LC-PFN (NeurIPS 2023): https://arxiv.org/abs/2310.20447
- LCDB 1.1 (NeurIPS 2025): https://arxiv.org/abs/2505.15657
- Successive halving (2015): https://arxiv.org/abs/1502.07943
- Hyperband (2016): https://arxiv.org/abs/1603.06560
- ASHA (2018): https://arxiv.org/abs/1810.05934
- One epoch is all you need? (ESANN 2023): https://arxiv.org/abs/2307.15422
- Short-horizon bias (ICLR 2018): https://arxiv.org/abs/1803.02021
- DataComp (2023): https://arxiv.org/abs/2304.14108
- DsDm (2024): https://arxiv.org/abs/2401.12926
- RegMix (2024): https://arxiv.org/abs/2407.01492, and its follow-up https://arxiv.org/abs/2502.15950
- Data-filtering scaling laws (CVPR 2024): https://arxiv.org/abs/2404.07177
- AugReg (TMLR 2022), §4.4: https://arxiv.org/abs/2106.10270
- WSD and cooldowns (NeurIPS 2024): https://arxiv.org/abs/2405.18392
- MiniCPM (WSD, 2024): https://arxiv.org/abs/2404.06395
- Schedule-free (NeurIPS 2024): https://arxiv.org/abs/2405.15682
- LAWA (2022): https://arxiv.org/abs/2209.14981
- When do curricula work? (ICLR 2021): https://arxiv.org/abs/2012.03107

Sound matching:
- InverSynth: https://arxiv.org/abs/1812.06349
- InverSynth II: https://ismir2023program.ismir.net/poster_209.html
- DDSP review (Frontiers 2024): https://www.frontiersin.org/journals/signal-processing/articles/10.3389/frsip.2023.1284100/full
- Inverse-synthesis pitch: https://research.google/pubs/self-supervised-pitch-detection-by-inverse-audio-synthesis/
- torchsynth: https://arxiv.org/abs/2104.12922
- Masuda & Saito, IEEE/ACM TASLP 31:863–875 (2023), read through the DDSP review
- Sound2Synth: https://arxiv.org/abs/2205.03043
- Perceptual-neural-physical: https://arxiv.org/abs/2301.02886, and its follow-up https://arxiv.org/abs/2311.14213
- AST matching: https://arxiv.org/abs/2407.16643
- Symmetric flow matching (ISMIR 2025): https://arxiv.org/abs/2506.07199
- FM shared embeddings (DAFx 2026): https://arxiv.org/abs/2608.18226
- Loss studies: https://arxiv.org/abs/2506.22628 and https://arxiv.org/abs/2608.27698

Growing and continual learning:
- Net2Net: https://arxiv.org/abs/1511.05641
- LiGO: https://arxiv.org/abs/2303.00980
- GradMax: https://arxiv.org/abs/2201.05125
- Continual pre-training: https://arxiv.org/abs/2403.08763
- Warm-starting: https://arxiv.org/abs/1910.08475
- LwF: https://arxiv.org/abs/1606.09282
- EWC: https://arxiv.org/abs/1612.00796
- iCaRL: https://arxiv.org/abs/1611.07725

Statistics:
- Error bars for evals (2024): https://arxiv.org/abs/2411.00640
- Variance in ML benchmarks (MLSys 2021): https://arxiv.org/abs/2103.03098
- Seed scan (2021): https://arxiv.org/abs/2109.08203
- Power in NLP (2020): https://arxiv.org/abs/2010.06595
- Statistical precipice (2021): https://arxiv.org/abs/2108.13264
- Always-valid inference: https://arxiv.org/abs/1512.04922

Repository facts: docs/r3.md (R3b sections), ml/README.md, `ml/enth_ml/train/full-v0-b.toml`, brief.md. The scratch folder holds `01/notes.md`, the extracted paper text in `01/papers/` and `01/scripts/stats.py`.
