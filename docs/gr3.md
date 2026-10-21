# GR3: the kit for clause 1 (model + search against search alone)

- Date: 2026-10-21 (R3 lane EVK; lane EVb adds the arm-B runs). Job schema `enth.gr3-job/1`, runs schema `enth.gr3-run/1`.
- What it is: the protocol of GR3's clause 1 as built, the two commands, and how to read what they print. Clause 1: on held-out synth sounds, model + search beats the best search alone at equal time ([r3.md](r3.md#gr3-and-its-hygiene); the gate is in [phase-3.md](phase-3.md#the-gr2-and-gr3-checks)). Clauses 2 and 3 (listening, under 5 minutes) are lanes LT, SK3 and INTa.
- Code: `xtask/src/gr3.rs` and `xtask/src/gr3/` (the thin driver, the runs file, the statistics, the report); `tools/datagen/src/gr3.rs` and `tools/datagen/src/gr3/` (`enth-datagen gr3 run`: the arms with the engine crates in-process). xtask stays light: no engine crate.
- The sets are lane DS's, frozen in `content/gr3/` ([its README](../content/gr3/README.md)). Every test run is appended to `content/gr3/runs.jsonl`, and none is dropped.

Everything is local: no network, no telemetry, no paid run. Nothing here plays audio.

## Status

