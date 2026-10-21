# Listening tools: what to run, how long it takes, what it decides

- Date: 2026-10-19 (P1a lane T); test 4, the real-engine pairs, 2026-10-21 (FU1 lane PAIRS); test 5, GR3, 2026-10-21 (R3 lane LT). Nothing in P1a waits for these; they close gates and need your ears and headphones.
- Source: [`tools/listen`](../tools/listen) (package `enth-listen`) and, for the metric pairs test, [`spikes/spike-metric`](../spikes/spike-metric/).
- Everything is blind: file and trial names are random, the order is shuffled and the answer key is in `manifest.json` (`pairs-manifest.json` for test 4), which you must not open. Clips are level-matched with EBU R128 (-23 LUFS, so set the volume once and leave it).

## Once: build

```bash
cd ~/code/enth
cargo build --release -p enth-listen -p spike-metric        # about 3 minutes the first time
```

Sound goes through `pw-play --media-role=Production`, because your PipeWire keeps the default "Music" role muted. To use something else, add `--player "paplay"` (the WAV path is appended to the command). `--dry-run` skips playback entirely and answers with synthetic answers marked as such; every tool also takes `q` to quit (answers are saved after each one, so the same command resumes), `ra`/`rb`/`rx` to replay one sound and `r` to replay all.

## The five tests

