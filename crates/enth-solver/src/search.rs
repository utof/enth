//! **Owner: lane MT** (match). Search for a patch that sounds like a target: a heuristic start, then CMA-ES within a
//! render budget. The engine of `enth match`, and the "CMA-ES floor" that golden challenges and the agent baseline
//! are measured against.
//!
//! # What it does
//!
//! 1. **Free parameters** ([`FreeSet`]): the addresses a match may move, with their bounds from `ParamSpec`. A
//!    `Float` is searched on its normalised axis; an `Enum`, a `Toggle` or a small `Int` (a waveform, a filter
//!    mode) is categorical, chosen per run.
//! 2. **Probe.** One batch evaluates the start patch and, for each categorical *choice* (every combination of the
//!    categorical values when there are at most [`MAX_PROBED_CHOICES`], else the start's and its one-flip
//!    neighbours), a *heuristic point*: the start with [`Heuristic`] suggestions applied (envelope times from the
//!    descriptors, pitch from the measured f0, cutoff from a harmonic model of the waveform and filter mode being
//!    tried). The choices are ranked by that distance. The spike guessed the waveform from the audio; the
//!    descriptors do not carry it, so the probe asks the metric instead (`docs/dsp/match.md`).
//! 3. **Runs.** CMA-ES ([`cmaes`](crate::cmaes)) on the continuous axes, once per top-ranked choice (at most
//!    [`SearchCfg::runs_per_cycle`]), from that choice's heuristic point with a first step as narrow as the
//!    heuristic is precise ([`Suggestion::spread`]). A run ends when it collapses, stalls or becomes
//!    ill-conditioned.
//! 4. **Expand.** When the runs are done, the best choice's one-flip neighbours that were not probed are evaluated
//!    at its best continuous point and the best two get a run: a coordinate descent over the categorical axes, for
//!    when there are too many combinations to probe at once.
//! 5. **Restart.** Then the two best choices get **IPOP** restarts, one cycle after another: a random start
//!    (keeping what the heuristic measured precisely) and double the population.
//! 6. **Budget.** [`SearchCfg::budget`] caps the renders, probe included, and the cap is hard: the last batch is cut
//!    short rather than overshooting.
//!
//! # Determinism and parallelism
//!
//! A search is a pure function of its inputs and [`SearchCfg::seed`]: the stream comes from `enth_dsp`'s `derive`
//! and `SplitMix64`, the maths from `libm`, every ordering from `total_cmp` with an index tie-break. The solver
//! spawns no threads (`AGENTS.md`): the [`Objective`] evaluates a batch however it likes and returns the distances
//! in input order, so a search is identical for any number of workers. Population size is a *setting*
//! ([`SearchCfg::lambda`]), not derived from the worker count, which would change the algorithm; pick a `lambda`
//! that is a multiple of your worker count to keep the pool busy.
//!
//! ```
//! use enth_patch::{Op, Patch, apply};
//! use enth_render::{Performance, RenderSpec};
//! use enth_solver::search::{FreeSet, MetricTarget, SearchCfg, search_metric};
//!
//! let perf = Performance::note(60, 0.8, 0.3, 0.6);
//! let spec = RenderSpec::default();
//! // The target: a sine an octave down, which the search has to find from the init patch (a saw).
//! let mut target = Patch::init();
//! apply(&mut target, Op::SetParam { addr: "osc1.wave".parse().unwrap(), value: 0.0 }).unwrap();
//! apply(&mut target, Op::SetParam { addr: "osc1.semi".parse().unwrap(), value: -12.0 }).unwrap();
//! let target = MetricTarget::from_patch(&target, &perf, &spec).unwrap();
//!
//! let start = Patch::init();
//! let free = FreeSet::new(&start, &["osc1.wave".parse().unwrap(), "osc1.semi".parse().unwrap()]).unwrap();
//! let cfg = SearchCfg { budget: 40, ..SearchCfg::default() };
//! let found = search_metric(&start, &free, &target, &cfg).unwrap();
//! assert!(found.renders <= 40);
//! assert!(found.distance <= found.start_distance);
//! ```

pub mod free;
pub mod objective;

use core::fmt;
use std::collections::VecDeque;

use enth_dsp::util::rng::Rng;
use enth_patch::Patch;

use crate::cmaes::{Cmaes, CmaesError, WORST_FITNESS, default_lambda};
use crate::heuristic::{Heuristic, PRECISE, Suggestion};

pub use free::{Axis, AxisKind, FreeSet, MAX_CATEGORIES, Point};
pub use objective::{FAILED, MetricTarget, MetricWorker, Objective};

/// The most categorical choices probed at once: every combination of the categorical values up to this many, else the
/// start's choice and its one-flip neighbours.
pub const MAX_PROBED_CHOICES: usize = 32;

/// The most categorical choices a search keeps (the probe's and the expansions').
const MAX_CHOICES: usize = 96;

/// Why a search could not start or continue.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SearchError {
    /// The free set is empty.
    NoFreeParameters,
    /// A free address names no parameter of the start patch.
    UnknownParam {
        /// The address as text.
        addr: String,
    },
    /// A setting is out of range.
    BadCfg(&'static str),
    /// The [`Objective`] returned a different number of distances than candidates.
    ObjectiveCount {
        /// Candidates asked.
        expected: usize,
        /// Distances returned.
        got: usize,
    },
    /// The optimiser refused its setup (a bug in this module, reported rather than panicked).
    Cmaes(CmaesError),
}

impl SearchError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            SearchError::NoFreeParameters => "no_free_parameters",
            SearchError::UnknownParam { .. } => "unknown_param",
            SearchError::BadCfg(_) => "bad_search_cfg",
            SearchError::ObjectiveCount { .. } => "objective_count",
            SearchError::Cmaes(_) => "optimiser_failed",
        }
    }

    /// What to do about it.
    #[must_use]
    pub fn hint(&self) -> String {
        match self {
            SearchError::NoFreeParameters => {
                "name at least one parameter to search, e.g. filter1.cutoff".to_owned()
            }
            SearchError::UnknownParam { addr } => format!(
                "{addr} is not a parameter of the starting patch; list them with `enth params`"
            ),
            SearchError::BadCfg(what) => format!("fix the setting: {what}"),
            SearchError::ObjectiveCount { .. } => {
                "an objective returns exactly one distance per candidate, in order".to_owned()
            }
            SearchError::Cmaes(_) => "this is a bug in the search; report it".to_owned(),
        }
    }
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchError::NoFreeParameters => f.write_str("no parameter is free to search"),
            SearchError::UnknownParam { addr } => {
                write!(f, "'{addr}' names no parameter of the starting patch")
            }
            SearchError::BadCfg(what) => write!(f, "bad search setting: {what}"),
            SearchError::ObjectiveCount { expected, got } => write!(
                f,
                "the objective returned {got} distances for {expected} candidates"
            ),
            SearchError::Cmaes(e) => write!(f, "the optimiser refused its setup: {e}"),
        }
    }
}

