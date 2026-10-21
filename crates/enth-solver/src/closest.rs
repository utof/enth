//! **Owner: lane CLS** (closest, wave 1). Pre-declared by lane S0l (R2).
//!
//! The nearest passing patch from the player's own (docs/design.md §7, layer 3; docs/learn.md, "Knob feedback"):
//! CMA-ES from your patch over the step's editable set, penalising distance, answering per-parameter deltas. It is a
//! **"How close am I?" button** with progress and cancel, never live (a scoring render costs 15 to 30 ms and level-2
//! `match` runs took a median of 423 renders), and using it makes the redo a sibling challenge, as a Reveal does.
//!
//! The job payload is solver-native, `{round, patch, free, budget}` ([`ClosestJob`]), so `enth-learn` never calls this
//! crate (same layer) and the shell's worker composes nothing. Like the rest of the solver it spawns no threads, reads
//! no clocks and does no I/O: parallelism comes from the caller's evaluator ([`ClosestEval`]), which returns results
//! by job index, so the answer is the same for any worker count (CLS's acceptance: 1 and 8 workers). The free set is
//! usually 1 to 4 parameters; the cost on ten real lesson starts is in `docs/dsp/match.md`, "Closest".
//!
//! This is R2's feature. [`nearest`](crate::nearest) is R3's ("the nearest preset as a start", filled by R3a's k-NN
//! retrieval); the two names stay apart.
//!
//! # What it asks
//!
//! Of every patch that **passes the round** (the round's own scorer decides, [`ScoreReport::passed`]), which is the
//! closest to the player's patch, moving only the free parameters? "Closest" is knob travel: the Euclidean distance on
//! the parameters' normalised 0–1 axes (the axes the player turns: an octave of cutoff is the same step everywhere),
//! plus [`ClosestCfg::flip_cost`] for every categorical parameter (a waveform, a filter mode) that changes. The
//! answer therefore respects the player's route: a patch that is a long way from theirs, however well it sounds, loses
//! to a passing one next door. There is no target-driven start (`match`'s heuristic would jump to the target's
//! descriptors): the search begins at the player's patch and only the scorer pulls it.
//!
//! # How it searches
//!
//! The existing [`Search`](crate::search::Search) (probe, CMA-ES runs, IPOP restarts) drives the candidates, with no
//! heuristic and a distance that is a **feasibility ranking**, not a sum, so the scale of the two terms never
//! matters (CMA-ES uses only the ranks):
//!
//! | Candidate | Cost | Order |
//! | --- | --- | --- |
//! | passes | its knob distance from the start (a hair of its sound % breaks exact ties) | first, nearest first |
//! | does not pass | `offset + 100 * jnd` (the scorer's distance in just-noticeable steps; `offset` is above any knob distance) | after every pass, closest to passing first |
//! | cannot be scored | [`FAILED`] | last |
//!
//! So while nothing passes the search descends the scorer's own distance (as `match` does), and once something passes
//! it slides along the pass boundary towards the player's patch. The search stops early:
//!
//! - **the player's patch already passes** (one render): the answer is that patch, [`ClosestStop::StartPasses`];
//! - **it settles** ([`ClosestStop::Converged`]): with no categorical parameter free, when the nearest passing
//!   distance has not improved by [`ClosestCfg::tol`] (relative, with an absolute floor) for [`ClosestCfg::patience`]
//!   generations; the run that found a pass is the last one that runs (no random restart after it, which would only
//!   find a farther patch). With categorical parameters free, the top [`ClosestCfg::choices`] choices each get a run
//!   to its natural end, except a choice that changes more categorical parameters than the nearest pass already
//!   found does (it costs more than that pass however the knobs sit);
//! - **the budget** is used ([`ClosestStop::Budget`]), a hard cap on renders, or the caller's callback says stop
//!   ([`ClosestStop::Cancelled`]);
//! - **nothing is left to try** ([`ClosestStop::Exhausted`]): only categorical parameters are free and every choice
//!   was scored.
//!
//! A search that settled ends with a **polish**: bisection on the segment from the player's patch to the nearest
//! passing one found, three candidates a round for three rounds (9 renders, if the budget has them). The optimiser
//! stops a little short of the pass boundary; with one parameter free the polish finds it to 1/64 of the way, with more
//! it removes the slack along the player's path. Every candidate is scored, so it never makes the answer worse.
//!
//! When nothing passes the answer is the candidate nearest to passing, [`Closest::passed`] is false and
//! [`Closest::percent`] says how far it got: with the free set the step allows, the round cannot be passed (or not
//! within the budget). Every returned patch was really scored: the result never rests on the optimiser's mean.
//!
//! # What it does not promise
//!
//! The answer is **a** nearby passing patch, within about 10–30 % of the travel of a run ten times as long, not the
//! provably nearest. Landscapes with axes of very different scale (a pitch that must be right to a few cents beside a
//! sustain level whose pass region is a sliver) can defeat the search at any budget; the cost table in
//! `docs/dsp/match.md` counts those. A free set with a categorical parameter ranks its choices by one score at the
//! player's own continuous settings, which is a weak guide: [`ClosestCfg::choices`] is how many are tried (3 by
//! default; raise it, and the budget, for a set that mixes a waveform with several knobs).
//!
//! # Determinism and parallelism
//!
//! A result is a pure function of its inputs and [`ClosestCfg`]. The evaluator contract is `eval`'s and `search`'s:
//! one reading per candidate, in input order, each a pure function of its candidate (call [`reading`] per candidate
//! and nothing else). The population is a setting ([`ClosestCfg::lambda`]), never derived from a worker count, so 1
//! and 8 workers give the same [`Closest`], bit for bit (`tests/closest.rs`). Pick a `lambda` that is a multiple of
//! the pool size to keep the pool busy.
//!
//! ```
//! use enth_challenge::{SpecPreset, prepare_round};
//! use enth_solver::closest::{ClosestCfg, ClosestJob, RoundSerial, closest};
//!
//! // A level-1 round, and the player's patch: the start, which does not pass.
//! let round = prepare_round(&SpecPreset::by_id(1).unwrap().spec(1, 42)).unwrap();
//! let free: Vec<_> = round.params().iter().take(2).cloned().collect();
//! let job = ClosestJob { round: &round, patch: round.start(), free: &free, budget: 12 };
//! let found = closest(&job, &mut RoundSerial::new(), &ClosestCfg::default()).unwrap();
//! assert!(found.renders <= 12);
//! assert_eq!(found.deltas.len(), free.len());
//! ```
//!
//! A caller with a pool implements [`ClosestEval`] over `reading` (the CLI and the plugin worker use rayon; the
//! round is `Send + Sync`, so one shared reference serves every thread):
//!
//! ```ignore
//! struct Pool;
//! impl enth_solver::closest::ClosestEval for Pool {
//!     fn readings(&mut self, round: &PreparedRound, candidates: &[Patch]) -> Vec<Option<Reading>> {
//!         use rayon::prelude::*;
//!         candidates.par_iter().map(|p| enth_solver::closest::reading(round, p)).collect()
//!     }
//! }
//! ```

