# 01: Prior art and models (R3 Sound → patch)

- **Sub-researcher:** 01 · **Date:** 2026-10-21 · **Brief:** [brief.md](brief.md), axis 1
- **Labels:** *documented* = the authors' own numbers or statements (not reproduced by us); *measured* = checked by me today (licence files, commit dates); *anecdotal* = reviews and forums.
- **Method:** web search, arXiv and proceedings pages, papers read as PDF text where results mattered, repository health checked without authentication (the GitHub API rate-limited me after about 20 calls, so the later checks read the raw `LICENSE` file and the repository's `commits.atom` feed). Scratch: `~/code/enth-scratch/2026-10-21-r3/01/notes.md`.

## TL;DR

1. **No paper and no product turns "any real sound" into a patch well.** All published work trains on the synth's own renders, and out-of-domain scores drop sharply (Hayes 2025: spectral error 6.1 in-domain against 11.0 on NSynth and 15.4 on FSD50K). **Nobody has tested degraded uploads** (codec, noise, reverb, clipping), so R3's robustness goal is new ground.
2. **Permutation symmetry is why point regression fails.** The state of the art is generative: equivariant flow matching (ISMIR 2025 best student paper), discrete diffusion with render-reward fine-tuning (DDSynth-RL, ISMIR 2026), and joint audio–parameter flows for real audio (Hayes, Sept. 2026).
3. **Model + search at equal time has a single test, small and old:** a network warm start matched a GA's error in 40 % of its time (Shier 2021, 7 parameters). Synplant 2 publishes no method.
4. **Our setup suits a narrow model.** We have a fast deterministic renderer, CMA-ES, observability masks and JND steps. The model only has to propose **a few diverse starts**, above all for the categorical and structural choices where CMA-ES fails (level 5: 0 of 4 GR1 codes).
5. **First model:** a small CNN on our own `enth-analysis` features with mixed heads, **K = 8 winner-takes-all hypotheses** and a confidence head. The loss is masked by observability, uses canonical slots and is weighted in JND units. Every hypothesis is rendered, ranked and refined by CMA-ES, with the heuristic start always kept. It must first beat **k-NN retrieval over pre-rendered patches** and CMA-ES alone. Upgrades: flow matching, then render-reward fine-tuning on real audio.

## 1. Our problem in the literature's terms

- **Our space** (`enth-patch` catalogue, starter superset): 2 oscillators (wave enum of 5, semi, fine, level, pulse width, sync, table enum of 5, position), noise (colour, level), a state-variable filter (mode enum of 4, cutoff on a log scale, resonance, keytrack, drive), 2 ADSRs, 2 LFOs (shape enum of 7, rate, polarity, scope), reverb (10 parameters), compressor (7), voice and master, enable flags, and a modulation matrix of routes with depths. Roughly 60–90 live parameters, about a quarter of them discrete.
- **Its closest published analogue is Surge XT "Simple"** (92 parameters: oscillators, filters, envelopes, LFOs; discrete and routing controls excluded) and "Full" (165, many discrete, with routing and effects) in [Hayes et al. 2025](https://arxiv.org/abs/2506.07199). NI's Massive with 16 continuous parameters ([Bruford et al. 2024](https://arxiv.org/abs/2407.16643)) is close to our levels 2–3. Dexed (DX7 FM, 144–156 parameters) dominates the literature, but it is a different problem: FM ratios and the choice of algorithm.
- **Symmetries we have:** osc1 ↔ osc2 (broken only by `sync` on the first oscillator), lfo1 ↔ lfo2, the route set (unordered), and gauges such as the level of a lone source or `master.level`. Dead parameters are already defined by `observable()` (design.md §9). Quasi-symmetries also exist: detune sign, and the choice between filter keytrack and cutoff on a single note.
- **Where search fails today** (*measured* by lane INT, [r1s1-gates.md](../../../docs/r1s1-gates.md)): CMA-ES solves 12 of 20 GR1 codes, 3 of 4 at each of levels 1–4 and **0 of 4 at level 5** (structure and extra modules). Seed 1004 fails at every level, probably on its oscillator draws. Categorical and structural choices are the gap a model should fill.

## 2. Approaches 2018–2026

| Family | Representative work | What they report (*documented*) |
| --- | --- | --- |
| **Search only** | Roth & Yee-King, AES 2011 ([link](https://research.gold.ac.uk/19132)); Masuda & Saito, quality diversity, [DAFx 2021](https://dafx.de/paper-archive/details/UR_KEeXEtkRfUcNkctdbVw); INSTRUMENTAL, [arXiv 2603.15905](https://arxiv.org/abs/2603.15905) (Mar 2026) | The GA was best and the neural net poor (2011); a novelty objective improves the GA and gives diverse matches (2021); CMA-ES on 28 parameters: of 8 tweaks only an EQ boost helped (2026). Search fits the real target directly |
| **Point regression or classification** | InverSynth, TASLP 2019 ([arXiv 1812.06349](https://arxiv.org/abs/1812.06349)); Yee-King et al., TETCI 2018 ([link](https://research.gold.ac.uk/22516)); Sound2Synth, [IJCAI 2022](https://www.ijcai.org/proceedings/2022/682); Bruford et al. (NI), DAFx 2024 | A Dexed bi-LSTM matched closely in 25 % of cases and beat a GA. An AST on Massive (16 parameters, 1 M renders) gives parameter MSE 0.031, against 0.077 for an MLP. Out-of-domain: demos only, detune missed |
| **Audio loss through a differentiable synth or proxy** | DDSP (ICLR 2020); DDX7 ([arXiv 2208.06169](https://arxiv.org/abs/2208.06169)); Masuda & Saito (ISMIR 2021, TASLP 2023); DiffMoog ([arXiv 2401.12570](https://arxiv.org/abs/2401.12570)); InverSynth II ([ISMIR 2023](https://archives.ismir.net/ismir2023/paper/000076.pdf)); Neural proxies ([arXiv 2509.07635](https://arxiv.org/abs/2509.07635), JAES 2025); PNP ([arXiv 2311.14213](https://arxiv.org/abs/2311.14213), TASLP 2024) | DiffMoog: spectral loss alone "failed systematically", and FM chains did not converge (frequency losses are non-convex, as also shown in [Hayes et al. 2022](https://arxiv.org/abs/2210.14476)). InverSynth II's proxy plus fine-tuning gains little (mel ×100: 56.8 → 53.8 FM, 19.29 → 19.26 DX7). PNP: a Jacobian-weighted parameter loss approximates JTFS at the speed of a parameter loss |
| **Generative, multiple candidates** | FlowSynth ([arXiv 1907.00971](https://arxiv.org/abs/1907.00971)); Le Vaillant et al., DAFx 2021; **Hayes et al. 2025**; Peladeau et al., [DAFx 2025](https://www.dafx.de/paper-archive/details/tk0fwnF73tSMahxxOV9zPg); **DDSynth-RL** ([arXiv 2608.03032](https://arxiv.org/abs/2608.03032), ISMIR 2026); **Hayes 2026** ([arXiv 2609.29320](https://arxiv.org/abs/2609.29320)) | §3.1–3.4 |
| **Render-reward RL** | SynthRL ([IJCAI 2025](https://www.ijcai.org/proceedings/2025/1129)); the GRPO stage of DDSynth-RL | Label-free fine-tuning on real instrument audio improves out-of-domain matching |
| **Retrieval** | Braun & Finkelstein, [DAFx 2026](https://www.dafx.de/paper-archive/2026/papers/DAFx26_paper_33.pdf); "data-driven" nearest neighbour (Roth & Yee-King 2011) | A contrastive audio–parameter embedding retrieves DX7 presets, including held-out topologies; nearest neighbour came near the GA in 2011 |
| **Model + search** | Shier, MSc thesis 2021 ([UVic](https://dspace.library.uvic.ca/handle/1828/13593)); InverSynth II's inference-time fine-tuning; Synplant 2 | §3.5 |

## 3. Findings by question

### 3.1 Permutation symmetry and set prediction

- **The problem is formal.** Unordered targets force discontinuous output assignments ("the responsibility problem": [Zhang et al. 2019](https://arxiv.org/abs/1906.06565); [Hayes et al. 2023](https://arxiv.org/abs/2304.09499)).
- **Hayes 2025, k-osc task** (*documented*): with symmetric oscillators, plain MSE regression does poorly. Sorting targets, a Chamfer (permutation-invariant) loss and a non-equivariant flow are "partial solutions", and an equivariant flow is best. Without symmetry the ranking reverses, and forced equivariance hurts. **Param2Tok** learns which parameters to tie and does well in both cases.
- **Surge XT** (Hayes 2025, 2 M random patches, 10 k test sounds, about 40 M-parameter models, *documented*). Multi-scale spectral error:

| Model | Simple (92) | Full (165) | NSynth, out-of-domain | FSD50K, out-of-domain |
| --- | --- | --- | --- | --- |
| Flow, Param2Tok | **3.18** | **6.13** | **11.04** | **15.40** |
| Flow, MLP | 3.53 | 7.35 | n/a | n/a |
| AST regression (Bruford) | 6.51 | 14.73 | 24.04 | 19.09 |
| VAE + RealNVP | 26.23 | 28.86 | n/a | n/a |

- **What this means for us:** with only 2 oscillators and 2 LFOs, canonicalisation (sort by pitch, then level; sort LFOs by rate) or a min-over-two-permutations loss is cheap. The routes are better encoded as a fixed (source × destination) presence-and-depth grid than as a list, which removes slot symmetry altogether. The residual quasi-symmetries are left to K hypotheses plus CMA-ES. Hayes tested 4–32 symmetric oscillators; with our 2, learned equivariance should matter less (*inference, not measured*).

### 3.2 Multiple candidates

- **Generative samplers** (flows, diffusion) give K candidates by sampling. Their cost is 20–100 network evaluations per sample: Hayes 2025 uses 100 RK4 steps with classifier-free guidance, Hayes 2026 uses 20 Euler steps, and DDSynth-RL decodes 105 sequential steps and samples K = 8 for GRPO. **None of these papers reports wall-clock inference time.**
- **Multiple-choice learning** (K output heads, winner-takes-all loss) gives K hypotheses in **one forward pass**. Its known failure is collapse or greedy local minima, addressed by resilient MCL's learned scoring ([Letzelter et al., arXiv 2311.01052](https://arxiv.org/abs/2311.01052)) and annealed MCL ([Perera et al., arXiv 2407.15580](https://arxiv.org/abs/2407.15580), NeurIPS 2024). **I found no synth-matching paper that uses it** (gap).
- **Diversity in search:** Masuda & Saito's quality-diversity GA finds diverse matches by design. Synplant shows **four strands** of candidates that converge over time (*documented* by reviews, method undisclosed).
- **Peladeau et al. 2025:** per-parameter distributions with an entropy term "output multiple combinations" for one sound (two numerical experiments, details not read).

### 3.3 Discrete plus continuous parameters

- **Three published encodings:** (a) quantise everything into classes (InverSynth; DDSynth-RL uses 25 bins per continuous control over 102 Dexed tokens); (b) mixed heads, softmax for categorical and regression for numeric (Le Vaillant 2021, InverSynth II's split losses); (c) one-hot relaxation inside a continuous flow (Hayes 2025). DDSynth-RL argues that (c) mismatches categorical controls and that autoregressive models depend on a hand-picked order. Its masked discrete diffusion removes the order dependence.
- **On Dexed** (DDSynth-RL Table 1, 200 test examples, *documented*): in-domain multi-scale spectral error is 3.41 for autoregressive, 3.70 for discrete diffusion and 4.66 for flow matching. **The flow-matching advantage did not carry over to a discrete-heavy FM synth.**
- **For us:** discrete choices (wave, filter mode, LFO shape, enable flags, route presence) are exactly where CMA-ES is weakest (it cycles categoricals at restarts; CatCMA, [arXiv 2405.09962](https://arxiv.org/abs/2405.09962), is the search-side fix). So the model's **categorical heads matter more than its continuous accuracy.**

### 3.4 Out-of-domain and real sounds

| Evidence | Result (*documented* unless marked) |
| --- | --- |
| Hayes 2025 | Every model degrades sharply on NSynth and FSD50K; the ranking mostly holds |
| Hayes 2026, JDF | A joint flow plus **partial noising of the input at inference** (best at τ ≈ 0.1) cuts real-audio spectral error against a conditional flow from 15.50 to 7.68 on Surge and from 13.03 to 7.98 on Dexed. **About 617 k unpaired real clips added almost nothing.** No degraded-audio or listening tests |
| DDSynth-RL | GRPO with render rewards (MFCC, CLAP, CREPE, spectral, RMS) on unlabelled NSynth: out-of-domain error 4.41 → 3.11, **in-domain 3.70 → 4.48 (worse)** |
| SynthRL | RL on instrument recordings beats prior work in and out of domain (abstract only) |
| Bruford 2024 | Timbre and envelope "broadly captured", detune missed (subjective) |
| Synplant 2 reviews | Synth sources come "close to bang-on"; non-synthetic ones are less accurate; analysis is mono-only (*anecdotal*: [MusicRadar](https://www.musicradar.com/reviews/sonic-charge-synplant-2-review), [CDM 2023-10-16](https://cdm.link/2023/10/the-road-to-synplant-2/)) |
| Degraded uploads (MP3, noise, reverb, clipping) | **No study found** |

So unlabelled real audio helps when it trains *through the renderer* (rewards) and barely helps as a passive marginal. The roadmap's "Unlabelled real recordings train the audio side" is a hypothesis to test, not a plan. Search on the real target stays our main out-of-domain tool.

### 3.5 Model + search against search alone

- **Shier 2021, ch. 5** (*documented*; Dexed reduced to 7 parameters, a 2-operator FM synth, 250 test sounds). The warm start seeds NSGA-III with one LSTM prediction, 49 mutations of it and 50 random individuals, then runs 10 generations (about 30 s). The plain NSGA-III ran 25 generations (about 75 s).

| Method | MFCC MAE | Log-spectral distance | Parameter MAE |
| --- | --- | --- | --- |
| LSTM++ alone (~70 ms) | 4.1 | 79.9 | 0.148 |
| NSGA-III (~75 s) | 1.4 | 72.5 | 0.212 |
| Warm-start NSGA (~30 s) | **1.3** | 72.6 | 0.151 |

  So the hybrid gave "similar results ... in 40 % of the computation time". It also found that MFCC input beat mel input for every network.
- **InverSynth II:** 30 rounds of per-sound gradient fine-tuning through a learned proxy give a small gain.
- **Synplant:** a small CPU network trained for months on "semi-random sounds generated by the synth itself", whose early guesses are "pretty wild" and then improve ([CDM](https://cdm.link/2023/10/the-road-to-synplant-2/), *documented* by interview). This suggests a proposal-plus-refinement loop, but it is not confirmed.
- **No equal-wall-clock comparison against CMA-ES with a modern network exists.** GR3's in-domain test will be new evidence.

### 3.6 Scale reported (*documented*)

The training sets are 0.9–2 M renders (Hayes 2025: 2 M random Surge patches; Bruford: 1 M Massive one-shots from preset-fitted priors, about 24 h to render on a 2018 Mac mini; DDSynth-RL: 860 k perturbed Dexed presets). The models have 40 M or more parameters (AST or DiT); Shier's LSTM took about 70 ms per inference. Training compute is rarely stated. At our 7.5 ms per evaluation on one thread (toy synth), 1 M renders take hours here, not days (axis 2 sizes it).

## 4. Commercial products

| Product | What it does | Evidence |
| --- | --- | --- |
| **Sonic Charge Synplant 2, Genopatch** (released Oct 2023; €149 on [soniccharge.com](https://soniccharge.com/synplant) today) | Up to 2 s of a sample (summed to mono) → four strands of patches that improve over time, all on the local CPU, 48 "genes". Genopatch 1.0 (announced Mar 2023) was single-note | Method undisclosed: "a combination of different machine-learning algorithms", trained on the synth's own renders (*documented* by interview; quality *anecdotal*) |
| **MicroMusic / Replicate** | Audio → Vital preset (Serum too, according to one listing) | Free on Windows in May 2024 ([Synthtopia](https://www.synthtopia.com/content/2024/05/13/micromusic-ai-powered-synth-preset-generator-recreates-patches-from-audio)); paid plans in a Dec 2025 listing. Method and quality unknown (*anecdotal*) |
| **Tensynth** (OCAD University student project, May 2026) | Audio → 69 parameters, text → 100+ | Abstract only; the PDF is embargoed until 2027 |

No major vendor ships sound-to-patch for its own synth as of today (*search found none*; Native Instruments publishes research, Bruford 2024, with no product). Synplant's UX (several candidates as branches, refined live) is the reference the design already names.

## 5. Open-source code and licences (checked 2026-10-21, *measured*)

| Repository | Licence (from the file) | Last commit | Use for us |
| --- | --- | --- | --- |
| [ben-hayes/synth-permutations](https://github.com/ben-hayes/synth-permutations) (Hayes 2025) | MIT | 2025-10-10 | **Fine in `ml/`**: the flow, Param2Tok and DiT reference |
| [DDSynth-RL/DDSynthRL](https://github.com/DDSynth-RL/DDSynthRL) | Apache-2.0 | 2026-09-13 | Fine in `ml/`: discrete diffusion, GRPO |
| [christhetree/mod_discovery](https://github.com/christhetree/mod_discovery) | MPL-2.0 | 2026-03-25 | `ml/` only, after the owner's OK |
| [gwendal-lv/spinvae](https://github.com/gwendal-lv/spinvae) | **AGPL-3.0** | 2023-05-18 | **Reference only** |
| [inversynth/InverSynth2](https://github.com/inversynth/InverSynth2), [pcmbs/synth-proxy](https://github.com/pcmbs/synth-proxy), [Sound2Synth-Plug-Ins](https://github.com/Sound2Synth/Sound2Synth-Plug-Ins) (a Dexed fork, so GPL-derived) | **no licence file** | 2023-04-20, 2025-10-02, 2022-07-28 | **Reference only** |

These are permissive and usable as reference (licence, last commit): [magenta/ddsp](https://github.com/magenta/ddsp) Apache-2.0, 2026-10-19; [torchsynth](https://github.com/torchsynth/torchsynth) Apache-2.0, 2024-10-07; [fcaspe/ddx7](https://github.com/fcaspe/ddx7) Apache-2.0, 2022-08-15; [aisynth/diffmoog](https://github.com/aisynth/diffmoog) MIT, 2024-06-18; [hyakuchiki/SSSSM-DDSP](https://github.com/hyakuchiki/SSSSM-DDSP) MIT, 2022-06-10; [acids-ircam/flow_synthesizer](https://github.com/acids-ircam/flow_synthesizer) MIT, 2020-08-03; [gwendal-lv/preset-gen-vae](https://github.com/gwendal-lv/preset-gen-vae) MIT, 2026-07-22; [spiegelib](https://github.com/spiegelib/spiegelib) MIT, 2021-02-10; [PapayaResearch/synthax](https://github.com/PapayaResearch/synthax) MIT, 2024-03-29.

- **No published weights transfer to us:** every model is tied to one synth's parameter space, so we train our own.
- `ml/` lives in a public MIT repository, so the copy rule applies there too: permissive code only, even though `ml/` never ships.

## 6. What this means for enth

1. **Build no differentiable twin of the engine.** It would duplicate every module in PyTorch, break "one deterministic engine", and still fail on frequency and FM parameters (DiffMoog; Hayes 2022). Proxies gain little (InverSynth II). Use **perceptually weighted parameter losses** (the JND steps in the ADR-0007 note, or §7's JᵀJ; the idea behind PNP), then **render-reward fine-tuning**, which costs about 7.5 ms per reward with our renderer.
2. **The model's job is narrow.** It proposes categoricals, structure and the values the heuristic misses (resonance, filter-envelope decay). CMA-ES on the real target does the fitting, and it is also our best out-of-domain tool.
3. **Use the same features for training and inference:** `enth-analysis` in Rust through `enth-py`, so there is no skew between Python and Rust features.
4. **Reuse `observable()` and the canonical hash** for loss masks and canonical targets.
5. **Determinism:** snap proposals to the generator's reachable grid, so small cross-platform differences in inference rarely change a start. Whether bit-exact inference is needed is axis 3's call.

## 7. Recommendation: the first model ("propose v0")

| Part | Choice | Why |
| --- | --- | --- |
| Input | ~2 s from onset, loudness-normalised to −23 LUFS; `enth-analysis` log-mel (64 bands, 30 Hz–16 kHz, v1.1 pooling, 2–3 resolutions), plus f0 and loudness tracks. Pitch-relative: render at the detected note, predict `semi` relative to it | The same features as `sound/1`. Shier found MFCC beat mel for every network: worth an ablation |
| Encoder | Small 2D CNN, about 1–5 M parameters; also exports its embedding for retrieval | Fast with `tract` on a CPU (axis 3 to measure); Synplant ships "a small net on your CPU" |
| Heads | Softmax for each enum, toggle and enable flag; sigmoid route presence on a fixed (source × destination) grid; continuous values in the `ParamSpec` normalised domain | The routes grid removes slot symmetry; the normalised domain is already the ML boundary (design.md §1) |
| Multiple candidates | **K = 8 hypothesis head sets, winner-takes-all with annealing or relaxation, plus a confidence head** (rMCL-style) to rank before rendering | One forward pass, deterministic, simple ONNX graph; multimodality where it matters |
| Loss | Masked by `observable()`; symmetric slots canonicalised (oscillators by pitch then level, LFOs by rate) or min over the two permutations; continuous error in **JND units** (25 ct, 10 % for times and Hz, 2 dB), with a Huber loss; cross-entropy for discrete heads | Perceptual weighting without a differentiable synth (the PNP idea); trains nothing on what cannot be heard |
| Data | Generator v1 renders, levels 1–5, `musical` 50–100, with the augmentation chain (axis 2) | One prior for game, agent and ML (design.md §9) |
| Pipeline | Render all K (K evaluations) → rank by `sound/1` → CMA-ES from the top 2–4 with a reduced σ; **the heuristic start is always one of the candidates**; the wall-clock cap includes inference | Never worse than R3a; equal-time fairness for GR3 |
| Baselines to beat | (a) CMA-ES from the heuristic start at equal wall-clock (GR3); (b) **k-NN retrieval** over 50–200 k pre-rendered generator patches using the encoder or PCA'd pooled mel (25 MB at 50 k × 128 f32); (c) the model without search | (b) needs no training and gives several candidates for R3a; Roth & Yee-King found the data-driven method strong |

**Why not flows or diffusion first?** They are the best published for symmetric spaces (Hayes 2025). But they use 20–100 network evaluations per sample with models of about 40 M parameters, and their ONNX graphs and sampler loops are more complex. On a discrete-heavy synth, flow matching lost to autoregressive decoding (DDSynth-RL). Our CMA-ES stage also absorbs residual error. If K-head WTA fails the in-domain GR3 test, or its hypotheses collapse, move up.

## 8. Upgrade path (in order)

1. **Conditional flow matching** over the normalised continuous parameters plus one-hot categoricals, with a DiT and Param2Tok-style tokens (MIT reference code). Sample K with few Euler steps (Hayes 2026 used 20). Or **masked discrete diffusion** if the categoricals dominate (Apache reference code).
2. **Render-reward fine-tuning** (GRPO, as in DDSynth-RL or SynthRL) on unlabelled real one-shots and degraded uploads, with `sound/1` plus the pitch and envelope rows as the reward. It is cheap with our renderer. Watch for in-domain regression, which DDSynth-RL saw.
3. **Partial noising of the input at inference** (Hayes 2026) as a cheap robustness trick for off-manifold audio, once a generative model exists.
4. **Structure:** template choice by embedding retrieval (design.md §8 step 4), possibly with a joint audio–parameter contrastive embedding (Braun & Finkelstein 2026).

## 9. Gaps

- **Degraded inputs:** no published sound-matching result on codecs, noise, reverb or clipping.
- **WTA/MCL for synth matching:** untested in the literature.
- **Model + search at equal time:** only Shier 2021 (7 parameters, a GA).
- **Inference cost:** no paper reports CPU wall-clock time for its sampler; axis 3 must measure under `tract`.
- **Undisclosed or unclear:** Synplant's method (the four strands are UI evidence only); MicroMusic's method, price and platforms.
- **Read only as abstract or summary, so their numbers are not quoted:** Masuda & Saito TASLP 2023, SynthRL, Peladeau 2025, Le Vaillant 2021, Braun & Finkelstein 2026.
- **Weak evaluation:** small test sets (DDSynth-RL has 200 per split), almost no listening tests (InverSynth II's MOS with N = 20), and metrics (MSS, wMFCC, SOT) that are not our `sound/1`.
- **GitHub API rate limit:** stars and archived flags were checked only for the first nine repositories; licences and dates come from raw files and Atom feeds.

## 10. Recommendations

1. **R3a first, shippable alone:** heuristic start + CMA-ES from a file, plus **k-NN retrieval over a pre-rendered generator bank** for several candidates. Retrieval is also the baseline every learned model must beat.
2. **First learned model:** the §7 design, with K = 8 winner-takes-all hypotheses, a confidence head, an observability-masked loss in JND units and canonical slots. Render and rank all K, refine the top 2–4 with CMA-ES, and keep the heuristic start.
3. **No differentiable copy of the engine.** Perceptual parameter losses now, render rewards later.
4. **Compute training features in Rust** (`enth-py` calling `enth-analysis`), never in Python.
5. **Judge at equal wall-clock time, inference included,** reported per level (levels 4–5 and categorical-heavy codes are where a model should pay off).
6. **Treat "unlabelled real audio trains the audio side" as a hypothesis:** the 2026 evidence favours rewards through the renderer. Run a small ablation first.
7. **Upgrade** to flow matching or discrete diffusion if WTA collapses or fails GR3 in-domain, then GRPO on real and degraded audio.
8. **Only MIT or Apache code goes into `ml/`.**
