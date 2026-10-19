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