| Part | State |
| --- | --- |
| `gr3-run`: four arms, wall-clock and render modes, twins, the idle-rate probe | Built. Arm A and D run now. Arms B and C run only from a proposals file, as a dry run on dev, until R3b's model exists ([Arm B](#arm-b-where-the-model-plugs-in)) |
| `gr3-report`: the clause-1 statistics, fixed in advance | Built; checked against hand values (`stats.rs`, `report.rs` tests) |
| Dev tuning of arm A (`first_rung`, K, `min_spread` against the bank's size, the scored window) | Done: create's defaults kept; no variant did better on dev ([Dev tuning](#dev-tuning-arm-a)) |
| The arm-A baseline | Recorded on the 40-target core as a **render-equivalent estimate** (no quiet window): **29 of 40 pass (72.5 %), median 0.40 JND** at 4,000 renders ≈ 60 s ([The arm-A baseline](#the-arm-a-baseline)). The rest of the set waits for a quiet night |
| The idle-rate probe, then the timed numbers | **Open**: one command, about 10 minutes of a quiet machine ([The idle-rate probe](#the-idle-rate-probe)) |

## The protocol as built

### The targets

| Selection | `--set` | What it takes |
| --- | --- | --- |
| The core | `core` (the default) | The 40 pre-declared targets (`core = true`): 32 generator codes (29 one-note, 3 two-note) and 8 renders of 4 test presets |
| The rest | `rest` | Every filled test target outside the core: 74 today (60 generator, 14 preset renders) |
| Available | `available` | Every filled test target: 114 today |
| Full | `full` | All 122. **Refused while the owner's four G1 sounds are placeholders** |
| Dev | `--split dev` | The 30 dev targets: 20 generator codes and 10 anchor-preset renders. All tuning happens here |
| Named | `--ids a,b,c` | Exactly these, from the split |

**The freeze, as `gr3-run` enforces it** (content/gr3/README.md, "The freeze"):

- `synth.toml` must match `hashes.txt`.
- A generator code must regenerate to the fingerprint and phrase the manifest lists, and a preset file must have the listed SHA-256. A moved target is refused.
- **A test run never includes an amendable item whose entry is unfilled** (a G1 target whose `sha256` is still `pending`). This was ruled by the coordinator on 2026-10-21, before any test run, while `runs.jsonl` was empty. The one allowed amendment (the G1 sounds' hashes, the owner-NN slots) stays open until the first test run that includes an item it fills. The arm-A core baseline leaves every amendable item out, so it does not close the amendment.
- A test run needs a clean tree, so its build is a commit (the runs file itself aside). It always appends to `content/gr3/runs.jsonl`: `--out` is refused for test runs, and so is a proposals file. Dev runs go to a scratch file (default `$CARGO_TARGET_DIR/gr3/dev-runs.jsonl`).

### One target, step by step

1. **The upload.** The target patch is rendered through the target's own performance at 48 kHz with the default render seed (which is also `sound/1`'s scoring seed), mono. A generator code plays its phrase: phrase 1 is C3 held 1.5 s in 3 s; phrases 2 and 3 are two-note pairs in 4.75 s. A preset plays its manifest note: note a is the README's key at velocity 100, note b is a fifth up at velocity 80, both held 2 s in 3 s. No audio comes from disk: renders are bit-exact.
2. **The clock starts.** In wall-clock mode the tool first takes the quiet lock exclusively (`flock -x /tmp/enth-quiet.lock`), per target, so other lanes wait at most one target. Inside the clock, in order:
   - FE's intake (`Intake::new`), shared by every arm;
   - `Target::from_intake_scored` with the scored window (3 s by default);
   - the analysis;
   - the arm's portfolio (for arms B and C, the proposer's inference, charged to the clock);
   - the create job: one merged batch per round on a rayon pool of `--workers` threads, distances by index, so the result never depends on the worker count.
3. **Stopping.** Wall-clock mode stops the job from its progress callback once a round ends at or past the cap. Render mode caps it by its budget. Either way the run is an exact prefix of a longer one (the rungs never depend on the budget; [r3.md](r3.md#the-determinism-contract), point 3).
4. **Checkpoints.** After every round the kit keeps the best candidate by the arm's own objective (metric v1 on the scored window, ties to the lower branch). At each mark of the grid it records the state of **the last round that ended at or before the mark**: its renders, its objective distance, the provenance of its start and its patch's SHA-256. The grid is 2, 5, 10, 15, 20, 30, 45, 60, 90, 120, 180, 240, 300 and 600 s, cut at the cap (wall), or 100, 250, 500, 1,000, 2,000, 4,000 … renders (render mode; `--marks` replaces it). The cap is always a mark, and the round that crosses the cap counts towards nothing. **A checkpoint's renders are its render-equivalent twin's budget.**
5. **The clock stops and the lock is released.** Then, untimed, each checkpoint's patch is scored:
   - **`sound/1` against the target patch, through the target's performance**, passing at 90 %: the game's own round for a generator code (`prepare_round`, `score_round`), or a hand-made round through the preset's note;
   - `structure/1`;
   - **the independent metric no arm optimises**: the MFCC mean squared error between the two scoring renders (the Goodhart guard; both arms minimise a log-mel distance, and `sound/1` is log-mel too).
6. **One line is appended and flushed** per target. A failed target gets a `target_failed` line. A run starts with a `run` line and ends with a `run_end` line, so a run that stopped early shows.

**The register shift.** One note cannot tell the key from the oscillators' tuning. A target whose oscillators sit an octave up sounds like the key an octave higher, and the intake rightly reports that (on dev, `gen-d-l1-01` reads as C4 although its phrase plays C3). create then searches at the key it heard. So before scoring, the kit moves every candidate by the intake's key less the key the target's performance starts on, in whole semitones on every enabled oscillator: the rule k-NN transposes neighbours by. That judges the candidate **as played at the note create inferred**, which is how a user hears it. Every arm is judged the same way. The record keeps the shift (`register_shift`) and, when it is not 0, the raw score of the unmoved patch (`raw_score`). Without the shift, the first dev smoke run scored a nearly right sound (89.1 %, 1.06 JND once shifted) at 4.6 % (49 JND), because of its octave alone.

### The arms

They share the upload, the intake, the scored window, the free set, the workers, the seeds and the objective.

| Arm | Portfolio | Budget | Role |
| --- | --- | --- | --- |
| **A** | create's own: the heuristic start, the templates (`pluck_1osc`, `pad_2osc`, `bass_sub`), the k-NN neighbours (`Portfolio::build`) | The cap | The baseline: the best tuned search alone |
| **B** | A's, plus the model's starts as `Provenance::Model` (`Inputs::proposals`, after the neighbours) | The cap; the inference counts in the clock | The claim |
| **C** | The model's starts only | Their count: one screening round, no search | A diagnostic: the model unrefined |
| **D** | A's, with the templates replaced by the oracle template | The cap | A diagnostic: separates structure error from parameter error |

**Arm D's oracle template** is the target's own structure (its modules, their enable flags, its routes and their depths), with every parameter its performance reveals set to the catalogue default. The route depths are kept as structure.

#### Arm B: where the model plugs in

`tools/datagen/src/gr3/arm.rs`, trait `Proposer`:

```rust
pub trait Proposer {
    fn id(&self) -> String;
    fn propose(&self, target_id: &str, target: &Target, analysis: &Analysis) -> Result<Vec<Start>, String>;
    fn charged_s(&self, _target_id: &str) -> f64 { 0.0 }
}
```

`propose` runs inside the clock, after the intake and the analysis, so its time is the arm's time. Today the only proposer is `FileProposer`, which reads `enth.gr3-proposals/1`:

```json
{"schema": "enth.gr3-proposals/1", "model": "id",
 "targets": {"<target id>": {"charge_s": 0.8, "starts": [{"patch": {"…": "a patch file"}, "confidence": 0.9}]}}}
```

It exists for dry runs of the B and C paths on dev; a test run refuses it. The proposals must sit in the frame the intake hears (the register shift above). **Lane EVb** adds a proposer that calls `enth_solver::propose` (lane ONX's kernels and TR's model file) on the target and the analysis, and passes it where `FileProposer` is built in `gr3.rs` (`run_job`). Nothing else changes.

### Modes and twins

| Mode | Stops at | Lock | Use |
| --- | --- | --- | --- |
| `wall` (default) | `--seconds` (60), measured from the intake | `flock -x`, per target | The timed run; also the idle-rate probe |
| `renders` | `--budget N`, or each target's own budget (`--twin-of`) | None (renders do not depend on load) | Dev tuning; twins; render-equivalent runs |

- **A twin** (`--twin-of RUN [--at MARK]`) re-runs a recorded run in render mode. It takes each target's renders at the mark (default the cap) as that target's budget and copies the run's arm, split, settings, bank and workers. Because rungs never depend on the budget, the twin ends on exactly the patch the timed run had at that mark, on any machine. The tool's tests check this (a checkpoint's renders as a budget end on the same patch hash).
- **A render-equivalent run** (`--endpoint R --equivalent-seconds S --rate-source TEXT`) is a render run that stands for a timed one. Its first line records the assumption (`header.equivalence`: the renders, the seconds, the assumed rate and its source), and the report flags it. A measured idle rate replaces the assumption later without running again (`gr3-report --rate-from`), as long as the run's budget covers the renders the measured rate gives.

### Don't wrap it in `flock`

In wall-clock mode the tool takes the quiet lock exclusively. A shared lock held by its own parent would deadlock it, so never run `flock -s … cargo xtask gr3-run`. `gr3-run` builds `enth-datagen` under `flock -s` itself and releases that lock before running. Note that flock has no writer preference: on 2026-10-21 a wall-clock calibration waited 10 minutes without the exclusive lock, because three or four lanes kept overlapping shared holds. Timed runs need a quiet machine.

## Commands

```sh
# Dev tuning (scratch output; render mode is deterministic and ignores load)
cargo xtask gr3-run --arm A --split dev --mode renders --budget 4000 --out ~/scratch/dev.jsonl [--first-rung 25] [--k 4] \
    [--min-spread 0.5] [--bank-entries 2000] [--score-seconds 2]

# The arm-A core baseline as recorded on 2026-10-21 (render-equivalent; a test run: clean tree, appended to runs.jsonl)
cargo xtask gr3-run --arm A --set core --mode renders --budget 8000 --marks $(seq -s, 250 250 8000) \
    --endpoint 4000 --equivalent-seconds 60 --rate-source "TP's cost table (docs/r3.md): ..." --bank <bank>

# Timed (a quiet machine): the core at 60 s on the CLI's 8 workers, or the plugin's 4
cargo xtask gr3-run --arm A --set core --seconds 60 --workers 8

# The idle-rate probe (about 10 minutes, flock -x per target; writes $CARGO_TARGET_DIR/gr3/rate-probe.jsonl)
cargo xtask gr3-run --rate-probe

# A twin of a timed run (render mode, each target's renders at 60 s as its budget)
cargo xtask gr3-run --twin-of <RUN> --at 60

# Arm B or C as a dry run on dev
cargo xtask gr3-run --arm B --split dev --mode renders --budget 4000 --proposals proposals.json --out ~/scratch/dry.jsonl

# Reports
cargo xtask gr3-report                                   # list the runs
cargo xtask gr3-report --run <RUN>                       # one arm
cargo xtask gr3-report --run <CORE> --rate-from <PROBE> --probe-runs <probe file>   # the timed reading
cargo xtask gr3-report --a <RUN_A> --b <RUN_B> [--long-a <A300> --long-b <B300>]     # clause 1 and the decision
```

The bank is the file `cargo xtask bank` writes (default `$CARGO_TARGET_DIR/bank/enth-bank-v1.bin`; `--bank none` runs without one). `--bank-entries N` uses its first N entries. The builder takes candidates in order, so those are the bank `--entries N` builds. Settings left out take create's own defaults, or the run's own when twinning.

## How to read the report

**One run** (`--run`). The first lines name the build (commit, `+dirty`), the date, the settings, the bank and, for a render-equivalent run, the flag. Then a table with one row per stratum and per generator level:

| Column | Meaning |
| --- | --- |
| Pass, pass rate | `sound/1` ≥ 90 at the endpoint, with the 95 % Wilson interval |
| Median JND | `sound/1`'s distance in JND units at the endpoint (a target with no score counts as +∞) |
| JND at 10, 30, 60, 300 s (or 500 … 8,000 renders) | The anytime curve: the median at each mark every target has |
| KM median time to pass | Kaplan–Meier over the checkpoint grid (a target passes at the first mark whose score passes), censored at the endpoint; "not reached" while fewer than half have passed |
| Median MFCC | The independent metric |
| Median structure %, renders | `structure/1`, and renders at the endpoint |

**The strata** are `gen-1note`, **`gen-2note`** and `preset`. In `gen-2note`, the generator's octave and legato pairs are scored through their two-note phrase, but create hears and plays one note in every arm (the F1s judge). They are reported apart and stay in the primary population. Below the table come the machine (the cores other processes kept busy over the timed windows, lock waits, renders per second), the intake's warnings, the starts the best candidates grew from, and one line per target.

**Two runs** (`--a`, `--b`), paired by target. Per stratum: passes, the discordant counts, McNemar's one-sided p that B passes more often, the Wilcoxon signed-rank test on `d = JND_A − JND_B` (positive: B is closer) with W+ and its one-sided p, the Hodges–Lehmann estimate of d with its 95 % interval, and the same Wilcoxon on the MFCC metric. Then the decision.

### The decision (fixed in advance)

This was fixed before the first test run (EVK, 2026-10-21) and may change only before the first arm-B test run.

1. **Primary, at 60 s:** the pass rate by the exact McNemar test on the discordant targets, and the per-target JND difference by the exact Wilcoxon signed-rank test with the Hodges–Lehmann interval. A target with no score at the mark fails the pass rate and is left out of the JND test.
2. **"Better at 60 s"**: both one-sided tests favour B at p < 0.05. This is an intersection–union test: no multiplicity correction is needed, because both must hold.
3. **"Not worse at 300 s"** (the 40-target 300 s tail, or the same runs when they reach 300 s): neither one-sided test favours A at p < 0.05.
4. **B beats A** when 2 and 3 both hold. Everything else is reported and not gated: anytime curves, time to pass, `structure/1`, strata, levels and the MFCC guard.

### The statistics

| Test | As implemented (`xtask/src/gr3/stats.rs`) | Hand check |
| --- | --- | --- |
| McNemar | Exact: P(X ≥ B-only) with X ~ Bin(A-only + B-only, ½); two-sided is twice the smaller tail | 2 against 8: 56/1024 |
| Wilcoxon signed-rank | Exact permutation distribution of the signs given the ranks (doubled mid-ranks keep ties exact); zeros dropped | 1..5 all positive: 1/32; with a zero and ties, W+ = 13.5, p = 3/32 |
| Hodges–Lehmann | Median of the Walsh averages; the 95 % interval is Walsh averages k and M + 1 − k, with k from the untied signed-rank null | n = 8: k = 4, [2, 7] (the tables' critical value 3) |
| Kaplan–Meier | Product-limit over the checkpoint grid; median where S ≤ ½ | 1, 2, 2, 3+, 4: S = 0.8, 0.4, 0; median 2 |
| Wilson | 95 % score interval for one arm's pass rate | 0 of 10: [0, 0.2775]; 5 of 10: [0.2366, 0.7634] |

A sign test needs 69 targets to detect a 65 % win rate (37 for 70 %, 158 for 60 %). That is why clause 1 runs on about 120 targets, and why the 40-target core alone has limited power.

## Dev tuning (arm A)

The sweep is in `~/code/enth-scratch/r23/evk/dev-tune.jsonl` (lane EVK, 2026-10-21; build `06aef8c`, the binaries frozen for the whole sweep). It ran arm A on the 30 dev targets in render mode, 4,000 renders each, 6–8 workers, with each change made alone against create's defaults (K 8, `first_rung` 50, `min_spread` 0.25, the rebuilt 20,000-entry bank `66d5f380…`, the 3 s window). The machine was loaded throughout (load 16–42), which render mode ignores.

| Config | Change | Pass at 1,000 / 2,000 / 4,000 renders | Median JND at 4,000 | Paired at 4,000: Hodges–Lehmann Δ JND (95 % CI) |
| --- | --- | --- | --- | --- |
| **base** | create's defaults | **13 / 13 / 13** | **1.14** | – |
| bank2k | the bank's first 2,000 entries | 12 / 13 / 13 | 1.34 | −0.10 (−0.27 to 0.01) |
| s05 | `min_spread` 0.5 | 11 / 14 / 14 | 1.24 | −0.19 (−1.21 to −0.02) |
| bank2k-s05 | both | 9 / 11 / 11 | 1.40 | −0.19 (−0.47 to −0.05) |
| m25 | `first_rung` 25 | 11 / 12 / 13 | 1.41 | 0.00 (−0.15 to 0.00) |
| m100 | `first_rung` 100 | 11 / 13 / 12 | 1.29 | 0.00 (−0.11 to 0.00) |
| k4 | K 4 | 13 / 13 / 12 | 1.29 | −0.07 (−0.31 to 0.00) |
| k12 | K 12 | 11 / 13 / 12 | 1.42 | 0.00 (−0.14 to 0.00) |
| w2 | 2 s window, at equal CPU time | 12 / 14 / 14 | 1.09 | closer on 3 targets, farther on 13 |
| w5 | 5 s window, at equal CPU time | 11 / 13 / 13 | 1.14 | closer on 4, farther on 7 |

Δ is `JND_base − JND_variant`, so a negative value means the variant ends farther from the target. The window variants are read at equal CPU time: each target's base CPU budget for 4,000 renders, converted into the variant's renders by the two runs' CPU cost per render, read at the checkpoint at or below that (a mark every 250 renders). Not run, for time: `min_spread` 0.125, the 2,000-entry bank at 0.125, and no bank. TP's probe and the base run's provenance already show that retrieval helps: 28 of 30 best candidates grow from a k-NN neighbour.

**Chosen: create's defaults, unchanged** (K 8, `first_rung` 50, `eta` 2, 2 finalists, 16 neighbours, `min_spread` 0.25, the 20,000-entry bank, `SCORE_S` 3 s).

- No variant passes significantly more targets (every McNemar p ≥ 0.5). Every paired Hodges–Lehmann estimate is at or below 0, meaning the variant ends farther. A wider spread is clearly worse on either bank.
- **`min_spread` against the bank's size** (F1s should-fix 9): on dev, 20,000 entries beat 2,000 at the same spread (closer on 18 of 29 untied targets), and a spread of 0.5 hurts both, the small bank most (11 passes). So keep 20,000 entries at 0.25 nepers. TP's probe, which used the bank's own stream before any search, had found the small bank's best-of-16 starts slightly closer; after search, on GR3's dev targets, the large bank wins.
- **`first_rung` and K against the budget**: 50 beats 25 and 100, and K 8 beats 4 and 12 at 4,000 renders, with no difference at 1,000 and 2,000 large enough to change the pick.
- **The scored window**: at equal CPU time, 2 s passes one more target but ends farther on 13 of 30, and 5 s is level. `SCORE_S` stays 3 s.
- **Where the search stops helping:** the median JND is 1.14 at 1,000 renders and at 4,000. Generator levels 1–3 pass early (11 of 11 by 1,000 renders); levels 4–5 (2 of 9) and the anchor presets (0 of 10) stall. The lever is structure and the objective (OBJ's robust distance; arm B's starts), not the schedule.
- **The register shift** was not 0 on 17 of 30 dev targets: 15 at ±12, because generator patches often sit an octave off on `semi`. Unshifted, those score 4–5 %.
- **Cost:** CPU per render ran from 5 ms (a dry kick) to 135 ms (a level-5 patch with effects), a factor of 27. That is why `--rate-from` converts per target.

## The arm-A baseline

**Recorded on 2026-10-21 as a render-equivalent estimate, on the 40-target core.** Run `20261021T153018Z-1e40-A-renders8000-test-core` in `content/gr3/runs.jsonl`, build `8dfda49`.

- **Why an estimate.** No quiet window was available (six lanes and a non-enth job on the machine; the coordinator's option b), and the exclusive lock starves under overlapping shared holders. So the core ran in render mode at create's default **4,000 renders, standing for 60 s on 8 workers at an assumed 66.7 renders/s**. The source is TP's cost table ([r3.md](r3.md#what-lane-tp-built-f1s-2026-10-21), measured under `flock -x` at load 8.7–10.9, an upper bound on cost): "the default, 4,000 renders, is about 60 s on 8 workers for a 2–3 s clip of a patch with effects".
- **The flag.** The run's first line carries it (`header.equivalence`), and `gr3-report` prints it.
- **Headroom.** The run went to 8,000 renders with a checkpoint every 250, so the idle-rate probe can replace the assumption later without running again (`--rate-from`; [The idle-rate probe](#the-idle-rate-probe)). Its 4,000-render state is exactly what a 4,000-render run gives, because the run is a prefix of the longer one.
- **The settings are create's defaults**, as dev tuning chose: K 8, `first_rung` 50, `eta` 2, 2 finalists, 16 neighbours at `min_spread` 0.25, the 20,000-entry bank (SHA-256 `66d5f380…`), the 3 s scored window, seed 1.
- **The selection:** 32 generator codes (29 one-note, 3 two-note) and 8 test-preset renders. No G1 or owner item, so amendment 1 stays open.

At the endpoint (4,000 renders):

| Stratum | n | Pass | Pass rate (95 % CI) | Median JND | Median MFCC |
| --- | --- | --- | --- | --- | --- |
| **all** | 40 | **29** | **72.5 % (57.2–83.9)** | **0.40** | 0.20 |
| gen-1note | 29 | 25 | 86.2 % (69.4–94.5) | 0.36 | 0.08 |
| **gen-2note** | 3 | 0 | 0.0 % (0.0–56.1) | 13.35 | 6.80 |
| preset | 8 | 4 | 50.0 % (21.5–78.5) | 0.92 | 3.86 |
| level 1 / 2 / 3 / 4 / 5 | 4 / 7 / 7 / 7 / 7 | 4 / 6 / 6 / 7 / 2 | – | 0.08 / 0.01 / 0.36 / 0.51 / 1.88 | – |

**The anytime curve:**

| Renders (≈ s at the assumed rate) | 250 (4) | 500 (8) | 1,000 (15) | 2,000 (30) | 3,000 (45) | **4,000 (60)** | 6,000 (90) | 8,000 (120) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Passed | 20 | 23 | 28 | 29 | 30 | **29** | 29 | 28 |
| Median JND | 0.97 | 0.88 | 0.71 | 0.56 | 0.44 | **0.40** | 0.40 | 0.40 |

- **Time to pass:** the Kaplan–Meier median is 250 renders (more than half pass in the screening round and the first rung, from a k-NN neighbour). Level 4's median is 750 renders and level 5's 1,500; the two-note stratum's is not reached.
- **A pass can lapse:** 30 at 3,000 renders, 28 at 8,000. The search minimises metric v1 on the 3 s window, and `sound/1` is metric v1.1 through the whole performance, so a better objective can score a little worse.
- **Starts:** 39 of 40 best candidates grew from a k-NN neighbour, and one from the heuristic.
- **The register shift** was ±12 on 18 of 40 targets. Unshifted, 17 targets would pass instead of 29.
- **The 11 fails:**
  - all three two-note targets, with the intake warning `phrase`, plus `notes` and `chord` on one;
  - four level-5 one-note targets and one at each of levels 2 and 3;
  - Octave Arp a and b (its arpeggio reads as a phrase), and Snare Crack a and b (26.6 % and 88.0 %).

  The two-note stratum confirms the F1s judge's point: create plays one note of a two-note target in every arm.
- **Machine conditions:** render mode does not depend on load, which ran from 14 to 40 at the targets' starts. Other processes kept a median 7.7 cores busy (at most 9.9); a median 119 renders/s on 8 workers; 52.5 minutes for the 40 targets. CPU per render ran from 7 to 132 ms, median 24.

For B against this baseline, B's run must be read at the same equivalent time, with its inference charged: a twin of A's timing, or the same render budget less the inference at the measured idle rate.

### Running the rest overnight

The rest (74 filled targets) was not run: the machine was not quiet (load about 30). On a quiet night, run the same flags with `--set rest`, about 75 minutes at idle (two to three hours loaded):

```sh
cargo xtask gr3-run --arm A --set rest --mode renders --budget 8000 --workers 8 --marks "$(seq -s, 250 250 8000)" \
    --endpoint 4000 --equivalent-seconds 60 --rate-source "<the same text as the core's run line>" \
    --bank <the 66d5f380… bank> --note "arm-A rest: render-equivalent estimate"
```

If the idle-rate probe has run by then, prefer a timed run: `cargo xtask gr3-run --arm A --set rest --seconds 60 --workers 8`.

## The idle-rate probe

`cargo xtask gr3-run --rate-probe` runs arm A on eight dev targets (levels 2–5, a two-note phrase, a pad with reverb, a kick, a lead), 60 s each, on 8 workers, under `flock -x` per target. That is about 10 minutes of a quiet machine. Each target's renders at its 60 s checkpoint, over 60, is that target's idle rate, and the median is the machine's. Then:

```sh
cargo xtask gr3-report --run <CORE RUN> --rate-from <PROBE RUN> --probe-runs $CARGO_TARGET_DIR/gr3/rate-probe.jsonl
```

reads the render-equivalent core at the checkpoint at or below 60 s × the measured rate, and also **per target**: each target's renders are scaled by its CPU cost per render against the run's median, because a dry level-2 patch renders several times faster than a preset with reverb, and a timed run would give it that many more renders. A target whose share exceeds the run's budget is read at its cap and counted. The core was recorded to 8,000 renders with a mark every 250, so any idle rate up to about 133 renders/s converts without running again. The probe is a dev run, so its lines go to scratch, never to `runs.jsonl`; `--probe-runs` reads them from there.

**First probe (coordinator, 2026-10-21 20:00 MSK, run `20261021T165807Z-5505-A-wall60-dev-ids`, build `8fbff62`):** all 8 targets under `flock -x`, but other processes still kept 2.6–7.1 cores busy, so the rates are lower bounds on a truly idle machine. Median **174.3 renders/s** on 8 workers (per target 50–1,006 renders/s: a dry level-2 patch about 25,000 renders in 60 s, a level-5 patch about 3,000), against the 66.7 assumed from TP's table. 60 s is therefore about 10,500 renders, above the core's 8,000-render headroom, so the converted reading is capped at 8,000 for most targets: **28–29 of 40 pass (about 70 %), median 0.40 JND** (`gr3-report --run 20261021T153018Z-1e40-A-renders8000-test-core --rate-from 20261021T165807Z-5505-A-wall60-dev-ids`). The 4,000-render estimate (29 of 40) was therefore not optimistic: a quieter machine gives search alone more renders, not fewer. A future core run should record to at least 16,000 renders so 60 s converts without a cap.

## The runs file

One JSON object per line, schema `enth.gr3-run/1`, appended and flushed, never rewritten:

| `record` | Holds |
| --- | --- |
| `run` | `run_id`, `date`, `arm`, `mode`, `seconds` or `budget`, `workers`, `split`, `targets`, `cfg` (K, `first_rung`, `eta`, `finalists`, `neighbours`, `min_spread`, `lambda`, `seed`, `score_s`), `bank` (path, entries, SHA-256), `proposer`, `engine`, `machine` (threads, CPU, kernel, RAM, load), `header` (the build's commit and dirty files, rustc, the set's version and `synth.toml`'s SHA-256, the selection, `twin_of`, the command line, the note, `equivalence`) |
| `target` | `target` (id, split, kind, level, phrase, core, stratum), `date`, `cap`, `machine` (seconds, this process's CPU seconds, `others_cores`, load before and after, `locked`, `lock_wait_s`), `intake` (ms, key, note-on, hold, window, f0, bandwidth, warnings), `inference_s`, `register_shift`, `final` (renders, rounds, stop, starts, branches and their provenance, seconds, renders/s), `checkpoints` (mark, `t_s`, renders, rounds, objective, branch, provenance, patch SHA-256, `score` and `raw_score`: percent, JND, distance, passed, structure, MFCC), `notes` |
| `target_failed` | The target and the error |
| `run_end` | Counts, passes at the cap, seconds, load |

The checkpoint patches go to a sidecar that is not committed (`$CARGO_TARGET_DIR/gr3/patches/<run>.jsonl`). A twin replay rebuilds them bit for bit, and the hashes check it.

## Open items

- **The idle-rate probe** on a quiet machine (the owner, or a quiet overnight run), then the timed reading of the core baseline with `--rate-from`. No lock was held waiting for it (the coordinator's decision, 2026-10-21).
- **The rest of the set** (`--set rest`, 74 targets; about 75 minutes at 4,000 renders on a loaded machine, half that idle) overnight, and the 300 s tail on the core for "not worse at 300 s" (budget about 20,000 renders, or 300 s wall-clock).
- **Arm B's proposer** (lane EVb, after ONX, TR and MS), and C and D on the core for the diagnostics.
- **Degraded synth targets** (clause 1 "per degradation"): DEG's held-out files need a WAV upload path in the tool (a target that reads its upload from a file whose SHA-256 the manifest pins, scored against the clean target). Not built.
- **The plugin's worker count** (4) beside the CLI's 8: renders do not depend on it, but timed runs do.
- **The listening kit** (clause 2): LT's set manifest (`tools/listen/src/gr3/set.rs`) is filled from each arm's top-ranked candidate. The sidecar patches hold them.
