# 02: Scoring and coaching (R1)

- **Date:** 2026-10-20. **Sub-researcher:** 02, scoring and coaching. **Scratch:** `~/code/enth-scratch/2026-10-20-r1-s1/02/` (`notes.md`, `exp/` with two experiment binaries, `scripts/`, raw outputs `exp-run3.txt` and `relisten.json`).
- **Evidence labels:** *measured* means I ran it today. *Documented* means a doc, paper or licence file says so. *Anecdotal* means a forum post or a single review.
- **Privacy:** no request carried personal data. Licence files were fetched with User-Agent `enth-research/0.1`.

## 1. Summary

Metric v1 is ready to become the basis of sound %, with three fixes. The owner's first listening session supports it, but on the real engine it has two blind spots. First, it scores *equivalent* patches as different (random start phase and noise follow module ids). Second, it hardly sees short attacks. Both are cheap to fix. Structure % can be built from parts that already exist: `canonicalise`, the hint solver's inert-parameter rules, and the editor's `Overlay` rings. The coach can reuse `enth-solver::hint` unchanged, by turning the target's descriptors into an expectation list.

| Area | Recommendation | Evidence | Confidence |
| --- | --- | --- | --- |
| Metric gate (ADR-0007) | A already meets the ρ clause on session 1: ρ 0.78 [0.59, 0.87]. Run session 2 (test–retest) before choosing A or B (B ρ 0.85; τ-b of B − A in [−0.01, 0.20]) | *Measured*, aggregates only | Medium |
| Metric v1.1 | Pool mel power over about 85 ms before the log. The nuisance floor drops 2–20×; cutoff sensitivity and agreement with the owner stay (ρ 0.81) | *Measured*: 6 engine patches, 30 re-rendered pairs | Medium–high |
| Equivalent patches | Rename the player's modules to their matched target ids before the scoring render, and subtract the target's own floor | *Measured*: one relabelled oscillator costs 0.15–0.30, more than a 1/3-octave cutoff error | High |
| Attack blind spot | Descriptor components beside the headline (attack, pitch, centroid, envelope, noisiness, movement), shown as the breakdown | *Measured*: attack 5→20 ms moves v1 by ≤ 0.003; McAdams 1995 | High |
| Sound % mapping | Distance in JND units per challenge (jitter scale), then a psychometric curve fitted to ratings | *Measured*: cutoff ×2 is 0.05 on a pluck, 0.54 on a pad; rating 4.5 at A ≈ 0.26 | Medium |
| Structure % | Canonicalise, match modules per kind, compare active parameters with sensitivity-based tolerances, per-knob rings | *Documented*: `canon.rs`, `hint/space.rs`, `overlay.rs`; Hayes 2025; Salimi 2026 | Medium–high |
| Combined % | Sound alone decides pass. Combined = w·sound + (1 − w)·structure, w from 0.5 to 0.8 with difficulty | ADR-0007; judgement | Medium |
| A/B | Primary: two loudness-matched prerendered loops (attenuate the louder), sample-aligned. Secondary: a hidden second engine on live MIDI | *Documented*: p99 112–123 µs of 333 | High / medium |
| Time to pass | Shell timestamps, an edit log in the session; time = first passing submit | Layer rules | High |
| Coach hints | Target descriptors → `Expect` list → `enth-solver::hint` unchanged; revealed in levels | *Documented*: hints.md | High |
| CLI | `compare` and `suggest` on the game's scorer; `match` = the spike's CMA-ES ported; `explain` from the catalogue | agent-cli.md | High |
| Evals | 30 text goals, 25 seeded reference targets plus 5 CC0 or own recordings; GR1 baseline on the humans' 20 codes, with no-op and CMA-ES floors | p1a-g1.md | Medium |
| Embeddings | Not in R1: slow, large, and no better than MFCC on timbre ratings. LAION-CLAP (CC0 code, Apache-2.0 weights) may re-rank in R3 | Tian 2025; licence files | High |

## 2. Findings

### 2.1 Where the code stands (*documented*, read 2026-10-20)

