# 04: Evaluation, risks and the R3 lane plan

- Date: 2026-10-21. Sub-researcher 04 of the R3 Sound → patch session ([brief](brief.md)).
- Read: AGENTS.md, docs/README.md, architecture.md, research-process.md, roadmap.md, design.md §5–§9, ADR-0007, listening.md, phase-2.md, gr1.md, dsp/match.md, `content/gr1/`, `xtask/layers.toml`, the CLI's WAV path (`enth-cli/src/cmd/game.rs`, `matching.rs`), the plugin's game worker (`enth-plugin/src/game.rs`).
- Marks: *measured* (run here or in the repo's records), *documented*, *anecdotal*, *estimated* (my arithmetic). Scratch: `~/code/enth-scratch/2026-10-21-r3/04/`.

## 1. Summary

1. **The biggest surprise is measurable today: refinement fits the degradation.** I ran `enth match` on one synth target and four degraded copies, then scored each result against the *clean* target (§2):
   - clean 99 %;
   - MP3 at 64 kbps 92 % (the cutoff dropped from 6.9 to 4.6 kHz to copy the codec's low-pass);
   - 20 dB noise **10 %**;
   - reverb **25 %**;
   - clipping **64 %**.

   CMA-ES reproduces degradations with knobs. In model + search, it would undo what an augmentation-trained model learned. *Measured, n = 1.*
2. **So R3a (create from sound, no ML) needs a robust intake before it ships.** That means trim, noise floor, bandwidth, clipping, inferred performance, and a degradation-aware objective under a new metric id. Trimming the noisy tail alone moved 10 % to 56 % (*measured*).
3. **GR3's degraded-upload test must judge against the clean original.** The system sees only the degraded file. Judging against the upload rewards copying the noise.
4. **Clause 1 (synth, equal time).** Compare R3a's best search with model + search. Time runs on a machine with nothing else running (`flock -x`), with the same workers, start set and free set, and with inference charged to the budget. The headline is pass rate and JND distance at 60 s, paired by target, with anytime curves to 300 s. About **120 held-out targets** are enough (69 detect a 65 % win rate). Factory presets must stay out of prior fitting.
5. **Clause 2 (listening).** Forced choice, "which is closer to the reference?", level-matched, one top candidate per arm, with catch trials. Use 60 references (40 clean in four strata, 20 degraded). The owner's session is about 25 min, and each friend gets a 30-reference kit. It passes if the cluster-bootstrap lower bound is above 50 %.
6. **Clause 3 (5 minutes).** A cap guarantees it. Gate instead on time to the first candidate (≤ 15 s) and on no xruns while a search runs in the DAW. The search needs its own pool that can be cancelled: the plugin's `enth-game` worker is joined on drop, so a 60 s job would hang unload.
7. **The plan.** R3a: 9 lanes in 5 waves, about 14–18 agent-h. R3b: 9 lanes in 4 waves, about 11–16 agent-h plus 1–3 days of training. Four freezes: the held-out sets first, the intake and candidate API, the walking skeleton, the model contract. **R3 holds one heavy build slot**, and R2 keeps the other. Owner time: about 2.5–3.5 h.

## 2. Measurement: today's match on degraded uploads

*Measured* 2026-10-21 with the debug `enth` from main (2026-10-20), at load average about 10.

- **Target:** `pluck_1osc` with cutoff 7 kHz, resonance 20 %, env1 decay 500 ms, sustain 60 %, release 300 ms; C3 held 1.5 s in a 3 s clip.
- **Degradations** (`scripts/degrade.py`, seeded): ffmpeg 6.1.1 + libmp3lame 3.100 at 64 kbps; pink noise at 20 dB SNR; synthetic reverb RT60 1.2 s; clipping after +12 dB.
- **Search:** `enth match --start pluck_1osc --budget 400 --workers 3`, seed 1, 15 axes. The template is the right structure, with no noise or reverb module.
- **Scoring:** `enth compare <result> --target target.patch.json`, i.e. against the clean patch.

| Input | Sound % vs clean | Structure % | What the search did |
| --- | --- | --- | --- |
| Clean | **98.98** (pass) | 77.5 | cutoff 6.90 kHz |
| MP3 64 kbps | **91.92** (pass) | 66.2 | cutoff 4.58 kHz to match the codec low-pass (−45 dB at 12–14 kHz; no offset: ffmpeg honours LAME's gapless tag) |
| Noise 20 dB | **10.27** | 52.1 | cutoff 58 Hz, release 4.46 s |
| Noise, tail trimmed to 1.8 s | **55.98** | 76.3 | cutoff 8.21 kHz; release 720 ms (truth 300) |
| Reverb RT60 1.2 s | **24.87** | 59.1 | release 1.56 s |
| Clip +12 dB | **64.35** | 69.4 | cutoff 51 Hz, drive 6.2 dB, sustain 84 % |

**Limits:**

- One target, one seed, and 400 renders (the default is 1,500).
- A template with noise and reverb modules would fit those directly.
- It is an existence proof, not a rate. Lane OBJ repeats it on 40 targets × 4 degradations.

## 3. Running GR3 honestly

### 3.1 Clause 1: held-out synth sounds, equal time

**Arms.** All arms share the intake, start set (template portfolio), free set, workers and seeds.

| Arm | What | Role |
| --- | --- | --- |
| A, search alone | R3a's best configuration (heuristic, template portfolio, IPOP, budget split over K branches), tuned on dev | Baseline. It must be the best search, not GR1's 1,500-render floor |
| B, model + search | K proposals snapped to a grid (R8), refined from the same budget by successive halving | The claim |
| C, model only | Proposals, unrefined | Diagnostic for R1 |
| D, oracle structure | A, given the true template | Separates structure error from parameter error |

**Equal time.**

- B's clock includes intake, features and inference.
- Timed runs hold `flock -x /tmp/enth-quiet.lock` with no lanes running, at the plugin's worker count (4) and the CLI's (8). Today's load (about 10) would decide by contention, as listening.md warns.
- Each run also logs renders, so a render-equivalent twin (T × idle renders/s, minus inference) reproduces it.
- Scale (*estimated* from `content/gr1/floors.jsonl`, about 200 renders/s on 4 workers): 60 s is about 12,000 renders and 300 s about 60,000. Phase 0's 5,000-evaluation cap was sized on the toy synth.

**Endpoints** (fixed before the first test run):

- **Primary:** at 60 s, pass rate (sound/1 ≥ 90 against the target patch) by McNemar on discordant targets, plus per-target Δz in JND units (Wilcoxon, Hodges–Lehmann CI).
- **Secondary:** anytime curves (median z at 10, 30, 60 and 300 s), time to pass (Kaplan–Meier, censored at 300 s), structure/1, and results per level and per degradation.
- **"Beats":** better at 60 s (one-sided p < 0.05) and not worse at 300 s.
- **Anytime schedules:** budgets that double, as IPOP already does, so one 300 s run gives the whole curve.

**Sets** (frozen and hashed in wave 1, before any training):

| Set | Size | Source | Leak guard |
| --- | --- | --- | --- |
| Generator targets | 60 (12 per level) | generator v1, a seed range training never draws | dedupe against training at z < 1 JND |
| Hand-made | about 50 renders | the 22 `content/presets-candidates` and the owner's 4 G1 sounds, at 2 notes | **never fit priors on them** (design.md §8 plans priors "fitted to real presets"): use another set or a disjoint half |
| Degraded synth | 110 × 4 | held-out chain (§3.2) | scored against the clean target |
| Dev | 30 | disjoint | all tuning |

**Size and cost** (*estimated*):

- A sign test (exact, one-sided α = 0.05, power 0.8) needs **69** targets to detect a 65 % win rate, 37 for 70 % and 158 for 60 %.
- 120 targets × 2 arms × 60 s is about 4 h of exclusive machine time; a 40-target 300 s tail adds about 7 h. Run these overnight, 2–3 full runs at most. Dev runs use render budgets beside other lanes.

**Goodhart guard.** Both arms minimise a log-mel distance, and sound/1 is log-mel, so clause 1 measures search efficiency. Add one metric no arm optimises: metric C (MFCC), or a CLAP cosine on final candidates only (0.6 s per call, ADR-0007). Combes et al. 2025 kept their evaluation representation apart from the training loss (*documented*). The listening test decides perception.

### 3.2 Clause 2: blind listening on real and degraded sounds

**References (60)** live in `content/gr3/`. The manifest is committed; audio only when it is CC0 or the owner's.

| Stratum | n | Why |
| --- | --- | --- |
| Other subtractive synths' one-shots | 10 | reachable |
| FM, wavetable, digital | 10 | partly reachable |
| Acoustic and percussive, short sounds (< 200 ms) included | 10 | weak by design; **never filtered by duration** (owner rule) |
| Owner's picks | 10 | what the feature is for |
| Degraded copies of 20 of the above | 20 | **judged against the clean original** |

**The degradation chain** is in `ml/`, where ffmpeg and LAME are allowed. It is seeded, and its parameters go in the manifest. The held-out version uses **impulse responses, noises and codec settings disjoint from the training pools**. It covers:

- MP3 at 32–96 kbps (LAME low-passes about 11 kHz at 64k and 15.1 kHz at 96k: *documented*, old `lame.c` table; confirmed at 64k in §2);
- noise at 5–20 dB SNR, plus hum;
- reverb at RT60 0.4–2.5 s;
- clipping and limiting at +6 to +18 dB;
- phone-speaker EQ;
- stereo to mono.

**Licences and leaks.** FSD50K is drawn from Freesound, so dedupe test clips by Freesound id. Its dev set has 4,616 CC-BY-NC and 1,374 Sampling+ clips out of 40,966 (*documented*, Zenodo v1.0, 2020-10-02), so filter on the per-clip licence.

**Protocol** (`enth-listen gr3`, reusing listening.md's random names, hidden key, resume, replay and `--dry-run`):

- **Trial:** reference R, then A, then B; "which is closer to R?", forced choice. A listener who cannot tell scores 50 %.
- **Candidates:** each arm's **top-ranked** candidate by its own objective, never a human pick. Best-of-K is a fair secondary, since both arms give K branches.
- **Clips:** the same inferred performance, 48 kHz, stereo. Every clip, R included, is scaled to −23 LUFS (in-house BS.1770). A trial with a clip over −1 dBTP is lowered as a whole.
- **Balance:** A/B order counterbalanced per reference.
- **Catch trials (6):** 2 identical pairs (bias) and 4 against the init patch (attention). A listener under 80 % on attention is excluded.
- **Friends:** `--export-kit` writes random-named, loudness-matched WAVs and an `answers.txt` for any OS and player; the owner keeps `manifest.json`.
- **Decision rule** (fixed in advance): B is preferred with a 95 % cluster-bootstrap lower bound above 50 % (resample references, then listeners), and the degraded subset's point estimate is at least 50 %. Strata are reported, not gated.
- **Size** (*estimated*):
  - the owner hears 60 references plus 6 catch trials at about 15 s each (**20–25 min**, resumable);
  - each of 2 friends hears 30 references (about 15 min);
  - that is 120 judgements. With a design effect of about 1.2, a true 65 % gives a lower bound near 55 % (69–90 independent trials are needed). Owner-only detects about 70 %.
- **Optional, not gating:** a MUSHRA-style closeness rating (ITU-R BS.1534-3, 10/2015; the init patch as low anchor) on 10 references with arms A–D. It says whether the winner is good, not only better.

### 3.3 Clause 3: under 5 minutes

- **The cap.** `search_with`'s callback, which `--max-seconds` already uses, stops at the cap minus intake.
- **Measured instead:**
  - the maximum time from file to last candidate over every test run, with the test host playing a 16-voice patch;
  - time to first candidate (Synplant grows candidates progressively, *anecdotal*, MusicTech 2023-12-11);
  - `process-stats` p99 and p99.9 with a search running (new, and the important one).

### 3.4 Hygiene

- The held-out sets, their hashes and the exclusion lists freeze in wave 1. DG asserts that no training item is on them.
- Every test run is appended to `content/gr3/runs.jsonl` (build, date, load), and none is dropped. Tuning happens on dev only.
- The listening key stays closed until every listener is done. Whoever prepares stimuli does not audition them.

## 4. What could surprise us

| # | Risk | Evidence | Cheapest retirement |
| --- | --- | --- | --- |
| R1 | **Search fits the degradation**, undoing the model | §2, *measured* | FE and OBJ; arm C; clean references |
| R2 | **Domain gap**: synth-trained proposals miss real sounds | Synplant's accuracy "vary wildly" on real sources (*anecdotal*); Bruford et al. 2024 show out-of-domain results as audio examples only (*documented*) | R3a measures the real baseline first; a 10-sound pilot listen after the first small model; per-stratum reports |
| R3 | **Metric blind spots on real audio** | ADR-0007: a 5→20 ms attack moves A ≤ 0.003; detune reads as decorrelation. sound/1's JND scale needs the target *patch*, so **it cannot exist for a recording** (*documented*) | Raw distance and components for real audio; listening decides; an independent metric; the octave release in match's pitch pin |
| R4 | **Symmetric solutions**: a regressor averages modes | Hayes et al. 2025: regression degrades under permutation symmetry even with invariant losses; generative and equivariant models do better (*documented*) | K candidates from a generative or winner-takes-all head (axis 1); structure/1 and best-of-K |
| R5 | **Structure is never searched** | GR1 level 5: 0 of 4; `init` is one saw plus an envelope (*measured*) | R3a template portfolio as branches; R3b structure flags; results per level |
| R6 | **Leakage** | priors "fitted to real presets" against presets as a test set; FSD50K is Freesound | wave-1 freeze; disjoint preset halves; Freesound-id dedupe; disjoint impulse-response and noise pools |
| R7 | **Licences** | FSD50K mixes NC and Sampling+ (*documented*); Ubuntu's ffmpeg is `--enable-gpl` (*measured*) | a data manifest; no NC or SA data in shipped weights; ffmpeg and LAME only in `ml/`, never in a bundle or CI artefact |
| R8 | **Inference not bit-exact** across platforms (SIMD paths, wasm) | analogous to rustfft (design.md §5) | snap proposals (for example to 1/4096 of the normalised range) and log them in `enth.create/1`. The search is deterministic given its start, so results replay |
| R9 | **Real time and lifetime in the DAW** | `enth-game` is joined on drop ("a few hundred ms at most"); nice-plug's background worker is the plan pump (*documented*) | a separate pool, cancelled within one batch, with capped workers; `process-stats` under search |
| R10 | **Wrong inferred performance** (note, hold, chords, glides, loops) | `match` refuses WAVs without `--perf`; only `describe` infers note-off (*measured*) | intake infers note and hold and warns; `--perf` overrides |
| R11 | **Owner time, small samples** | ears not always available | forced choice; resumable; friend kits; owner-only fallback |
| R12 | **Training compute on the laptop** | a 6-core laptop with 15 GiB and a 6 GiB laptop GPU that also drives the display, whose X driver crashed once (model details redacted by the coordinator) (*documented*) | small models; the first GPU run attended, or CPU or a rented GPU; never beside heavy builds |
| R13 | **No audio loss** | roadmap: never a differentiable PyTorch copy of the engine | JᵀJ-weighted parameter loss (design.md §7) with canonicalisation. A learned embedding proxy is the **owner's call** |
| R14 | **Keyboard tracking** | MusicTech on Synplant 2 (*anecdotal*) | render ±1 octave; flag silence, clipping, runaway resonance |

## 5. The plan

### 5.1 Freezes

| Freeze | What | Gate |
| --- | --- | --- |
| **F0s** | S0s (stubs, `ml/` skeleton) and DS (held-out sets, hashes, exclusion lists) | Done-means green; hashes committed; the owner's picks or the default |
| **F1s** | Intake types (`Intake`, `Degradation`, `InferredPerf`), `Candidate` (patch, distance, provenance, trace), the `create` API, `enth.create/1` | Judge (Opus): CLI, plugin, eval kit and R3b build on it |
| **F2s** | SK3: `enth create` and the plugin's "Create from sound" end to end | cap enforced; cancel within one batch; `process-stats` green under search; `validate` and `reaper-test` green |
| **F3s** (R3b) | Model contract: input features, outputs (K × continuous, categorical, structure flags), parameter order tied to catalogue and generator versions, a tract-supported opset | Judge, before long training; a mismatched model header is refused (like `code_too_new`) |

### 5.2 Lanes

H marks a heavy build (plugin, egui, tract-onnx). Sizes are agent-hours.

| Lane | Owns (files) | Depends | Size | H/L | Acceptance |
| --- | --- | --- | --- | --- | --- |
| **S0s** Prep | stubs for every file below; `ml/` (uv); `content/gr3/`; `sizes/create.rs`; pins the summary agrees | main | 0.75–1.5 | H once | Done-means green; `ml/` in no cargo or CI build |
| **DS** Held-out sets | `content/gr3/{synth,real,degraded}.toml`, `hashes.txt`, `exclude-*.txt` | S0s; owner | 0.5–1 | L | sizes as §3; licences from source; hashes match |
| **DEG** Degradation | `ml/enth_ml/degrade/` | S0s | 0.75–1.5 | L | same seed, same bytes; held-out pools disjoint (tested) |
| **FE** Intake | `enth-analysis/src/intake/{trim,floor,bandwidth,clip,perf,resample}.rs` (pure, wasm32) | F0s | 1.5–2.5 | L | trim within 5 ms; bandwidth within 1/3 octave; note right on ≥ 95 % of pitched clips; chord warning; short clips kept |
| **OBJ** Robust objective | `enth-analysis/src/distance/robust.rs`, a new id (sound/1 unchanged) | FE | 1–2 | L | 40 dev targets × 4 degradations: median sound/1 vs clean improves on v1.1; clean within 1 point |
| **TP** Portfolio | `enth-solver/src/create/{portfolio,schedule,candidate}.rs` | F1s | 1.5–2.5 | L | K branches; identical for 1 and 8 workers; schedule independent of T |
| **LT** Listening | `tools/listen/src/gr3.rs` (`--export-kit`, `--analyze`) | S0s | 1–1.5 | L | dry-run report; bootstrap checked against Python |
| **EVK** GR3 kit | `xtask/src/gr3/`, `content/gr3/runs.jsonl`, `docs/gr3.md` | TP, DS | 1–2 | L | four arms; statistics checked against hand values; wall-clock and render modes; arm A baseline recorded |
| **SK3** Skeleton (→ F2s) | `enth-cli/src/cmd/create.rs`; `enth-plugin/src/create.rs` (pool, cancel); `enth-session/src/create.rs` (jobs, candidates, load as one undo step, no host writes) | F1s | 2–3.5 | **H** | the F2s row |
| **CLC** CLI | `enth create` flags, schema snapshots, `docs/agent-cli.md` | SK3 | 0.75–1.5 | L | versioned schema; insta; `--perf` override |
| **PUI** Create UI | `enth-ui/src/editor/create/*` (folder browser, branches, audition, progress) | F2s | 2–3.5 | **H** | kittest flow; 0 idle frames; leak gate unchanged |
| **INTa** R3a ship | the R3a table, `docs/r3.md` | all R3a | 1.5–2.5 | **H** | full gate, REAPER, leak; baseline in; owner's 5-sound check |
| **DG** Dataset generator | `tools/datagen/` (exempt; no PyO3 until decided) | F3s | 1–2 | L | deterministic shards; exclusions asserted; throughput |
| **AUG** Augmentation | `ml/enth_ml/augment/`, data manifest | DEG | 1–2 | L | licences recorded; no NC or SA data in shipped-model training |
| **TR** Training | `ml/enth_ml/{model,train,export}/` | DG, AUG | 2–4, plus 1–3 days of runs | runtime-heavy | PyTorch-against-tract parity on fixtures; a model card |
| **ONX** Inference | `enth-solver/src/propose/{onnx,snap}.rs` (bytes passed in) | F3s | 1.5–3 | **H** | wasm-check; snapped proposals equal on Linux and Windows CI |
| **MS** Model + search | successive halving in `create/schedule.rs` | ONX, TP | 1–2 | L | deterministic given proposals; dev gain |
| **EVb** GR3 runs | `runs.jsonl`, `docs/gr3.md` | MS, EVK | 1–2, plus about 11 h of machine time | L | clause 1 by its rule; kits made |
| **PUIb**, **INTb** | model loading (bundled or an R2 pack); GR3 table | MS; all | 1–2; 1.5–2.5 | **H** | branches from the model; GR3 rows filled |

### 5.3 Waves

R3 uses at most one heavy slot at a time; the other stays free for R2.

| Wave | Heavy (R3) | Light | Starts when |
| --- | --- | --- | --- |
| 1 | S0s (→ F0s) | DS, DEG, LT, DOCS3 | summary read |
| 2 | — | FE, OBJ, TP (→ F1s, judged), EVK | F0s |
| 3 | SK3 (→ F2s) | CLC; EVK on dev | F1s |
| 4 | PUI | arm A baseline on test (overnight) | F2s |
| 5 | INTa | owner's R3a check → **R3a ships** | R3a merged |
| 6 | ONX | F3s (judged), DG, AUG, a pilot TR with a 10-sound pilot listen | R3a shipped |
| 7 | — | TR, MS, EVb on dev | ONX, DG |
| 8 | PUIb | EVb test run; listening kits | MS |
| 9 | INTb | GR3 listening, report | PUIb |

**Sizes** (*estimated*): R3a 14.8–26.5 agent-h, R3b 10.5–20.5. P1b took about 0.5× its estimate's midpoint and R1/S1 is expected at about 0.9×, so expect about 14–18 h and 11–16 h. The critical path S0s → TP → SK3 → PUI → INTa is about 8–11 h of wall time.

### 5.4 Owner gates

| Gate | What | Time | Default |
| --- | --- | --- | --- |
| Real picks | 10 sounds you care about (local is fine); OK the agent's 30 CC0 picks | 30–45 min | 40 CC0 picks by an agent blind to results |
| Proxy loss (R13) | allow a learned embedding proxy in `ml/`? | 2 min | no |
| GPU | the first laptop GPU run, attended | 30 min | CPU or rented |
| R3a check | create from 5 of your sounds in the plugin | 20–30 min | ship on EVK's numbers |
| Model delivery | bundled, or a pack (with R2) | 2 min | bundled if ≤ 10 MB, else CLI first |
| GR3 listen | 66 trials, resumable | 20–25 min | GR3 waits |
| Friend kits | send 2, collect answers | 10 min | owner-only |

### 5.5 Cut list, in order

1. Friends' listening.
2. Opus and AAC augmentation.
3. Drag-and-drop (the folder browser stays).
4. The MUSHRA diagnosis.
5. The playability check.
6. A model download.

Topology search and text-to-patch stay out until GR3, as the roadmap says.

## 6. Gaps

- §2 is one target and one seed in a debug build with 400 renders. Rates come from OBJ.
- I found no synth-matching study (searched 2026-10-21) with a formal listening test on out-of-domain or degraded inputs, or with an equal-time model + search comparison. The protocol here is designed, not borrowed.
- The design effect (1.2) and the 65 % effect size are assumptions. LT should report the observed values.
- Not checked: cross-platform output of tract or ort, baseview drag-and-drop (axis 3); GPU costs (axis 2); the plugin's worker count beside a busy DAW (SK3 measures it); Synplant's internals (reviews only).

## 7. Recommendations

1. **Ship R3a alone,** with the robust intake (FE) and a degradation-aware objective under a new id (OBJ).
2. **Freeze held-out sets in wave 1**: hashed, disjoint preset halves, Freesound-id dedupe, disjoint impulse-response and noise pools.
3. **Clause 1:** arm A is the best tuned search; arms C and D are diagnostics. Run 60 s and 300 s with `flock -x` and render-equivalent twins; McNemar and Wilcoxon on about 120 targets; one independent metric.
4. **Clause 2:** forced choice against the **clean** reference, top candidates only, level-matched, with catch trials and a pre-registered cluster bootstrap; `enth-listen gr3` with friend kits.
5. **Clause 3:** cap by construction. Gate on time to first candidate and `process-stats` under search, with a separate pool that can be cancelled.
6. **Snap and log proposals,** so results replay without bit-exact inference.
7. **One heavy slot for R3,** so R2 runs beside it.
8. **Owner decisions:** the picks, R13, the GPU run, model delivery.

## Sources

- Hayes, Saitis, Fazekas, arXiv 2506.07199, 2025-06-08, ISMIR 2025 best student paper: <https://arxiv.org/abs/2506.07199>
- Combes, Weinzierl, Obermayer, "Neural Proxies for Sound Synthesizers", arXiv 2509.07635, 2025-09-09: <https://arxiv.org/abs/2509.07635>
- Bruford et al., AST sound matching, arXiv 2407.16643, July 2024: <https://arxiv.org/html/2407.16643v1>
- MusicTech, Synplant 2 review, 2023-12-11: <https://musictech.com/reviews/plug-ins/sonic-charges-synplant-2-review/>
- FSD50K, Zenodo 4060432, v1.0, 2020-10-02: <https://zenodo.org/records/4060432>
- ITU-R BS.1534-3 (10/2015): <https://www.itu.int/rec/R-REC-BS.1534>
- LAME low-pass table (old `lame.c`, "indicative"): <https://bazaar.launchpad.net/~x3lectric/xbmc/svn-trunk/revision/22289/lib/liblame/libmp3lame/lame.c>; 128k at about 17 kHz: <https://hydrogenaudio.org/index.php?msg=600283>
- Local, 2026-10-21: ffmpeg 6.1.1-3ubuntu5 (`--enable-gpl --enable-libmp3lame`), libmp3lame 3.100.
