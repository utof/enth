# Spike: metric and search (Phase 0, Task 6)

- Date: 2026-10-19
- Code: `spikes/spike-metric` (package `spike-metric`), branch `lane-c/metric`. [phase-0.md](../phase-0.md) says `spikes/metric`; the directory carries the package name, like `spike-shell`.
- Decision it feeds: [ADR-0007](../adr/0007-metric-v1.md) (provisional metric v1)
- Research behind it: `_local/sessions/2026-10-19-phase-0-spikes/04-metric-search-final.md` (the researcher's probe is the starting point of this code; `heurcheck` reproduces its numbers exactly)
- Verdict: **the search side passes, the metric side waits for ears.** On the toy 10-parameter synth, CMA-ES with a heuristic start recovers 17 of 20 random targets to the strict proxy threshold using distance A (multi-resolution log-mel L1), with a median of 3.5 s and a worst case of 10.9 s per solve on 8 workers. Nothing here says which distance matches the owner's ears: that needs the `listen` sessions below, which the owner has not run yet. Until then A is the provisional metric v1.

## What was built

| Piece | Where | Notes |
| --- | --- | --- |
| Toy voice | `render.rs` | Two polyBLEP oscillators (saw, square or triangle) into a TPT state-variable lowpass. 10 parameters: wave, detune, cutoff, resonance, filter-env amount, filter-env decay, amp A/D/S/R (the researcher's 10; filter-env decay is the tenth). 48 kHz, 2 s, A3 held for 1.5 s. Seeded and deterministic; every transcendental is `libm`, so the render is bit-identical on every platform and its hash is pinned in a test |
| Features | `features.rs`, `yin.rs` | EBU R128 integrated loudness (`ebur128`) normalises both clips to −23 LUFS first. Log-mel at three resolutions (FFT 2048, 1024, 512; hop = n/2; 64 bands, 30 Hz to 16 kHz), 10 ms crest envelope (max-held over ±50 ms, because detuned oscillators make the plain RMS beat), energy-weighted spectral-centroid track, MFCC c0 to c12, YIN f0 (FFT-based difference function). `rustfft` and `realfft` only |
| Distances | `distance.rs` | **A** multi-resolution log-mel L1. **B** A + envelope L1 + centroid L1, each divided by its median over 50 random pairs. **C** MFCC mean squared error. A `Components` struct keeps every component (log-mel, envelope, centroid, MFCC, f0 in cents) for coaching text |
| CMA-ES | `cmaes.rs` | Ask–tell, Hansen 2016 defaults, box bounds by clamp plus a quadratic penalty, restart signals (collapse, condition, stall). 340 lines, no dependencies beyond `libm` |
| Search | `search.rs` | Restarting state machine over the nine continuous parameters. The waveform is categorical: fixed per restart and switched at restarts, cycling in the heuristic's order. IPOP (λ doubles after each 3-restart cycle) or BIPOP-style (large and small-population regimes). Several searches can run side by side in one combined batch (`drive`) |
| Heuristic start | `heuristic.rs` | Beat rate gives detune, a crest envelope gives A/D/S/R, harmonic roll-off gives cutoff and the filter envelope, even/odd harmonics give the waveform order |
| Parallel evaluator | `par.rs` | rayon pool, one `Analyzer` per worker, **work queue** (atomic counter) with results written by index, so the output does not depend on the worker count |
| Listening test | `listen.rs`, `listen_cmd.rs`, `stats.rs`, `wav.rs` | Design, loudness-matched stimuli, answer files, Kendall τ-b and Spearman with bootstrap CIs, triplet agreement, paired τ differences, test–retest |
| CLI | `main.rs` | `solve`, `bench`, `listen`, plus `heurcheck` |

The maths modules (everything except `par`, `main` and the CLI files) have no I/O, clocks, threads or global RNG, and the library checks for `wasm32-unknown-unknown`. They are laid out to move into `enth-analysis` (`features`, `yin`, `distance`) and `enth-solver` (`cmaes`, `search`, `heuristic`). The solver never reads a clock: the caller enforces any wall-clock cap in its `after_batch` callback.

## Results

Machine: the owner's i7-9750H laptop (6 cores, 12 threads per the research report; `nproc` says 12), rustc 1.99.1, release build, 8 workers unless stated. Other agents were running throughout. Every timing run held `flock -x /tmp/enth-quiet.lock`, but processes outside the lock (a Python at 100 to 140 %, Cinnamon at 80 to 90 %) kept the machine busy, so **all times are upper bounds**. The `uptime` load is given per run; during my own 8-thread runs it includes those 8 threads (a run starting at load 4 reads about 10 once it is going). Solve counts and evaluation counts do not depend on load. Raw output is in `~/code/enth-scratch/lane-c/runs/`.

### Cost per evaluation

An evaluation is render + loudness normalisation + features + distance, against cached target features.

| Part | Run 1 (load 14, 200 reps) | Run 2 (load 8, 300 reps) |
| --- | --- | --- |
| Render, 2 s, 1 voice | 2.16 ms | 2.08 ms |
| Features at hop n/2, including loudness normalisation | 5.21 ms | 4.85 ms |
| of which ebur128 loudness | 1.12 ms | 1.05 ms |
| Features at hop n/4 | 9.41 ms | 8.79 ms |
| YIN f0 track (not used in the search) | 2.11 ms | 1.95 ms |
| Distance A / B / C (cached features) | 0.057 / 0.057 / 0.003 ms | 0.053 / 0.059 / 0.004 ms |
| **One evaluation, one thread** | **7.77 ms (128.7 /s)** | **7.36 ms (135.8 /s)** |

Hop n/2 halves the analysis cost compared with n/4 and is the default. The researcher measured 7.9 ms (126.9 /s) on the same machine; this port, with `libm` and loudness normalisation, costs the same.

### Worker scaling

Work-queue evaluator, distance A, batch = the CMA-ES population. Run 1: 800 evaluations per row at load 10 to 12; run 2: 1,200 per row at load 6.5 to 7.4.

| Workers | Run 1, batch 16 | Run 1, batch 40 | Run 2, batch 16 | Run 2, batch 40 |
| --- | --- | --- | --- | --- |
| 1 | 124.4 /s | 122.5 /s | 124.0 /s | 130.4 /s |
| 2 | 1.78× | 1.77× | 2.00× | 1.85× |
| 4 | 3.05× | 3.04× | 3.47× | 3.21× |
| 8 | **4.29×** | **4.61×** | **4.68×** | **4.65×** |

8 workers give about 4.3 to 4.7×, not 8×: the laptop has 6 physical cores, and the other processes take part of the rest. Results are **bit-identical for 1, 2, 4 and 8 workers** in every row (the same hash of all distances). `tests/determinism.rs` checks the same for 1, 2, 3 and 8 workers on all three distances, and for a whole two-search run with 1 and 4 workers.

Whole solves agree too. Distance A, random start, λ = 16, 6 targets, 3,000-evaluation cap, load 9 to 10 at the start:

| Workers | Checksum of all results | Median wall per solve | Max |
| --- | --- | --- | --- |
| 1 | 14865.314049534 | 23.71 s | 26.16 s |
| 8 | 14865.314049534 | 4.80 s | 5.70 s |

The same results, 4.9× faster per solve.

### Solve rates

20 random targets (the researcher's seeded list), 8 workers, λ = 16, a cap of 5,000 evaluations per search, IPOP restarts, search seed base 1000. A solve stops when the best distance reaches **T1**. **T1 and T3 are proxies, not listening results**: the median distance from the target to 16 renders with every continuous parameter jittered by ±1 % (T1, strict) or ±3 % (T3, loose) of its range, computed with the distance being tested. "Evaluations" and "wall" are to T1 over the targets that reached it; a search that doesn't reach T1 runs to the cap, which is what the "max" column shows (about 8.8 s for 5,000 evaluations at 570 /s, 11 s under heavier load).

| Distance | Start | T1 | T3 | Median evals to T1 | Median wall to T1 | Wall per solve, median / max | Waveform right | Load at start → end |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| A log-mel L1 | heuristic | **17** | 19 | 545 | 1.29 s | 3.47 s / 10.9 s | 20 | 9.8 → 13.0 |
| A | random | 13 | 18 | 1,680 | 3.87 s | 6.27 s / 13.8 s | 19 | 13.0 → 13.8 |
| A | both, side by side | **17** | 19 | 1,105 | 2.16 s | 5.54 s / 21.6 s | 20 | 13.8 → 13.2 |
| B composite | heuristic | 14 | 18 | 913 | 1.63 s | 5.02 s / 10.0 s | 19 | 4.0 → 10.1 |
| B | random | 10 | 16 | 2,128 | 4.30 s | 8.57 s / 10.6 s | 15 | 10.1 → 10.5 |
| B | both | 15 | 19 | 1,809 | 2.98 s | 6.14 s / 19.9 s | 19 | 10.5 → 12.2 |
| C MFCC | heuristic | 11 | 15 | 945 | 1.95 s | 8.15 s / 10.6 s | 18 | 11.9 → 12.1 |
| C | random | 5 | 10 | 3,408 | 6.63 s | 8.86 s / 11.0 s | 15 | 12.1 → 11.3 |
| C | both | 13 | 17 | 2,129 | 4.18 s | 13.49 s / 20.2 s | 18 | 11.3 → 10.0 |

"Both" runs the heuristic-start and the random-start search in one combined batch, each with its own cap of 5,000, and stops at the first T1; its evaluations are summed over both searches. Its worst case is about twice a single search (21.6 s), still far below a minute. The union of the separate A heuristic and A random runs reaches T1 on 18 of 20, so the side-by-side run (17) isn't beating independent luck.

Findings:

- **The heuristic start pays off** for every distance: +4 to +6 targets at T1 over a random start, with 30 to 45 % of the evaluations. It guessed the waveform first on 16 of 20 targets, and the search had the right waveform on 20 of 20 (A).
- **A is the best single objective** at the strict threshold (17 against 14 for B and 11 for C), as in the research. C (MFCC) is clearly worst. This is each distance scored by its own thresholds, so it is not a verdict on what sounds the same.
- **Search noise is about ±2 targets.** The replicates for A with λ = 16 on three search seeds:

  | Start | Seed 1000 | Seed 2000 | Seed 3000 |
  | --- | --- | --- | --- |
  | heuristic | 17 (T3 19) | 14 (T3 17) | 15 (T3 17) |
  | random | 13 (T3 18) | 12 (T3 17) | 10 (T3 17) |

  The heuristic-over-random gap (about 4 targets) holds on all three seeds. Smaller differences between configurations are not evidence.
- **λ = 16 is not better than λ = 10 here.** The research reported λ = 16 beating the default, which this port doesn't reproduce: random start, λ = 10 gives 17 / 10 / 13 on the three seeds against 13 / 12 / 10 for λ = 16; heuristic start gives 15 against 17 / 14 / 15. Median wall per solve is about the same (3.4 s against 3.5 s heuristic, 6.2 against 6.3 s random). Keeping λ a multiple of the worker count (16 for 8) only helps keep the queue full; it isn't needed for the solve rate.
- **BIPOP-style restarts add nothing inside 5,000 evaluations** (A, λ = 16, seed 1000: heuristic 15, random 13, against 17 and 13 for IPOP). IPOP stays the default; the small-population regime would need a bigger budget to matter.

### Common judge

Every final result is re-judged by all three distances against each distance's own thresholds. The table counts targets (of 20) whose result meets the strict threshold, then the loose one, when judged by A / B / C. Rows are the distance that searched; heuristic start.

| Searched with | Judged strict A / B / C | Judged loose A / B / C |
| --- | --- | --- |
| A | 17 / 5 / 5 | 19 / 15 / 17 |
| B | 10 / 14 / 2 | 16 / 18 / 12 |
| C | 11 / 4 / 11 | 15 / 11 / 15 |

Each distance wins its own judge, as it must. B's and C's strict thresholds are much tighter than A's: results found by A mostly miss B's strict bar (5 of 20), which is where the envelope and centroid terms bite. Whether that is the ear disagreeing with A or just B being picky is exactly what the listening test settles. Loosely, A-found results pass C's loose bar on 17 of 20, about as many as C-found results (15).

### Heuristic start quality

`heurcheck`, 20 targets: waveform first guess 16/20 (18/20 in the first two). Median absolute error in normalised units (a random guess is about 0.33): detune 0.011, cutoff 0.125, resonance 0.328, filter-env amount 0.227, filter-env decay 0.304, attack 0.147, decay 0.256, sustain 0.184, release 0.023. Identical to the researcher's probe, which also confirms that the render and heuristic ports are faithful. Resonance and filter-envelope decay are not recovered from the audio by this heuristic.

### Against the researcher's probe

| Setup | Probe (04-final) | This spike |
| --- | --- | --- |
| A, heuristic, λ 16: T1 / T3 | 15 / 17 | 17 / 19 (seeds 1000/2000/3000: 17, 14, 15) |
| A, heuristic: wall per solve, median / max | 2.6 s / 10.6 s | 3.5 s / 10.9 s (1.7 s / 9.4 s on the lightest seed) |
| A, random, λ 16: T1 / T3 | 14 / 16 | 13 / 18 |
| B, heuristic: T1 / T3 | 13 / 19 | 14 / 18 |
| C, heuristic: T1 / T3 | 9 / 14 | 11 / 15 |
| One evaluation, one thread | 7.9 ms | 7.4 to 7.8 ms |
| 8-worker throughput | 4.4 to 5.4× | 4.3 to 4.7× |

B differs by design: this B uses the crest envelope, which the research recommended. Everything else agrees within the ±2 target noise.

## Listening test: not yet run

The owner's sessions are the missing half of Task 6. The tool is built and tested end to end with synthetic answers (`listen --dry-run`, below); no audio was played while building it.

### How `listen` works

- **Stimuli:** 30 target/candidate pairs, 5 from each of 6 distance-A quantile bins, so they run from near-identical to very different. Each pair is two sounds with the scorer's note, 2 s each, loudness-matched to −23 LUFS (BS.1770 integrated, the same measure the scorer uses), with a 10 ms fade-out and a 0.5 s gap. There is no fade-in: the voice starts at exactly zero, and a fade-in would cut the transients of percussive sounds and change what the scorer hears. If either clip would peak above 0.98 after matching, both are scaled down together so the pair stays level-matched. Candidates are perturbed copies of the target (jitter with a log-uniform σ from 0.4 % to 60 % of each parameter's range, sometimes a different waveform) plus a few unrelated patches.
- **Anchors:** 2 identical pairs (expected rating 5) and 2 obviously different pairs (expected 1) in each session. Mean anchor ratings below 4.5 or above 1.5 flag a bad session.
- **Two sessions, 34 trials each:** the same 34 pairs reshuffled, with a fresh random A/B order and fresh random file names, so you are blind to the metric and cannot tell which pairs repeat. Session 1 against session 2 gives the test–retest ceiling that no metric can beat.
- **Answers:** 1 to 5, "how similar are these two sounds" (1 = very different, 5 = indistinguishable). Saved to `answers-session{1,2}.json` after every answer, so you can quit and resume.
- **Analysis:** for each distance, Kendall τ-b and Spearman ρ between the negated distance and your mean rating, 95 % bootstrap CIs over pairs (2,000 resamples); triplet agreement (over pairs you rated differently, how often the metric orders them the same way), also for the closer half of the bins only; paired τ-b differences between distances; the test–retest τ-b and ρ. A distance **passes the provisional gate** when ρ ≥ 0.7, its lower CI bound is ≥ 0.4, and τ-b is within 0.15 of your own test–retest τ-b. If the τ-b difference between the best distance and A has a CI that includes 0, the report recommends A, the simpler. With n = 30, ρ = 0.7 has a 95 % CI of about [0.42, 0.86], so only large differences between distances can be separated.

### How to run it (about 2 × 15 minutes)

You need headphones and PipeWire (`pw-play` is at `/usr/bin/pw-play`). From the repo root, after merging the branch:

```
cargo build --release -p spike-metric

# Session 1. The first call designs the pairs (about 3 s) and writes ~29 MB of WAV files.
./target/release/spike-metric listen --dir ~/enth-listen --session 1

# Session 2, ideally on another day. Same command, same folder.
./target/release/spike-metric listen --dir ~/enth-listen --session 2

# After both sessions: correlations per distance, written to ~/enth-listen/listen-report.json too.
./target/release/spike-metric listen --dir ~/enth-listen --analyze
```

In each session: set the headphone volume once and don't touch it between trials or sessions. Each trial plays the two sounds (4.5 s) and asks for a rating. Type `1` to `5` and Enter, `r` to replay, `q` to quit (run the same command to resume). Don't open `manifest.json`: it holds the distances and would unblind you. Another player works too: `--player <command that plays one WAV>`.

To see the whole flow without any sound, run `./target/release/spike-metric listen --dry-run --dir /tmp/enth-dry`. It designs the pairs, answers both sessions with synthetic ratings made from the distances, and prints the analysis (marked as synthetic). It never calls a player and keeps everything in `/tmp/enth-dry/dry-run`. A full-size dry run took 3 s to design and about 20 s of CPU in all.

### What to do with the result

- Gate passes for A (and A is not beaten): ADR-0007 stays as written, then move to *Accepted* after R1's friends' ratings.
- A passes but B or C is separated from A with a CI excluding 0: switch v1 to that distance (the components are already computed).
- Nothing passes, or the anchors fail: first check the session (headphones, attention, volume). If the sessions are sound, the metric needs work before the Recreate score depends on it; the roadmap's fallback is to score by structure on stepped parameters at the lowest difficulties.

## Not done

- **No ABX "sounds the same" check on the solved results.** The research proposed ABX on the 20 results of the winning distance; this needs the owner's ears and has no tool yet. Until then "solved" means the T1 or T3 proxy, nothing more. The listening test itself rates designed pairs, not solver outputs.
- **The listening sessions themselves**, as above.
- **No A-versus-B decision.** The data above cannot make it; see the common judge.
- **One synth, one note, one voice.** Real templates with modulation, effects or several notes make an evaluation dearer and the landscape harder. The 7.4 ms evaluation is for 1 voice.
- **The cross-platform tolerance** was not re-measured here (the research measured ≤ 2.7e-6 relative between native and wasm). The render is bit-identical across platforms by construction (`libm` plus IEEE arithmetic, pinned hash); the features use `std` maths in `ebur128` and are only equal within a tolerance.
- **YIN f0** is built and tested (a sine at 110 to 880 Hz within 5 cents; the rendered note within 15 cents) but is not part of A, B or C.

## What to change in the plan

- [design.md](../design.md) §7 budget: use **evaluations of 7 to 8 ms** (render + analysis), **4.3 to 4.7×** at 8 workers on this 6-core machine, and **a 5,000-evaluation cap** (median solve 3.5 s, worst 11 s for one search, 22 s for two side by side).
- design.md §8: the heuristic start is confirmed (+4 to +6 targets at T1); a heuristic and a random search side by side did not beat one heuristic search here (17 against 17), so don't promise the portfolio yet. The λ-to-workers rule is about queue use, not solve rate.
- design.md §5: the render is bit-identical across platforms when written with `libm` and IEEE operations only; analysis is equal only within a tolerance (about 1e-5 relative on distances).
- phase-0.md Task 6: record that the listening protocol is two sessions of 34 trials with anchors and a retest, and that "recovery success rate and time" are in this file.
- [ADR-0007](../adr/0007-metric-v1.md) is added as *Proposed*.

## How to reproduce

```
cargo build --release -p spike-metric                     # below, `spike-metric` is ./target/release/spike-metric
spike-metric heurcheck                                                          # heuristic accuracy
spike-metric bench --reps 300 --evals 1200 --workers 1,2,4,8 --batches 16,40   # cost and scaling
spike-metric solve --distance a --start all --targets 20 --workers 8            # one row group of the solve table
spike-metric solve --distance a --start random --targets 6 --lambda 16 --budget 3000 --workers 1   # compare checksums with --workers 8
spike-metric listen --dry-run --dir /tmp/enth-dry
cargo test -p spike-metric                                                      # 36 tests, also part of `cargo test --workspace`
```

Run timings under `flock -x /tmp/enth-quiet.lock` on a machine with nothing else running; note `uptime`. `--distance` takes `a|b|c|all`, `--start` takes `heuristic|random|both|all`, `--restarts ipop|bipop`, `--json FILE` keeps every per-target result.

The tests (all part of `cargo test --workspace`, about 30 s once built) cover: same seed gives the same render hash and the hashes are pinned; 1, 2, 3 and 8 workers give bit-identical distances for A, B and C, and a whole search is identical for 1 and 4 workers; CMA-ES converges on a quadratic (to 1e-14 without the restart rule), on an ill-conditioned ellipsoid, and lands on the bound when the optimum is outside the box; identical audio scores 0 for A, B and C, distances are symmetric, loudness is normalised away, small changes score below large ones, a wrong waveform scores farther than a small detune change; YIN finds the note; the listening design is balanced and deterministic, stimuli are level-matched, the analysis rewards a matching metric and finds nothing in noise; and the binary runs `listen --dry-run`, `solve` and `bench` end to end.