impl std::error::Error for SearchError {}

impl From<CmaesError> for SearchError {
    fn from(e: CmaesError) -> Self {
        SearchError::Cmaes(e)
    }
}

/// Settings of a search. [`Default`] is what `enth match` uses.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchCfg {
    /// The seed of every random choice (restart means, the optimisers' streams).
    pub seed: u64,
    /// The most renders (objective evaluations, the probe included). Hard.
    pub budget: usize,
    /// Population of the first run; `0` is Hansen's default for the number of continuous axes. A setting, never
    /// derived from a worker count (that would make results depend on it): use a multiple of the pool size to keep
    /// it busy. Doubles each cycle (IPOP).
    pub lambda: usize,
    /// Initial step (in the 0–1 normalised units) around a heuristic point.
    pub sigma_start: f64,
    /// Initial step of a random restart.
    pub sigma_random: f64,
    /// Stop as soon as the best distance is at or below this.
    pub stop_at: Option<f64>,
    /// How many categorical choices get a run after the probe (the best by probe distance); later rounds run two.
    pub runs_per_cycle: usize,
    /// Apply the [`Heuristic`] suggestions (when a heuristic is given). Off, every point starts from the start patch.
    pub use_heuristic: bool,
}

impl Default for SearchCfg {
    fn default() -> Self {
        SearchCfg {
            seed: 1,
            budget: 1500,
            lambda: 0,
            sigma_start: 0.15,
            sigma_random: 0.3,
            stop_at: None,
            runs_per_cycle: 4,
            use_heuristic: true,
        }
    }
}

impl SearchCfg {
    fn validate(&self) -> Result<(), SearchError> {
        if self.budget == 0 {
            return Err(SearchError::BadCfg("budget must be at least 1 render"));
        }
        if self.lambda != 0 && self.lambda < 4 {
            return Err(SearchError::BadCfg(
                "lambda must be 0 (default) or at least 4",
            ));
        }
        if !(self.sigma_start.is_finite()
            && self.sigma_start > 0.0
            && self.sigma_random.is_finite()
            && self.sigma_random > 0.0)
        {
            return Err(SearchError::BadCfg(
                "sigma_start and sigma_random must be positive",
            ));
        }
        if self.runs_per_cycle == 0 {
            return Err(SearchError::BadCfg("runs_per_cycle must be at least 1"));
        }
        Ok(())
    }
}

/// Which part of the search a batch belonged to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The probe: the start and the heuristic points.
    Probe,
    /// A generation of CMA-ES run `n` (0-based, counted over the whole search).
    Run(usize),
}

/// One batch of the search, after it was evaluated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceStep {
    /// Renders used so far, this batch included.
    pub renders: usize,
    /// What the batch was.
    pub stage: Stage,
    /// The best distance in this batch.
    pub batch_best: f64,
    /// The best distance so far.
    pub best: f64,
}

/// Why a search ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopCause {
    /// The render budget was used.
    Budget,
    /// The best distance reached [`SearchCfg::stop_at`].
    Goal,
    /// The caller's `after_batch` asked to stop (a wall-clock cap lives there).
    Cancelled,
    /// There was nothing more to try (no continuous axis to run CMA-ES on, and no better categorical choice).
    Exhausted,
}

/// What a search found.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchResult {
    /// The best patch found (the start with the free parameters set).
    pub best: Patch,
    /// Its distance.
    pub distance: f64,
    /// The start patch's own distance (the "before"); `NaN` if the budget ended before it was evaluated.
    pub start_distance: f64,
    /// The distance of the heuristic start for the start's categorical choice, if a heuristic changed anything.
    pub heuristic_distance: Option<f64>,
    /// What the heuristic suggested for the start's categorical choice (empty without a heuristic).
    pub suggestions: Vec<Suggestion>,
    /// Renders used (at most the budget).
    pub renders: usize,
    /// CMA-ES runs begun.
    pub runs: usize,
    /// Why it ended.
    pub stop: StopCause,
    /// One entry per evaluated batch.
    pub trace: Vec<TraceStep>,
}

/// One categorical choice under test.
#[derive(Clone, Debug)]
struct Choice {
    cat: Vec<usize>,
    /// Where its warm runs start on the continuous axes: the heuristic point, or the best point of the choice it was
    /// expanded from.
    mean: Vec<f64>,
    /// Per continuous axis, the scale of the first step around `mean` (1 = the default step; the heuristic narrows
    /// the axes it measured less precisely). CMA-ES runs on `u / scale`.
    scale: Vec<f64>,
    /// Per continuous axis, whether the heuristic measured it precisely enough to leave it alone: a pinned axis
    /// keeps its `mean` in every run (the pitch is a needle in the metric: see [`PRECISE`]).
    pinned: Vec<bool>,
    /// Its distance when probed (`NaN` before).
    probe: f64,
    /// The best distance any candidate of this choice reached, and where (on the continuous axes).
    best: f64,
    best_cont: Vec<f64>,
    /// Its neighbours have been probed.
    expanded: bool,
}

/// A CMA-ES run to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    /// From the choice's `mean`, with the default first step.
    Warm(usize),
    /// From a random point (keeping the precisely measured axes), with a larger step and population.
    Random(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Probe,
    Expand,
    Runs,
    Done,
}