- `enth-analysis::distance` has `metric_v1` (`logmel-l1/1`) and `components`. A graph query finds no product caller: only tests and the spike use it.
- **No `enth-challenge` crate exists** (L4 is reserved in `layers.toml`). `enth-solver` has only `hint` and `eval`; the spike's `cmaes`, `search` and `heuristic` were never moved over. `Evaluator` returns `Descriptors`, so a sound-distance search needs a `Features` output too.
- `enth-patch::canon::canonicalise` drops disabled modules and depth-0 routes, sorts the commutative sections and relabels ids. It "keys the structure score", but it orders by parameter bits, so it **cannot pair up** two *different* patches' modules: that needs an assignment step.
- The inert-parameter rules are `pub(crate)` in `enth-solver/src/hint/space.rs` (hints.md: they belong in the catalogue).
- `editor/overlay.rs` draws a score per parameter as the knob's ring. `challenge_bar` and `results` regions are reserved behind the `recreate` feature.
- `agent_eval/goal.rs` has no target-sound field, and `content/agent-evals/results/` is empty: the G1 run used subagents.

### 2.2 Sound %: what the owner's session 1 says (*measured*, 2026-10-20)

I joined `~/enth-listen/answers-session1.json` with `manifest.json` (scripts `s1_prelim.py`, `s1_bins.py`, `s1_pooled.py`). Only aggregates are reported here, so session 2 stays blind.

- **Anchors:** identical pairs got 5 and 5; obviously different pairs got 1 and 1. The session is sound.
- **Ceiling effect:** 15 of the 30 test pairs were rated 5.
- **Correlations** (Spearman with 2,000-resample CIs; τ-b in brackets):

  | Distance | ρ | 95 % CI | τ-b |
  | --- | --- | --- | --- |
  | A | 0.78 | [0.59, 0.87] | 0.62 |
  | B | 0.85 | [0.72, 0.91] | 0.71 |
  | C | 0.83 | [0.69, 0.89] | 0.67 |
  | Crest envelope alone | 0.86 | [0.74, 0.92] | 0.71 |
  | Centroid alone | 0.55 | [0.19, 0.78] | — |

  The paired τ-b difference B − A has a CI of [−0.011, 0.201], which just includes 0.
- **Mean rating per distance-A quantile bin:** A ≤ 0.35 is rated 4.6–5.0; 0.38–0.59 is 3.6; 0.67–1.17 is 2.6; 1.6–2.5 is 2.2.
- **Logistic fit** (rating = 1 + 4/(1 + (A/0.89)^1.6), RMSE 0.87): rating 4.5 at A = 0.26, rating 4 at 0.45, rating 3 at 0.89.
- **The spike's proxies sit inside the "sounds the same" zone:** T1 median 0.12 (0.045–0.28) and T3 median 0.21 (0.09–0.37), from `~/code/enth-scratch/lane-c/runs/solve-A.json`. They look strict rather than loose, so the ABX gate (14 of 20 same) will probably pass. That is inferred, not measured.

**What the owner's remaining listening must provide.**
- **Session 2** (15 min): the test–retest τ-b the gate needs, and twice the data for A against B. B's edge is its envelope term, as in Salimi et al. (2025), where envelope DTW won on AM targets.
- **A real-engine pairs session** (about 40 pairs, 15–20 min). All ratings so far are on a one-note toy synth without modulation, effects or noise, and §2.3 shows the engine differs. Include nuisance pairs, JND jitters and graded single moves (attack, release, detune, cutoff, noise). It decides pooling, the attack and envelope weights, and the psychometric curve. `enth-listen` has the blind design; it needs the engine and the randomizer as stimulus source.
- **Friends' ratings during GR1:** "how similar is yours, 1–5?" before the score is shown (no anchoring; ADR-0007 review point b), then "fair / too harsh / too generous".

### 2.3 Metric v1 on the real engine: the nuisance floor and blind spots (*measured*)

`exp/src/main.rs` renders 6 patches through `enth-render` at repo HEAD and compares them with `enth-analysis` features at 48 kHz. The patches are the four templates, a high-passed noise hat and a band-passed noise pad. Full output: `exp-run3.txt`. These are distances of metric v1, with the 85 ms-pooled variant after the arrow.

