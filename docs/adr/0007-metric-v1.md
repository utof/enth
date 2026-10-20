# ADR-0007: Provisional metric v1 — multi-resolution log-mel L1

- Status: **Proposed** — stays so until the owner's two listening sessions ([owner checklist](../spikes/owner-checklist.md), A1–A2; the G0 metric gate is pending them as of 2026-10-19), then recalibrate again with friends' ratings in R1
- Date: 2026-10-19

## Context

- The Recreate game's `sound` scorer, the sound → patch search and the agent CLI's `compare` all need one number for "how close do these two sounds look to a listener" ([design.md](../design.md) §6, §8).
- The Phase 0 metric spike ([docs/spikes/metric.md](../spikes/metric.md)) built three candidate distances on a 10-parameter toy synth: **A** multi-resolution log-mel L1, **B** A plus a crest-envelope term and an energy-weighted centroid-track term, **C** MFCC mean squared error.
- Measured with CMA-ES (20 random targets, 8 workers, 5,000-evaluation cap), each distance judged by its own "close enough" proxy thresholds: A recovers 17 targets at the strict threshold from a heuristic start (B 14, C 11) and 13 from a random start (B 10, C 5). Search noise between seeds is about ±2 targets.
- Those thresholds are proxies, not hearing. Re-judging each distance's results with the other distances shows B and C are stricter judges than A (A's results meet B's strict bar on 5 of 20 targets), and the data cannot say whether that is the ear or just fussiness. **No listening data exists yet:** the `listen` tool is built and tested with synthetic answers, and the owner has not run it.
- The literature gives no winner: the best loss depends on the scenario, and published listening validations are small (Salimi et al. 2026: 4 author-raters, 2 to 3 parameters; Tian et al. 2025: MFCC competitive, multi-scale spectral weaker; INSTRUMENTAL: no listening test). With 30 rated pairs, a correlation of 0.7 has a 95 % CI of about [0.42, 0.86], so Phase 0 can only reject a bad metric, not crown a best one.
- Speed is not the constraint: an evaluation (render + features + distance) costs 7 to 8 ms on one thread, and a solve takes a median 3.5 s (worst 11 s) on 8 workers.

## Decision

**Provisional metric v1 is distance A**, used inside a `sound` scorer that reports its components separately:

1. Render both clips with the challenge's performance at 48 kHz, scale each to **−23 LUFS** (EBU R128 integrated loudness; ungated whole-clip fallback for very short sounds; clips below −70 LUFS count as silent), so a louder sound never scores closer.
2. Log-mel at three resolutions (FFT 2048, 1024 and 512, hop = n/2, Hann window, 64 triangular mel bands from 30 Hz to 16 kHz, `ln(magnitude + 1e-5)`).
3. **A = the mean absolute difference per resolution, averaged over the three resolutions.**
4. Report the other components with it for coaching text and the agent CLI: crest-envelope difference, centroid-track difference, MFCC difference, f0 difference in cents, and the categorical waveform mismatch as its own line. B and C stay in the code as controls and for the listening comparison; only A is a headline number.
5. Calibrate per challenge as design.md §6 says (starting patch = 0, pass threshold = 100). The metric's version is part of the scorer id (`sound/1`, provisional) so a later change never alters old scores.

Search settings that go with it (from the spike, not part of the metric): CMA-ES on the continuous parameters with the categorical ones switched at restarts, a heuristic start from the target audio, IPOP restarts, population at least the worker count, a cap of 5,000 evaluations per search plus a caller-enforced wall-clock cap.

**Review points.** Revisit this ADR (a) when the owner has rated two sessions with `spike-metric listen`; (b) with friends' ratings in R1; (c) when a real template with modulation, effects or several notes replaces the toy synth.

## Consequences

- The scorer is cheap (0.05 ms per distance on cached features) and simple, and the same features give the components the game and the agent CLI need.
- The score is only as good as the match between A and the owner's ears, which is unmeasured. Until the listening gate is met the Recreate `sound` weight should stay modest, and the lowest difficulties should lean on `structure` scoring over stepped parameters (see the Q4 plan-risks note).
- A depends on `ebur128` and `realfft`. Distances are bit-identical only on the same build and platform; across platforms and wasm32 they agree within about 1e-5 relative (the research measured at most 2.7e-6). Tests that compare scores across platforms use that tolerance. The renderer itself, written with `libm` and IEEE operations only, is bit-identical everywhere.
- Features are crate-ready: `features`, `yin` and `distance` move into `enth-analysis` (L3), `cmaes`, `search` and `heuristic` into `enth-solver` (L5), with the caller supplying the parallel evaluator.

## Alternatives considered

- **B (A + envelope + centroid).** Scores each component by its median over random pairs. Slightly more sensitive to timbre-shape differences (it is the stricter judge), but it lost on the search (14 against 17 solved at the strict threshold) and has two more things to tune (the component scales, the envelope window). It stays the first challenger: if the listening data separates B from A with a CI that excludes 0, switch to it.
- **C (MFCC).** Compact and competitive in some listening studies (Tian et al. 2025), but the worst search objective here (11 solved against 17), and it throws away fine spectral detail. Kept as the control.
- **Learned embeddings (CLAP, EnCodec, DAC).** About 0.6 s per CLAP call on this CPU, 60 to 90 times an evaluation; not usable inside the search. May re-rank a few final candidates later. Weights must be Apache, MIT or CC0 (LAION-CLAP, DAC).
- **JTFS and soft-DTW envelope losses.** Good in specific scenarios in the literature, but no Rust implementation and no evidence they scale to ten parameters.
- **Raw parameter distance.** Punishes correct answers: solves that met the strict threshold still had a median parameter error of about 0.15 of range. That belongs to the separate `structure` scorer (design.md §7).