/// The ask-tell state machine behind [`search`]. Drive it with [`Search::ask`] and [`Search::tell`] when the
/// caller wants to interleave searches, show progress or cap wall-clock time itself; otherwise use [`search`].
#[derive(Debug)]
pub struct Search {
    base: Patch,
    free: FreeSet,
    heuristic: Option<Heuristic>,
    cfg: SearchCfg,
    rng: Rng,
    /// Where the start patch sits on every axis.
    start_point: Point,
    n_cont: usize,
    phase: Phase,
    choices: Vec<Choice>,
    /// Runs still to do, in order.
    todo: VecDeque<Job>,
    /// Random-restart rounds begun.
    cycle: usize,
    runs: usize,
    es: Option<Cmaes>,
    run_choice: usize,
    /// The continuous axes (indices into the continuous list) the running CMA-ES moves: the choice's unpinned ones.
    run_axes: Vec<usize>,
    /// The scale per continuous axis the running CMA-ES uses (the choice's, but 1 for an axis whose pin is released).
    run_scale: Vec<f64>,
    /// The points of the batch `ask` returned, and whether each patch could be built.
    pending: Vec<(Point, bool)>,
    /// For a probe or expand batch: the choice each point belongs to (`None` for the start point).
    pending_choice: Vec<Option<usize>>,
    pending_full: bool,
    evals: usize,
    best_f: f64,
    best_point: Point,
    start_f: Option<f64>,
    heuristic_f: Option<f64>,
    suggestions: Vec<Suggestion>,
    trace: Vec<TraceStep>,
    stop: Option<StopCause>,
}

/// Every combination of the categorical values, the start's first, then in axis order; `None` if there are more
/// than `cap`.
fn all_assignments(free: &FreeSet, start: &[usize], cap: usize) -> Option<Vec<Vec<usize>>> {
    let sizes: Vec<usize> = free
        .categorical()
        .iter()
        .map(|&i| match &free.axes()[i].kind {
            AxisKind::Categorical { values } => values.len(),
            AxisKind::Continuous => 1,
        })
        .collect();
    let mut total = 1usize;
    for s in &sizes {
        total = total.checked_mul(*s)?;
        if total > cap {
            return None;
        }
    }
    let mut out: Vec<Vec<usize>> = Vec::with_capacity(total);
    for n in 0..total {
        let mut rest = n;
        let mut cat = vec![0; sizes.len()];
        for k in (0..sizes.len()).rev() {
            cat[k] = rest % sizes[k];
            rest /= sizes[k];
        }
        out.push(cat);
    }
    out.sort_by_key(|c| c != start);
    Some(out)
}

impl Search {
    /// Prepare a search from `start` over `free`, with an optional heuristic start.
    ///
    /// # Errors
    /// [`SearchError::BadCfg`] for a setting out of range, [`SearchError::UnknownParam`] if `free` names a parameter
    /// `start` lacks.
    pub fn new(
        start: &Patch,
        free: &FreeSet,
        heuristic: Option<&Heuristic>,
        cfg: &SearchCfg,
    ) -> Result<Search, SearchError> {
        cfg.validate()?;
        if let Some(bad) = free.axes().iter().find(|a| start.value(&a.addr).is_none()) {
            return Err(SearchError::UnknownParam {
                addr: bad.addr.to_string(),
            });
        }
        let n_cont = free.continuous().len();
        let start_point = free.point_of(start);
        let heuristic = heuristic.filter(|_| cfg.use_heuristic).cloned();
        Ok(Search {
            base: start.clone(),
            free: free.clone(),
            heuristic,
            cfg: cfg.clone(),
            rng: Rng::new(enth_dsp::util::rng::derive(cfg.seed, SEARCH_STREAM)),
            start_point: start_point.clone(),
            n_cont,
            phase: Phase::Probe,
            choices: Vec::new(),
            todo: VecDeque::new(),
            cycle: 0,
            runs: 0,
            es: None,
            run_choice: 0,
            run_axes: Vec::new(),
            run_scale: Vec::new(),
            pending: Vec::new(),
            pending_choice: Vec::new(),
            pending_full: false,
            evals: 0,
            best_f: f64::INFINITY,
            best_point: start_point,
            start_f: None,
            heuristic_f: None,
            suggestions: Vec::new(),
            trace: Vec::new(),
            stop: None,
        })
    }

    /// The renders left in the budget.
    fn remaining(&self) -> usize {
        self.cfg.budget.saturating_sub(self.evals)
    }

    /// Whether the search is over (`ask` returns nothing).
    #[must_use]
    pub fn finished(&self) -> bool {
        self.phase == Phase::Done || self.stop.is_some()
    }

    /// The heuristic point of a categorical choice: the start with the heuristic's suggestions (for a patch carrying
    /// that choice) applied on the normalised axes, the first-step scale of every axis, and the suggestions.
    fn heuristic_point(&self, cat: &[usize]) -> (Point, Vec<f64>, Vec<Suggestion>) {
        let mut point = Point {
            cont: self.start_point.cont.clone(),
            cat: cat.to_vec(),
        };
        let mut scale = vec![1.0; self.n_cont];
        let Some(h) = &self.heuristic else {
            return (point, scale, Vec::new());
        };
        let Some(with_choice) = self.free.apply(&self.base, &point) else {
            return (point, scale, Vec::new());
        };
        let suggestions = h.suggest(&with_choice, &self.free);
        for s in &suggestions {
            if let Some(k) = self
                .free
                .continuous()
                .iter()
                .position(|&i| self.free.axes()[i].addr == s.addr)
            {
                let spec = self.free.axes()[self.free.continuous()[k]].spec;
                point.cont[k] = f64::from(spec.to_norm(s.value));
                scale[k] = s.spread.clamp(MIN_SCALE, 1.0);
            }
        }
        (point, scale, suggestions)
    }

