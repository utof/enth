# Listening tools: what to run, how long it takes, what it decides

- Date: 2026-10-19 (P1a lane T). Nothing in P1a waits for these; they close two gates and need your ears and headphones.
- Source: [`tools/listen`](../tools/listen) (package `enth-listen`) and, for the pairs test, [`spikes/spike-metric`](../spikes/spike-metric/).
- Everything is blind: file and trial names are random, the order is shuffled and the answer key is in `manifest.json`, which you must not open. Clips are level-matched with EBU R128 (-23 LUFS, so set the volume once and leave it).

## Once: build

```bash
cd ~/code/enth
cargo build --release -p enth-listen -p spike-metric        # about 3 minutes the first time
```

Sound goes through `pw-play --media-role=Production`, because your PipeWire keeps the default "Music" role muted. To use something else, add `--player "paplay"` (the WAV path is appended to the command). `--dry-run` skips playback entirely and answers with synthetic answers marked as such; every tool also takes `q` to quit (answers are saved after each one, so the same command resumes), `ra`/`rb`/`rx` to replay one sound and `r` to replay all.

## The three tests

| Test | Decides | Runs | Total |
| --- | --- | --- | --- |
| 1. ABX on solver results | G0 search gate, ADR-0007's proxy thresholds | 8 sessions of 24 trials | about 50 min, in as many sittings as you like, plus a 3-7 min design |
| 2. Blind reverb A/B | G1, ADR-0006 (our reverb or the fundsp fallback) | 1 session of 36 trials | about 15 min, plus a 10 s design |
| 3. Metric pairs (existing) | G0 metric gate, ADR-0007 | 2 sessions of 34 trials, ideally on different days | about 30 min |

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

## Checking the tools without listening

```bash
./target/release/enth-listen reverb-ab --dry-run --dir /tmp/enth-reverb-dry      # 7 s: design, synthetic answers, report
./target/release/enth-listen abx --dry-run --dir /tmp/enth-abx-dry               # the full 20-target design (3-7 min) and a synthetic report
./target/release/enth-listen abx --dry-run --targets 4 --workers 2 --dir /tmp/enth-abx-dry-small
```

Dry runs live in `<dir>/dry-run`, never call a player and label the report SYNTHETIC. Do not rely on their numbers.

## What to send back

`~/enth-abx/abx-report.json`, `~/enth-reverb/reverb-report.json` and the printed reports. Clicks, discontinuities and level corrections found in the clips appear only in the reports (never during a session, where they would give the answer away). The agent that receives them updates ADR-0006, ADR-0007 and the roadmap.
