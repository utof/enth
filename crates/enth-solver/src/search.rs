//! **Owner: lane MT** (match). Search for a patch that sounds like a target: a heuristic start, then CMA-ES within a
//! render budget. The engine of `enth match`, and the "CMA-ES floor" that golden challenges and the agent baseline
//! are measured against.
//!
//! # What it does
//!
//! 1. **Free parameters** ([`FreeSet`]): the addresses a match may move, with their bounds from `ParamSpec`. A
//!    `Float` is searched on its normalised axis; an `Enum`, a `Toggle` or a small `Int` (a waveform, a filter
//!    mode) is categorical, chosen per run.
//! 2. **Probe.** One batch evaluates the start patch and, for the start's categorical choice and each one-flip
//!    neighbour of it, a *heuristic point*: the start with [`Heuristic`] suggestions applied (envelope times from the
//!    descriptors, pitch from the measured f0, cutoff from a harmonic model of the waveform being tried). The
//!    choices are ranked by that distance. The spike guessed the waveform from the audio; the descriptors do not
//!    carry it, so the probe asks the metric instead (`docs/dsp/match.md`).
//! 3. **Runs.** CMA-ES ([`cmaes`](crate::cmaes)) on the continuous axes, once per top-ranked categorical choice
//!    (at most [`SearchCfg::runs_per_cycle`]), from that choice's heuristic point with a small step. When a run
//!    collapses, stalls or becomes ill-conditioned, the next one starts. After a cycle the choices are re-ranked by
//!    the best distance each reached and the two best get **IPOP** restarts: a random start and double the
//!    population, every cycle.
//! 4. **Budget.** [`SearchCfg::budget`] caps the renders, probe included, and the cap is hard: the last batch is cut
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

use enth_dsp::util::rng::Rng;
use enth_patch::Patch;

use crate::cmaes::{Cmaes, CmaesError, StopReason, WORST_FITNESS, default_lambda};
use crate::heuristic::{Heuristic, PRECISE, Suggestion};

pub use free::{Axis, AxisKind, FreeSet, MAX_CATEGORIES, Point};
pub use objective::{FAILED, MetricTarget, MetricWorker, Objective};