    /// The candidates to evaluate next, each a patch; pass their distances to [`Search::tell`] in the same order.
    /// Empty when the search is finished.
    pub fn ask(&mut self) -> Vec<Patch> {
        if self.finished() {
            return Vec::new();
        }
        let room = self.remaining();
        if room == 0 {
            self.stop = Some(StopCause::Budget);
            return Vec::new();
        }
        let mut points: Vec<(Point, Option<usize>)> = Vec::new();
        match self.phase {
            Phase::Probe => {
                let start_point = self.start_point.clone();
                points.push((start_point.clone(), None));
                let cats = all_assignments(&self.free, &start_point.cat, MAX_PROBED_CHOICES)
                    .unwrap_or_else(|| {
                        let mut cats = vec![start_point.cat.clone()];
                        cats.extend(self.free.neighbours(&start_point.cat));
                        cats.truncate(MAX_PROBED_CHOICES);
                        cats
                    });
                for (ci, cat) in cats.into_iter().enumerate() {
                    let (hp, scale, sug) = self.heuristic_point(&cat);
                    if ci == 0 {
                        self.suggestions = sug;
                    }
                    self.choices.push(Choice {
                        cat,
                        mean: hp.cont.clone(),
                        pinned: scale.iter().map(|s| *s < PRECISE).collect(),
                        scale,
                        probe: f64::NAN,
                        best: f64::INFINITY,
                        best_cont: hp.cont.clone(),
                        expanded: false,
                    });
                    // The incumbent's heuristic point is the start itself when the heuristic changed nothing.
                    if ci == 0 && hp == start_point {
                        continue;
                    }
                    points.push((hp, Some(ci)));
                }
                self.pending_full = true;
            }
            Phase::Expand => {
                // The new choices are the last ones added; each is probed at the point it will start from.
                for (ci, c) in self.choices.iter().enumerate() {
                    if c.probe.is_nan() {
                        points.push((
                            Point {
                                cont: c.mean.clone(),
                                cat: c.cat.clone(),
                            },
                            Some(ci),
                        ));
                    }
                }
                self.pending_full = true;
            }
            Phase::Runs => {
                if self.es.is_none() {
                    self.begin_run();
                }
                let Some(es) = self.es.as_mut() else {
                    self.phase = Phase::Done;
                    return Vec::new();
                };
                let choice = &self.choices[self.run_choice];
                let (cat, scale) = (choice.cat.clone(), self.run_scale.clone());
                let pinned_at = choice.mean.clone();
                let axes = self.run_axes.clone();
                let sampled = es.ask();
                self.pending_full = sampled.len() <= room;
                points.extend(sampled.into_iter().map(|v| {
                    // Pinned axes keep the heuristic's value; the others come from the optimiser, un-scaled.
                    let mut cont = pinned_at.clone();
                    for (&k, v) in axes.iter().zip(&v) {
                        cont[k] = (v * scale[k]).clamp(0.0, 1.0);
                    }
                    (
                        Point {
                            cont,
                            cat: cat.clone(),
                        },
                        None,
                    )
                }));
            }
            Phase::Done => return Vec::new(),
        }
        if points.len() > room {
            points.truncate(room);
            self.pending_full = false;
        }
        let mut patches = Vec::with_capacity(points.len());
        self.pending.clear();
        self.pending_choice.clear();
        for (point, choice) in points {
            if let Some(p) = self.free.apply(&self.base, &point) {
                patches.push(p);
                self.pending.push((point, true));
            } else {
                patches.push(self.base.clone());
                self.pending.push((point, false));
            }
            self.pending_choice.push(choice);
        }
        patches
    }

    /// The continuous axes of a choice that CMA-ES may move: the unpinned ones, or every one once the pins are
    /// released (a wrong measurement must not rule out the answer for good).
    fn movable_axes(&self, choice: usize) -> Vec<usize> {
        let released = self.cycle >= PIN_CYCLES;
        (0..self.n_cont)
            .filter(|&k| released || !self.choices[choice].pinned[k])
            .collect()
    }

    /// Start the next CMA-ES run of the schedule: the next queued job whose choice has an axis to move.
    fn begin_run(&mut self) {
        while let Some(job) = self.todo.pop_front() {
            let (Job::Warm(choice) | Job::Random(choice)) = job;
            let axes = self.movable_axes(choice);
            if axes.is_empty() {
                continue;
            }
            // CMA-ES runs on v = u / scale, so a narrow scale makes a narrow first step (and a box [0, 1 / scale]).
            // A released pin starts from the full step again.
            let released = self.cycle >= PIN_CYCLES;
            let scale: Vec<f64> = (0..self.n_cont)
                .map(|k| {
                    if released && self.choices[choice].pinned[k] {
                        1.0
                    } else {
                        self.choices[choice].scale[k]
                    }
                })
                .collect();
            let warm_mean = self.choices[choice].mean.clone();
            let (mean_u, sigma, lambda) = match job {
                Job::Warm(_) => (
                    warm_mean,
                    self.cfg.sigma_start,
                    self.lambda_for(axes.len(), 0),
                ),
                Job::Random(_) => {
                    // A random restart re-randomises what the descriptors left open. The draw is taken for every
                    // axis, so the stream does not depend on which are pinned.
                    let mean: Vec<f64> = (0..self.n_cont).map(|_| self.rng.unit_f64()).collect();
                    (
                        mean,
                        self.cfg.sigma_random,
                        self.lambda_for(axes.len(), self.cycle.min(MAX_DOUBLINGS)),
                    )
                }
            };
            let mean: Vec<f64> = axes.iter().map(|&k| mean_u[k] / scale[k]).collect();
            let upper: Vec<f64> = axes.iter().map(|&k| 1.0 / scale[k]).collect();
            let seed = self.rng.next_u64();
            match Cmaes::new(mean, sigma, lambda, vec![0.0; axes.len()], upper, seed) {
                Ok(es) => {
                    self.es = Some(es);
                    self.run_choice = choice;
                    self.run_axes = axes;
                    self.run_scale = scale;
                    self.runs += 1;
                }
                Err(_) => self.phase = Phase::Done,
            }
            return;
        }
        // Nothing left that can run: ask the scheduler (it expands, restarts or ends the search).
        self.schedule();
    }

    /// The population of a run over `dims` axes after `doublings` doublings: the setting, or Hansen's default.
    fn lambda_for(&self, dims: usize, doublings: usize) -> usize {
        let base = if self.cfg.lambda > 0 {
            self.cfg.lambda
        } else {
            default_lambda(dims)
        };
        base << doublings
    }