| Case | init (1 saw) | pad_2osc | bass_sub | noise pad | pluck / hat |
| --- | --- | --- | --- | --- | --- |
| Other render seed (free-phase start, noise realisation) | 0.04–0.15 → 0.002–0.007 | 0.27–0.28 → 0.11–0.15 | 0.03–0.09 → 0.003–0.006 | 0.215 → 0.075 | ≤ 0.019 |
| **Relabel one oscillator id** (seeds follow ids) | **0.154** → 0.008 | **0.303** → 0.148 | 0.100 → 0.006 | 0.216 → 0.075 | 0.019 / 0.016 |
| Generators reordered, same ids | 0 | 0 | 0 | 0 | 0 |
| semi +12 against semi +11 with fine +100; master −1 dB | 0 and 0 | 0 and 0 | 0 and 0 | — and 0 | 0 and 0 (pluck); — and 0 (hat) |
| Cutoff ×1.12 / ×1.26 / ×2 | (no filter) | 0.096 / 0.192 / 0.539 | 0.076 / 0.155 / 0.463 | 0.077 / 0.158 / 0.477 | ×2: 0.049 / 0.049 |
| **Attack ×4** (5→20 ms; the pad 600→2,400 ms) | **0.0027** | 0.270 | **0.0004** | 0.129 | 0.002 / 0.011 |
| Release ×2 | 0.323 | 0.216 | 0.037 | 0.328 | 0 |
| Fine +3 / +10 / +25 ct | 0.093 / 0.104 / 0.114 → 0.008 / 0.014 / 0.025 | 0.31 / 0.33 / 0.33 | 0.056 / 0.066 / 0.077 | — | — |

What this means for the score:

1. **Equivalent patches lose up to the pass threshold.** Re-adding an oscillator (it becomes `osc2`) changes its random start phase: 0.30 on the detuned pad. That is more than a 1/3-octave cutoff error and more than session 1's rating-4.5 point (0.26).
   - **Fix (a):** the structure matcher (§2.5) renames the player's modules to the matched target ids before the scoring render, so the seeds match.
   - **Fix (b):** measure the target's own floor once per challenge (2 renders at other seeds) and score `d_eff = max(0, d − floor)`. This also covers noise.
   - **Fix (c):** pooling (point 5).
2. **Detune reads as decorrelation, not pitch.** On one saw, +3 ct and +25 ct score almost alike (0.09 and 0.11), and the pad saturates at 0.31–0.33. `f0_cents` reads 3.2 / 9.9 / 24.9 ct, so pitch needs its own component.
3. **Short attacks are invisible, and tails weigh heavily.** A 5→20 ms attack moves v1 by 0.0004–0.003. A 400→800 ms release moves it about as much as saw → pulse (0.32 against 0.39): frame-wise log L1 weighs every quiet tail frame like a loud one. Log-rise time is a primary timbre dimension (McAdams et al. 1995, *documented*), so the score needs an attack term from the descriptors.
4. **The scale depends on the sound type.** A cutoff off by an octave is 0.049 on a pluck and 0.46–0.54 on sustained sounds. A global pass threshold would be unfair; per-challenge calibration (design.md §6) is right.
5. **Pooling** mel power over about 85 ms before the log keeps cutoff sensitivity (pad ×1.26: 0.192 → 0.193) and removes most of the single-oscillator phase floor. On the owner's pairs, re-rendered exactly (the recomputed A matches the manifest to 0.0), pooling at 43, 85 and 170 ms gives ρ 0.81–0.82 against A's 0.78, with every τ-b difference CI including 0. Detuned beating (0.15) and noise (0.075) still need fixes (a) and (b).

### 2.4 Turning a distance into a 0–100 score people agree with