## How we'd know we're wrong

- The owner's listening gate fails for A: Spearman ρ under 0.7, its lower 95 % CI bound under 0.4, or τ-b more than 0.15 below the owner's own test–retest τ-b. Or the anchors fail (identical pairs under 4.5, obviously different pairs over 1.5), which means fix the session before judging the metric.
- A paired bootstrap shows another distance has a higher τ-b with a CI that excludes 0.
- An ABX check on the solver's results (not built yet) finds that fewer than 14 of 20 results "sound the same" at the strict threshold. Then the proxy thresholds are too loose.
- Friends' ratings in R1 disagree with the owner's.
- A real template (modulation, effects, several notes) shows A scoring clearly different sounds as close, or the score depending on something listeners ignore.

## Note — 2026-10-20 (R1/S1 research): five fixes for the sound score

Sources: the R1 + S1 research session (`_local/sessions/2026-10-20-r1-s1-recreate-and-instrument/`): report 02, measured on the real engine and on the owner's session 1; report 05 for the loudness; the summary for the decisions. The status stays **Proposed**: session 2 (test–retest) has not run, so the listening gate cannot close. This note changes how the `sound` scorer uses metric v1; the Decision text above is left as written.

- **Session 1 supports A.** Distance A meets the ρ clause on the owner's session 1: ρ 0.78 [0.59, 0.87], with sound anchors (identical pairs rated 5 and 5, obviously different ones 1 and 1). B reached 0.85, but the paired τ-b difference B − A has a CI of [−0.011, 0.201], which just includes 0. So **A against B waits for session 2**, which gives the test–retest ceiling and twice the data. Report 02 gives aggregates only, so session 2 stays blind.
- **1. Metric v1.1 pools mel power over about 85 ms before the log** (`logmel-pool85-l1/1`, beside `logmel-l1/1`). On six real-engine patches the nuisance floor (another render seed: the free-running start phase, the noise realisation) drops 2–20× and cutoff sensitivity stays (a pad's cutoff × 1.26: 0.192 → 0.193). On the owner's pairs, re-rendered exactly, ρ is 0.81–0.82 against A's 0.78, with every τ-b difference's CI including 0. It waits for session 2 too, under its own scorer id.
- **2. The player's modules are mapped onto the target's before the scoring render.** Start phase and noise follow module ids (design.md §5's seed rule), so equivalent patches scored as different: re-adding an oscillator (it becomes `osc2`) cost 0.15–0.30, more than a third-of-an-octave cutoff error and more than session 1's rating-4.5 point (A ≈ 0.26). The structure matcher renames the player's modules to the matched target ids, and the target's own floor (two renders at other seeds, once per challenge) is subtracted: `d_eff = max(0, d − floor)`. A different module count (three oscillators against two) still leaves a floor.
- **3. Descriptor components sit beside the headline.** A 5 → 20 ms attack moves A by at most 0.003, and detune reads as decorrelation rather than pitch (+3 ct and +25 ct score almost alike on one saw). So the report shows component rows, each with a direction and a close / a bit / far badge: pitch in cents, brightness, attack, decay and sustain, release, noisiness, and movement (beating, tremolo, sweep). They are the coaching text, the agent's `compare` output and the fairness lever (log-rise time is a primary timbre dimension, McAdams et al. 1995). The attack row inherits the descriptor's limits on low notes.
- **4. Calibration is per challenge, in JND units, with a curve fitted to ratings.** An octave of cutoff is 0.05 on a pluck and 0.46–0.54 on sustained sounds, so one global threshold is unfair. Per challenge, the scale s is the median distance of the target against about 16 renders with every active parameter moved by one just-noticeable step (`check`'s defaults to start: 25 ct, 10 % for times and Hz, 2 dB), and z = d_eff / s. Sound % = 100 / (1 + (z / z₅₀)^k), with z₅₀ and k fitted to ratings; a pass is z at or under the rating-4.5 point. **This replaces Decision item 5's "starting patch = 0, pass threshold = 100" for sound %**: the start sits nearer or farther from round to round, so the same remaining error would score differently (a progress bar may still run from start to pass). The curve is fitted after session 2 and a real-engine pairs session (every rating so far is on the toy synth), then refitted with GR1's ratings.
- **5. The scored path gets a `libm` loudness.** The −23 LUFS gain came from `ebur128`, which uses `std` maths, so a score could differ in its last bits between platforms: harmless in a solo round, fragile for golden challenges in CI and for R5's codes. Lane SS writes an in-house BS.1770 loudness on `libm` (K-weighting biquads, 400 ms blocks with 75 % overlap, the −70 LUFS and −10 LU gates, a fixed summation order) for scoring; `ebur128` stays for display and true peak. With the scalar FFT path (design.md §5), the Consequences' "agree within about 1e-5" becomes bit-identical for sound %, and golden challenges still assert a margin of at least 1e-3 from the pass threshold.
- **Around the metric** (design.md §9, [phase-2.md](../phase-2.md)): only sound decides pass; combined = w·sound + (1 − w)·structure, with w from 0.5 at the lowest difficulty (where structure is unambiguous, as the Consequences ask) to 0.8 at the highest; structure % uses the generator's observability mask. Learned embeddings stay out of R1: slow, and no better than MFCC on timbre ratings (Tian et al. 2025). LAION-CLAP may re-rank results in R3.
- **Review points.** (a) and (b) still stand: session 2, then friends' ratings in GR1, asked before the score shows so it cannot anchor them. (c) has begun: the real engine showed the blind spots above.