use core::fmt;

use enth_challenge::{PreparedRound, ScoreReport, score_round};
use enth_patch::{ParamAddr, Patch};

use crate::search::{AxisKind, FAILED, FreeSet, Point, Search, SearchCfg, SearchError, StopCause};

/// A render budget that is enough for a lesson step's 1 to 4 free parameters on the starts that were measured (the
/// cost table in `docs/dsp/match.md`, "Closest"); a job names its own.
pub const DEFAULT_BUDGET: usize = 300;

/// The relative tie-break a candidate's sound % adds to the cost of a passing candidate (100 − % times this, at most
/// 1e-5): between two passing candidates the same distance apart, the one that sounds closer wins, and nothing else
/// changes.
const TIE: f64 = 1e-6;

/// The smallest improvement in the nearest passing distance that counts as progress (0.2 % of a knob's travel).
const ABS_FLOOR: f64 = 0.002;

/// What a candidate's sound distance in just-noticeable steps counts for in the cost of a candidate that does not
/// pass. CMA-ES uses only the ranks, so it changes nothing there; it makes the optimiser's own stall test (no 1 %
/// improvement in 15 generations) see a move of a few tenths of a step against the `offset`, which is not small beside
/// a jnd near 1 (a pitch a few cents closer is such a move).
const JND_WEIGHT: f64 = 100.0;

/// The polish after the search settles: this many candidates on the segment from the player's patch to the nearest
/// passing one per round, and this many rounds. Each round narrows the bracket around the boundary to a fraction of
/// `1 / (POLISH_WAYS + 1)` of what it was.
const POLISH_WAYS: usize = 3;
const POLISH_ROUNDS: usize = 3;

/// A nearest passing patch closer than this to the player's has nothing left to polish (0.5 % of a knob's travel).
const POLISH_MIN: f64 = 0.005;

// ---------------------------------------------------------------------------------------------------------------
// What the evaluator returns
// ---------------------------------------------------------------------------------------------------------------

/// What the round's scorer says about one candidate: the three numbers `closest` needs from a [`ScoreReport`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// The sound score, 0–100 (`SoundScore::percent`).
    pub percent: f64,
    /// The sound distance in just-noticeable steps (`SoundScore::jnd`): the smooth quantity the search descends
    /// while nothing passes.
    pub jnd: f64,
    /// Whether the candidate passes the round (`ScoreReport::passed`): the round's own verdict.
    pub passed: bool,
}

impl Reading {
    /// The reading of a score report.
    #[must_use]
    pub fn of(report: &ScoreReport) -> Reading {
        Reading {
            percent: report.sound.percent,
            jnd: report.sound.jnd,
            passed: report.passed,
        }
    }
}

/// Score one candidate against the round with the round's own scorers ([`score_round`]); `None` if it cannot be
/// scored (a render that fails). **The one function an evaluator calls per candidate**, so every evaluator agrees.
#[must_use]
pub fn reading(round: &PreparedRound, patch: &Patch) -> Option<Reading> {
    score_round(round, patch)
        .ok()
        .map(|report| Reading::of(&report))
}

/// Scores batches of candidates against a round. Implemented by the caller, which owns parallelism.
///
/// Contract (the same as `Evaluator` and `Objective`): `readings(round, c)` returns exactly `c.len()` values,
/// `out[i]` for `c[i]`, and `out[i]` depends only on `round` and `c[i]` (not on the batch, the order of work or the
/// worker that ran it). Call [`reading`] per candidate to get that for free.
pub trait ClosestEval {
    /// The reading of every candidate, in input order.
    fn readings(&mut self, round: &PreparedRound, candidates: &[Patch]) -> Vec<Option<Reading>>;
}

/// A closure that scores a batch is an evaluator.
impl<F> ClosestEval for F
where
    F: FnMut(&PreparedRound, &[Patch]) -> Vec<Option<Reading>>,
{
    fn readings(&mut self, round: &PreparedRound, candidates: &[Patch]) -> Vec<Option<Reading>> {
        self(round, candidates)
    }
}

/// The single-threaded evaluator: [`reading`] for each candidate in turn.
#[derive(Clone, Copy, Debug, Default)]
pub struct RoundSerial;