    /// The distances of the batch the last [`Search::ask`] returned, in the same order. A non-finite value counts as
    /// [`FAILED`]; a candidate that could not be built counts as failed whatever is passed.
    ///
    /// # Errors
    /// [`SearchError::ObjectiveCount`] if `distances` does not match the batch; nothing changes.
    pub fn tell(&mut self, distances: &[f64]) -> Result<(), SearchError> {
        if distances.len() != self.pending.len() || self.pending.is_empty() {
            return Err(SearchError::ObjectiveCount {
                expected: self.pending.len(),
                got: distances.len(),
            });
        }
        let stage = match self.phase {
            Phase::Probe | Phase::Expand => Stage::Probe,
            _ => Stage::Run(self.runs.saturating_sub(1)),
        };
        let fit: Vec<f64> = distances
            .iter()
            .zip(&self.pending)
            .map(|(d, (_, ok))| if *ok && d.is_finite() { *d } else { FAILED })
            .collect();
        let mut batch_best = f64::INFINITY;
        for ((point, _), &f) in self.pending.iter().zip(&fit) {
            self.evals += 1;
            batch_best = batch_best.min(f);
            if f < self.best_f {
                self.best_f = f;
                self.best_point = point.clone();
            }
        }
        self.trace.push(TraceStep {
            renders: self.evals,
            stage,
            batch_best,
            best: self.best_f,
        });
        match self.phase {
            Phase::Probe => self.finish_probe(&fit),
            Phase::Expand => self.finish_expand(&fit),
            Phase::Runs => {
                let c = self.run_choice;
                for ((point, _), &f) in self.pending.iter().zip(&fit) {
                    if f < self.choices[c].best {
                        self.choices[c].best = f;
                        self.choices[c].best_cont.clone_from(&point.cont);
                    }
                }
                if self.pending_full {
                    let es_fit: Vec<f64> = fit
                        .iter()
                        .map(|f| if *f >= FAILED { WORST_FITNESS } else { *f })
                        .collect();
                    if let Some(es) = self.es.as_mut() {
                        es.tell(&es_fit)?;
                        if es.stop_reason().is_some() {
                            self.es = None;
                            self.schedule();
                        }
                    }
                }
            }
            Phase::Done => {}
        }
        if self.cfg.stop_at.is_some_and(|goal| self.best_f <= goal) {
            self.stop = Some(StopCause::Goal);
        } else if self.evals >= self.cfg.budget {
            self.stop = Some(StopCause::Budget);
        } else if self.phase == Phase::Done {
            self.stop = Some(StopCause::Exhausted);
        }
        Ok(())
    }

    /// Record a probe or expand batch against its choices.
    fn record_probe(&mut self, fit: &[f64]) {
        for (k, choice) in self.pending_choice.iter().enumerate() {
            if let Some(ci) = *choice {
                let c = &mut self.choices[ci];
                c.probe = fit[k];
                if fit[k] < c.best {
                    c.best = fit[k];
                    c.best_cont.clone_from(&self.pending[k].0.cont);
                }
            }
        }
    }

    /// The choices ranked by `key`, best first, ties by index.
    fn ranked(&self, key: impl Fn(&Choice) -> f64, among: impl Fn(usize) -> bool) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.choices.len()).filter(|&i| among(i)).collect();
        order.sort_by(|&a, &b| {
            let k = |i: usize| {
                let v = key(&self.choices[i]);
                if v.is_nan() { f64::INFINITY } else { v }
            };
            k(a).total_cmp(&k(b)).then(a.cmp(&b))
        });
        order
    }

    fn finish_probe(&mut self, fit: &[f64]) {
        self.start_f = fit.first().copied();
        let incumbent_probed = self.pending_choice.contains(&Some(0));
        self.record_probe(fit);
        // The incumbent's heuristic point was the start itself when no separate point was evaluated.
        if incumbent_probed {
            self.heuristic_f = Some(self.choices[0].probe);
        } else if let Some(c) = self.choices.first_mut() {
            c.probe = fit[0];
            if fit[0] < c.best {
                c.best = fit[0];
                c.best_cont.clone_from(&self.pending[0].0.cont);
            }
        }
        // A probe cut short by the budget leaves later choices unranked: they sort last.
        let top = self.ranked(|c| c.probe, |_| true);
        self.todo = top
            .into_iter()
            .take(self.cfg.runs_per_cycle)
            .map(Job::Warm)
            .collect();
        self.phase = Phase::Runs;
        if self.n_cont == 0 {
            self.todo.clear();
        }
        self.es = None;
        self.schedule_if_idle();
    }

    fn finish_expand(&mut self, fit: &[f64]) {
        self.record_probe(fit);
        let fresh: Vec<usize> = self.pending_choice.iter().filter_map(|c| *c).collect();
        let mut order = fresh;
        order.sort_by(|&a, &b| {
            self.choices[a]
                .probe
                .total_cmp(&self.choices[b].probe)
                .then(a.cmp(&b))
        });
        self.todo = order.into_iter().take(2).map(Job::Warm).collect();
        if self.n_cont == 0 {
            self.todo.clear();
        }
        self.phase = Phase::Runs;
        self.es = None;
        self.schedule_if_idle();
    }

    /// If no run is queued, decide what comes next: expand the best choice, or restart.
    fn schedule_if_idle(&mut self) {
        if self.todo.is_empty() {
            self.schedule();
        }
    }

    /// A run ended: pick the next piece of work. Runs queued first; when none are left, the best choice's neighbours
    /// are probed once (a coordinate descent over the categorical axes), and then the two best choices restart.
    fn schedule(&mut self) {
        if !self.todo.is_empty() {
            self.phase = Phase::Runs;
            return;
        }
        let Some(best) = self.ranked(|c| c.best, |_| true).first().copied() else {
            self.phase = Phase::Done;
            return;
        };
        if !self.choices[best].expanded {
            self.choices[best].expanded = true;
            let (mean, cat) = (
                self.choices[best].best_cont.clone(),
                self.choices[best].cat.clone(),
            );
            let mut added = false;
            for n in self.free.neighbours(&cat) {
                if self.choices.iter().any(|c| c.cat == n) || self.choices.len() >= MAX_CHOICES {
                    continue;
                }
                let (_, scale, _) = self.heuristic_point(&n);
                self.choices.push(Choice {
                    cat: n,
                    mean: mean.clone(),
                    pinned: scale.iter().map(|s| *s < PRECISE).collect(),
                    scale,
                    probe: f64::NAN,
                    best: f64::INFINITY,
                    best_cont: mean.clone(),
                    expanded: false,
                });
                added = true;
            }
            if added {
                self.phase = Phase::Expand;
                return;
            }
            return self.schedule();
        }
        if self.n_cont == 0 {
            self.phase = Phase::Done;
            return;
        }
        self.cycle += 1;
        self.todo = self
            .ranked(|c| c.best, |_| true)
            .into_iter()
            .take(2)
            .map(Job::Random)
            .collect();
        self.phase = Phase::Runs;
    }

    /// Stop the search now (the caller's cap).
    pub fn cancel(&mut self) {
        if self.stop.is_none() {
            self.stop = Some(StopCause::Cancelled);
        }
    }

    /// The last evaluated batch.
    #[must_use]
    pub fn last_step(&self) -> Option<&TraceStep> {
        self.trace.last()
    }

    /// The best distance so far.
    #[must_use]
    pub fn best_distance(&self) -> f64 {
        self.best_f
    }

    /// Renders used so far.
    #[must_use]
    pub fn renders(&self) -> usize {
        self.evals
    }

    /// What has been found so far.
    #[must_use]
    pub fn result(&self) -> SearchResult {
        let best = self
            .free
            .apply(&self.base, &self.best_point)
            .unwrap_or_else(|| self.base.clone());
        SearchResult {
            best,
            distance: self.best_f,
            start_distance: self.start_f.unwrap_or(f64::NAN),
            heuristic_distance: self.heuristic_f,
            suggestions: self.suggestions.clone(),
            renders: self.evals,
            runs: self.runs,
            stop: self.stop.unwrap_or(StopCause::Budget),
            trace: self.trace.clone(),
        }
    }
}

