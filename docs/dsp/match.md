# Match (`enth-solver::search`, `heuristic`, `cmaes`)

- Date: 2026-10-20. R1/S1 lane MT. Crate: `crates/enth-solver` (L5; depends on `enth-params`, `enth-patch`, `enth-render`, `enth-analysis`, `enth-challenge`, `enth-dsp`, `libm`; no I/O, clocks, threads or global RNG; builds for wasm32).
- What it is: the engine of `enth match`, and the "CMA-ES floor" of the GR1 agent baseline. Given a target (audio, a patch, or descriptors), a start patch and the parameters that may move, it searches for a patch that sounds like the target within a render budget, and returns the best patch, its distance and a trace.
- Ported from our own Phase 0 spike (`spikes/spike-metric`, [metric.md](../spikes/metric.md)), no third-party source. Nothing needs a `THIRD_PARTY_NOTICES.md` entry.
- Not here: `enth match` itself (lane CL), the real scorer of a challenge (lane SS, which plugs in as an [`Objective`](#plugging-in-a-distance)), `nearest` (a documented stub, cut to R2).

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

**Cost.** A render at the scoring spec with the generated phrase takes roughly 15-30 ms of one worker (6-12 s for 1,500 renders on 4 workers, with other agents running on the machine); the search itself (CMA-ES on at most 11 axes, the heuristic) is negligible next to it. Results are bit-identical for 1, 2, 3 and 8 workers (`tests/match.rs`).

**Golden challenges.** The golden challenge set (`tests/golden/challenges/`, lane GC) did not exist on this branch's base (GC had not started), so the solve rate above is on the randomizer's fixed-seed targets, the closest stand-in. Run the floor against the golden codes once they exist: the example takes its targets from `random::target`, and a `--code` flag is a few lines once CT's code type is wired.

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