impl RoundSerial {
    /// A serial evaluator.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ClosestEval for RoundSerial {
    fn readings(&mut self, round: &PreparedRound, candidates: &[Patch]) -> Vec<Option<Reading>> {
        candidates.iter().map(|p| reading(round, p)).collect()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The job, the settings, the answer
// ---------------------------------------------------------------------------------------------------------------

/// The job: `{round, patch, free, budget}`. The shell's worker holds the prepared round (the game worker already
/// does), takes the player's patch and the step's editable parameters, and calls [`closest`]; nothing is composed
/// outside the solver.
#[derive(Clone, Copy, Debug)]
pub struct ClosestJob<'a> {
    /// The round: its target and scorers decide what passes.
    pub round: &'a PreparedRound,
    /// The player's patch: where the search starts and what "closest" is measured from.
    pub patch: &'a Patch,
    /// The parameters that may move (a lesson step's editable set, usually 1 to 4).
    pub free: &'a [ParamAddr],
    /// The most renders, the first (the player's own patch) included. **Hard**: no batch overshoots it.
    pub budget: usize,
}

/// Settings of a search; [`Default`] is what the "How close am I?" button uses.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosestCfg {
    /// The seed of every random choice (the stream is `search`'s, `derive(seed, "MATCHSEA")`).
    pub seed: u64,
    /// Population of the first CMA-ES run; 0 is Hansen's default for the number of continuous axes (4, 6, 7, 8 for 1
    /// to 4). A setting, never derived from a worker count; use a multiple of the pool size to keep it busy.
    pub lambda: usize,
    /// The first CMA-ES step, in the normalised 0–1 units around the player's patch.
    pub sigma: f64,
    /// What changing one categorical parameter (a waveform, a filter mode) costs, in the knob distance's units: 1 is
    /// as far as turning a knob across its whole range.
    pub flip_cost: f64,
    /// How many categorical choices get a CMA-ES run (the best by their score at the player's continuous settings,
    /// which ranks them only roughly: a set that mixes a waveform with several knobs wants more, and a larger budget).
    /// Without a categorical parameter free there is only one.
    pub choices: usize,
    /// Settled: the nearest passing distance has improved by less than this fraction of itself (and less than 0.002
    /// absolute) over [`patience`](Self::patience) generations. Only used with no categorical parameter free.
    pub tol: f64,
    /// How many generations (batches) the nearest passing distance may stay unimproved before the search settles.
    pub patience: usize,
}

impl Default for ClosestCfg {
    fn default() -> Self {
        ClosestCfg {
            seed: 1,
            lambda: 0,
            sigma: 0.1,
            flip_cost: 1.0,
            choices: 3,
            tol: 0.03,
            patience: 3,
        }
    }
}

impl ClosestCfg {
    fn validate(&self) -> Result<(), ClosestError> {
        let ok = self.sigma.is_finite()
            && self.sigma > 0.0
            && self.flip_cost.is_finite()
            && self.flip_cost >= 0.0
            && self.tol.is_finite()
            && self.tol >= 0.0
            && self.patience >= 1
            && self.choices >= 1
            && (self.lambda == 0 || self.lambda >= 4);
        if ok {
            Ok(())
        } else {
            Err(ClosestError::BadCfg(
                "sigma > 0, flip_cost >= 0, tol >= 0, patience >= 1, choices >= 1, lambda 0 or >= 4",
            ))
        }
    }
}

/// How one free parameter moved: the player's value and the nearest passing patch's.
#[derive(Clone, Debug, PartialEq)]
pub struct Delta {
    /// Which parameter.
    pub addr: ParamAddr,
    /// The player's value (plain, real units).
    pub from: f32,
    /// The nearest passing patch's value (plain, real units).
    pub to: f32,
    /// The move on the normalised 0–1 axis, signed (to − from): how much of the knob's travel. For a categorical
    /// parameter 0 when it stayed and 1 when it changed.
    pub norm: f64,
    /// Whether the parameter is categorical (an enum, a switch, a small integer): its `norm` is a flag, not a travel.
    pub categorical: bool,
}

impl Delta {
    /// Whether the parameter moved at all (a real move, however small).
    #[must_use]
    pub fn moved(&self) -> bool {
        self.norm != 0.0
    }
}

/// Why a search ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClosestStop {
    /// The player's patch already passes (one render): the answer is that patch.
    StartPasses,
    /// The search settled on a passing patch.
    Converged,
    /// The render budget was used.
    Budget,
    /// The caller's callback asked to stop.
    Cancelled,
    /// Nothing more to try: only categorical parameters were free and every choice was scored.
    Exhausted,
}

impl ClosestStop {
    /// A stable snake-case name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ClosestStop::StartPasses => "start_passes",
            ClosestStop::Converged => "converged",
            ClosestStop::Budget => "budget",
            ClosestStop::Cancelled => "cancelled",
            ClosestStop::Exhausted => "exhausted",
        }
    }
}

/// What a search found.
#[derive(Clone, Debug, PartialEq)]
pub struct Closest {
    /// The nearest passing patch found: the player's patch with the free parameters set. When nothing passed
    /// ([`passed`](Self::passed) is false) it is the candidate nearest to passing instead. It was really scored.
    pub patch: Patch,
    /// One entry per free parameter, in the order of the job's `free` (duplicates dropped): what moved and how far.
    pub deltas: Vec<Delta>,
    /// Whether [`patch`](Self::patch) passes the round.
    pub passed: bool,
    /// The sound score of [`patch`](Self::patch), 0–100.
    pub percent: f64,
    /// The sound score of the player's own patch.
    pub start_percent: f64,
    /// The knob distance from the player's patch to [`patch`](Self::patch) (the Euclidean distance on the free
    /// axes' normalised 0–1 units, plus [`ClosestCfg::flip_cost`] per categorical change): 0 when the start passes.
    pub distance: f64,
    /// Renders used (scored candidates), at most the budget.
    pub renders: usize,
    /// The render count at which a passing patch was first seen, if one was.
    pub first_pass_renders: Option<usize>,
    /// Why it ended.
    pub stop: ClosestStop,
}

/// What a callback sees after every batch: enough for a progress line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// Renders used so far.
    pub renders: usize,
    /// The job's budget.
    pub budget: usize,
    /// The knob distance of the nearest passing patch so far, if one was found.
    pub passing: Option<f64>,
    /// The sound score of the best candidate so far (the nearest passing one, else the one nearest to passing).
    pub percent: f64,
}

/// Why a job could not run.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClosestError {
    /// The free set or the search settings are refused (an empty set, an address the patch lacks, no budget).
    Search(SearchError),
    /// A setting of [`ClosestCfg`] is out of range.
    BadCfg(&'static str),
    /// The round could not score the player's own patch.
    StartUnscorable,
    /// The evaluator returned a different number of readings than candidates.
    EvalCount {
        /// Candidates asked.
        expected: usize,
        /// Readings returned.
        got: usize,
    },
}