| Test | Decides | Runs | Total |
| --- | --- | --- | --- |
| 1. ABX on solver results | G0 search gate, ADR-0007's proxy thresholds | 8 sessions of 24 trials | about 50 min, in as many sittings as you like, plus a 3-7 min design |
| 2. Blind reverb A/B | G1, ADR-0006 (our reverb or the fundsp fallback) | 1 session of 36 trials | about 15 min, plus a 10 s design |
| 3. Metric pairs (existing) | G0 metric gate, ADR-0007 | 2 sessions of 34 trials, ideally on different days | about 30 min |
| 4. Real-engine pairs | `sound/2`'s curve (sound % from distance) on real-engine sounds, and the pitch guard's unit (ADR-0007's 2026-10-20 notes) | 1 session of 44 trials; a retest on another day is optional | about 15-20 min, plus a 1-2 min design |
| 5. GR3: model + search against search alone | GR3's clause 2 ([r3.md](r3.md#gr3-and-its-hygiene)): is the model's patch closer to a real or degraded sound than search alone's? | 1 session of 67 trials (60 references and 7 catch trials) for you; a kit of 37 for each of two friends | about 25 min for you, about 15 min per friend, plus 1-2 min to design |

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

### 5. GR3: is model + search closer to the reference than search alone?

GR3's second clause ([r3.md](r3.md#gr3-and-its-hygiene), report 04 §3.2): on real and degraded sounds, a blind listening test prefers model + search (**arm B**) to R3a's best search alone (**arm A**). The tool is `enth-listen gr3` (`tools/listen/src/gr3.rs`, lane LT). It needs the **set manifest** that lane EVK writes once the arms have run (below); until then use `--dry-run`, which builds its own synthetic set.

```bash
cd ~/code/enth
cargo build --release -p enth-listen                                                     # if you have not yet
./target/release/enth-listen gr3 --set <set.toml> --dir ~/enth-gr3                       # designs once (1-2 min), then your 67 trials; q quits, the same command resumes
./target/release/enth-listen gr3 --dir ~/enth-gr3 --export-kit ~/enth-gr3-kits           # the friends' kits: ~/enth-gr3-kits/kit1 and kit2
./target/release/enth-listen gr3 --dir ~/enth-gr3 --analyze                              # when you and both friends are done
```

**Your session: about 25 minutes** (the printed estimate uses the real clip lengths: each trial's audio once, a fifth more for replays, 7 s to answer). Each trial plays **R** (the reference), **A**, **B** with 0.8 s between them; type `a` or `b` for the one that is closer to R. There is no "same" answer: go with your first impression. `rr`, `ra`, `rb` replay one sound, `r` all three, `q` quits; the answer is saved after every trial and the same command resumes. The screen shows only `[n/67]`: no reference, stratum or arm is ever named. **Do not open `manifest.json`** (the key).

**What a trial is.**

| Rule | Value |
| --- | --- |
| Candidates | Each arm's top-ranked candidate by its own objective, from the set manifest; never a human pick |
| Reference | The sound itself; for a **degraded** upload, the **clean original** (the arms searched the degraded file, you judge against the clean one) |
| Level | Every clip, the reference included, at −23 LUFS (BS.1770 through the same `ebur128` pin as `enth-analysis`, stereo, the ungated fallback for very short sounds); a trial with a clip over −1 dBTP after matching is lowered **as a whole** (the key lists how far), so the clips of a trial stay level-matched |
| Order | Which arm plays first is dealt per reference, alternating inside each (stratum, degraded) group, so half of every group has B first; a friend hears each reference in the opposite order to yours, so every reference is heard in both orders |
| Catch trials (7 per listener) | 2 **identical pairs** (A and B are the same candidate: no right answer, they show a bias towards a position) and 5 **attention** trials (one arm's candidate against the obviously wrong one, the Init patch rendered with the same performance; the wrong one is first in two or three of them). Attention trials use clean references that have a `wrong` file. A friend's catch trials use references outside their kit when there are enough, so a candidate is not heard in a catch trial and again in a main trial |
| Exclusion | A listener under **80 %** on the attention trials is excluded from the preference. With five of them, **one miss is 80 %: the listener is kept; two misses are 60 %: the listener is out**. (A listener who can no longer reach 80 % because of misses plus unanswered trials is out as well.) |
| Short sounds | Never filtered by duration; a clip is not touched beyond its gain |

**The set manifest** (`enth.listen.gr3-set/1`, TOML; what EVK and DS fill). Relative paths are relative to the file. Audio is WAV: PCM 16, 24 or 32 bit or float 32 or 64 bit (`enth render`'s default), mono or stereo (mono is copied to stereo). Other formats (the Ogg previews of `content/gr3`, MP3, FLAC) are converted to WAV first, for example `ffmpeg -i in.ogg out.wav`; the tool refuses anything else and names the file. **Every clip is played at 48 kHz:** a file at another rate (the arms render at their input file's own rate, so a clean 96 kHz original, a degraded upload and a phone recording rarely agree) is converted by a Kaiser-windowed sinc resampler (cutoff at 97 % of the lower Nyquist frequency, about 90 dB of stopband; a tone keeps its level to 0.01 dB), and a file already at 48 kHz is not touched. The key records each file's own rate.

```toml
schema = "enth.listen.gr3-set/1"
name = "gr3-2026-10"                          # free text, printed in the report
arm_a = "search alone (R3a, tuned on dev)"    # optional labels; arm B is the arm the rule tests
arm_b = "model + search"

[[item]]
id = "r017"                                   # the reference id: unique, stable; only the key and the report show it
stratum = "fm-wavetable"                      # reporting group, a free string: one report row per stratum
degraded = true                               # the arms were given a degraded upload
reference = "audio/r017-upload.wav"           # the audio the arms were given
clean_reference = "audio/r017.wav"            # REQUIRED when degraded, forbidden otherwise: what you hear as R
a = "cands/r017-a.wav"                        # arm A's top-ranked candidate
b = "cands/r017-b.wav"                        # arm B's top-ranked candidate
wrong = "cands/r017-init.wav"                 # optional: the Init patch, same performance (needed for at least 5 clean items)
```

The judged reference is `clean_reference` when there is one, else `reference`. Items judged against the **same file** (a degraded copy and its clean original) form **one cluster** of the bootstrap. The tool checks the schema, unique ids, the degraded rules and that every file exists, and records the SHA-256 of the set file and of every audio file in the key: a later run that finds a changed file refuses and names it.

**The friends' kit** (`--export-kit <dir>`, about 15 minutes each). The folder is designed for two kits (`--friends 2`, the default; `--friends 0` for an owner-only test; decided when the folder is first designed). The references are dealt to the kits inside each (stratum, degraded) group, so each kit has half of every group, 30 of the 60, plus its 7 catch trials. Each of `kit1`, `kit2` holds:

- `NN-K-xxxxxx.wav`: three random-named WAVs per trial, **NN** the trial number, **K** 1 for the reference, 2 for sound A, 3 for sound B, so sorting by name puts them in playing order; 24-bit stereo, already level-matched. Any player on any system works.
- `answers.txt`: `kit: kitN`, instructions and one empty `NN:` line per trial; the friend types `a` or `b` after the colon (blank skips a trial, which keeps the result provisional; case, `=`, `-` and spaces are forgiven) and sends the file back.
- `README.txt`: what to do, in a few sentences.

Send each folder (zipped) to one friend and nothing else: never `manifest.json` or `--dir`. When a file comes back, save it as `<name>.txt` in `~/enth-gr3/friends/`; the name is the listener's label in the report (use "friend-a", not a real name). A typo in a file is reported with its line number, never guessed at.

**The decision rule, fixed in advance** (`--analyze`; code in `tools/listen/src/gr3/analyze.rs` and `boot.rs`):

1. Listeners under 80 % on attention are excluded; catch trials never enter the preference.
2. The estimate is the pooled share of the included judgements where arm B's candidate was called closer.
3. **B is preferred** when the lower end of the 95 % **cluster-bootstrap** interval is above 50 % **and** the degraded subset's point estimate is at least 50 %. Strata are reported, not gated.
4. The bootstrap resamples **references, then listeners**, 10,000 replicates, from a fixed seed (`0x475233424f4f5453`, "GR3BOOTS"; deliberately not a flag). A replicate draws as many clusters as there are, with replacement; then, for each drawn cluster, as many of the listeners who judged it, with replacement; its value is wins over judgements. The interval is the 2.5th and 97.5th percentile (linear interpolation between order statistics); "the lower bound" is the 2.5th. With one listener per reference the second stage changes nothing, and with two it is the more cautious reading, which the report shows (the interval over references alone is printed beside it, not used).
5. The report states the **observed design effect** and the **observed share**, next to what the plan assumed (1.2 and 65 %): the bootstrap variance of the references-only replicates over the variance of the same share if every judgement were an independent trial, `p(1 − p)/n`; and the effective sample size `n / deff`.

`--analyze` shows **nothing but progress** until you and every kit have answered every trial (the listening key stays closed until every listener is done); `--partial` overrides that when a friend will not answer, and the result is then marked PROVISIONAL. It prints per-listener results (answered, attention, how often each listener picked the first sound, with an exact p-value against a fair coin: the order is balanced, so this is a position bias; the identical pairs), the pooled result with both intervals, the Wilson interval (ignoring clustering), clean and degraded, a row per stratum, each included listener's share, and writes `gr3-report.json` (`enth.listen.gr3-report/1`). "Whoever prepares stimuli does not audition them": the people who run EVK should not be the ones who listen, or should say so in the report.

**Files** (in `--dir`):

| File | Schema | Holds |
| --- | --- | --- |
| `manifest.json` | `enth.listen.gr3-manifest/1` | The key: the seed, the set file's path and hashes, every reference's stratum and cluster, every trial of every listener (which arm plays in A and B, the kind, the random clip names, the gains, the true peak), the number of kits. Do not open it before everyone is done |
| `clips/<id>.wav` | 24-bit stereo | Your clips, random names, already scaled |
| `answers.json` | `enth.listen.gr3-answers/1` | Your answers, saved after each one (`n`, trial id, `a`/`b`, replays, milliseconds) |
| `friends/<name>.txt` | text | Each friend's returned `answers.txt` |
| `gr3-report.json` | `enth.listen.gr3-report/1` | What `--analyze` prints |

**Checked against Python.** `tools/listen/tests/gr3/bootstrap_ref.py` (standard library only; run it as `python3 -I`) implements the same procedure: the same `SplitMix64` stream and the same draws in the same order, so the two agree to rounding, not just statistically. `cargo test -p enth-listen --test gr3` compares the Rust bootstrap (a) with the numbers the script printed for a committed synthetic answers file (`synthetic.json` -> `expected.json`: 40 clusters, 3 listeners, 120 judgements; pooled, clean, degraded and each stratum, to 1e-12; always run), and (b) with a live run of the script, on that file and on the judgements of a dry run (skipped with a note if `python3` is missing; `ENTH_REQUIRE_PYTHON=1` turns that into a failure). To regenerate: `python3 -I bootstrap_ref.py synth > synthetic.json; python3 -I bootstrap_ref.py boot synthetic.json > expected.json`.

**What it is for.** GR3's clause 2 passes when the report says `B PREFERRED`. Owner-only detects about a 70 % preference; with two friends and a design effect near 1.2, a true 65 % puts the lower bound near 55 % ([r3.md](r3.md#gr3-and-its-hygiene)). The design effect and the 65 % are assumptions: the report prints what was observed.

## Checking the tools without listening

```bash
./target/release/enth-listen reverb-ab --dry-run --dir /tmp/enth-reverb-dry      # 7 s: design, synthetic answers, report
./target/release/enth-listen abx --dry-run --dir /tmp/enth-abx-dry               # the full 20-target design (3-7 min) and a synthetic report
./target/release/enth-listen abx --dry-run --targets 4 --workers 2 --dir /tmp/enth-abx-dry-small
./target/release/enth-listen pairs --dry-run --dir /tmp/enth-pairs-dry          # the 40-pair design (1-2 min), synthetic answers from sound/1's own curve, the report
./target/release/enth-listen gr3 --dry-run --dir /tmp/enth-gr3-dry                # about 20 s in a debug build, less in a release one: a synthetic 60-reference set, the design, both kits, synthetic answers, the report
./target/release/enth-listen gr3 --dry-run --set <set.toml> --dir /tmp/enth-gr3-dry   # the same on EVK's real set: checks its files, levels and true peaks before anyone listens
```

Dry runs live in `<dir>/dry-run`, never call a player and label the report SYNTHETIC. Do not rely on their numbers. (GR3's dry run makes kit 2 miss two attention trials on purpose, to show the exclusion.)

## What to send back

`~/enth-abx/abx-report.json`, `~/enth-reverb/reverb-report.json`, `~/enth-listen/pairs-report.json` and `~/enth-listen/pairs-fit.json`, `~/enth-gr3/gr3-report.json` (never `manifest.json` before everyone is done), and the printed reports. Clicks, discontinuities and level corrections found in the clips appear only in the reports (never during a session, where they would give the answer away). The agent that receives them updates ADR-0006, ADR-0007 and the roadmap, and fits `sound/2` from `pairs-fit.json` as test 4 describes.
