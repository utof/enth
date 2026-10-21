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
| Dev tuning of arm A (`first_rung`, K, `min_spread` against the bank's size, the scored window) | Done ([Dev tuning](#dev-tuning-arm-a)) |
| The arm-A baseline | Recorded on the 40-target core as a **render-equivalent estimate**, since no quiet window was available ([The arm-A baseline](#the-arm-a-baseline)) |
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

**The register shift.** One note cannot tell the key from the oscillators' tuning. A target whose oscillators sit an octave up sounds like the key an octave higher, and the intake rightly reports that (on dev, `gen-d-l1-01` reads as C4 although its phrase plays C3). create then searches at the key it heard. So before scoring, the kit moves every candidate by the intake's key less the key the target's performance starts on, in whole semitones on every enabled oscillator: the rule k-NN transposes neighbours by. That judges the candidate **as played at the note create inferred**, which is how a user hears it. Every arm is judged the same way. The record keeps the shift (`register_shift`) and, when it is not 0, the raw score of the unmoved patch (`raw_score`). Without the shift, the first dev smoke run scored a correct sound at 4.6 % (49 JND) because of its octave alone.

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
| KM median time to pass | Kaplan–Meier over the checkpoint grid (a target passes at the first mark whose score passes), censored at the cap; "not reached" while fewer than half have passed |
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

Filled in below from `~/code/enth-scratch/r23/evk/dev-tune.jsonl` (render mode, 4,000 renders, 8 workers, 30 dev targets, `gr3-report --a base --b <config> --at 4000`).

## The arm-A baseline

Filled in below.

## The idle-rate probe

`cargo xtask gr3-run --rate-probe` runs arm A on eight dev targets (levels 2–5, a two-note phrase, a pad with reverb, a kick, a lead), 60 s each, on 8 workers, under `flock -x` per target. That is about 10 minutes of a quiet machine. Each target's renders at its 60 s checkpoint, over 60, is that target's idle rate, and the median is the machine's. Then:

```sh
cargo xtask gr3-report --run <CORE RUN> --rate-from <PROBE RUN> --probe-runs $CARGO_TARGET_DIR/gr3/rate-probe.jsonl
```

reads the render-equivalent core at the checkpoint at or below 60 s × the measured rate, and also **per target**: each target's renders are scaled by its CPU cost per render against the run's median, because a dry level-2 patch renders several times faster than a preset with reverb, and a timed run would give it that many more renders. A target whose share exceeds the run's budget is read at its cap and counted. The core was recorded to 8,000 renders with a mark every 250, so any idle rate up to about 133 renders/s converts without running again. The probe is a dev run, so its lines go to scratch, never to `runs.jsonl`; `--probe-runs` reads them from there.

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