/// The most categorical choices probed (the start's and its one-flip neighbours).
pub const MAX_PROBED_CHOICES: usize = 24;

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
    /// How many categorical choices get a run in the first cycle (the best by probe distance); later cycles run two.
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
            return Err(SearchError::BadCfg("lambda must be 0 (default) or at least 4"));
        }
        if !(self.sigma_start.is_finite()
            && self.sigma_start > 0.0
            && self.sigma_random.is_finite()
            && self.sigma_random > 0.0)
        {
            return Err(SearchError::BadCfg("sigma_start and sigma_random must be positive"));
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
    /// There was nothing more to try (no continuous axis to run CMA-ES on, and the probe is done).
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
    /// The continuous start of its runs in the first cycle: the heuristic point.
    mean: Vec<f64>,
    /// Per continuous axis, the scale of the first step around `mean` (1 = the default step; the heuristic narrows
    /// the axes it measured precisely). CMA-ES runs on `u / scale`.
    scale: Vec<f64>,
    probe: f64,
    best: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Probe,
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
    lambda0: usize,
    n_cont: usize,
    phase: Phase,
    choices: Vec<Choice>,
    order: Vec<usize>,
    cycle: usize,
    pos: usize,
    runs: usize,
    es: Option<Cmaes>,
    run_choice: usize,
    /// The points of the batch `ask` returned, and whether each patch could be built.
    pending: Vec<(Point, bool)>,
    /// For a probe batch: the choice each point belongs to (`None` for the start point).
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
        if free.axes().iter().any(|a| start.value(&a.addr).is_none()) {
            let bad = free.axes().iter().find(|a| start.value(&a.addr).is_none());
            return Err(SearchError::UnknownParam {
                addr: bad.map(|a| a.addr.to_string()).unwrap_or_default(),
            });
        }
        let n_cont = free.continuous().len();
        let lambda0 = if cfg.lambda > 0 {
            cfg.lambda
        } else {
            default_lambda(n_cont.max(1))
        };
        let start_point = free.point_of(start);
        let heuristic = heuristic.filter(|_| cfg.use_heuristic).cloned();
        Ok(Search {
            base: start.clone(),
            free: free.clone(),
            heuristic,
            cfg: cfg.clone(),
            rng: Rng::new(enth_dsp::util::rng::derive(cfg.seed, SEARCH_STREAM)),
            start_point: start_point.clone(),
            lambda0,
            n_cont,
            phase: Phase::Probe,
            choices: Vec::new(),
            order: Vec::new(),
            cycle: 0,
            pos: 0,
            runs: 0,
            es: None,
            run_choice: 0,
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
    /// that choice) applied on the normalised axes.
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
                let mut cats = vec![start_point.cat.clone()];
                cats.extend(self.free.neighbours(&start_point.cat));
                cats.truncate(MAX_PROBED_CHOICES);
                for (ci, cat) in cats.into_iter().enumerate() {
                    let (hp, scale, sug) = self.heuristic_point(&cat);
                    if ci == 0 {
                        self.suggestions = sug;
                    }
                    self.choices.push(Choice {
                        cat,
                        mean: hp.cont.clone(),
                        scale,
                        probe: f64::NAN,
                        best: f64::INFINITY,
                    });
                    // The incumbent's heuristic point is the start itself when the heuristic changed nothing.
                    if ci == 0 && hp == start_point {
                        continue;
                    }
                    points.push((hp, Some(ci)));
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
                let (cat, scale) = (choice.cat.clone(), choice.scale.clone());
                let sampled = es.ask();
                self.pending_full = sampled.len() <= room;
                points.extend(sampled.into_iter().map(|v| {
                    let cont = v
                        .iter()
                        .zip(&scale)
                        .map(|(v, s)| (v * s).clamp(0.0, 1.0))
                        .collect();
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
            match self.free.apply(&self.base, &point) {
                Some(p) => {
                    patches.push(p);
                    self.pending.push((point, true));
                }
                None => {
                    patches.push(self.base.clone());
                    self.pending.push((point, false));
                }
            }
            self.pending_choice.push(choice);
        }
        patches
    }

    /// Start the next CMA-ES run of the schedule.
    fn begin_run(&mut self) {
        let cycle_len = self.cycle_len();
        let choice = self.order[self.pos.min(cycle_len - 1)];
        self.run_choice = choice;
        // CMA-ES runs on v = u / scale, so a narrow scale makes a narrow first step (and a box [0, 1 / scale]).
        let scale = self.choices[choice].scale.clone();
        let heuristic_mean = self.choices[choice].mean.clone();
        let (mean_u, sigma, lambda) = if self.cycle == 0 {
            (heuristic_mean, self.cfg.sigma_start, self.lambda0)
        } else {
            // A random restart re-randomises what the descriptors left open and keeps what they measured precisely.
            let mean: Vec<f64> = (0..self.n_cont)
                .map(|k| {
                    let r = self.rng.unit_f64();
                    if scale[k] < PRECISE { heuristic_mean[k] } else { r }
                })
                .collect();
            (
                mean,
                self.cfg.sigma_random,
                self.lambda0 << self.cycle.min(MAX_DOUBLINGS),
            )
        };
        let mean: Vec<f64> = mean_u.iter().zip(&scale).map(|(u, s)| u / s).collect();
        let upper: Vec<f64> = scale.iter().map(|s| 1.0 / s).collect();
        let seed = self.rng.next_u64();
        match Cmaes::new(mean, sigma, lambda, vec![0.0; self.n_cont], upper, seed) {
            Ok(es) => {
                self.es = Some(es);
                self.runs += 1;
            }
            Err(_) => self.phase = Phase::Done,
        }
    }

    fn cycle_len(&self) -> usize {
        let want = if self.cycle == 0 {
            self.cfg.runs_per_cycle
        } else {
            2
        };
        want.min(self.order.len()).max(1)
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
            Phase::Probe => Stage::Probe,
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
            Phase::Runs => {
                let c = self.run_choice;
                self.choices[c].best = self.choices[c].best.min(batch_best);
                if self.pending_full {
                    let es_fit: Vec<f64> = fit
                        .iter()
                        .map(|f| if *f >= FAILED { WORST_FITNESS } else { *f })
                        .collect();
                    if let Some(es) = self.es.as_mut() {
                        es.tell(&es_fit)?;
                        if es.stop_reason().is_some_and(|r| {
                            matches!(r, StopReason::TolX | StopReason::Condition | StopReason::Stall)
                        }) {
                            self.end_run();
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

    fn finish_probe(&mut self, fit: &[f64]) {
        self.start_f = fit.first().copied();
        let mut incumbent_probed = false;
        for (k, choice) in self.pending_choice.iter().enumerate() {
            if let Some(ci) = *choice {
                self.choices[ci].probe = fit[k];
                self.choices[ci].best = self.choices[ci].best.min(fit[k]);
                incumbent_probed |= ci == 0;
            }
        }
        // The incumbent's heuristic point was the start itself when no separate point was evaluated.
        if !incumbent_probed {
            if let Some(c) = self.choices.first_mut() {
                c.probe = fit[0];
                c.best = c.best.min(fit[0]);
            }
        } else {
            self.heuristic_f = Some(self.choices[0].probe);
        }
        // A probe cut short by the budget leaves later choices unranked: they sort last.
        let mut order: Vec<usize> = (0..self.choices.len()).collect();
        order.sort_by(|&a, &b| {
            let key = |i: usize| {
                let p = self.choices[i].probe;
                if p.is_nan() { f64::INFINITY } else { p }
            };
            key(a).total_cmp(&key(b)).then(a.cmp(&b))
        });
        self.order = order;
        self.phase = if self.n_cont == 0 {
            Phase::Done
        } else {
            Phase::Runs
        };
    }

    fn end_run(&mut self) {
        self.es = None;
        self.pos += 1;
        if self.pos >= self.cycle_len() {
            self.cycle += 1;
            self.pos = 0;
            // Re-rank by the best each choice reached, ties by the earlier order.
            let rank: Vec<usize> = self.order.clone();
            let mut order = rank.clone();
            order.sort_by(|&a, &b| {
                self.choices[a]
                    .best
                    .total_cmp(&self.choices[b].best)
                    .then(
                        rank.iter()
                            .position(|&x| x == a)
                            .cmp(&rank.iter().position(|&x| x == b)),
                    )
            });
            self.order = order;
        }
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
        if let Some(step) = s.last_step().copied() {
            if !s.finished() && !after_batch(&step) {
                s.cancel();
            }
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