/// The stream number the search derives its generator from, so a seed shared with other parts of a caller does not
/// reuse their stream.
const SEARCH_STREAM: u64 = 0x4D41_5443_4853_4541; // "MATCHSEA"

/// The random-restart cycles that keep the precisely measured axes pinned at the heuristic's value; later cycles move
/// every axis again, so a wrong measurement costs renders and not the answer.
const PIN_CYCLES: usize = 2;

/// The narrowest scale a suggestion may ask for: a floor so a step never underflows the optimiser's tolerances.
const MIN_SCALE: f64 = 1e-3;

/// The most times the population doubles (`lambda << 4` = 16×): beyond it a budget of thousands of renders never
/// gets to run.
const MAX_DOUBLINGS: usize = 4;

/// Search for a patch close to a target: `objective` scores batches of candidates, `heuristic` (the target's
/// descriptors) sets the start. See the [module docs](self).
///
/// # Errors
/// As [`Search::new`] and [`Search::tell`]: a bad setting, an unknown parameter, an objective that returns the wrong
/// number of distances.
pub fn search(
    start: &Patch,
    free: &FreeSet,
    heuristic: Option<&Heuristic>,
    objective: &mut dyn Objective,
    cfg: &SearchCfg,
) -> Result<SearchResult, SearchError> {
    search_with(start, free, heuristic, objective, cfg, &mut |_| true)
}

/// [`search`] with a callback after every batch; return `false` from it to stop ([`StopCause::Cancelled`]). The
/// solver reads no clock, so a wall-clock cap lives in the caller's callback.
///
/// # Errors
/// As [`search`].
pub fn search_with(
    start: &Patch,
    free: &FreeSet,
    heuristic: Option<&Heuristic>,
    objective: &mut dyn Objective,
    cfg: &SearchCfg,
    after_batch: &mut dyn FnMut(&TraceStep) -> bool,
) -> Result<SearchResult, SearchError> {
    let mut s = Search::new(start, free, heuristic, cfg)?;
    loop {
        let candidates = s.ask();
        if candidates.is_empty() {
            break;
        }
        let distances = objective.distances(&candidates);
        s.tell(&distances)?;
        if let Some(step) = s.last_step().copied()
            && !s.finished()
            && !after_batch(&step)
        {
            s.cancel();
        }
    }
    Ok(s.result())
}

