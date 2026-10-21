# Match (`enth-solver::search`, `heuristic`, `cmaes`)

- Date: 2026-10-20. R1/S1 lane MT. Crate: `crates/enth-solver` (L5; depends on `enth-params`, `enth-patch`, `enth-render`, `enth-analysis`, `enth-challenge`, `enth-dsp`, `libm`; no I/O, clocks, threads or global RNG; builds for wasm32).
- What it is: the engine of `enth match`, and the "CMA-ES floor" of the GR1 agent baseline. Given a target (audio, a patch, or descriptors), a start patch and the parameters that may move, it searches for a patch that sounds like the target within a render budget, and returns the best patch, its distance and a trace.
- Ported from our own Phase 0 spike (`spikes/spike-metric`, [metric.md](../spikes/metric.md)), no third-party source. Nothing needs a `THIRD_PARTY_NOTICES.md` entry.
- Not here: `enth match` itself (lane CL), the real scorer of a challenge (lane SS, which plugs in as an [`Objective`](#plugging-in-a-distance)), `nearest` (a documented stub, cut to R2). `closest`, the "How close am I?" search of R2's Learn, is a separate module on the same machinery: see [Closest](#closest-closestrs-r2-lane-cls) below.

## API (for the CLI, lane CL, and the scorer, lane SS)

```rust
use enth_solver::search::{FreeSet, MetricTarget, SearchCfg, search, search_metric, search_with};

// Free parameters: the addresses a match may move, with their bounds from ParamSpec.
let free = FreeSet::new(&start, &addrs)?;            // or FreeSet::from_mask(&start, &mask)

// 1. Metric v1 against a target on one thread: the default distance.
let target = MetricTarget::from_audio(&mid, &perf, &spec)?;   // or from_patch(&patch, &perf, &spec)
let found = search_metric(&start, &free, &target, &SearchCfg::default())?;

// 2. Any distance, any parallelism: an Objective scores a batch, results in input order.
let found = search(&start, &free, heuristic.as_ref(), &mut objective, &cfg)?;

// 3. A wall-clock cap, progress: the callback runs after every batch; false stops (StopCause::Cancelled).
let found = search_with(&start, &free, heuristic.as_ref(), &mut objective, &cfg, &mut |step| t0.elapsed() < cap)?;
```

`SearchResult { best, distance, start_distance, heuristic_distance, suggestions, renders, runs, stop, trace }`. `best` is the start patch with only the free parameters set, through `Op::SetParam` in one `apply_all` transaction (`AGENTS.md`). `stop` is one of `Budget`, `Goal` (`SearchCfg::stop_at` reached), `Cancelled` (the callback) or `Exhausted` (nothing left to try: no continuous axis and every categorical choice ranked). `trace` has one `TraceStep { renders, stage, batch_best, best }` per batch (`Stage::Probe`, or `Stage::Run(n)`), enough for a progress line or a convergence plot. `SearchError::code()` and `hint()` give the stable codes (`no_free_parameters`, `unknown_param`, `bad_search_cfg`, `objective_count`).

`Search` is the ask-tell machine behind `search`: `ask()` returns the next batch of candidate patches, `tell(&distances)` takes their distances, in the same order. Use it to interleave searches or to drive the batches yourself.

### Settings (`SearchCfg`, `Default` is what `enth match` uses)

| Field | Default | Meaning |
| --- | --- | --- |
| `seed` | 1 | the seed of every random choice; the stream is `derive(seed, "MATCHSEA")`, so a seed shared with other code does not reuse their stream |
| `budget` | 1500 | the most renders, probe included. **Hard**: the last batch is cut short, never overshoots |
| `lambda` | 0 | population of the first CMA-ES run; 0 is Hansen's default for the number of continuous axes. A setting, never derived from the worker count (that would change the algorithm). Pick a multiple of the pool size to keep the pool busy; it doubles each restart cycle (IPOP, at most 16×) |
| `sigma_start`, `sigma_random` | 0.15, 0.3 | first step in the normalised 0-1 units around a heuristic point, and of a random restart |
| `stop_at` | none | stop as soon as the best distance is at or below this |
| `runs_per_cycle` | 4 | how many categorical choices get a run after the probe (the best by probe distance); restart rounds run two |
| `use_heuristic` | true | apply the heuristic's suggestions (when a `Heuristic` is passed) |

### Plugging in a distance

```rust
pub trait Objective {
    /// The distance of every candidate, in input order. `out[i]` depends only on `candidates[i]`.
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64>;
}
```

A closure `FnMut(&Patch) -> f64` is one. A non-finite value counts as failed (`FAILED = 1e4`), as does a candidate that cannot be built. The default is metric v1 (`enth_analysis::distance::metric_v1`: multi-resolution log-mel L1 on clips normalised to -23 LUFS): `MetricTarget` holds the target's features, descriptors and performance (immutable, `Sync`); `MetricTarget::worker()` makes a `MetricWorker` (one per thread, `Send`, itself an `Objective`).

**A pool is the caller's business.** The solver spawns no threads. For `enth match`, one rayon pool of `MetricWorker`s over one shared `MetricTarget`, with `par_iter` over the batch and results collected by index; the rayon example is in the docs of `search::objective`, and a deterministic scoped-thread version (an atomic work queue, results written by index) is `crates/enth-solver/examples/support/pool.rs`, used by the integration test. For lane SS: build the challenge's real scorer as an `Objective` the same way (one scorer per worker, built from one immutable target) and pass it to `search`; nothing else changes.

## Algorithm

The searched space is the **free set**: a `Float` or a large `Int` (more than 8 values) is a *continuous axis*, searched on its normalised `[0, 1]` axis through `ParamSpec::to_norm` / `from_norm`; an `Enum`, a `Toggle` or a small `Int` (at most 8 values; a waveform, a filter mode, a sync switch) is a *categorical axis*, chosen per run. A *choice* is one assignment of every categorical axis.

1. **Probe.** One batch: the start patch, and for each choice a *heuristic point* (the start with that choice and the heuristic's suggestions applied). Every combination of the categorical values is a choice when there are at most 32 (`MAX_PROBED_CHOICES`), otherwise the start's choice and its one-flip neighbours. The choices are ranked by that distance. The spike guessed the waveform from the audio's even/odd harmonics; the descriptors do not carry the waveform or the filter mode, so the search asks the metric: with a wave (5 values) and a sync switch free that is 10 renders, and the best choice by probe distance gets the first run.
2. **Runs.** CMA-ES on the continuous axes, once per top-ranked choice (at most `runs_per_cycle`), warm-started at the choice's heuristic point. A run ends when it collapses (`TolX`), stalls (15 generations without improvement) or becomes ill-conditioned (`1e14`).
3. **Expand.** When the runs are done, the best choice's one-flip neighbours that were not probed are evaluated at its best continuous point, and the best two get a run: a coordinate descent over the categorical axes, for when there are too many combinations to probe at once (a level-5 patch with several waves and modes).
4. **Restart.** Then **IPOP** restarts of the two best choices, cycle after cycle: a random start for the continuous axes (the pinned ones excepted, see below), `sigma_random`, and double the population.
5. **Budget.** `budget` counts every render and is never exceeded: a batch that does not fit is truncated, and a truncated CMA-ES generation is not told to the optimiser.

### The pitch is a needle, so a precise pitch is pinned

The metric is a log-mel L1 distance: a pitch a tenth of a semitone off is as bad as a wrong waveform. On the randomizer's level-2 target `0x4D540000` the distance is 0.063-0.08 for every `osc1.semi` except `-12.00`, where it is exactly 0, while the cutoff, the resonance and the pulse width all have smooth valleys around the truth. CMA-ES cannot walk to a needle: the heuristic's pitch (`-12.000084`) was right to a hundredth of a cent, and the optimiser wandered off it (to `-11.46`) and never came back. Run 2 and 3 of the measurement left this target at 0.0686 after 1,500 renders.

So the heuristic's suggestions carry a `spread`, the width of the first step relative to the default (pitch 0.02, cutoff 0.3, envelope times 0.5, detune 0.5). A suggestion with a spread below `PRECISE = 0.1` (the pitch) is **pinned**: the warm runs and the first random-restart cycle hold it at the heuristic's value and run CMA-ES on the other axes only. From the second restart cycle (`PIN_CYCLES = 2`) the pin is released and the axis gets the full step again, so a wrong measurement (an octave error, a pitch that is not the oscillator's) costs renders and not the answer. The synthetic unit tests `a_pitch_measured_to_a_cent_is_kept_while_the_rest_is_searched` and `a_wrong_measurement_is_released_after_the_pinned_cycles` show both halves (they fail with `PIN_CYCLES` 0 and 1000 respectively), and `tests/match.rs` has the real-metric regression for `0x4D540000` (ignored in debug, 3 s in release).

The other suggestions narrow the first step instead (`v = u / scale` in the optimiser, box `[0, 1 / scale]`, scale at least `1e-3`): the default step of 0.15 on a +-48 semitone axis would be a 14-semitone jump.

## The heuristic (`heuristic.rs`)

`Heuristic::new(&Descriptors, &Performance)` and `suggest(&Patch, &FreeSet) -> Vec<Suggestion { addr, value, from, spread }>`. Pure, no renders. It reads the target's `Descriptors` (`enth describe`'s numbers), so it works for any target `describe` can measure. Each suggestion is made only when its parameter is free and its module is in the patch.

| Descriptor | Parameter | Rule (calibrated on the engine) | Measured on a rendered target |
| --- | --- | --- | --- |
| `envelope.attack_ms` | amp envelope `attack` | the descriptor is the time to within 1 dB of the peak: knob = descriptor / 0.77 (below 15 ms the descriptor reads the measurement floor and is taken as it is) | within 1.3x |
| `envelope.release_ms` | `release` | the -40 dB point: knob = descriptor / 0.5 | within 1.3x |
| `envelope.decay_ms`, `sustain_db` | `decay`, `sustain` | sustain: the descriptor is 1.15x the knob's level; decay: ratio table by sustain, 0.45 at 0 down to 0.01 at 1 | decay within 2x, sustain within 8 % |
| `pitch.f0_hz` | `osc1.semi` and `fine` | the interval between the note played and the measured f0, whole semitones to `semi` and the cents to `fine` when both are free | semi exact |
| `spectrum.centroid_hz` | filter `cutoff` | a harmonic model of the first oscillator's wave (saw, pulse, triangle, sine) through the filter's response (lp, hp, bp, with the patch's resonance), its centroid matched on a 1/12-octave grid from 20 Hz to 20 kHz; the model follows the engine's centroids within 1-13 % | within 1.6x |
| `detune.beating_hz` | `osc2.fine` | cents = 1200 log2(1 + beating / pitch) (the spike's rule) | - |

The cutoff depends on the waveform and the filter mode, so `suggest` takes the patch **with the categorical choice already applied**; the search calls it once per choice. It does not guess waveforms, resonance, drive, key tracking, filter envelopes, LFOs or effects.

## Measured floor

Ten randomizer targets: `enth_challenge::random::target` for the level 1 and 2 presets (`PRESETS[level - 1].spec(1, 0x4D540000 + i)`, `i` = 0..4), rendered with the generated phrase at the scoring spec, metric v1, the free set the randomizer drew (the "drawn" set, 2-6 axes), search seed `1 + i`, `budget = 1500`, 4 workers, a release build. "Solved" is a distance at or below 0.005 in metric v1: a line I drew, about a tenth of the 0.06-0.08 that a pitch a few cents off costs on a level-2 target. What counts as solved in the game is SS's scorer's call. Reproduce with:

```
cargo build --release -p enth-solver --example match_floor
target/release/examples/match_floor --levels 1,2 --targets 5 --budget 1500 --workers 4 [--stop-at 0.005] [--no-heuristic] [--free observable]
```

| Setup | Median before | Median heuristic start | Median after | Solved | Median renders (max) |
| --- | --- | --- | --- | --- | --- |
| Drawn set, heuristic, full budget (no `stop_at`) | 1.5621 | 0.5049 | **0.0002** (worst 0.0026) | **10 of 10** | 1500 (used up) |
| Drawn set, heuristic, `stop_at` 0.005 | 1.5621 | 0.5049 | 0.0021 | **10 of 10** | **21** (574) |
| Drawn set, no heuristic, `stop_at` 0.005 | 1.5621 | - | 0.0064 | 5 of 10 | 800 |
| Observable set (11 axes), heuristic, `stop_at` 0.005 | 1.5621 | 0.4673 | 0.0051 | 5 of 10 (8 of 10 at 0.02) | 760 |
| Observable set (11 axes), no heuristic, `stop_at` 0.005 | 1.5621 | - | 0.1004 | 3 of 10 | 1500 |

By level (drawn set, heuristic, `stop_at` 0.005): level 1 median before 1.5085, after 0.0002, 6 renders; level 2 median before 1.5889, after 0.0035, 423 renders (574 at most). Six of the ten need 21 renders or fewer: the probe finds the right waveform and the heuristic's start is already inside the valley. "Before" is the start (init) patch's distance, 0.22-2.2 over the ten targets.

The heuristic is worth about 5 more solved targets of 10 on the drawn set and 2 more on the 11-axis set, and 40x fewer renders when it works. The observable set (the agent does not know which parameters were drawn) is the harder case and the one the GR1 CMA-ES floor should use alongside the drawn one: with 11 axes five of the ten targets end between 0.005 and 0.1 when the 1,500 renders are used up. A larger budget would close part of that; it was not measured.

**Cost.** A render at the scoring spec with the generated phrase takes roughly 15-30 ms of one worker (6-12 s for 1,500 renders on 4 workers, with other agents running on the machine); the search itself (CMA-ES on at most 11 axes, the heuristic) is negligible next to it. Results are bit-identical for 1, 2, 3 and 8 workers (`tests/match.rs`), and `match_floor` prints the same table (distances, renders, runs) for 1 and 8 workers over the ten targets.

**Golden challenges.** The golden challenge set (`tests/golden/challenges/`, lane GC) did not exist on this branch's base (GC had not started), so the solve rate above is on the randomizer's fixed-seed targets, the closest stand-in. Run the floor against the golden codes once they exist: the example takes its targets from `random::target`, and a `--code` flag is a few lines once CT's code type is wired. **Done through the CLI** (R1/S1 lane INT, 2026-10-20): `enth match --target <code> --free observable --budget 1500` on the golden challenges' 20 level codes, scored by `sound/1`, passes 11 of 20 (the untouched start 0 of 20), and on GR1's own 20 codes 12 of 20 (`content/gr1/floors.jsonl`; [r1s1-gates.md](../r1s1-gates.md), R11). On engine 0.3.0 (FU1 lane DRV, the filter drive's make-up gain) the golden codes still give 11 of 20 and GR1's give 13 of 20 (level 5 seed 1001 crosses the mark at 90.18 %).

## Determinism

A search is a pure function of its inputs and `SearchCfg::seed`. The maths is `libm` (no `mul_add`, no `powi`), the summation order is fixed, every ordering is `total_cmp` with an index tie-break, the random stream is `enth_dsp`'s `derive` and SplitMix64. The `Objective` contract (one distance per candidate, in input order, each a pure function of its candidate) makes the whole search identical for any worker count; `tests/match.rs` compares a serial `MetricWorker` with pools of 1, 2, 3 and 8 on a real target, and two different populations. `cmaes` has its own test: the same seed gives the same run, bit for bit.

## CMA-ES (`cmaes.rs`)

Ask-tell, Hansen 2016 defaults (population `4 + floor(3 ln n)`, the weights, `cs`, `damps`, `cc`, `c1`, `cmu`), the box by clamping the sample and adding a quadratic penalty to the fitness, the covariance's eigen-decomposition by cyclic Jacobi rotations, written in-crate (no linear-algebra dependency). `Cmaes::new(mean, sigma, lambda, lower, upper, seed)`, `ask()`, `tell(&fitness)`, `stop_reason()`. Ported from `spike-metric/src/cmaes.rs` with `powi` replaced by `libm::pow` and multiplications, and the random numbers from `enth_dsp::util::rng::Rng` with a Box-Muller `normal()` on `libm`. Non-finite fitness counts as `WORST_FITNESS = 1e300`. Tests: sphere, Rosenbrock, an ill-conditioned ellipsoid, the bounds, the restart signals, the same seed gives the same run, errors.

## Limits

- **Structure is not searched.** Only the parameters of the modules that are in the start patch move. Enable flags, routes, added or removed modules (the randomizer's level 5: second oscillator, filter, effect, LFO routes) are outside the search; the template decides them. `nearest` (cut to R2, a documented stub in `nearest.rs`) would choose a better start; until then a target whose structure the start lacks cannot be matched, and `match_floor` measures levels 1-2 only.
- **More than 32 categorical combinations** are walked one flip at a time (Expand), which finds a target several flips away only if each flip helps by itself. A target that needs two simultaneous flips from every neighbour is missed.
- **Automation and time-varying parameters** are not searched: the performance is fixed (`MetricTarget`'s), and a parameter is one number for the whole render.
- **Seeds** (`osc1.seed`, 65,536 values) are continuous axes in name only: the metric is flat or noise in them. Leave them out of the free set.
- **The heuristic assumes the first oscillator and the amplitude envelope** carry the pitch and the envelope descriptors, and the first filter the centroid. A patch that routes differently gets weaker suggestions, not wrong ones, because every suggestion is probed against the start before it is used.
- The distance is whatever the `Objective` returns. Metric v1 is provisional (ADR-0007: the listening test decides); SS's scorer replaces it without a change here.

## Closest (`closest.rs`, R2 lane CLS)

- Date: 2026-10-21. Crate: `crates/enth-solver` (same layer, same rules: no threads, clocks, I/O or global RNG; builds for wasm32).
- What it is: the **"How close am I?" button** of a lesson ([learn.md](../learn.md), "Knob feedback"; design §7, layer 3). After a failed Submit, it runs CMA-ES from **the player's own patch** over the step's editable parameters (usually 1 to 4) and answers the nearest patch that **passes the round**, with per-parameter deltas. It is a button with progress and cancel, never live (a scoring render costs 15 to 40 ms), and using it makes the redo a sibling challenge, as a Reveal does.
- Not `nearest`: that name is R3's (the nearest preset as a start, filled by R3a's k-NN retrieval). The two stay apart.

### API

```rust
use enth_solver::closest::{ClosestCfg, ClosestJob, RoundSerial, closest, closest_with};

// The job is solver-native, so enth-learn (same layer) never calls this crate and the shell's worker composes nothing.
let job = ClosestJob { round: &prepared_round, patch: &players_patch, free: &editable_set, budget: 300 };
let found = closest(&job, &mut RoundSerial::new(), &ClosestCfg::default())?;          // runs to the end
let found = closest_with(&job, &mut pool, &cfg, &mut |p| !user_pressed_cancel(p))?;   // progress and cancel
```

| Piece | Meaning |
| --- | --- |
| `ClosestJob { round, patch, free, budget }` | `round`: the `PreparedRound` whose scorer decides what passes; `patch`: the player's patch, where the search starts and what "closest" is measured from; `free`: the parameters that may move; `budget`: the most renders, the first (the player's own patch) included. **Hard**: no batch overshoots it. `DEFAULT_BUDGET` is 300 |
| `ClosestEval` | scores a batch against the round, results in input order. The caller owns parallelism: `RoundSerial` is one thread; a pool calls `closest::reading(round, patch)` per candidate (`examples/support/round_pool.rs` is a scoped-thread version) |
| `Closest` (the result) | `patch` (the player's patch with the free parameters set; if nothing passed, the candidate nearest to passing), `deltas`, **`passed`**, `percent`, `start_percent`, `distance`, **`renders`**, `first_pass_renders`, `stop` |
| `Delta` | per free parameter, in the order of `free`: `addr`, `from` and `to` (plain units), `norm` (the signed move on the knob's normalised 0-1 axis; for a categorical parameter 0 or 1), `categorical`; `moved()` |
| `Progress` | after every batch: `renders`, `budget`, `passing` (the nearest passing distance so far), `percent` |
| `ClosestStop` | `StartPasses` (the player's patch already passes: one render, answer is that patch), `Converged`, `Budget`, `Cancelled` (the callback returned `false`), `Exhausted` (only categorical parameters free and all scored) |
| `ClosestError` | `code()` and `hint()` like `SearchError`: the search codes (`no_free_parameters`, `unknown_param`, `bad_search_cfg`) plus `bad_closest_cfg`, `start_unscorable`, `evaluator_count` |

**Budget and cancel.** `budget` is a count of renders and is never exceeded (a batch that does not fit is cut short; a cut-short CMA-ES generation is not told to the optimiser). The solver reads no clock, so a wall-clock cap lives in the caller's callback; `false` from it ends the search at the next batch boundary with `Cancelled`, and the result is still the best candidate scored so far. Every returned patch was really scored (never the optimiser's mean), and the answer is never ranked below the player's own patch (which is one of the candidates).

**What "closest" means.** Knob travel: the Euclidean distance on the free parameters' normalised 0-1 axes (an octave of cutoff is the same step everywhere), plus `flip_cost` (1.0, a whole knob's travel) for each categorical parameter that changes. Candidates are ranked by **feasibility** (CMA-ES uses only ranks, so the scale of the terms never matters): a passing candidate costs its knob distance, a failing one costs more than any knob distance plus its sound distance in just-noticeable steps, a candidate that cannot be scored is last. While nothing passes the search descends the scorer's own distance (as `match` does); once something passes it slides along the pass boundary towards the player's patch. There is no target-driven start: the heuristic of `match` is off, so the answer never jumps to the target's descriptors.

**Stopping.** The player's patch is scored alone first (a pass ends the job there). Without a categorical parameter free the search settles when the nearest passing distance has improved by less than `tol` (3 % of itself, 0.002 absolute) over `patience` (3) generations, and the run that found a pass is the last (no random restart after it, which would only find a farther patch). With categorical parameters free the top `choices` (3) choices each get a run. A settled search ends with a **polish**: bisection on the segment from the player's patch to the nearest pass, 3 candidates a round for 3 rounds (9 renders), which finds the boundary to 1/64 of the way with one parameter free and removes slack along the path with more.

**What it promises.** *A* nearby passing patch, within about 10-30 % of the travel of a run several times as long (see [open ideas](#open-ideas-closest)); not the provably nearest. A `passed: false` answer means "not found within the budget" (the free set may not be able to pass the round at all), so a caller must not present its deltas as a solution.

### Determinism

The result is a pure function of the job and `ClosestCfg`. The evaluator contract is `search`'s: one reading per candidate, in input order, each a pure function of its candidate (call `reading` per candidate and nothing else). The population (`lambda`) is a setting, never derived from the worker count, so 1 and 8 workers give the same `Closest`, bit for bit. The tests:

- `tests/closest.rs` runs real generator v1 rounds (level 2 and 3 presets) through the real scorer (`score_round`): `the_result_is_identical_for_any_worker_count` compares `RoundSerial` with pools of 1, 2, 3 and 8 workers and a second run of the 8-pool (`assert_eq!` on the whole `Closest`), `the_population_is_a_setting_and_results_follow_it_not_the_pool` does it for `lambda` 8 and shows that another `lambda` is another search, and `a_waveform_among_the_free_parameters_is_searched_too` compares 1 and 8 workers with a categorical parameter free. The others check that the answer passes by the round's own verdict and is no farther than the target's own settings, that a passing start costs one render, that the budget is hard and a callback cancels, and that a round the free set cannot pass returns the nearest to passing.
- `closest.rs` has 10 unit tests on analytic landscapes (a ball around a target in the free axes: the nearest pass lies on the segment to it, a radius short of the target, for 1 to 4 axes; the same inputs give the same answer, the seed is an input; refusals carry a code and a hint; a candidate that cannot be scored ranks last).
- `examples/closest_cost.rs` runs every job on every worker count asked for (`--workers 1,8`) and **stops if the answers differ**, so the table below is also a determinism check over 66 jobs.

### Cost on lesson-like starts

A lesson's Recreate step is a generated round whose every other control is already set and whose editable set is 1 to 4 parameters. Each row is a generator v1 round (`PRESETS[level - 1].spec(1, 0x434C0000 + i)`) with the target patch's `k` randomized parameters taken back out of place: the first `k` of the order a course meets the controls in (cutoff, resonance, attack, release, pitch, ...) that the round moves, knobs only. Two kinds of start: **untouched** (the `k` parameters at the template's values: the step as it opens) and **partial** (each moved half-way to the target's value on its axis: a try that is not there yet). Ten rounds: level 2 seeds 0-4 and level 3 seeds 0, 1, 2, 4, 6 (3 and 5 repeat a target; `k = 4` exists on level 3 only, level 2 moves at most three of these). Default settings, `budget 300`, 1 and 8 workers. Seven jobs whose partial start passes anyway cost one render each (0.04 s) and are left out of the table.

```
cargo build --release -p enth-solver --example closest_cost
target/release/examples/closest_cost --levels 2,3 --rounds 5 --ks 1,2,3,4 --starts untouched,partial --workers 1,8 --budget 300 [--verbose]
```

| k | start | rounds | passed | renders, median (max) | first pass, median | knob distance, median | distance / the target's own | seconds, 1 worker, median (max) | seconds, 8 workers, median (max) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | untouched | 10 | 10/10 | 40 (54) | 7 | 0.091 | 0.66 | 1.86 (2.26) | 0.65 (1.44) |
| 1 | partial | 7 | 7/7 | 38 (50) | 7 | 0.047 | 0.50 | 1.54 (2.05) | 0.63 (1.20) |
| 2 | untouched | 10 | 10/10 | 61 (82) | 15 | 0.110 | 0.54 | 2.43 (3.42) | 0.85 (1.09) |
| 2 | partial | 8 | 8/8 | 52 (64) | 13 | 0.040 | 0.42 | 2.36 (3.46) | 0.66 (0.98) |
| 3 | untouched | 8 | 7/8 | 73 (300) | 24 | 0.154 | 0.77 | 3.02 (11.29) | 0.96 (3.15) |
| 3 | partial | 7 | 7/7 | 73 (206) | 20 | 0.071 | 0.42 | 3.08 (7.72) | 0.88 (2.34) |
| 4 | untouched | 5 | 4/5 | 74 (300) | 32 | 0.206 | 0.57 | 3.01 (12.75) | 1.00 (3.23) |
| 4 | partial | 4 | 4/4 | 66 (98) | 23 | 0.168 | 0.64 | 2.57 (4.10) | 0.68 (1.05) |

**57 of 59 passed**, 4,280 renders in all; 1 and 8 workers gave identical results on all 66 jobs. All 57 passes stopped by settling; the 2 failures used the whole budget of 300. For `k` of 1 or 2 every pass was found within 39 renders; for `k` of 3 or 4 the median first pass is at render 20 to 32. The typical job ends in 40 to 75 renders: 1.5 to 3 s on one worker and about 1 s on eight.

- **Distance.** The median answer is 42 to 77 % of the knob travel to the target's own settings (`tgtdist`): the nearest passing patch is usually next door to the player's, not at the target. In 1 of 57 (level 2 seed 4, `k = 3`, partial: 0.094 against 0.081) the answer is a pass farther away than the target's own settings.
- **Seconds.** The 8-worker speed-up is 3.3x over the table (180 s against 55 s of wall time in all), not 8x: a generation is `lambda` candidates (4, 6, 7 and 8 for `k` of 1 to 4), and the pool cannot be busier than that. A larger `lambda` fills a pool but costs renders (below).
- **Load.** Measured 2026-10-21 on a 12-thread VM under the exclusive build lock (`flock -x /tmp/enth-quiet.lock`, acquired at once), but the machine was not idle: the load average was 3.7 at the start and 5.6 at the end (other agents' sessions and the desktop do not take the lock). A render cost 42 ms serial and 13 ms effective on eight workers in this run, against 15 to 30 ms on a quiet machine, so **the seconds are upper bounds**; renders and distances do not depend on load.
- **With a categorical parameter free** (`--mixed --ks 2,3 --budget 400`: the waveform joins the free set when the round moves fewer than `k` of the course-order knobs, which happens in 2 of the 10 rounds, level 2 seeds 1 and 2, at `k = 3`; `osc1.wave` is then free beside the cutoff and the resonance): both passed, one at 332 renders (settled) and one at the budget of 400 (first pass at render 255), with distances of 1.13 and 1.41 that the flip cost (1.0) dominates. Give such a set a budget of 400 or more.

### Settings (`ClosestCfg`, `Default` is what the button uses)

| Field | Default | Meaning |
| --- | --- | --- |
| `seed` | 1 | the seed of every random choice (`search`'s stream, `derive(seed, "MATCHSEA")`) |
| `lambda` | 0 | population of the first CMA-ES run; 0 is Hansen's default for the number of continuous axes (4, 6, 7, 8 for 1 to 4). A setting, never derived from a worker count; pick a multiple of the pool size to keep it busy |
| `sigma` | 0.1 | first CMA-ES step in the normalised 0-1 units around the player's patch |
| `flip_cost` | 1.0 | what changing one categorical parameter costs, in knob-distance units |
| `choices` | 3 | how many categorical choices get a run (ranked by one score at the player's own continuous settings, a weak guide: raise it, and the budget, for a set that mixes a waveform with several knobs) |
| `tol`, `patience` | 0.03, 3 | settled: the nearest passing distance gained less than `tol` of itself (and 0.002) over `patience` generations |

The defaults came from sweeps over the same 59 jobs (budget 300; the tool is `closest_cost` with `--sigma`, `--lambda`, `--patience`, `--tol`). Around `patience` 4 and `tol` 0.02 (57 of 59 in 4,797 renders): `sigma` 0.05 passed 56 in 4,271 renders and 0.2 passed 55 in 4,987; `lambda` 8 passed 57 in 5,529; `patience` 8 with `tol` 0.01 passed 57 in 6,671. The defaults pass 57 in 4,280. No setting passed more than 57, and the defaults are among the cheapest that reach it.

### Known failure modes

Two of the 59 jobs did not pass in 300 renders (level 2 seed 0 `k = 3` untouched, level 3 seed 6 `k = 4` untouched). Both look like **multi-scale landscapes**: axes whose pass region is of very different scale in the same free set. A long run (below) finds the pass of the first, at render 602, and not of the second, at 1,000 renders.

- **Precision next to plateaus.** Level 2 seed 0 frees `filter1.cutoff`, `filter1.resonance` and `osc1.semi`. The pitch is a needle (see [the pitch is a needle](#the-pitch-is-a-needle-so-a-precise-pitch-is-pinned)): `osc1.semi` must be right to about 1e-4 semitone, and the search does find it (+12.000, the target's), but meanwhile the resonance has moved 3 % of its knob towards a target that is 31 % of the knob away (13.1 against 41.5), and the best sound score is 85.6 % against the pass mark of about 90 %. CMA-ES has one step size for all axes, and these want very different ones (the lane's diagnosis; not separately proven). In the lane's first baseline the same shape showed with an envelope `sustain` whose pass region was a 0.02-wide sliver at 0.04 of its linear 0-100 axis, and 1,200 renders did not fix those rows.
- **A corner.** Level 3 seed 6 frees `filter1.cutoff`, `filter1.resonance`, `env1.attack` and `env1.release`. The search ends with the attack at 0 ms (the target has 94.3 ms) and the release at 4.9 ms (the target has 358 ms), at the bottom of both axes, 0.72 of knob travel from the start, with 80.8 % at best. Why it goes there was not worked out.
- **A pass farther than the target's own settings** (1 of 57): the search settled on a pass region farther out, and the polish only slides along the segment towards the start, so it cannot find a nearer region elsewhere. The answer is a pass, but not a near one.
- **A categorical choice is ranked roughly** (one score at the player's own continuous settings), so a waveform that only works with different knobs can be missed unless `choices` is raised.

None of these returns a wrong answer: a pass is a pass by the round's own scorer. They return `passed: false` (with the candidate nearest to passing), which the Learn view must read as "I could not find one", not as "this step cannot be passed".

### Open ideas (Closest)

- **Per-axis scaling: tried, no gain, reverted.** Probe renders along each free axis to scale CMA-ES's first step per axis (an additive `Search::set_axis_scales` in `search.rs`). On the first baseline's 66 jobs it passed 54 against 55 and gave a larger median distance in 6 of 8 rows (for example `k = 4` untouched 0.39 to 0.78); the other two rows (`k = 1`) were equal. `search.rs` is untouched.
- **Long runs: smaller answers, and one more pass, for several times the renders.** `--patience 15 --tol 0 --budget 1000` on the 24 searched jobs of this table with `k` of 3 or 4 (8 workers) passed 23 of 24 (the defaults, 22): level 2 seed 0 now passes, at 717 renders (first pass at 602), and level 3 seed 6 still fails (0.91 away, 80.8 %). Of the 22 jobs that passed both ways the distance was the same or smaller on all (mean 13 % smaller, up to 39 %: for example level 3 seed 0 `k = 4` untouched 0.182 to 0.126), at 3.4x the renders in all (the lane's earlier measurement on the first baseline: 2 to 30 % smaller for 2.5x). The button's default is the cheap run; a "look harder" mode could be the long one.
- **Multi-scale failures** want something other than more renders: a per-axis step learned from a first few generations (the covariance adaptation does this, slowly), a separate bisection on the axes that are needles (pitch), or taking the pin idea from `match` (hold a precisely measured axis, search the rest). Not measured.
- **The pool.** The 3.3x speed-up on 8 workers is a `lambda` limit; speculative extra candidates would fill the pool but change the search (they would have to be a setting, not a worker count).