**The proposal (`sound/1`):**
1. **Renders:** the challenge performance at 48 kHz, after the id matching of §2.5. Mid channel, −23 LUFS, scalar FFT.
2. **`d_eff`** = the pooled log-mel L1 minus the target's nuisance floor, clamped at 0.
3. **Per-challenge scale s** = the median d of the target against about 16 renders with every *active* parameter jittered by one JND (the spike's T1 idea, about 0.1–1 s once per challenge). Start from `check`'s defaults: 25 ct, 10 % for times and Hz, 2 dB. z = d_eff / s is in "just noticeable" units, comparable across a hat and a pad.
4. **Sound %** = 100 / (1 + (z/z₅₀)^k), with z₅₀ and k fitted to ratings (session 1's toy start: rating 3 at about 7 × T1). Pass when z ≤ z_pass, the rating-4.5 point.
5. **Show pass as a badge** with headroom above it (92 against 99). My suggestion, not evidence.

**Why not "start = 0, threshold = 100"** (design.md §6)? The start patch sits nearer or farther from round to round, so the same remaining error would score differently, and clamping hides progress. A *progress bar* can still use start → pass.

**Per-descriptor breakdown, the fairness lever.** Show 6–7 rows: pitch (ct), brightness, attack, decay and sustain, release, noisiness, movement (beating, tremolo, sweep). Each has a direction ("yours is brighter") and a close / a bit / far badge in the same tolerances. All are existing descriptors with documented accuracy.
- **Prior art.** Syntorial gives "a score based on how close you are" on notched parameters (MusicTech, 2014-02-11), and its scoring was "overhauled" in 2014 (Wikipedia). SoundGym scores in %, with lives and a window that narrows (Sound On Sound). Its forum users call EQ Mirror's score erratic, a near miss at 27 % and a wrong curve at 75 % (*anecdotal*). Lesson: a score must move with what people hear, and say why.

### 2.5 Structure %

`structure/1`:
1. **Canonicalise both patches**, drop inert parameters (the solver's rules, moved into the catalogue) and fold the equivalences: total pitch = 100·semi + fine; pulse 50 % = the pulse wavetable at 0 (0.02–0.05 apart, *measured*); level 0 = absent; overall level ignored (loudness is normalised, 0 measured); periodic parameters wrap; phase and seed ignored in free mode.
2. **Match modules per kind** at minimum cost. Brute-force permutations suffice for a handful of modules. Generators commute; chains (`voice_fx`, `fx`) pay a fixed order penalty; an unmatched module costs "missing" or "extra" by audibility; routes compare as parameters keyed by (matched source, destination), with depth 0 = absent. Hayes et al. (ISMIR 2025) show that permutation symmetry breaks naive parameter regression; Salimi et al. (2026) score only the critical parameters two synths share (*documented*).
3. **Per-parameter score:** sᵢ = 1 within tolᵢ, falling to 0 at 4·tolᵢ, in normalised units. tolᵢ is the move that shifts the sound by one JND, from the same renders as the §2.4 scale. A parameter that never reaches one JND is inert for this target.
4. **Structure %** = Σ wᵢ·sᵢ / Σ wᵢ plus the module terms, with weights from design.md §7's Δθᵀ JᵀJ Δθ, capped.
5. **Per-knob rings after submit** through the existing `Overlay`: Syntorial-style right/wrong.
6. **When sound passes but structure is low**, show the nearest passing solution later (design.md §7 layer 3). It never lowers a pass.

Raw parameter distance punishes right answers: spike solves that met the strict threshold still had a median parameter error of 0.15 of range (design.md, *documented*). That is why the weighting and tolerances are essential.

### 2.6 Combined % and time to pass

- **`combined/1`** = w·sound + (1 − w)·structure. w = 0.5 at the lowest difficulty, where structure is unambiguous and ADR-0007 asks to lean on it, rising to 0.8 at the highest, where many routes are valid. Only sound decides pass. The weights are versioned in the scorer id.
- **Time to pass.** Core crates read no clocks, so the shell passes monotonic timestamps to the session (L6), which logs `(t, ops)` and submits.
  - Time to pass = the first passing **submit**, Syntorial-style; a live pass light would give the answer away.
  - Replaying the log through the scorer gives "you were at a pass at 1:42". Scoring an edit costs 10–150 ms on the worker.
  - Pause while the editor is hidden. Duels (R5) use wall time.

### 2.7 Level-matched instant A/B in the plugin

- **Primary: two prerendered buffers.** At round start and after each settled edit, the worker renders the challenge performance for target and mine (2–85 ms for 2 s, *documented*).
  - Loudness-match with the scorer's meter by **attenuating the louder one**: MetricAB does level-matched instant switching, and MusicTech notes that boosting the reference can clip.
  - Loop both sample-aligned and switch with a 5–10 ms equal-power crossfade. It works in any host and in a standalone friend build, without MIDI, and plays exactly what the scorer hears.
- **Secondary: a hidden second engine** fed the host's MIDI, so the target is playable from the keyboard. The G1 patch's steady p99 is 112–123 µs against the 333 µs criterion (*documented*), so two engines would use about 240 µs (inferred) and headroom falls from about 2.5× to 1.3×. Cap the target's polyphony and measure it.
- **Never persist the target patch:** store the challenge code and regenerate it.
- **Caveat (unverified, search snippet only):** listener-chosen levels are reported about 2.2 dB from BS.1770-4 on average (Berendes 2022; the AES page refused access).

### 2.8 Coach hints from measured sensitivity

- **Reuse the solver as is.** Turn the target's descriptors into an `Expect` list (`feature ~ target ± default tolerance`; `== null` for absent movement) and run `enth-solver::hint` on the player's patch. It returns one measured multi-parameter move and `blocked` verdicts that name missing structure, in 10–16 renders and 0.3–0.8 s (hints.md, *documented*).
- **Reveal it in levels:** the worst descriptor ("attack: target is slower"), then the module, then the direction, then apply (marked "assisted").
- **Limits:** no categorical or structural moves, and route depths and `semi` are not moved; the unmeasured text hints cover those. `suggest` with a target adds cheap single-parameter rankings on the sound distance.

### 2.9 CLI `compare`, `suggest`, `match`, `explain`

- **`compare <wav|patch> --target <wav|patch>`** → `enth.compare/1`: sound % with components and descriptor rows; structure % with per-parameter rows when both are patches; the scorer ids, floor and scale. It calls the same `enth-challenge` code as the plugin, so CLI = game.
- **`suggest`** (expectations or a target): ranked, measured single moves.
- **`match --free <addrs> --budget N`:** port the spike's `cmaes`, `search` and `heuristic` into `enth-solver`, as ADR-0007 planned (17 of 20 toy targets at T1, median 3.5 s on 8 workers, *documented*). The CLI passes a rayon `Features` evaluator.
- **`explain`:** deterministic text from `learn.desc`, route `means` and the catalogue. No LLM.

### 2.10 Agent evals and the GR1 baseline

- **Today** (p1a-g1.md, *documented*): given track 30/30, self-written 15/30 (Claude Sonnet 5.5, 2026-10-19); the mechanical follower solves 2 of 10, because hints cannot add structure.
- **Goals, 10 → 30.** Add 20 across the descriptor families the first ten under-use: sub sine, tremolo keys, acid squelch, detuned bass, noise riser (`filter_sweep env up`), flute, reverse swell, chiptune pulse. Add three "edit" goals that start from a flawed patch (clicky, harsh, clipping). There is no reverb, stereo-width or vibrato descriptor (vibrato only through `pitch.stability`), so those goals wait.
- **Reference sounds.** Schema `enth.agent-eval/2` gains `target = { code = … }` or a hidden patch. The harness puts only `target.wav` in the temp dir; the judge is `enth compare`'s sound pass. Use 25 randomizer targets (5 per difficulty) and 5 recordings that are CC0 or the owner's.
- **The GR1 baseline.** Choose 20 fixed challenge codes (4 per difficulty) that every human plays first, which also makes scores comparable across people. The agent plays each 3 times: report pass@1 per difficulty, median sound %, renders, tokens and cost.
  - Beside it, two floors: **no-op** (the start patch's score) and **CMA-ES only** (`match` on every parameter, same budget). They show what the LLM adds. A `match` call counts its internal renders, capped.
  - 10 of 20 has a Wilson 95 % CI of [0.30, 0.70], so only large differences separate.

### 2.11 Embeddings, DTW and prior art on the metric

- **Embedding licences** (from the files and model cards, 2026-10-20):
  - LAION-CLAP: code CC0-1.0 (last push 2025-05-15); weights Apache-2.0 (`laion/clap-htsat-unfused`, `larger_clap_music`).
  - MS-CLAP MIT; CDPAM MIT (last push 2023-03-24); DAC MIT; ViSQOL Apache-2.0 (a quality metric, not timbre).
  - OpenL3 and PANNs: code MIT, but weights CC BY 4.0, which `deny.toml` does not allow.
- **Embeddings are no better here.** Tian et al. (ISMIR 2025, 2,614 ratings): MFCC "remains competitive", multi-scale spectral loss underperforms, CDPAM fits musical timbre poorly, CLAP style embeddings are best. At about 0.6 s per CLAP call (ADR-0007), they would at most re-rank results in R3, on `tract` 0.23.9 (2026-10-15).
- **DTW** forgives timing. In Recreate both sides play the same performance, so frames already line up, and timing errors are exactly what to score. DTW belongs to R3's audio targets.
- **Sound-matching papers** (InverSynth 2019, Sound2Synth 2022 on GPL Dexed, DiffMoog 2024) predict parameters offline. Salimi et al. (2025) find that the best loss depends on the synth, and parameter, spectral and listening measures agree only moderately. That supports showing components beside one headline number.

### 2.12 Code structure

| Crate (layer) | New or changed |
| --- | --- |
| `enth-patch` (L1) | "Inert when…" metadata (from `hint/space.rs`); an equivalence table |
| `enth-analysis` (L3) | `pooled_logmel_l1` (`logmel-pool85-l1/1`) beside `logmel-l1/1`; `DescriptorDiff { feature, ours, target, z, direction }` |
| **`enth-challenge` (L4, new)** | `Challenge`, `ScoringSpec`, scorers `sound/1`, `structure/1`, `combined/1`, `time/1`; `Calibration { floor, scale, z_pass, curve }`; the module matcher and relabelling; a versioned `ScoreReport`; `trait BatchEval<Out>` (the solver's `Evaluator` builds on it) |
| `enth-solver` (L5) | The spike's `cmaes`, `search`, `heuristic`; `suggest`; target → `Expect` for the coach |
| `enth-session` (L6) | Round state: hidden target, edit log with shell timestamps, submits, hint level; A/B buffers |
| `enth-ui` (L7, `recreate`) | Challenge bar (timer, A/B, submit), results drawer, per-knob `Overlay` rings |
| `enth-cli`, `xtask` | `compare`, `suggest`, `match`, `explain`; `agent-eval/2`, baselines, `enth-listen pairs --engine` |

Tests: scores on the scalar path, with a tolerance and a tie rule at the pass line across platforms (design.md §5). Property tests: relabelled, reordered, inert-changed and level-changed patches score 100 % structure and pass sound. `ScoreReport` fixtures.

## 3. Risks and open questions

1. **The calibration rests on 30 toy-synth pairs** from one person in one session. Until the real-engine session and GR1's ratings exist, "pass = rating 4.5" is an estimate.
2. **Detuned beating and noise keep a floor** after pooling (0.15 and 0.075). Id matching removes it only when the modules match: 3 oscillators against the target's 2 still leave one. Subtracting the target's floor is approximate.
3. **The tolerances are `check`'s defaults** (25 ct, 10 %, 2 dB). 25 ct is generous for pitch; listening should tune them.
4. **The attack term inherits the descriptor's limits:** a 3 ms attack reads 15 ms at A1 (analysis.md). Weight it lower below about C2.
5. **A live sound meter turns play into hill-climbing** on the number. Keep it hidden by default (an assist mode at most); that is the owner's call, as is the display policy (a pass badge at 90, or pass = 100).
6. **The second engine's CPU** is inferred, not measured.

## 4. Gaps

- **Session 2 is not run,** so there is no test–retest and the ADR-0007 gate cannot close (`spike-metric listen --session 2`, then `--analyze`). There is no listening data on the real engine at all.
- Not read: the ITU-T P.1401 and ITU-R BS.1534 texts (search results only) and Berendes 2022 (HTTP 403). Syntorial and SoundGym publish no score formula.
- Not measured: jitter-calibration cost on real R1 targets, two live engines' CPU, the agent's cost on reference-sound goals.
- My experiment covers 6 patches, one note (C3) and the default performance. The randomizer does not exist yet, so its output is untested.

## 5. Sources (all accessed 2026-10-20)

- **In-repo:** `docs/adr/0007-metric-v1.md`, `docs/spikes/metric.md`, `docs/dsp/analysis.md`, `docs/dsp/hints.md`, `docs/agent-cli.md`, `docs/p1a-g1.md`, `docs/design.md` §5–9, `docs/roadmap.md`, `docs/listening.md`, `docs/editor.md`, `crates/enth-patch/src/canon.rs`, `crates/enth-solver/src/hint/space.rs`, `crates/enth-analysis/src/distance.rs`, `xtask/src/agent_eval/goal.rs`.
- **Measured here:** `~/code/enth-scratch/2026-10-20-r1-s1/02/` (`exp/src/main.rs`, `exp/src/relisten.rs`, `exp-run3.txt`, `relisten.json`, `scripts/s1_*.py`); inputs `~/enth-listen/{manifest.json,answers-session1.json}` and `~/code/enth-scratch/lane-c/runs/solve-A.json`.
- **Papers:**
  - Salimi, Hindle, Zaïane, "Evaluating Sound Similarity Metrics for Differentiable, Iterative Sound-Matching", arXiv 2506.22628 v2, 2025-10-08: https://arxiv.org/abs/2506.22628
  - Salimi et al., "Evaluating Loss Functions in Differentiable Out-of-Domain Sound-Matching with Partial Parameter Distance", arXiv 2608.27698, 2026-08-27: https://arxiv.org/abs/2608.27698
  - Tian, Lattner, Saitis, "Assessing the Alignment of Audio Representations with Timbre Similarity Ratings", ISMIR 2025, arXiv 2507.07764: https://arxiv.org/abs/2507.07764
  - Hayes, Saitis, Fazekas, "Audio synthesizer inversion in symmetric parameter spaces with approximately equivariant flow matching", ISMIR 2025, arXiv 2506.07199: https://arxiv.org/abs/2506.07199
  - Barkan et al., "InverSynth", TASLP 2019, arXiv 1812.06349: https://arxiv.org/abs/1812.06349
  - Chen et al., "Sound2Synth", IJCAI 2022, arXiv 2205.03043: https://arxiv.org/abs/2205.03043
  - Uzrad et al., "DiffMoog", arXiv 2401.12570, 2024-01-23: https://arxiv.org/abs/2401.12570
  - McAdams et al., "Perceptual scaling of synthesized musical timbres", Psychological Research 58, 1995: https://three-mode.leidenuniv.nl/pdf/m/mcadams_etal1995pr.pdf
  - Madaghiele et al., NIME 2026 (RL sound matching, five rewards): https://nime.org/proc/nime2026_89/
- **Licences** (raw files from `raw.githubusercontent.com`): `LAION-AI/CLAP/main/LICENSE` (CC0-1.0); `marl/openl3/main/LICENSE` (MIT) plus its README (weights CC BY 4.0); `qiuqiangkong/audioset_tagging_cnn/master/LICENSE.MIT`; `pranaymanocha/PerceptualAudio/master/LICENSE`; `google/visqol/master/LICENSE`; `microsoft/CLAP/main/LICENSE`; `descriptinc/descript-audio-codec/main/LICENSE`. Also https://huggingface.co/api/models/laion/clap-htsat-unfused (apache-2.0), https://zenodo.org/api/records/3987831 (PANNs weights, cc-by-4.0) and https://api.github.com/repos/sonos/tract/releases/latest (v0.23.9, 2026-10-15).
- **Products:**
  - MusicTech, Syntorial review, 2014-02-11: https://musictech.com/reviews/audible-genius-syntorial-review/
  - Wikipedia, Syntorial: https://en.wikipedia.org/wiki/Syntorial
  - Sound On Sound on SoundGym, 2021-09-04: https://www.soundonsound.com/node/4923779 and https://www.soundonsound.com/node/4923945
  - Plugin Alliance MetricAB: https://www.plugin-alliance.com/products/metric-ab
  - MusicTech MetricAB review: https://musictech.com/reviews/plug-ins/adptr-audio-plugin-alliance-metricab/
  - Rekkerd, MetricAB v1.5: https://rekkerd.org/adptr-audio-updates-metric-ab-referencing-plugin-to-v1-5/