impl ClosestError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            ClosestError::Search(e) => e.code(),
            ClosestError::BadCfg(_) => "bad_closest_cfg",
            ClosestError::StartUnscorable => "start_unscorable",
            ClosestError::EvalCount { .. } => "evaluator_count",
        }
    }

    /// What to do about it.
    #[must_use]
    pub fn hint(&self) -> String {
        match self {
            ClosestError::Search(e) => e.hint(),
            ClosestError::BadCfg(what) => format!("fix the setting: {what}"),
            ClosestError::StartUnscorable => {
                "the round could not score this patch; check that it renders".to_owned()
            }
            ClosestError::EvalCount { .. } => {
                "an evaluator returns exactly one reading per candidate, in order".to_owned()
            }
        }
    }
}

impl fmt::Display for ClosestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClosestError::Search(e) => write!(f, "{e}"),
            ClosestError::BadCfg(what) => write!(f, "bad closest setting: {what}"),
            ClosestError::StartUnscorable => {
                f.write_str("the round cannot score the player's patch")
            }
            ClosestError::EvalCount { expected, got } => write!(
                f,
                "the evaluator returned {got} readings for {expected} candidates"
            ),
        }
    }
}

impl std::error::Error for ClosestError {}

impl From<SearchError> for ClosestError {
    fn from(e: SearchError) -> Self {
        ClosestError::Search(e)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The entry points
// ---------------------------------------------------------------------------------------------------------------

/// Find the passing patch nearest to the player's: see the [module docs](self). Runs to the end (never cancelled).
///
/// # Errors
/// [`ClosestError`]: an empty or unknown free set, no budget, a setting out of range, a start the round cannot
/// score, an evaluator that returns the wrong number of readings.
pub fn closest(
    job: &ClosestJob<'_>,
    eval: &mut dyn ClosestEval,
    cfg: &ClosestCfg,
) -> Result<Closest, ClosestError> {
    closest_with(job, eval, cfg, &mut |_| true)
}

/// [`closest`] with a callback after every batch (progress); return `false` from it to stop
/// ([`ClosestStop::Cancelled`]). The solver reads no clock, so a wall-clock cap lives in the caller's callback.
///
/// # Errors
/// As [`closest`].
pub fn closest_with(
    job: &ClosestJob<'_>,
    eval: &mut dyn ClosestEval,
    cfg: &ClosestCfg,
    after_batch: &mut dyn FnMut(&Progress) -> bool,
) -> Result<Closest, ClosestError> {
    let round = job.round;
    run(
        job.patch,
        job.free,
        job.budget,
        cfg,
        &mut |candidates| eval.readings(round, candidates),
        after_batch,
    )
}

// ---------------------------------------------------------------------------------------------------------------
// The search
// ---------------------------------------------------------------------------------------------------------------

/// The geometry of the free set: where the player's patch sits and how far a candidate is from it.
struct Space {
    free: FreeSet,
    start: Point,
    flip_cost: f64,
}

impl Space {
    /// The knob distance of `patch` from the player's: Euclidean on the normalised continuous axes, plus the flip cost
    /// for every categorical parameter that differs.
    fn distance(&self, patch: &Patch) -> f64 {
        let at = self.free.point_of(patch);
        let mut sum = 0.0;
        for (a, b) in at.cont.iter().zip(&self.start.cont) {
            let d = a - b;
            sum += d * d;
        }
        let flips = at
            .cat
            .iter()
            .zip(&self.start.cat)
            .filter(|(a, b)| a != b)
            .count();
        libm::sqrt(sum) + self.flip_cost * flips as f64
    }

    /// The least knob distance any candidate with `patch`'s categorical choices can have: the flip cost of every
    /// categorical parameter that differs from the player's. A choice whose floor is no less than the nearest passing
    /// distance found cannot improve on it.
    fn flip_floor(&self, patch: &Patch) -> f64 {
        let at = self.free.point_of(patch);
        let flips = at
            .cat
            .iter()
            .zip(&self.start.cat)
            .filter(|(a, b)| a != b)
            .count();
        self.flip_cost * flips as f64
    }

    /// Above every knob distance a candidate can have, so a passing candidate always outranks a failing one.
    fn offset(&self) -> f64 {
        libm::sqrt(self.free.continuous().len() as f64)
            + self.flip_cost * self.free.categorical().len() as f64
            + 1.0
    }

    /// The cost of a candidate (lower is better): see the module docs' table.
    fn cost(&self, distance: f64, reading: Option<Reading>) -> f64 {
        match reading {
            Some(r) if r.jnd.is_finite() && r.percent.is_finite() => {
                if r.passed {
                    distance + TIE * (100.0 - r.percent).max(0.0)
                } else {
                    self.offset() + JND_WEIGHT * r.jnd.max(0.0)
                }
            }
            _ => FAILED,
        }
    }
}

/// The best candidate seen so far.
struct Best {
    cost: f64,
    patch: Patch,
    reading: Reading,
    distance: f64,
}

/// The state that follows a search batch by batch.
struct Track {
    best: Option<Best>,
    /// The knob distance of the nearest passing patch, and after every batch since the first pass, a history of it.
    pass: Option<f64>,
    history: Vec<f64>,
    first_pass_renders: Option<usize>,
    /// The number of CMA-ES runs begun when the first pass was seen.
    pass_runs: usize,
    renders: usize,
}

impl Track {
    /// Take note of a batch whose first `cached` readings were known before (not rendered again): the best candidate,
    /// the nearest pass, the render count. Returns the cost of every candidate, for the optimiser.
    fn record_costs(
        &mut self,
        space: &Space,
        batch: &[Patch],
        readings: &[Option<Reading>],
        cached: usize,
    ) -> Vec<f64> {
        let mut costs = Vec::with_capacity(batch.len());
        for (i, (patch, reading)) in batch.iter().zip(readings).enumerate() {
            if i >= cached {
                self.renders += 1;
            }
            let distance = space.distance(patch);
            let cost = space.cost(distance, *reading);
            costs.push(cost);
            let Some(r) = *reading else { continue };
            if r.passed && self.first_pass_renders.is_none() {
                self.first_pass_renders = Some(self.renders);
            }
            if r.passed && self.pass.is_none_or(|d| distance < d) {
                self.pass = Some(distance);
            }
            if self.best.as_ref().is_none_or(|b| cost < b.cost) {
                self.best = Some(Best {
                    cost,
                    patch: patch.clone(),
                    reading: r,
                    distance,
                });
            }
        }
        costs
    }

    /// [`record_costs`](Self::record_costs) for a batch the optimiser does not see (the player's patch, the probes).
    fn record(
        &mut self,
        space: &Space,
        batch: &[Patch],
        readings: &[Option<Reading>],
        cached: usize,
    ) {
        self.record_costs(space, batch, readings, cached);
    }

    fn progress(&self, budget: usize) -> Progress {
        Progress {
            renders: self.renders,
            budget,
            passing: self.pass,
            percent: self.best.as_ref().map_or(0.0, |b| b.reading.percent),
        }
    }

    /// Whether the nearest passing distance has stopped improving: over `patience` generations it gained less than
    /// `tol` of itself and less than the absolute floor.
    fn settled(&self, cfg: &ClosestCfg) -> bool {
        let n = self.history.len();
        if n <= cfg.patience {
            return false;
        }
        let (then, now) = (self.history[n - 1 - cfg.patience], self.history[n - 1]);
        then - now < (cfg.tol * now).max(ABS_FLOOR)
    }
}

/// The search behind [`closest`], over a scoring function: the candidates of a batch in, their readings out in the
/// same order. Split from the round so the tests drive it on analytic landscapes.
///
/// The order of the renders: the player's patch alone (a pass ends the job there), then the search proper, whose first
/// candidate is the player's patch again, answered from the first render.
#[allow(
    clippy::too_many_lines,
    reason = "one loop: ask, score, tell, and the four ways to stop, kept together so the order of the steps reads top to bottom"
)]
fn run(
    start: &Patch,
    free_addrs: &[ParamAddr],
    budget: usize,
    cfg: &ClosestCfg,
    score: &mut dyn FnMut(&[Patch]) -> Vec<Option<Reading>>,
    after_batch: &mut dyn FnMut(&Progress) -> bool,
) -> Result<Closest, ClosestError> {
    cfg.validate()?;
    let free = FreeSet::new(start, free_addrs)?;
    let categorical = !free.categorical().is_empty();
    // The search counts the player's patch as its first render, which `run` has already paid for (below).
    let search_cfg = SearchCfg {
        seed: cfg.seed,
        budget,
        lambda: cfg.lambda,
        sigma_start: cfg.sigma,
        runs_per_cycle: cfg.choices,
        use_heuristic: false,
        ..SearchCfg::default()
    };
    let mut search = Search::new(start, &free, None, &search_cfg)?;
    let space = Space {
        start: free.point_of(start),
        flip_cost: cfg.flip_cost,
        free,
    };
    let mut track = Track {
        best: None,
        pass: None,
        history: Vec::new(),
        first_pass_renders: None,
        pass_runs: 0,
        renders: 0,
    };

    // The player's patch, alone: if it passes there is nothing to search and nothing else is rendered.
    let one = score(core::slice::from_ref(start));
    if one.len() != 1 {
        return Err(ClosestError::EvalCount {
            expected: 1,
            got: one.len(),
        });
    }
    let Some(start_reading) = one[0] else {
        return Err(ClosestError::StartUnscorable);
    };
    track.record(&space, core::slice::from_ref(start), &one, 0);
    if start_reading.passed {
        return Ok(finish(
            &space,
            start,
            start,
            start_reading,
            start_reading,
            0.0,
            &track,
            ClosestStop::StartPasses,
        ));
    }

    // CMA-ES runs allowed before a pass was found: all of them. Once one is found, the run that found it is the last
    // (or, with categorical choices, the top `choices` runs, whichever is later): a random restart would only find a
    // patch farther from the player's. A run for a choice that changes more categorical parameters than the nearest
    // pass is long cannot beat it (a change costs `flip_cost` however the knobs sit), so it is not started either.
    let warm_runs = if categorical { cfg.choices } else { 1 };
    let mut ended: Option<ClosestStop> = None;
    let mut runs_seen = 0;

    let mut first = true;
    while ended.is_none() {
        let batch = search.ask();
        if batch.is_empty() {
            break;
        }
        if let Some(nearest) = track.pass {
            let runs = search.result().runs;
            let new_run = runs > runs_seen;
            runs_seen = runs;
            if new_run
                && runs > track.pass_runs
                && (runs > warm_runs || space.flip_floor(&batch[0]) >= nearest)
            {
                ended = Some(ClosestStop::Converged);
                break;
            }
        }
        // The first candidate of the first batch is the player's patch, whose reading is known.
        let cached = usize::from(first);
        let mut readings: Vec<Option<Reading>> = vec![Some(start_reading); cached];
        if batch.len() > cached {
            readings.extend(score(&batch[cached..]));
        }
        first = false;
        if readings.len() != batch.len() {
            return Err(ClosestError::EvalCount {
                expected: batch.len() - cached,
                got: readings.len() - cached,
            });
        }
        let had_pass = track.pass.is_some();
        let costs = track.record_costs(&space, &batch, &readings, cached);
        search.tell(&costs)?;
        if !had_pass && track.pass.is_some() {
            track.pass_runs = search.result().runs;
        }
        if let Some(d) = track.pass {
            track.history.push(d);
        }
        if !categorical && track.settled(cfg) {
            ended = Some(ClosestStop::Converged);
            break;
        }
        if !search.finished() && !after_batch(&track.progress(budget)) {
            search.cancel();
        }
    }

    let mut stop = ended.unwrap_or(match search.result().stop {
        StopCause::Cancelled => ClosestStop::Cancelled,
        StopCause::Exhausted => ClosestStop::Exhausted,
        StopCause::Budget | StopCause::Goal => ClosestStop::Budget,
    });
    // The search settles on a passing patch somewhere along the pass boundary; the last renders pull it back along
    // the segment to the player's patch (exact when one parameter is free, a gain in more), if the budget has any left.
    if stop == ClosestStop::Converged
        && polish(&space, start, &mut track, budget, score, after_batch)?
    {
        stop = ClosestStop::Cancelled;
    }

    // The answer: the best candidate scored (the player's patch itself is one).
    let (patch, reading, distance) = match &track.best {
        Some(b) => (&b.patch, b.reading, b.distance),
        None => (start, start_reading, 0.0),
    };
    Ok(finish(
        &space,
        start,
        patch,
        reading,
        start_reading,
        distance,
        &track,
        stop,
    ))
}

