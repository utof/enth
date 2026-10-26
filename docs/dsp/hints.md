# Hints (`enth-solver::hint`)

- Date: 2026-10-19. P1a lane H; 2026-10-20, P1b lane EV (verdicts name the module, the new descriptor families); 2026-10-20, R1/S1 lane SN (the [coach](#coach-and-sensitivity-r1s1-lane-sn) and the sensitivity it reads; `hint/` itself is unchanged). Crate: `crates/enth-solver` (L5; depends on `enth-params`, `enth-patch`, `enth-render`, `enth-analysis`, `libm`; no I/O, clocks, threads or global RNG; builds for wasm32).
- What it is: the `fix` and `blocked` of `enth check` and `enth step` ([agent-cli.md](../agent-cli.md#expectations-enth-check), [design.md §7](../design.md#7-how-far-were-your-knobs-when-many-answers-are-right)). Given a patch, the goal's performance and an expectation list with failures, it returns **one** move of several parameters that makes failing items pass while every passing item keeps passing, measured by real renders, or a verdict naming the item that cannot pass and why.
- Ported from our own research prototype (report 03 §5 and F7, `~/code/enth-scratch/2026-10-19-p1a/03/proto/src/solver.rs`), no third-party source. Nothing needs a `THIRD_PARTY_NOTICES.md` entry.

## API (for the CLI, lane C)

```rust
pub fn hint(
    patch: &Patch,               // the patch being checked
    perf: &Performance,          // the goal's performance: base, verification and final renders use it
    spec: &RenderSpec,           // sample rate and render seed
    expects: &[Expect],          // enth_analysis::parse_list(...)
    eval: &mut dyn Evaluator,    // the caller's parallelism
    cfg: &HintCfg,               // HintCfg { max_renders: fix_budget, ..HintCfg::default() }
) -> Result<HintResult, HintError>;

pub trait Evaluator {
    /// Exactly one result per job, in job order; each a pure function of its job.
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>>;
}
pub struct Job<'a> { pub patch: Patch, pub perf: &'a Performance, pub spec: RenderSpec, pub want: Want }
pub fn measure(job: &Job<'_>, describer: &mut Option<Describer>) -> Result<Descriptors, EvalError>;
pub fn note_timing(perf: &Performance) -> NoteTiming;
pub struct Serial; // impl Evaluator: `measure` for each job in turn
```

The CLI's rayon evaluator is one line, and must call `measure` so that its descriptors are the solver's:

```rust
jobs.par_iter().map_init(|| None, |d, job| enth_solver::measure(job, d)).collect()
```

`measure` renders, then describes the mid signal `(L + R) / 2` with `note_timing(perf)` (the first note, its note-off even past the clip end) on the scalar FFT path. `describe` and `check` should use the same two functions for the base render, so the hint's `before` equals the CLI's own results (without noise).

**Mapping `HintResult` to `enth.check/1`:**

| `enth.check/1` | From |
| --- | --- |
| `fix.measured` | always `true` |
| `fix.pass_all`, `fix.single_param_enough` | `Fix::pass_all`, `Fix::single_param_enough` |
| `fix.renders` | `HintResult::stats.renders` (every evaluation, the base included) |
| `fix.ops` | `Fix::moves` → `{"set": move.addr, "to": move.to_text()}`. `to_text()` parses back to exactly the value that was rendered (the fix is rounded to the 4 significant digits the CLI prints before its final render) |
| `fix.after` | `Fix::after[i].got` per item (real readings of the final render, `noise: None`) |
| `fix.note` | the CLI's text; `Fix::still_failing()` lists what still fails |
| `blocked[]` | `HintResult::blocked`: `expect`, `why`, `at_bound` (addresses as text), plus `reason.code()` and `conflicts_with` (item indices) |
| no `fix` | `HintResult::fix` is `None`: no measured move improved the list without breaking a passing item |

`HintError` codes: `no_items`, `no_budget`, `no_notes`, `evaluator_contract`, or the base render's own code (a patch that does not compile, a bad performance).

## Algorithm

1. **Base.** One render and describe with the goal's performance; every item judged with `Expect::evaluate` (the scalar path, so pass/fail is identical on every platform). If everything passes, stop: one render.
2. **Free parameters.** Continuous (`Float`) parameters that are modulatable or automatable, not structural, not periodic, of an enabled module. Not free: enums, integers and toggles (waveform, filter mode, voice mode, polyphony, seeds); start phases (periodic, and the descriptors' own nuisance); transposition (`osc.semi`, unless `HintCfg::transpose`: it duplicates `fine` and the step would prefer fractions of a semitone, see Limits). *Inert* parameters are skipped because they cannot change this render: a disabled module; a modulator that is neither the amplitude envelope nor a route source; `osc.pw` off the pulse wave and `osc.position` off the wavetable (when nothing is automated); an `lfo`'s `rate` when its `sync` is on (the speed then comes from `beats` and the tempo) and its `beats` when `sync` is off (when nothing is automated; `beats` is an enum, so it is not free anyway, and generator v2's mask applies the same rule); `voice.glide` with one note; an `adsr`'s `velocity` at full velocity and its `release` when no note-off happens in the clip. `HintCfg::only` and `frozen` narrow the set.
3. **Search domain.** Each parameter on its normalised axis `u` (0–1, its spec's scale); moves are reported in plain units. Each item as a value in its own domain: ln for Hz and ms features, the value for dB, cents (pitch against a note) and ratios; `clipping` through the sample peak in dBFS. Its unit is the item's *aim margin*: `Feature::hint_margin` (×1.1 for Hz and ms, 1.5 dB for levels), 3 ct for pitch.
4. **Probe.** One render per free parameter at `u ± 0.1` toward the middle of its range, all in one batch: a finite-difference Jacobian. A change under 0.1 aim margins, or under one 1 ms envelope frame for time features, counts as zero, so descriptor quantisation does not hand every parameter a meaningless slope. When the budget cannot cover every parameter, those whose `ParamSpec::perceptual` descriptor is in a failing item's family go first (pitch, with the tonal pitch; envelope; brightness, with the filter-sweep features, which the filter's cutoff and resonance move; noisiness; beating; tremolo; level), then untagged parameters of a module that has such a parameter (a filter's resonance with its cutoff), then parameters related to any item; unrelated parameters are probed only when the verification renders are still covered. **Clip length:** only when every item is spectral (pitch, brightness, noisiness) does the probe use a 0.5 s clip; envelope and beating items probe with the goal's full performance (sustain, release, decay and slow beats cannot be measured on 0.5 s). Verification and the final render always use the full performance.
5. **Step.** Minimise `Σ wᵢ·dist(gᵢ + Jᵢ·Δu, bandᵢ)² + λ‖Δu‖²` with `0 ≤ u + Δu ≤ 1`, by a projected active-set Gauss-Newton loop (parameters at a bound whose gradient pushes outward are held for the iteration). A failing item's band is its interval shrunk by one aim margin (aim inside, so the fix is not thin), weight `1/margin²`. A passing item's band edge is half a margin inside its interval, or its current value when it is thinner than that, with 100 times the weight: keeping passing items passing is close to a hard constraint.
6. **Verify.** Render the step. Accept it only if **no item that passes stops passing**, and it fails fewer items or as many with a smaller total shortfall (in aim margins); then halve λ. Otherwise multiply λ by 4. Either way a Broyden rank-1 update corrects the Jacobian with what the render measured. At most `max_steps` (6) steps.
7. **Unreachable items.** A failing item that the model cannot bring into its band even with every parameter serving it alone is left out of the steps (it would drag every parameter to a bound for nothing). When the steps stall, one *long shot* includes those items and is kept only if it is an improvement (no passing item breaks, and fewer items fail or the same number with a smaller total shortfall; the step loop then continues from there, where the model's slope is steeper): a dB reading driven by a linear-% parameter (sustain 70 % → below −30 dB) falls much faster near zero than its slope says, so the linear model under-promises there. A hopeless item costs that one render and moves nothing unless the move also shrinks the shortfall.
8. **Absent descriptors** (no beating yet, no pitch) have no slope. When one appears in an accepted step, its row is measured there (one render per parameter that moves it). When the steps stall without it, the solver looks further out along the parameters whose `perceptual` descriptor is in its family (`osc.fine` for beating), `u ± 0.3` then `± 0.6`, one offset per parameter per round, jumps to the point closest to passing, measures the row and steps again. This is the `ParamSpec.perceptual`-ordered probe of report 03: it cut the prototype's 26–30 render wide probe to 2–4.
9. **Finalise.** Moves under 3 % of their range are dropped and the rest rounded to 4 significant digits (`ParamSpec::display_rounded` and back through `parse`), then rendered once more. If that breaks an item the best point passed, rounding alone is tried, then the exact values (`Fix::rounded = false`). What an agent reads is what was measured.
10. **Blocked.** For each item still failing, the local model at the final point decides. If a joint move of the free parameters reaches its band, it is not blocked, only unfinished (it shows as failing in `fix.after`). Otherwise:

| `reason` | When | `why` names |
| --- | --- | --- |
| `performance` | The release of a note never released inside the clip; a reading that is only a lower bound because the sound outlasts the clip; a `duration <` bound below the hold that the parameters cannot reach | The performance, and any parameter at its bound (a held note sounds for at least its hold unless its sustain is near silence) |
| `at_bound` | The parameters that would move it the right way reach their bounds before it passes (`at_bound`) | Those parameters |
| `conflict` | It could pass alone, but the move pushes other items out (`conflicts_with`) | Those items |
| `no_effect` | No free parameter moves it (a module it needs is missing) | The probed parameters |
| `absent` | Its descriptor never appeared | The parameters moved to look for it; the structure is the limit |
| `unmodelled` | `== null` items: judged by renders, never aimed at (`!= null` is handled as an absent descriptor) | — |

**Naming the module** (2026-10-20, eval finding 3: on `bass_sub`, `pitch.note ~ C2` was blocked with "the patch structure is the limit" and no module named). For `absent`, `no_effect` and `at_bound` verdicts, `hint/culprit.rs` appends the module behind the item, read off the patch at the final point: for pitch items the oscillator whose transposition matches how far off the reading is ("it reads C1, 12 st below the note: osc2 is transposed -12 st (osc2.semi) and sets the period; the fix never moves semi, so set osc2.semi yourself"), the noise module that hides the period (with `tonal_pitch` as the alternative), or that there is no oscillator; for beating a single oscillator; no noise module; no filter; nothing routed to a level (tremolo) or to a cutoff (filter sweeps). Like the inert-parameter rules, this is catalogue knowledge in the solver.

## Determinism and parallelism

The solver is serial and pure. Every batch goes to the caller's `Evaluator`, which returns results by job index; every loop runs in a fixed order over patch-ordered parameters and list-ordered items; the maths is `f64` with `libm` (no `std` transcendentals, no fused multiply-add). So a hint is identical for any worker count (tested with 1 and 4 real threads against the serial evaluator, and with a batch evaluated back to front) and on every platform: `tests/synthetic.rs` pins an FNV-1a digest of a hint over an analytic evaluator, independent of the engine's modules.

## Costs

An evaluation is one render plus `describe_some` with only the descriptor groups the items need. Measured 2026-10-19 on the owner's i7-9750H, release build, under heavy load from other lanes (load average 15–29), so upper bounds:

| Case | Clip | Renders | Batches | Serial | 4 workers | Result |
| --- | --- | --- | --- | --- | --- | --- |
| Conflict: pitch ± 6 ct and beating in [3, 6] Hz, both oscillators detuned ±15 ct | 2 s | 12 | 5 | 0.43 s | 0.24 s | 2 moves, all pass |
| g5-like lead, 8 items (sustain, beating, release failing) | 3 s | 14 | 6 | 0.84 s | 0.41 s | 6 moves, all pass |
| g4-like bell, 8 items (attack, decay, duration failing; beating absent) | 5 s | 16 | 9 | 0.71 s | 0.61 s | 5 moves, all pass |
| Infeasible: sustain > −6 dB and duration < 500 ms with a 1.5 s hold | 2 s | 10 | 3 | 0.31 s | 0.18 s | `blocked` (`performance`, `env1.release` at its bound) |

The probe batch parallelises; verification steps are serial (each depends on the last). A typical hint is 1 base + one probe per related free parameter (6–11 for a two-oscillator patch) + 1–4 verified steps + 1 final render: 10–16 renders, inside the CLI's default `--fix-budget 16` (`HintCfg::max_renders`, which counts every evaluation, the base included). In the dev profile the render-based tests (`tests/hints.rs`, about 70 evaluations of 2 s clips) take about 3 s.

The follower of report 03 on the real engine (`tests/follower.rs`, `#[ignore]`; template: two saws at 0 ct, a filter, the amplitude envelope; one hint applied per step), rerun 2026-10-19 on branch `p1a/ch` (real oscillators, SVF and ADSR; the CLI's `enth_solver::measure` path), release build:

| Goal | Result |
| --- | --- |
| g1 pluck | pass after 3 hint steps (34 renders): `env1.decay` 1.444 s is the last move. Before the long shot kept smaller shortfalls it failed with no fix: the first long shot left two items failing (sustain and duration) and was rejected, so both were reported blocked |
| g2 pad | pass, 1 hint step (14 renders): `filter1.cutoff` 836.5 Hz, `resonance` 2.57 %, `keytrack` 4.4 %, `env1.attack` 570 ms, `env1.release` 2.67 s |
| g3 buzzy bass | pass, 1 hint step (11 renders): `filter1.cutoff` 14.0 kHz |
| g4 bell | fail after 1 hint step (26 renders), `blocked` empty: only `sustain < -35dB` is left, 5 dB short, and the model's steps do not reach it (unfinished, not blocked) |
| g5 supersaw lead | pass, 1 hint step (11 renders): `osc1.fine` +20 ct |

The g1 and g4 failures share a cause: a dB reading driven by the sustain percentage is strongly non-linear, and the spurious slope of `env1.decay` on `sustain_db` (about -1.6 dB per unit, just above the 0.15 dB dead zone of the finite differences) makes the damped step spread over decay and release instead of sustain. g1 is now rescued by the long shot accepting a smaller shortfall; g4 is the open case (try a second probe batch from the improved point, or exclude parameters whose slope on the item has the wrong sign at the second step).

A random sweep on real renders (40 random two-oscillator patches, envelope settings, notes and holds, each with five random items on attack, sustain, beating, release and duration; one hint each, default budget): 38 got a fix, 22 of them passing every item; no fix ever broke a passing item; at most 16 renders (543 in total); 14 items were reported blocked (mostly `conflict`, for example a short release against a long duration). The script is kept in `~/code/enth-scratch/p1a/h/scratch-tests/zz_real_sweep.rs`.

## Limits and follow-ups

- **No categorical or structural moves yet.** Waveforms, filter modes, voice mode, LFO shapes and adding or removing modules are not moved; an item only they could fix is reported as `no_effect`, `absent` or `at_bound`, and the CLI adds its structural text hint (`"measured": false`). Designed in report 03 §5 step 8: try each alternative of an enum only when the continuous solve leaves failures, with its own probe if needed.
- **Route depths and macros are not moved**: they are not `ParamSpec` parameters (`SetDepth`, `SetMacro`). The S2a compiler refuses routes today; once lane S2b lands modulation, route depths are the next free variables.
- **Transposition** (`osc.semi`) moves only with `HintCfg::transpose`, probed by one semitone (a 10 % step of ±48 st changes which pitch the analysis locks onto). Off by default because on the normalised axis a cent of `semi` is 48 times cheaper than a cent of `fine`, so the damped step would prefer fractions of a semitone to a detune.
- **Local model.** The Jacobian is local and descriptors are not smooth everywhere (YIN octave jumps, the beating estimator's 0.15–25 Hz band, envelope thresholds). The render check makes a wrong model cost renders, never a wrong answer; `blocked` verdicts are the model's opinion at the final point and can be pessimistic for strongly non-linear items (the long shot covers the common case).
- **Unfinished is not blocked.** An item the model says a joint move can reach, but that the budget or the step limit left failing, appears only as failing in `fix.after`. Raising `--fix-budget` or running another `check` continues from the improved patch.
- **Inert-parameter rules and the module naming are product knowledge in the solver** (`hint/space.rs`, `hint/culprit.rs`). They belong in the catalogue as "active when" and "what makes this descriptor" metadata; move them when the catalogue grows one.
- **Route depths and `semi` are not moved**, so a filter sweep that is too shallow or a transposed oscillator is only named, never fixed: the verdict says which knob to turn. (An LFO's `rate` is free when the LFO is a route source, so a filter-LFO rate item can be fixed.)
- **Noise is not part of the hint.** The fix aims one aim margin inside each band, and acceptance uses the deterministic render; the CLI's `noise` and `thin` (two nuisance re-renders) are not recomputed for the fixed patch.
- **No conflict list on success.** Report 03's cosine conflict detector was untested on real conflicts and is not built; conflicts are reported per blocked item from the model (`conflicts_with`).

## Coach and sensitivity (R1/S1, lane SN)

The Recreate game's hint button. Two modules in `enth-solver`, `sensitivity.rs` and `coach.rs`; `hint/` is **not** touched: the coach hands the unchanged solver an `Expect` list built from the target's descriptors and turns the verified `Fix` into four graded hints. Same rules as the rest of the crate: pure, no I/O, threads, clocks or global RNG, `libm` only, wasm32-buildable. The tests are `tests/sensitivity.rs` (7) and `tests/coach.rs` (11).

### Sensitivity (`sensitivity::sensitivity`)

How much each randomized parameter moves each descriptor, **measured**, not catalogued: one batch of `1 + n` evaluations (the target, then each continuous randomized parameter moved one just-noticeable step), through the caller's `Evaluator` (results by job index, so the worker count never matters). The step is `enth_challenge::priors::jnd` (cutoff 2 st, times ×1.5, levels 3 dB, cents 3), toward the middle of the axis when the value sits at an end. Everything is compared in the solver's own domain (cents above 1 Hz for pitch, ln for Hz and ms features, the unit otherwise), and a descriptor's change below `INERT_BELOW` (0.25) aim margins counts as no change.

| Result | Meaning |
| --- | --- |
| `ParamSensitivity::delta(f)`, `slope(f)` | The change of descriptor `f` over one step, and per unit of the normalised axis (signed: which way the descriptor goes when the knob is raised) |
| `reach()`, `inert()` | The largest effect in aim margins over all descriptors; `inert` when it is under `INERT_BELOW` |
| `tolerance_norm()` | How far the knob can move before its strongest descriptor leaves the tolerance band (normalised axis) |
| `Sensitivity::combined(f)` | The root-sum-square of the single-step effects of every randomized parameter on `f`: what the descriptor does when every knob is one step off at once |
| `Sensitivity::tolerance(f)` | `combined(f)` clamped to `tolerance_bounds(f)`: 2 to 8 aim margins (pitch 10 to 25 ct). This is the width the coach and lane ST give a descriptor |
| `discrete`, `unprobed` | Randomized parameters that are not continuous (a waveform), and continuous ones the budget or a failed render left out (tagged ones are probed first) |

Budget: `SensitivityCfg::max_renders` (48), the target's render included. Errors are values (`SensitivityError::{NoBudget, NoNotes, Base, Contract}`, each with a `code()`), never panics.

### The coach (`coach::CoachTarget`)

```rust
let mut ct = CoachTarget::for_round(&round, &mut eval, &CoachCfg::default())?; // once per round
let plan = ct.plan(&patch, &mut eval, &cfg)?;                                  // once per patch revision
let hint = plan.hint(level);                                                  // levels 1..=4, free
```

`CoachTarget::prepare` does the target's half (render, describe, sensitivity) and builds one `Item` per descriptor the target has: pitch against the target's note, brightness, attack, decay, sustain level, release and noisiness as bands `target ± tolerance` (in ln for Hz and ms, so the band is a ratio), and beating, tremolo and the filter sweep as a band or, when the target has none, `== null`. Loudness, peak and clipping are left out (the A/B switch matches the level; the sound score ignores it). `plan` then calls `hint(patch, perf, spec, &expects, eval, &HintCfg { only: randomized ∪ the SetParams that differ from the target, .. })` and maps the result. `CoachCfg::default()` is `SensitivityCfg::default()` and `HintCfg { max_renders: 24, transpose: true, .. }` (`osc.semi` is a randomized parameter, so "raise Semitones" is a valid level-3 hint).

The levels, with the structured `Hint` fields each fills (empty before their level):

| Level | Says | Fields | Example text |
| --- | --- | --- | --- |
| 1 | Which aspect is off, and how | `aspect`, `difference` | "Check the brightness: your sound is darker than the target." |
| 2 | Which module or region | `module` | "Your sound is darker than the target: look at the filter (filter1)." |
| 3 | Which parameter, and which way | `param`, `turn` | "…: raise Cutoff on the filter (filter1)." |
| 4 | A concrete value range | `range`; for a route hint `route` (lane HINT, FU1: its source and destination, which level 4's text names; level 3 names only the destination, `param`) | "…: set Cutoff on the filter (filter1) to between 1.6kHz and 2.6kHz." |

A categorical parameter has no `turn` or `range`: its level-3 text says to change it, level 4 names the setting. `Hint::measured` is true when the text comes from a verified render of the solver's fix and false for a categorical or advice hint.

**No early leak.** Levels 1 to 3 contain no number at all but module ids (`filter1` is a name). Level 4 gives a range, not the target's value: the verified fix's value plus and minus the distance at which the strongest descriptor stays within tolerance (`tolerance / |slope|`, at least one probe step, at most 0.15 of the axis), rounded outward to two significant digits. The target's patch is read for two things, which parameters the player may be told about and whether a move is compensating (below), and never quoted. `tests/coach.rs` checks that no text before level 4 contains a digit outside a module id, on a synthetic landscape and on real rounds at several difficulties.

**Which hint** (`CoachPlan::verdict`):

| Verdict | When | `measured` |
| --- | --- | --- |
| `Measured` | A move of the fix serves the worst failing aspect (largest predicted effect by the sensitivity slope; one that is not compensating first) | true |
| `Structure` | A categorical difference, found by comparing the player's patch with the target (waveform, filter mode, a module switched on or off, a route to add, remove or re-weigh), in a module that can affect the aspect. It wins over a *compensating* measured move: one that turns the knob away from the target's own value or lands more than 0.15 of the axis from it (closing the cutoff to hide the wrong waveform) | false |
| `Advice` | No move and no difference serves any failing aspect: general advice for the worst one, no value | false |
| `Close` | Nothing fails: every level says so | false |

Aspects are ranked by how far off they are in tolerance units; one missing from or extra in the player's sound counts as 8 and ranks first. Hints for a missing tremolo or filter sweep point at the missing route (level 2: "look at the modulation routes"; level 3: "add a modulation route to the filter's cutoff"; level 4 adds the depth range), not at an LFO shape.

### Determinism and costs

The sensitivity and the plan send every render through the caller's `Evaluator` in fixed-order batches and read results by job index; the maths is the crate's usual `libm` `f64`. The same inputs give the same text for 1, 4 and 8 workers and for a batch evaluated back to front (`the_worker_count_does_not_change_the_hints`, `the_result_is_pure_and_the_same_for_any_worker_count`).

| Job | Budget | Renders | Release, serial, load about 6 |
| --- | --- | --- | --- |
| `CoachTarget::prepare`, once per round | `SensitivityCfg::max_renders` (48) | `1 +` the continuous randomized parameters (difficulty 100: about 20) | 0.1 to 0.3 s; 2.3 s at difficulty 100 |
| `CoachTarget::plan`, per patch revision | `HintCfg::max_renders` (24), the player's base render included | 6 to 19 measured | 0.16 to 0.53 s |
| `coach_hint` (one shot) | The sum of the two | at most 72 | |

`prepare` and the probe batch of `plan` parallelise (the evaluator decides); the solver's verification steps are serial. The budget tests count the evaluator's calls: `prepare` stays within `sensitivity.max_renders`, `plan` within `hint.max_renders`.

### What a caller needs

- **The Hint job (lane SKR):** keep one `CoachTarget` per round, built lazily at the first hint (`for_round`, which plays the round's phrase at the scoring rate and seed, so the coach hears what the scorer hears). On a `Hint` request run `ct.plan(&patch, eval, &cfg)?.hint(level)` and map it with `CoachHint::new(h.level, h.text.clone())` plus what it points at (`CoachHint` lives in `enth-session`, above the solver; since FU1 the plugin's `coach_hint` copies `aspect`, `difference`, `module`, `param`, `turn` and `route`, never `range`: docs/session.md, "What a hint points at"). Cache the plan per patch revision: levels 2 to 4 of the same patch are then free. The evaluator must call `enth_solver::measure` (for example rayon `par_iter().map_init`) so the probes run in parallel. A hint makes the round *assisted* (the session's bookkeeping). `plan.result.fix` holds the exact ops for a future "apply this move"; do not show it to a player.
- **The UI (lane RUI):** show `Hint::text`; it is self-contained at every level. The structured fields (`aspect`, `difference`, `module`, `param`, `turn`, `route`; `range` stays in the worker) highlight the module or knob (lane HINT, FU1: docs/editor.md, "Hint highlighting"), but only the fields of the levels the player has asked for are filled. At `Close` the text is the same at every level and nothing is highlighted.
- **Lane ST:** `sensitivity(...)`, `ParamSensitivity::{inert, reach, tolerance_norm, slope}`, `Sensitivity::tolerance` and `tolerance_bounds` give the structure and time tolerances without a second measurement.

### Limits

- **Categorical moves are named, not measured.** The solver does not move waveforms, modes, routes or module on/off, so those hints are `measured: false` and the coach cannot say how much better the patch gets.
- **Noisy descriptors.** Decay is the noisiest; the attack descriptor does not resolve below about 10 ms (a probe of a shorter attack reads as no change), so a very fast attack gets a tolerance from the clamp, not from a measurement.
- **One fix, one hint.** The plan names the single best aspect and move; it does not list everything wrong. After the player acts, ask again.
- **Cost at the highest difficulty.** `prepare` with 20 randomized parameters is about 2.3 s serial; run it on a parallel evaluator and not on the UI thread.
