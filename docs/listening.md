# Listening tools: what to run, how long it takes, what it decides

- Date: 2026-10-19 (P1a lane T); test 4, the real-engine pairs, 2026-10-21 (FU1 lane PAIRS). Nothing in P1a waits for these; they close gates and need your ears and headphones.
- Source: [`tools/listen`](../tools/listen) (package `enth-listen`) and, for the metric pairs test, [`spikes/spike-metric`](../spikes/spike-metric/).
- Everything is blind: file and trial names are random, the order is shuffled and the answer key is in `manifest.json` (`pairs-manifest.json` for test 4), which you must not open. Clips are level-matched with EBU R128 (-23 LUFS, so set the volume once and leave it).

## Once: build

```bash
cd ~/code/enth
cargo build --release -p enth-listen -p spike-metric        # about 3 minutes the first time
```

Sound goes through `pw-play --media-role=Production`, because your PipeWire keeps the default "Music" role muted. To use something else, add `--player "paplay"` (the WAV path is appended to the command). `--dry-run` skips playback entirely and answers with synthetic answers marked as such; every tool also takes `q` to quit (answers are saved after each one, so the same command resumes), `ra`/`rb`/`rx` to replay one sound and `r` to replay all.

## The four tests

| Test | Decides | Runs | Total |
| --- | --- | --- | --- |
| 1. ABX on solver results | G0 search gate, ADR-0007's proxy thresholds | 8 sessions of 24 trials | about 50 min, in as many sittings as you like, plus a 3-7 min design |
| 2. Blind reverb A/B | G1, ADR-0006 (our reverb or the fundsp fallback) | 1 session of 36 trials | about 15 min, plus a 10 s design |
| 3. Metric pairs (existing) | G0 metric gate, ADR-0007 | 2 sessions of 34 trials, ideally on different days | about 30 min |
| 4. Real-engine pairs | `sound/2`'s curve (sound % from distance) on real-engine sounds, and the pitch guard's unit (ADR-0007's 2026-10-20 notes) | 1 session of 44 trials; a retest on another day is optional | about 15-20 min, plus a 1-2 min design |

### 1. ABX: does the search result sound the same as the target?

```bash
./target/release/enth-listen abx --dir ~/enth-abx                    # designs: 20 searches on 8 workers, then lists the sessions
./target/release/enth-listen abx --dir ~/enth-abx --session next     # one session (about 6 min); repeat until all 8 are done
./target/release/enth-listen abx --dir ~/enth-abx --analyze          # per target: same or different, and the G0 count
```

- The design runs the real search (CMA-ES, distance A, 5,000 evaluations, 60 s cap per target) for 20 seeded targets on 8 workers, so it uses the whole machine: close other work first, or the cap is hit by contention rather than by the search. The report prints the load average. It writes about 12 MB of clips. It takes 3-7 min (6.5 min on a machine at load 36).
- Each trial plays A, B, then X, where X is an exact copy of A or B. Type `a` or `b` for what X is. Each session has one trial per target (target and solution in balanced order), 2 trials where A and B are identical (just guess) and 2 obviously different pairs, which check that you are listening.
- A target counts as *different* only if you are right in 7 of its 8 trials (one-sided exact binomial test, p <= 0.05). Everything else is *same*, which means "not caught", not "proved identical". The report states how often a listener who hears the difference 85 % of the time is caught (66 % with 8 trials), and shows a stricter count (p <= 0.15).
- **G0 passes at 14 or more of 20 judged same.** If fewer, ADR-0007 says the proxy thresholds are too loose; the report also shows whether the targets whose search reached the strict proxy (T1) were the ones that sounded the same. To see whether more search helps, design a second set with `--stop-at never --dir ~/enth-abx-long`.
- Stimuli are the spike-metric toy synth (10 parameters) and its CMA-ES solver, used as a library with the spike's seeded targets and thresholds: the real engine has only a reference saw and a pass-through filter so far, and `enth-solver` is empty. Switch to the engine when its oscillators and filter land.

### 2. Reverb A/B: our Signalsmith port against fundsp

```bash
./target/release/enth-listen reverb-ab --dir ~/enth-reverb              # designs (10 s), then 36 trials
./target/release/enth-listen reverb-ab --dir ~/enth-reverb --analyze
```

- Three dry sounds from the real engine (a slow chord pad, four plucks, two percussive hits) go through our `reverb` module, fundsp 0.23 `reverb2_stereo` and `reverb4_stereo`, each calibrated to the same measured RT60 (2 s) and the same stereo loudness. Each trial plays A then B: say which reverb you prefer (`a`, `b`, or `=` if you truly cannot choose; use it sparingly). Every source and pair of reverbs is heard 4 times, in both orders.
- The percussive hits are peak-limited, so that source plays about 3 dB quieter than the others; clips within one source are matched to 0.0 LU.
- `--analyze` gives preference with a 95 % confidence interval per pair, per source and overall, your own consistency on repeats, any bias towards A or B, and a one-line reading.
- **ADR-0006 is confirmed if ours is not clearly worse.** The reading says "this is ADR-0006's condition 'the port loses the blind A/B'" only if a fundsp reverb is preferred over ours with p <= 0.05; then the fallback (`reverb2_stereo` or `reverb4_stereo`, pinned `=0.23.0`) is the one to use. The sources use the placeholder oscillator and filter; render again in a new folder after they land.

### 3. Metric pairs (spike-metric)

```bash
./target/release/spike-metric listen --dir ~/enth-listen --session 1      # about 15 min
./target/release/spike-metric listen --dir ~/enth-listen --session 2      # about 15 min, ideally another day
./target/release/spike-metric listen --dir ~/enth-listen --analyze
```

Rate each pair from 1 (very different) to 5 (indistinguishable). Details and the gate (Spearman rho >= 0.7, lower bound >= 0.4, tau-b within 0.15 of your test-retest) are in [owner-checklist.md](spikes/owner-checklist.md#a-listening-no-windows-headphones) and [metric.md](spikes/metric.md#how-to-run-it-about-2--15-minutes).

### 4. Real-engine pairs: how similar do two sounds seem at a given `sound/1` distance?

```bash
./target/release/enth-listen pairs --dir ~/enth-listen                  # designs once (1-2 min, all cores), then shows the status
./target/release/enth-listen pairs --dir ~/enth-listen --session 1      # about 15-20 min; q quits, the same command resumes
./target/release/enth-listen pairs report --dir ~/enth-listen           # agreement per distance bin; writes pairs-report.json and pairs-fit.json
./target/release/enth-listen pairs --dir ~/enth-listen --session 2      # optional, another day: the same pairs reshuffled (a retest)
```

The files start with `pairs-`, so they sit beside spike-metric's in `~/enth-listen`. Each trial plays **A, B, then X** (an exact copy of A or B): type `a` or `b` for which one X is, then **rate how similar A and B sound**, 1 (very different) to 5 (indistinguishable), as in metric session 2. Some trials play the same sound twice: guess X and rate what you hear.

**What it is for.** `sound/1`'s curve (sound % from the JND distance) is bridged from the toy synth's ratings ([challenge.md](challenge.md#calibration-the-jnd-scale-the-bridge-and-the-curve)). These ratings are on real-engine sounds at known `sound/1` distances, so a later lane can fit `sound/2`'s curve to them, and check the pitch guard's unit, without changing `sound/1` or its pins.

**The pairs** (lane PAIRS' design; `tools/listen/src/pairs/plan.rs` and `design.rs`):

| Choice | Why |
| --- | --- |
| 40 test targets from the frozen generator v1, levels 1-5 in turn (8 each), the level presets, 40-bit seeds | The sounds the game deals. Each target has a short code, so it can be pasted into the game |
| Each answer is its target moved by *m* of `sound/1`'s own calibration steps (10 % for Hz and times, 2 dB for levels, 10 ct for pitch, 5 points for other percentages) | The JND unit is defined by these steps (the median distance of the one-step jitter set), so the distances fall where planned; the move is deterministic |
| Three moves: every observable parameter (`all`, 14 pairs), one parameter the generator randomized (`one`, 13), osc 1's pitch (`pitch`, 13) | `all` is the scale's own move and the kind of pair the toy synth was rated on (the bridge assumes they are heard alike at equal JND); `one` is a realistic near-miss (everything right but one knob); `pitch` weighs the pitch guard (25 ct per JND), whose unit ADR-0007's note leaves to this session |
| Planned bins in `sound/1` JND: 0-0.5 (5 pairs), 0.5-1 (8), 1-2 (8), 2-4 (7), 4-8 (7), 8+ (5) | The pass mark is at 0.99 JND and 50 % at 5.11. Dense on both sides of the pass mark, where the curve matters most; 4-8 pins the 50 % point; the ends anchor the flat parts |
| The step count is picked per pair from a ladder (0.1 to 32 steps, about √2 apart): the one whose distance lands in the planned bin, nearest its centre | A fixed *m* lands anywhere (a quarter step of everything is already half a JND on some targets). On the default plan 37 of 40 land in their bin (4/8/9/8/6/5 pairs per bin; engine 0.2.0, 2026-10-21) |
| Checks: 2 identical pairs, 2 obviously different (a level-1 against a level-5 target: another wave, every oscillator an octave or more apart) | As in tests 1 and 3: attention and use of the scale. Identical pairs should be rated 5, different ones 1 |
| A, B, X, then the rating | The rating is the curve's input, as in session 2. X adds a bias-free check per bin: 75 % right is the usual "just noticeable", so it should cross 75 % near 1 JND if `sound/1`'s scale is right |
| Both clips through the target's phrase, at the scoring rate and seed, in stereo; level-matched as the game's A/B (`ab_gains_db` on the mid channel), then the pair at -23 LUFS; a 10 ms fade-out; a shared peak limit at -0.18 dBFS | You hear what a player hears in A/B, at one volume for the whole session |
| The plan (targets, moves, planned bins, orders, file names) comes from the seed alone, without audio; its hash is pinned (`232f245171b51b20` for seed 20261021 and 40 pairs) | It never moves with a DSP change (lane DRV's Drive fix moves the audio): only the chosen step counts and the distances do, and they are the same on every machine for one engine version |

**Files** (in `--dir`):

| File | Schema | Holds |
| --- | --- | --- |
| `pairs-manifest.json` | `enth.listen.pairs-manifest/1` | The key: every pair's plan, code, move, step count, ladder of distances and `sound/1` measurement, every clip's gain, every trial. Do not open it before you are done |
| `pairs-clips/<id>.wav` | 24-bit stereo, 48 kHz | Random names; a missing file is rendered again |
| `pairs-session<N>.json` | `enth.listen.pairs-results/1` | Saved after every answer. Per answer, spike-metric's fields (`n`, `id`, `rating`, `replays`, `ms`) plus `x` and `x_correct`, the pair (`pair`, `role`, `level`, `kind`) and its `distance` (metric v1.1), `jnd` and `percent` (the current `sound/1` score) |
| `pairs-report.json` | `enth.listen.pairs-report/1` | What `report` prints |
| `pairs-fit.json` | `enth.listen.pairs-fit/1` | What a `sound/2` fit reads: one row per pair (role, level, move, parameter, code, step count, `distance`, `floor`, `scale`, `z_metric`, the pitch and odd-share differences and floors, `pitch_term`, `wave_term`, `jnd`, `percent`, the rating per session, the mean rating, X per session), `sound/1`'s curve and guard units, and the proposed curve |

**The report** prints, per distance bin (by `sound/1`'s measured JND): the pairs, their median distance and mean rating, how many the ear calls the same (mean rating at least 4.5, the curve's pass point), how many `sound/1` passes (at least 90 %), how many of those two verdicts agree, and the share of X named right with a 95 % interval. Then the pass agreement over all pairs (both pass, ear only, `sound/1` only, neither), Spearman and Kendall of the rating against the distance, the checks, the retest when both sessions are played, the proposed curve and one line on whether `sound/1`'s pass mark is consistent with the ear, stricter or looser.

**How a later lane fits `sound/2`** (the report already does steps 2 and 3 as a proposal):

1. Read `pairs-fit.json`. Refuse `"synthetic": true`. Keep the rows with `"role": "test"` and a `mean_rating`. If the engine has changed since the session (`engine`, against `enth_patch::file::ENGINE_VERSION`), re-measure first: for each pair of the manifest, `enth_listen::pairs::sounds(&pair)` gives the target and the answer, and `measure_pair` on `sound/1` prepared on the target gives the new `jnd`.
2. Fit `rating = 1 + 4 / (1 + (jnd / c)^q)` by least squares (`enth_listen::pairs::fit_ratings`, or any optimiser). Rating 3 is the 50 % point and rating 4.5 the pass mark, so `sound/2`'s curve is `Curve { z50: c, k: q · ln 9 / ln 7 }`, and the pass mark stays 90 at `jnd = c · 7^(−1/q)`.
3. Adopt it only if the checks are sound (identical at least 4.5, different at most 1.5), the fit is not at the edge of its range (`at_bound`), Spearman's lower bound is well above 0, and the pass point's interval is narrow enough to matter; compare with the retest if there is one.
4. The guard's units, if the data support it: replace `jnd` by `z_metric + max(0, pitch_delta_ct − pitch_floor_ct) / u_p + max(0, odd_delta − odd_floor) / u_w` and fit `c`, `q`, `u_p` (now 25 ct) and `u_w` (now 0.25) together; the `pitch` rows carry `u_p`. Change a unit only when its interval excludes the current value. No pair moves the waveform directly (it is discrete); the wave term appears where pulse widths or filters move it.
5. GR1's blind ratings (`RoundResult::rating`, against each first submit's `jnd`) are the same kind of point: fit them alone and together with these, and report both. `sound/2` lands as a new `Curve` constant and a new arm in `score::sound::prepare`, and `sound/1` stays as it is ([challenge.md](challenge.md#how-sound2-replaces-sound1)).

## Checking the tools without listening

```bash
./target/release/enth-listen reverb-ab --dry-run --dir /tmp/enth-reverb-dry      # 7 s: design, synthetic answers, report
./target/release/enth-listen abx --dry-run --dir /tmp/enth-abx-dry               # the full 20-target design (3-7 min) and a synthetic report
./target/release/enth-listen abx --dry-run --targets 4 --workers 2 --dir /tmp/enth-abx-dry-small
./target/release/enth-listen pairs --dry-run --dir /tmp/enth-pairs-dry          # the 40-pair design (1-2 min), synthetic answers from sound/1's own curve, the report
```

Dry runs live in `<dir>/dry-run`, never call a player and label the report SYNTHETIC. Do not rely on their numbers.

## What to send back

`~/enth-abx/abx-report.json`, `~/enth-reverb/reverb-report.json`, `~/enth-listen/pairs-report.json` and `~/enth-listen/pairs-fit.json`, and the printed reports. Clicks, discontinuities and level corrections found in the clips appear only in the reports (never during a session, where they would give the answer away). The agent that receives them updates ADR-0006, ADR-0007 and the roadmap, and fits `sound/2` from `pairs-fit.json` as test 4 describes.