/// Bisect on the segment from the player's patch to the nearest passing patch found: [`POLISH_WAYS`] candidates per
/// round, [`POLISH_ROUNDS`] rounds, each round keeping the bracket between the last candidate that failed and the first
/// that passed. The boundary of the pass region is where the nearest passing patch lies, and the optimiser, which
/// stops when its distance stops improving, leaves it a little short of the segment's end: with one parameter free
/// this finds the end of the interval to 1/64 of the way, with more it removes the slack along the path. Every
/// candidate is scored, so a polished answer is never worse than the patch it started from. Returns whether the
/// callback cancelled.
fn polish(
    space: &Space,
    start: &Patch,
    track: &mut Track,
    budget: usize,
    score: &mut dyn FnMut(&[Patch]) -> Vec<Option<Reading>>,
    after_batch: &mut dyn FnMut(&Progress) -> bool,
) -> Result<bool, ClosestError> {
    let Some(best) = track.best.as_ref().filter(|b| b.reading.passed) else {
        return Ok(false);
    };
    if best.distance < POLISH_MIN || space.start.cont.is_empty() {
        return Ok(false);
    }
    let to = space.free.point_of(&best.patch);
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..POLISH_ROUNDS {
        let room = budget.saturating_sub(track.renders);
        if room == 0 {
            break;
        }
        let ts: Vec<f64> = (1..=POLISH_WAYS)
            .map(|i| lo + (hi - lo) * i as f64 / (POLISH_WAYS + 1) as f64)
            .take(room)
            .collect();
        let batch: Vec<Patch> = ts
            .iter()
            .map(|&t| {
                let at = Point {
                    cont: space
                        .start
                        .cont
                        .iter()
                        .zip(&to.cont)
                        .map(|(a, b)| a + t * (b - a))
                        .collect(),
                    cat: to.cat.clone(),
                };
                space
                    .free
                    .apply(start, &at)
                    .unwrap_or_else(|| start.clone())
            })
            .collect();
        let readings = score(&batch);
        if readings.len() != batch.len() {
            return Err(ClosestError::EvalCount {
                expected: batch.len(),
                got: readings.len(),
            });
        }
        track.record(space, &batch, &readings, 0);
        match readings.iter().position(|r| r.is_some_and(|r| r.passed)) {
            Some(j) => {
                hi = ts[j];
                if j > 0 {
                    lo = ts[j - 1];
                }
            }
            None => lo = ts[ts.len() - 1],
        }
        if !after_batch(&track.progress(budget)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The answer of a finished job.
#[allow(
    clippy::too_many_arguments,
    reason = "assembles one result from the pieces `run` keeps; a struct for it would only be built here"
)]
fn finish(
    space: &Space,
    start: &Patch,
    patch: &Patch,
    reading: Reading,
    start_reading: Reading,
    distance: f64,
    track: &Track,
    stop: ClosestStop,
) -> Closest {
    Closest {
        deltas: deltas(&space.free, start, patch),
        patch: patch.clone(),
        passed: reading.passed,
        percent: reading.percent,
        start_percent: start_reading.percent,
        distance,
        renders: track.renders,
        first_pass_renders: track.first_pass_renders,
        stop,
    }
}

/// How every free parameter differs between the player's patch and the answer.
fn deltas(free: &FreeSet, from: &Patch, to: &Patch) -> Vec<Delta> {
    free.axes()
        .iter()
        .map(|axis| {
            let a = from.value(&axis.addr).unwrap_or(axis.start);
            let b = to.value(&axis.addr).unwrap_or(a);
            let categorical = matches!(axis.kind, AxisKind::Categorical { .. });
            let norm = if categorical {
                if a == b { 0.0 } else { 1.0 }
            } else {
                f64::from(axis.spec.to_norm(b)) - f64::from(axis.spec.to_norm(a))
            };
            Delta {
                addr: axis.addr.clone(),
                from: a,
                to: b,
                norm,
                categorical,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::{Op, apply};

    fn addr(s: &str) -> ParamAddr {
        s.parse().unwrap()
    }

    fn set(p: &mut Patch, a: &str, v: f32) {
        apply(
            p,
            Op::SetParam {
                addr: addr(a),
                value: v,
            },
        )
        .unwrap();
    }

    /// The four continuous parameters of the init patch, and a categorical one.
    const AXES: [&str; 4] = ["osc1.semi", "osc1.fine", "osc1.pw", "env1.attack"];

    /// An analytic round: the target is a point of the free axes, and a candidate passes inside a ball of `radius`
    /// (normalised units) around it. `jnd` is the distance in radii, so the pass mark is 1. No render, microseconds.
    struct Ball {
        free: FreeSet,
        target: Point,
        radius: f64,
        /// A categorical axis the target needs: passing also requires these indices.
        need_cat: Vec<usize>,
        calls: usize,
    }

    impl Ball {
        fn new(start: &Patch, addrs: &[&str], at: &[f64], radius: f64) -> Ball {
            let addrs: Vec<ParamAddr> = addrs.iter().map(|s| addr(s)).collect();
            let free = FreeSet::new(start, &addrs).unwrap();
            let mut target = free.point_of(start);
            target.cont = at.to_vec();
            Ball {
                need_cat: target.cat.clone(),
                free,
                target,
                radius,
                calls: 0,
            }
        }

        fn read(&mut self, p: &Patch) -> Reading {
            self.calls += 1;
            let at = self.free.point_of(p);
            let mut sum = 0.0;
            for (a, b) in at.cont.iter().zip(&self.target.cont) {
                sum += (a - b) * (a - b);
            }
            let wrong_cat = at.cat != self.need_cat;
            let d = libm::sqrt(sum) + if wrong_cat { 10.0 * self.radius } else { 0.0 };
            let jnd = d / self.radius;
            Reading {
                percent: 100.0 / (1.0 + jnd * jnd * jnd * jnd),
                jnd,
                passed: jnd <= 1.0,
            }
        }

        fn eval(&mut self) -> impl FnMut(&[Patch]) -> Vec<Option<Reading>> + '_ {
            move |c: &[Patch]| c.iter().map(|p| Some(self.read(p))).collect()
        }
    }

    fn at(start: &Patch, addrs: &[&str], norm: &[f64]) -> Patch {
        let mut p = start.clone();
        for (a, u) in addrs.iter().zip(norm) {
            let spec = start.spec(&addr(a)).unwrap();
            set(&mut p, a, spec.from_norm(*u as f32));
        }
        p
    }

    fn solve(
        start: &Patch,
        addrs: &[&str],
        ball: &mut Ball,
        budget: usize,
        cfg: &ClosestCfg,
    ) -> Closest {
        let free: Vec<ParamAddr> = addrs.iter().map(|s| addr(s)).collect();
        let mut eval = ball.eval();
        run(start, &free, budget, cfg, &mut eval, &mut |_| true).unwrap()
    }

    #[test]
    fn the_nearest_pass_is_on_the_ball_towards_the_player() {
        // The nearest point of a ball to a point outside it lies on the segment between them, a radius short of the
        // target: the knob distance is |start - target| - radius, whatever the number of axes.
        for n in 1..=4 {
            let addrs = &AXES[..n];
            let init = Patch::init();
            let start_norm = vec![0.2; n];
            let player = at(&init, addrs, &start_norm);
            let target_norm: Vec<f64> = (0..n).map(|i| 0.55 + 0.05 * i as f64).collect();
            let mut ball = Ball::new(&player, addrs, &target_norm, 0.08);
            let from_target: f64 = libm::sqrt(
                start_norm
                    .iter()
                    .zip(&target_norm)
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum(),
            );
            let best = from_target - 0.08;
            let found = solve(&player, addrs, &mut ball, 600, &ClosestCfg::default());
            assert!(found.passed, "{n} axes: {found:?}");
            assert!(
                found.distance >= best - 1e-3,
                "{n} axes: cannot beat the optimum {best}: {}",
                found.distance
            );
            assert!(
                found.distance <= best + 0.03,
                "{n} axes: {} is far from the optimum {best}",
                found.distance
            );
            assert_eq!(found.deltas.len(), n);
            assert!(
                found.deltas.iter().all(|d| d.norm > 0.0),
                "{n} axes: toward the target"
            );
            assert_eq!(found.stop, ClosestStop::Converged, "{n} axes");
            assert!(found.renders <= 600 && found.renders == ball.calls);
            assert!(found.first_pass_renders.is_some_and(|r| r <= found.renders));
            assert!(found.start_percent < 90.0);
        }
    }

    #[test]
    fn a_passing_start_costs_one_render_and_is_returned_as_it_is() {
        let init = Patch::init();
        let player = at(&init, &AXES[..2], &[0.5, 0.5]);
        let mut ball = Ball::new(&player, &AXES[..2], &[0.52, 0.5], 0.08);
        let found = solve(&player, &AXES[..2], &mut ball, 100, &ClosestCfg::default());
        assert_eq!(found.stop, ClosestStop::StartPasses);
        assert!(found.passed);
        assert_eq!(found.renders, 1);
        assert_eq!(ball.calls, 1);
        assert_eq!(found.patch, player);
        assert_eq!(found.distance, 0.0);
        assert!(found.deltas.iter().all(|d| !d.moved() && d.from == d.to));
    }

    #[test]
    fn an_unreachable_round_returns_the_nearest_to_passing_and_uses_the_budget() {
        // The target needs `osc1.pw` far from where it is, and only the semitones are free: no pass exists.
        let init = Patch::init();
        let player = at(&init, &AXES[..3], &[0.3, 0.5, 0.2]);
        let mut ball = Ball::new(&player, &AXES[..3], &[0.6, 0.5, 0.9], 0.08);
        let free = [AXES[0]];
        let found = {
            let free_addrs: Vec<ParamAddr> = free.iter().map(|s| addr(s)).collect();
            let mut eval = |c: &[Patch]| c.iter().map(|p| Some(ball.read(p))).collect::<Vec<_>>();
            run(
                &player,
                &free_addrs,
                60,
                &ClosestCfg::default(),
                &mut eval,
                &mut |_| true,
            )
            .unwrap()
        };
        assert!(!found.passed);
        assert_eq!(found.stop, ClosestStop::Budget);
        assert_eq!(found.renders, 60, "the budget is hard and used");
        assert!(
            found.percent >= found.start_percent,
            "nearest to passing, not worse than the start"
        );
        assert!(found.first_pass_renders.is_none());
        // The semitone knob moved towards the target's.
        assert!(found.deltas[0].norm > 0.0);
    }

    #[test]
    fn the_budget_is_hard_even_for_one_render() {
        let init = Patch::init();
        let player = at(&init, &AXES[..2], &[0.1, 0.1]);
        for budget in [1, 2, 5, 13] {
            let mut ball = Ball::new(&player, &AXES[..2], &[0.8, 0.8], 0.08);
            let found = solve(
                &player,
                &AXES[..2],
                &mut ball,
                budget,
                &ClosestCfg::default(),
            );
            assert!(
                found.renders <= budget,
                "budget {budget}: {}",
                found.renders
            );
            assert_eq!(found.renders, ball.calls);
        }
    }

    #[test]
    fn a_callback_sees_progress_and_can_cancel() {
        let init = Patch::init();
        let player = at(&init, &AXES[..2], &[0.1, 0.1]);
        let mut ball = Ball::new(&player, &AXES[..2], &[0.8, 0.8], 0.08);
        let free: Vec<ParamAddr> = AXES[..2].iter().map(|s| addr(s)).collect();
        let mut seen: Vec<Progress> = Vec::new();
        let mut eval = ball.eval();
        let found = run(
            &player,
            &free,
            500,
            &ClosestCfg::default(),
            &mut eval,
            &mut |p| {
                seen.push(*p);
                seen.len() < 3
            },
        )
        .unwrap();
        assert_eq!(found.stop, ClosestStop::Cancelled);
        assert_eq!(seen.len(), 3);
        assert!(seen.windows(2).all(|w| w[0].renders < w[1].renders));
        assert!(seen.iter().all(|p| p.budget == 500));
        assert_eq!(found.renders, seen[2].renders);
    }

    #[test]
    fn the_same_inputs_give_the_same_answer_and_the_seed_is_an_input() {
        let init = Patch::init();
        let player = at(&init, &AXES[..3], &[0.2, 0.3, 0.2]);
        let go = |seed: u64, lambda: usize| {
            let mut ball = Ball::new(&player, &AXES[..3], &[0.7, 0.6, 0.8], 0.08);
            solve(
                &player,
                &AXES[..3],
                &mut ball,
                400,
                &ClosestCfg {
                    seed,
                    lambda,
                    ..ClosestCfg::default()
                },
            )
        };
        assert_eq!(go(1, 0), go(1, 0));
        assert_eq!(go(5, 8), go(5, 8));
        assert_ne!(go(1, 0).patch, go(2, 0).patch);
    }

    #[test]
    fn a_waveform_that_is_needed_is_found_and_one_that_is_not_stays() {
        // `osc1.wave` is categorical. The target is a saw (the init patch's) in the ball: the player at a sine must
        // flip it; the player at the saw must not.
        let init = Patch::init();
        let addrs = ["osc1.wave", "osc1.semi"];
        let saw = at(&init, &addrs[1..], &[0.3]);
        let mut sine = saw.clone();
        set(&mut sine, "osc1.wave", 0.0);

        let mut ball = Ball::new(&saw, &addrs, &[0.5], 0.08);
        let found = solve(&sine, &addrs, &mut ball, 400, &ClosestCfg::default());
        assert!(found.passed, "{found:?}");
        assert!(found.deltas[0].categorical && found.deltas[0].moved());
        assert_eq!(
            found.patch.value(&addr("osc1.wave")),
            saw.value(&addr("osc1.wave"))
        );

        let mut ball = Ball::new(&saw, &addrs, &[0.5], 0.08);
        let found = solve(&saw, &addrs, &mut ball, 400, &ClosestCfg::default());
        assert!(found.passed);
        assert!(!found.deltas[0].moved(), "the waveform was right and stays");
        assert!(found.deltas[1].moved());
    }

    #[test]
    fn only_categorical_parameters_free_ends_exhausted() {
        let init = Patch::init();
        let addrs = ["osc1.wave"];
        let mut sine = init.clone();
        set(&mut sine, "osc1.wave", 0.0);
        let mut ball = Ball::new(&init, &addrs, &[], 0.08);
        let found = solve(&sine, &addrs, &mut ball, 100, &ClosestCfg::default());
        assert!(found.passed);
        assert_eq!(found.stop, ClosestStop::Exhausted);
        assert!(
            found.renders <= 6,
            "one probe of the five waves: {}",
            found.renders
        );
    }

    #[test]
    fn bad_jobs_are_refused_with_a_code_and_a_hint() {
        let init = Patch::init();
        let mut ball = Ball::new(&init, &AXES[..1], &[0.5], 0.08);
        let cfg = ClosestCfg::default();
        let no_budget = {
            let free = [addr(AXES[0])];
            let mut eval = ball.eval();
            run(&init, &free, 0, &cfg, &mut eval, &mut |_| true).unwrap_err()
        };
        assert_eq!(no_budget.code(), "bad_search_cfg");
        let none = {
            let mut eval = ball.eval();
            run(&init, &[], 10, &cfg, &mut eval, &mut |_| true).unwrap_err()
        };
        assert_eq!(none.code(), "no_free_parameters");
        let unknown = {
            let mut eval = ball.eval();
            run(
                &init,
                &[addr("filter1.cutoff")],
                10,
                &cfg,
                &mut eval,
                &mut |_| true,
            )
            .unwrap_err()
        };
        assert_eq!(unknown.code(), "unknown_param");
        assert!(unknown.hint().contains("filter1.cutoff"));
        let bad = {
            let free = [addr(AXES[0])];
            let mut eval = ball.eval();
            let cfg = ClosestCfg {
                sigma: 0.0,
                ..ClosestCfg::default()
            };
            run(&init, &free, 10, &cfg, &mut eval, &mut |_| true).unwrap_err()
        };
        assert_eq!(bad.code(), "bad_closest_cfg");
        let unscorable = {
            let free = [addr(AXES[0])];
            let mut eval = |c: &[Patch]| vec![None; c.len()];
            run(&init, &free, 10, &cfg, &mut eval, &mut |_| true).unwrap_err()
        };
        assert_eq!(unscorable.code(), "start_unscorable");
        let miscount = {
            let free = [addr(AXES[0])];
            let mut eval = |_: &[Patch]| Vec::new();
            run(&init, &free, 10, &cfg, &mut eval, &mut |_| true).unwrap_err()
        };
        assert_eq!(miscount.code(), "evaluator_count");
    }

    #[test]
    fn a_candidate_that_cannot_be_scored_ranks_last_not_first() {
        // Every candidate but the player's fails to score: the answer is the player's own patch, not a failure.
        let init = Patch::init();
        let player = at(&init, &AXES[..2], &[0.2, 0.2]);
        let free: Vec<ParamAddr> = AXES[..2].iter().map(|s| addr(s)).collect();
        let mut first = true;
        let mut eval = |c: &[Patch]| {
            c.iter()
                .map(|_| {
                    if first {
                        first = false;
                        Some(Reading {
                            percent: 40.0,
                            jnd: 3.0,
                            passed: false,
                        })
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        let found = run(
            &player,
            &free,
            30,
            &ClosestCfg::default(),
            &mut eval,
            &mut |_| true,
        )
        .unwrap();
        assert!(!found.passed);
        assert_eq!(found.patch, player);
        assert_eq!(found.percent, 40.0);
    }
}