/// [`search`] against a [`MetricTarget`], on one thread: metric v1 as the distance and the target's descriptors for
/// the heuristic start. For a pool, build one [`MetricWorker`] per thread from the same target and implement
/// [`Objective`] over them (see [`objective`]).
///
/// # Errors
/// As [`search`].
pub fn search_metric(
    start: &Patch,
    free: &FreeSet,
    target: &MetricTarget,
    cfg: &SearchCfg,
) -> Result<SearchResult, SearchError> {
    let heuristic = target
        .descriptors()
        .map(|d| Heuristic::new(d, target.performance()));
    let mut worker = target.worker();
    search(start, free, heuristic.as_ref(), &mut worker, cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_analysis::Descriptors;
    use enth_patch::{
        ModuleId, ModuleInstance, ModuleKind, Op, ParamAddr, Position, Section, apply,
    };
    use enth_render::Performance;

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

    /// The oscillator axes of the synthetic searches: two categorical (wave, sync), three continuous.
    const SMALL: [&str; 5] = [
        "osc1.wave",
        "osc1.sync",
        "osc1.semi",
        "osc1.fine",
        "osc1.pw",
    ];

    /// Five categorical axes' worth of choices (5 × 2 × 5 × 3 = 150 combinations, more than are probed at once) and
    /// the three continuous ones.
    const MANY: [&str; 7] = [
        "osc1.wave",
        "osc1.sync",
        "osc1.table",
        "osc1.phase_mode",
        "osc1.semi",
        "osc1.fine",
        "osc1.pw",
    ];

    fn free_of(start: &Patch, addrs: &[&str]) -> FreeSet {
        let a: Vec<ParamAddr> = addrs.iter().map(|s| addr(s)).collect();
        FreeSet::new(start, &a).unwrap()
    }

    /// A hidden target patch, `flips` away from the init patch in the categorical axes and moved on three
    /// continuous ones.
    fn hidden_target(many: bool) -> Patch {
        let mut t = Patch::init();
        set(&mut t, "osc1.wave", 3.0);
        set(&mut t, "osc1.sync", 1.0);
        if many {
            set(&mut t, "osc1.table", 3.0);
        }
        set(&mut t, "osc1.semi", -7.0);
        set(&mut t, "osc1.fine", 33.0);
        set(&mut t, "osc1.pw", 70.0);
        t
    }

    /// The synthetic landscape: one unit per categorical axis that differs from the target, plus the Euclidean
    /// distance on the normalised continuous axes. Counts its calls.
    struct Landscape {
        free: FreeSet,
        target: Point,
        calls: usize,
    }

    impl Landscape {
        fn new(free: &FreeSet, target: &Patch) -> Landscape {
            Landscape {
                free: free.clone(),
                target: free.point_of(target),
                calls: 0,
            }
        }

        fn at(&self, p: &Patch) -> f64 {
            let pt = self.free.point_of(p);
            let cat = pt
                .cat
                .iter()
                .zip(&self.target.cat)
                .filter(|(a, b)| a != b)
                .count();
            let cont: f64 = pt
                .cont
                .iter()
                .zip(&self.target.cont)
                .map(|(a, b)| (a - b) * (a - b))
                .sum();
            f64::from(u32::try_from(cat).unwrap()) + libm::sqrt(cont)
        }
    }

    impl Objective for Landscape {
        fn distances(&mut self, candidates: &[Patch]) -> Vec<f64> {
            self.calls += candidates.len();
            candidates.iter().map(|p| self.at(p)).collect()
        }
    }

    #[test]
    fn finds_a_target_two_categorical_flips_away() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        let mut land = Landscape::new(&free, &hidden_target(false));
        let cfg = SearchCfg {
            budget: 800,
            stop_at: Some(0.02),
            ..SearchCfg::default()
        };
        let found = search(&start, &free, None, &mut land, &cfg).unwrap();
        assert_eq!(found.stop, StopCause::Goal, "{found:?}");
        assert!(found.distance <= 0.02);
        assert_eq!(found.best.value(&addr("osc1.wave")), Some(3.0));
        assert_eq!(found.best.value(&addr("osc1.sync")), Some(1.0));
        assert!(found.start_distance > 2.0, "two flips and a move away");
        assert_eq!(found.renders, land.calls);
        // The probe took every one of the ten combinations at once: the start and nine others.
        assert_eq!(found.trace[0].stage, Stage::Probe);
        assert_eq!(found.trace[0].renders, 10);
    }

    #[test]
    fn walks_the_categorical_axes_when_there_are_too_many_combinations() {
        let start = Patch::init();
        let free = free_of(&start, &MANY);
        let mut land = Landscape::new(&free, &hidden_target(true));
        let cfg = SearchCfg {
            budget: 3000,
            stop_at: Some(0.02),
            ..SearchCfg::default()
        };
        let found = search(&start, &free, None, &mut land, &cfg).unwrap();
        assert_eq!(found.stop, StopCause::Goal, "{found:?}");
        for (a, v) in [("osc1.wave", 3.0), ("osc1.sync", 1.0), ("osc1.table", 3.0)] {
            assert_eq!(found.best.value(&addr(a)), Some(v), "{a}");
        }
        // The first batch is the start and its one-flip neighbours, not the 150 combinations.
        assert!(
            found.trace[0].renders <= MAX_PROBED_CHOICES,
            "{:?}",
            found.trace[0]
        );
    }

    #[test]
    fn the_budget_is_a_hard_cap_for_any_size() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        for budget in [1usize, 2, 5, 7, 10, 11, 13, 29, 100] {
            let mut land = Landscape::new(&free, &hidden_target(false));
            let cfg = SearchCfg {
                budget,
                ..SearchCfg::default()
            };
            let found = search(&start, &free, None, &mut land, &cfg).unwrap();
            assert!(
                found.renders <= budget,
                "budget {budget}: {}",
                found.renders
            );
            assert_eq!(found.renders, land.calls, "budget {budget}");
            assert_eq!(found.renders, budget, "a search this hard uses the budget");
            assert_eq!(found.stop, StopCause::Budget);
            assert!(found.start_distance.is_finite());
            assert!(found.distance <= found.start_distance);
            assert_eq!(found.trace.last().unwrap().renders, budget);
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_search_and_another_seed_a_different_one() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        let run = |seed: u64| {
            let mut land = Landscape::new(&free, &hidden_target(false));
            let cfg = SearchCfg {
                seed,
                budget: 300,
                ..SearchCfg::default()
            };
            search(&start, &free, None, &mut land, &cfg).unwrap()
        };
        let (a, b, c) = (run(5), run(5), run(6));
        assert_eq!(a, b, "same seed, same everything");
        assert_ne!(a.trace, c.trace, "another seed walks another path");
    }

    #[test]
    fn a_goal_stops_and_the_callback_cancels() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        // A goal above the start's own distance is met by the first batch.
        let mut land = Landscape::new(&free, &hidden_target(false));
        let cfg = SearchCfg {
            stop_at: Some(100.0),
            ..SearchCfg::default()
        };
        let found = search(&start, &free, None, &mut land, &cfg).unwrap();
        assert_eq!(found.stop, StopCause::Goal);
        assert_eq!(found.trace.len(), 1);

        let mut land = Landscape::new(&free, &hidden_target(false));
        let mut seen = Vec::new();
        let found = search_with(
            &start,
            &free,
            None,
            &mut land,
            &SearchCfg::default(),
            &mut |step| {
                seen.push(step.renders);
                seen.len() < 3
            },
        )
        .unwrap();
        assert_eq!(found.stop, StopCause::Cancelled);
        assert_eq!(found.trace.len(), 3);
        assert_eq!(seen.len(), 3);
        assert_eq!(found.renders, land.calls);
    }

    /// An objective that breaks the contract: no distances at all.
    struct Empty;

    impl Objective for Empty {
        fn distances(&mut self, _: &[Patch]) -> Vec<f64> {
            Vec::new()
        }
    }

    #[test]
    fn a_wrong_number_of_distances_is_an_error_and_changes_nothing() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        let err = search(&start, &free, None, &mut Empty, &SearchCfg::default()).unwrap_err();
        assert_eq!(
            err,
            SearchError::ObjectiveCount {
                expected: 10,
                got: 0
            }
        );
        assert_eq!(err.code(), "objective_count");

        // The driver can recover: the batch is still owed.
        let mut s = Search::new(&start, &free, None, &SearchCfg::default()).unwrap();
        let batch = s.ask();
        assert_eq!(s.tell(&[1.0]).unwrap_err().code(), "objective_count");
        assert_eq!(s.renders(), 0);
        s.tell(&vec![1.0; batch.len()]).unwrap();
        assert_eq!(s.renders(), batch.len());
    }

    #[test]
    fn non_finite_distances_count_as_failures() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        let mut s = Search::new(&start, &free, None, &SearchCfg::default()).unwrap();
        let batch = s.ask();
        let mut d = vec![f64::NAN; batch.len()];
        d[3] = 2.0;
        s.tell(&d).unwrap();
        assert_eq!(s.best_distance(), 2.0);
        assert_eq!(s.last_step().unwrap().batch_best, 2.0);
    }

    #[test]
    fn with_nothing_continuous_the_search_ends_when_the_choices_are_ranked() {
        let start = Patch::init();
        let free = free_of(&start, &["osc1.wave", "osc1.sync"]);
        let mut land = Landscape::new(&free, &hidden_target(false));
        let found = search(&start, &free, None, &mut land, &SearchCfg::default()).unwrap();
        assert_eq!(found.stop, StopCause::Exhausted);
        assert_eq!(found.distance, 0.0);
        assert_eq!(found.renders, 10, "one probe of every combination");
        assert_eq!(found.runs, 0);
        assert_eq!(found.best.value(&addr("osc1.wave")), Some(3.0));
        assert_eq!(found.best.value(&addr("osc1.sync")), Some(1.0));
    }

    #[test]
    fn bad_settings_and_foreign_free_sets_are_errors() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        let bad = |f: fn(&mut SearchCfg)| {
            let mut cfg = SearchCfg::default();
            f(&mut cfg);
            Search::new(&start, &free, None, &cfg).unwrap_err()
        };
        for e in [
            bad(|c| c.budget = 0),
            bad(|c| c.lambda = 3),
            bad(|c| c.sigma_start = 0.0),
            bad(|c| c.sigma_random = f64::NAN),
            bad(|c| c.runs_per_cycle = 0),
        ] {
            assert_eq!(e.code(), "bad_search_cfg", "{e}");
            assert_ne!(e.hint(), "");
        }

        // A free set made for a patch with a filter, run on one without.
        let mut with_filter = Patch::init();
        apply(
            &mut with_filter,
            Op::AddModule {
                at: Position {
                    section: Section::VoiceFx,
                    index: 0,
                },
                module: ModuleInstance::new(ModuleId::new("filter1").unwrap(), ModuleKind::Svf),
            },
        )
        .unwrap();
        let foreign = free_of(&with_filter, &["filter1.cutoff"]);
        let e = Search::new(&start, &foreign, None, &SearchCfg::default()).unwrap_err();
        assert_eq!(
            e,
            SearchError::UnknownParam {
                addr: "filter1.cutoff".to_owned()
            }
        );
    }

    /// The heuristic of a target whose only measured descriptor is a pitch of `semitones` from the played C4.
    fn pitch_heuristic(semitones: f64) -> Heuristic {
        use enth_analysis::descriptors::ModulationDesc;
        use enth_analysis::loudness::Level;
        use enth_analysis::pitch::Pitch;
        let c4 = 440.0 * libm::exp2((60.0 - 69.0) / 12.0);
        let desc = Descriptors {
            dur_s: 1.0,
            silent: false,
            pitch: Some(Pitch {
                f0_hz: c4 * libm::exp2(semitones / 12.0),
                note: "C3".to_owned(),
                cents_dev: 0.0,
                stability: 1.0,
                voiced: 1.0,
            }),
            tonal: None,
            envelope: None,
            spectrum: None,
            noise_ratio: None,
            detune: None,
            modulation: ModulationDesc::default(),
            loudness: Level {
                lufs_i: Some(-23.0),
                peak_dbfs: -6.0,
                true_peak_dbtp: -6.0,
                clipped_pct: 0.0,
                clipping: false,
            },
        };
        Heuristic::new(&desc, &Performance::note(60, 0.8, 0.5, 1.0))
    }

    /// The target of the pitch tests: `semi` and no fine detune, a pulse with hard sync at width 70.
    fn pitched_target(semi: f32) -> Patch {
        let mut t = Patch::init();
        set(&mut t, "osc1.wave", 3.0);
        set(&mut t, "osc1.sync", 1.0);
        set(&mut t, "osc1.semi", semi);
        set(&mut t, "osc1.pw", 70.0);
        t
    }

    #[test]
    fn a_pitch_measured_to_a_cent_is_kept_while_the_rest_is_searched() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        let target = pitched_target(-12.0);
        // A needle on the pitch, like the metric's (a tenth of a semitone off costs as much as a wrong waveform):
        // the rest of the landscape is smooth, and does not point at the semitone.
        let semi_axis = free
            .continuous()
            .iter()
            .position(|&i| free.axes()[i].addr == addr("osc1.semi"))
            .unwrap();
        let mut needle = |p: &Patch| {
            let mut pt = free.point_of(p);
            let mut t = free.point_of(&target);
            pt.cont[semi_axis] = 0.0;
            t.cont[semi_axis] = 0.0;
            let sum: f64 = pt
                .cont
                .iter()
                .zip(&t.cont)
                .map(|(a, b)| (a - b) * (a - b))
                .sum();
            let wrong_pitch = (p.value(&addr("osc1.semi")).unwrap() + 12.0).abs() > 0.01;
            let cat = free.point_of(p).cat != free.point_of(&target).cat;
            libm::sqrt(sum) + if wrong_pitch { 0.07 } else { 0.0 } + if cat { 1.0 } else { 0.0 }
        };
        let heuristic = pitch_heuristic(-12.0);
        let cfg = SearchCfg {
            budget: 3000,
            stop_at: Some(0.01),
            ..SearchCfg::default()
        };
        let found = search(&start, &free, Some(&heuristic), &mut needle, &cfg).unwrap();
        assert_eq!(found.stop, StopCause::Goal, "{found:?}");
        let semi = found.best.value(&addr("osc1.semi")).unwrap();
        assert!((semi + 12.0).abs() < 0.01, "{semi}");
        assert_eq!(
            found.suggestions.len(),
            2,
            "pitch gives semi and fine: {:?}",
            found.suggestions
        );
        assert!(found.suggestions.iter().all(|s| s.spread < PRECISE));
    }

    #[test]
    fn a_wrong_measurement_is_released_after_the_pinned_cycles() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        // The descriptors say -5 semitones; the target is +7. A smooth landscape: once the pin is released CMA-ES
        // walks to it.
        let mut land = Landscape::new(&free, &pitched_target(7.0));
        let heuristic = pitch_heuristic(-5.0);
        let cfg = SearchCfg {
            budget: 20_000,
            stop_at: Some(0.03),
            ..SearchCfg::default()
        };
        let found = search(&start, &free, Some(&heuristic), &mut land, &cfg).unwrap();
        assert_eq!(found.stop, StopCause::Goal, "{found:?}");
        let semi = found.best.value(&addr("osc1.semi")).unwrap();
        assert!((semi - 7.0).abs() < 3.0, "{semi}");
        assert!(found.runs > 1, "the pinned runs came first: {}", found.runs);
    }

    #[test]
    fn a_closure_is_an_objective_and_the_result_applies_exactly() {
        let start = Patch::init();
        let free = free_of(&start, &SMALL);
        let land = Landscape::new(&free, &hidden_target(false));
        let mut f = |p: &Patch| land.at(p);
        let cfg = SearchCfg {
            budget: 60,
            ..SearchCfg::default()
        };
        let found = search(&start, &free, None, &mut f, &cfg).unwrap();
        // The reported patch is the start with only free parameters set, and its distance is what was reported.
        assert_eq!(land.at(&found.best), found.distance);
        let mut untouched = found.best.clone();
        for a in SMALL {
            set(&mut untouched, a, start.value(&addr(a)).unwrap());
        }
        assert_eq!(untouched, start);
    }
}
