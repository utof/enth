//! **Owner: lane TP** (the schedule's first version, freeze point F1s); **MS** adds the model's proposals (R3b).
//! Pre-declared by lane S0s (R3).
//!
//! [`CreateJob`]: K branches refined by successive halving (docs/r3.md, "K branches and successive halving"), as an
//! ask/tell state machine like [`Search`], so the shell's orchestrator thread drives it, posts candidates between
//! rounds and cancels it.
//!
//! 1. **Screening.** When the portfolio has more starts than K, one batch renders every start once; the heuristic
//!    start is always kept and the best K − 1 others (by distance, ties to the earlier start) become the branches,
//!    in start order. With K starts or fewer, every start is a branch and there is no screening round.
//! 2. **Branches.** Each branch is its own [`Search`] from its start, over the parameters the held phrase reveals
//!    (`enth_challenge::random::observable`, as `enth match --free observable`), with the target's heuristic and a
//!    seed derived from the job's seed and its start's index (so a replay from recorded starts draws the same).
//! 3. **Rounds.** Each round asks every eligible branch for its next batch, in branch order, concatenates them into
//!    one batch for the caller's objective, and splits the distances back to each branch's `tell` by index: the
//!    result is the same for any worker count, and 8 branches × λ 16 keep a pool busy.
//! 4. **Successive halving, in renders.** The budget left after screening is split into three rungs by
//!    [`CreateCfg::rungs`] (10 %, 30 %, the rest). In a rung each live branch gets an equal share and asks for
//!    batches until it has used it (a share ends at a batch boundary, so it can run over by less than one batch);
//!    when every live branch has used its share or ended, the rung ends and the branches are ranked by their best
//!    distance (ties to the lower branch): after rung 0 the best half survive (at least
//!    [`CreateCfg::finalists`]), after rung 1 the finalists. A rung's budget is computed from what is left when it
//!    starts, so an overrun in one rung comes out of the next.
//! 5. **The budget is hard.** A round whose batches would pass the budget keeps whole batches, best branch first,
//!    while they fit, and is the last round. Nothing here depends on time: the shell's progress callback is the only
//!    clock, and stopping there truncates the run.

use enth_challenge::phrases::{self, HELD};
use enth_challenge::random::observable;
use enth_dsp::util::rng::derive;
use enth_patch::Patch;

use super::candidate::{
    BranchStep, Candidate, CandidateState, CreateResult, CreateStop, Provenance, RoundInfo,
    StartOutcome,
};
use super::portfolio::Portfolio;
use super::{Analysis, CreateCfg, CreateError, Target};
use crate::heuristic::Heuristic;
use crate::search::{FAILED, FreeSet, Search, SearchCfg, SearchError};

/// The stream a branch's search seed derives from (the job's seed, then this, then the start's index).
const BRANCH_STREAM: u64 = 0x4352_4541_5445_4252; // "CREATEBR"

/// The rungs of successive halving.
const RUNGS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Screen,
    Rung(usize),
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Pending {
    None,
    Screen,
    /// `(branch, batch length)` in batch order.
    Branches(Vec<(usize, usize)>),
}

#[derive(Debug)]
struct Branch {
    start: usize,
    /// `None` when the start has no parameter the held phrase reveals: the branch keeps its start.
    search: Option<Search>,
    state: CandidateState,
    rung: usize,
    rung_used: usize,
    trace: Vec<BranchStep>,
}

impl Branch {
    fn live(&self) -> bool {
        matches!(
            self.state,
            CandidateState::Proposed | CandidateState::Searching
        )
    }
}

/// A branch's state between rounds, for a progress display.
#[derive(Clone, Debug, PartialEq)]
pub struct BranchView {
    /// The branch.
    pub branch: usize,
    /// Its start (an index into the job's starts).
    pub start: usize,
    /// Where the start came from.
    pub provenance: Provenance,
    /// Where it stands.
    pub state: CandidateState,
    /// Its best distance so far (`f64::INFINITY` before its first batch).
    pub distance: f64,
    /// Its renders so far.
    pub renders: usize,
}

/// The job between rounds: renders against the budget, and every branch.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// Rounds evaluated so far.
    pub rounds: usize,
    /// Renders used so far.
    pub renders: usize,
    /// The budget.
    pub budget: usize,
    /// The rung the job is in (`None` while screening or once done).
    pub rung: Option<usize>,
    /// Why it ended, once it has.
    pub stop: Option<CreateStop>,
    /// Every branch, in branch order (empty until the screening round is told).
    pub branches: Vec<BranchView>,
}

/// A create job as an ask/tell state machine (see the [module docs](self)). [`create`](super::create) drives one
/// to its end; a shell that posts candidates between rounds drives it itself:
///
/// ```ignore
/// let mut job = CreateJob::new(&target, &analysis, portfolio, &cfg)?;
/// loop {
///     let batch = job.ask();
///     if batch.is_empty() { break; }
///     job.tell(&pool.distances(&batch))?;   // in input order, whatever the worker count
///     post(job.candidates());
///     if cancelled() { job.cancel(); }
/// }
/// let result = job.result();
/// ```
#[derive(Debug)]
pub struct CreateJob {
    cfg: CreateCfg,
    starts: Vec<StartOutcome>,
    notes: Vec<String>,
    heuristic: Option<Heuristic>,
    branches: Vec<Branch>,
    phase: Phase,
    used: usize,
    pending: Pending,
    share: usize,
    rounds: Vec<RoundInfo>,
    stop: Option<CreateStop>,
    last_round: bool,
}

impl CreateJob {
    /// A job over `portfolio`'s starts for `target` (measured as `analysis`).
    ///
    /// # Errors
    /// [`CreateError::BadCfg`] for a setting out of range, [`CreateError::NoStarts`] for an empty portfolio,
    /// [`CreateError::Search`] if a branch's search refuses its setup.
    pub fn new(
        target: &Target,
        analysis: &Analysis,
        portfolio: Portfolio,
        cfg: &CreateCfg,
    ) -> Result<CreateJob, CreateError> {
        cfg.validate()?;
        if portfolio.starts.is_empty() {
            return Err(CreateError::NoStarts);
        }
        let perf = target.performance();
        let heuristic = analysis
            .descriptors
            .as_ref()
            .map(|d| Heuristic::new(d, &perf));
        let starts: Vec<StartOutcome> = portfolio
            .starts
            .into_iter()
            .map(|s| StartOutcome {
                provenance: s.provenance,
                patch: s.patch,
                screen_distance: None,
                branch: None,
            })
            .collect();
        let mut job = CreateJob {
            cfg: cfg.clone(),
            starts,
            notes: portfolio.notes,
            heuristic,
            branches: Vec::new(),
            phase: Phase::Screen,
            used: 0,
            pending: Pending::None,
            share: 0,
            rounds: Vec::new(),
            stop: None,
            last_round: false,
        };
        if job.starts.len() <= cfg.k {
            let all: Vec<usize> = (0..job.starts.len()).collect();
            job.make_branches(&all)?;
            job.begin_rung(0);
        }
        Ok(job)
    }

    /// The settings.
    #[must_use]
    pub fn cfg(&self) -> &CreateCfg {
        &self.cfg
    }

    /// Whether the job is over (`ask` returns nothing).
    #[must_use]
    pub fn finished(&self) -> bool {
        self.phase == Phase::Done
    }

    /// Renders used so far.
    #[must_use]
    pub fn renders(&self) -> usize {
        self.used
    }

    fn make_branches(&mut self, kept: &[usize]) -> Result<(), CreateError> {
        let phrase = phrases::phrase(HELD)
            .unwrap_or_else(|| unreachable!("the held phrase is in the table"));
        for (b, &si) in kept.iter().enumerate() {
            let patch = &self.starts[si].patch;
            let scfg = SearchCfg {
                seed: derive(derive(self.cfg.seed, BRANCH_STREAM), si as u64),
                budget: self.cfg.budget,
                lambda: self.cfg.lambda,
                stop_at: self.cfg.stop_at,
                ..SearchCfg::default()
            };
            let search = match FreeSet::from_mask(patch, &observable(patch, phrase)) {
                Ok(free) => Some(Search::new(patch, &free, self.heuristic.as_ref(), &scfg)?),
                Err(SearchError::NoFreeParameters) => None,
                Err(e) => return Err(e.into()),
            };
            let state = if search.is_some() {
                CandidateState::Proposed
            } else {
                CandidateState::Done
            };
            self.starts[si].branch = Some(b);
            self.branches.push(Branch {
                start: si,
                search,
                state,
                rung: 0,
                rung_used: 0,
                trace: Vec::new(),
            });
        }
        Ok(())
    }

    fn best_of(&self, b: usize) -> f64 {
        let br = &self.branches[b];
        match &br.search {
            Some(s) if s.renders() > 0 => s.best_distance(),
            _ => self.starts[br.start]
                .screen_distance
                .unwrap_or(f64::INFINITY),
        }
    }

    /// Branch indices ranked by best distance, ties to the lower branch.
    fn ranked(&self, among: &[usize]) -> Vec<usize> {
        let mut out = among.to_vec();
        out.sort_by(|&a, &b| self.best_of(a).total_cmp(&self.best_of(b)).then(a.cmp(&b)));
        out
    }

    fn begin_rung(&mut self, r: usize) {
        let live: Vec<usize> = (0..self.branches.len())
            .filter(|&b| self.branches[b].live())
            .collect();
        if live.is_empty() {
            self.finish(CreateStop::Exhausted);
            return;
        }
        let remaining = self.cfg.budget.saturating_sub(self.used);
        let rung_budget = if r + 1 >= RUNGS {
            remaining
        } else {
            let rest: f64 = self.cfg.rungs[r..].iter().sum();
            let frac = if rest > 0.0 {
                self.cfg.rungs[r] / rest
            } else {
                0.0
            };
            (remaining as f64 * frac) as usize
        };
        self.share = rung_budget.div_ceil(live.len()).max(1);
        for b in &mut self.branches {
            b.rung_used = 0;
        }
        for &b in &live {
            self.branches[b].rung = r;
        }
        self.phase = Phase::Rung(r);
    }

    fn end_rung(&mut self, r: usize) {
        let live: Vec<usize> = (0..self.branches.len())
            .filter(|&b| self.branches[b].live())
            .collect();
        if live.is_empty() {
            self.finish(CreateStop::Exhausted);
            return;
        }
        if r + 1 >= RUNGS {
            self.finish(CreateStop::Budget);
            return;
        }
        let keep = if r == 0 {
            live.len().div_ceil(2).max(self.cfg.finalists)
        } else {
            self.cfg.finalists
        }
        .min(live.len());
        for &b in &self.ranked(&live)[keep..] {
            self.branches[b].state = CandidateState::Pruned;
        }
        self.begin_rung(r + 1);
    }

    fn finish(&mut self, cause: CreateStop) {
        if self.stop.is_none() {
            self.stop = Some(cause);
        }
        for b in &mut self.branches {
            if b.live() {
                b.state = CandidateState::Done;
            }
        }
        self.phase = Phase::Done;
        self.pending = Pending::None;
    }

    /// Stop now (the caller's cap or cancel): what has been found so far is the result.
    pub fn cancel(&mut self) {
        if !self.finished() {
            self.finish(CreateStop::Cancelled);
        }
    }

    /// The next batch to evaluate, in order; pass its distances to [`CreateJob::tell`]. Empty once the job is over.
    pub fn ask(&mut self) -> Vec<Patch> {
        loop {
            match self.phase {
                Phase::Done => return Vec::new(),
                Phase::Screen => {
                    self.pending = Pending::Screen;
                    return self.starts.iter().map(|s| s.patch.clone()).collect();
                }
                Phase::Rung(r) => {
                    if self.used >= self.cfg.budget {
                        self.finish(CreateStop::Budget);
                        continue;
                    }
                    let eligible: Vec<usize> = (0..self.branches.len())
                        .filter(|&b| {
                            self.branches[b].live() && self.branches[b].rung_used < self.share
                        })
                        .collect();
                    if eligible.is_empty() {
                        self.end_rung(r);
                        continue;
                    }
                    let mut batches: Vec<(usize, Vec<Patch>)> = Vec::new();
                    for b in eligible {
                        let batch = match self.branches[b].search.as_mut() {
                            Some(s) => s.ask(),
                            None => Vec::new(),
                        };
                        if batch.is_empty() {
                            self.branches[b].state = CandidateState::Done;
                        } else {
                            batches.push((b, batch));
                        }
                    }
                    if batches.is_empty() {
                        continue;
                    }
                    let room = self.cfg.budget - self.used;
                    let total: usize = batches.iter().map(|(_, p)| p.len()).sum();
                    if total > room {
                        let ids: Vec<usize> = batches.iter().map(|(b, _)| *b).collect();
                        let mut keep = vec![false; self.branches.len()];
                        let mut sum = 0usize;
                        for b in self.ranked(&ids) {
                            let len = batches
                                .iter()
                                .find(|(x, _)| *x == b)
                                .map_or(0, |(_, p)| p.len());
                            if sum + len <= room {
                                keep[b] = true;
                                sum += len;
                            }
                        }
                        for (b, _) in &batches {
                            if !keep[*b] {
                                self.branches[*b].state = CandidateState::Done;
                            }
                        }
                        batches.retain(|(b, _)| keep[*b]);
                        self.last_round = true;
                        if batches.is_empty() {
                            self.finish(CreateStop::Budget);
                            continue;
                        }
                    }
                    for (b, _) in &batches {
                        self.branches[*b].state = CandidateState::Searching;
                    }
                    self.pending =
                        Pending::Branches(batches.iter().map(|(b, p)| (*b, p.len())).collect());
                    return batches.into_iter().flat_map(|(_, p)| p).collect();
                }
            }
        }
    }

    /// The distances of the batch the last [`CreateJob::ask`] returned, in the same order. A non-finite distance
    /// counts as failed.
    ///
    /// # Errors
    /// [`CreateError::ObjectiveCount`] if `distances` does not match the batch (nothing changes), or
    /// [`CreateError::Search`] if a branch refuses its share (a bug).
    pub fn tell(&mut self, distances: &[f64]) -> Result<(), CreateError> {
        let expected = match &self.pending {
            Pending::None => 0,
            Pending::Screen => self.starts.len(),
            Pending::Branches(s) => s.iter().map(|(_, n)| n).sum(),
        };
        if expected == 0 || distances.len() != expected {
            return Err(CreateError::ObjectiveCount {
                expected,
                got: distances.len(),
            });
        }
        let round = self.rounds.len();
        match std::mem::replace(&mut self.pending, Pending::None) {
            Pending::None => unreachable!("checked above"),
            Pending::Screen => {
                for (s, &d) in self.starts.iter_mut().zip(distances) {
                    s.screen_distance = Some(if d.is_finite() { d } else { FAILED });
                }
                self.used += distances.len();
                self.rounds.push(RoundInfo {
                    round,
                    renders: self.used,
                    rung: None,
                    live: distances.len(),
                });
                let heuristic = self
                    .starts
                    .iter()
                    .position(|s| matches!(s.provenance, Provenance::Heuristic));
                let mut others: Vec<usize> = (0..self.starts.len())
                    .filter(|&i| Some(i) != heuristic)
                    .collect();
                let d = |i: usize| self.starts[i].screen_distance.unwrap_or(FAILED);
                others.sort_by(|&a, &b| d(a).total_cmp(&d(b)).then(a.cmp(&b)));
                let mut kept: Vec<usize> = heuristic
                    .into_iter()
                    .chain(others)
                    .take(self.cfg.k)
                    .collect();
                kept.sort_unstable();
                self.make_branches(&kept)?;
                if self.used >= self.cfg.budget {
                    self.finish(CreateStop::Budget);
                } else {
                    self.begin_rung(0);
                }
            }
            Pending::Branches(slices) => {
                let mut at = 0usize;
                let mut goal = false;
                let live = slices.len();
                for (b, len) in slices {
                    let d = &distances[at..at + len];
                    at += len;
                    self.used += len;
                    let br = &mut self.branches[b];
                    br.rung_used += len;
                    if let Some(s) = br.search.as_mut() {
                        s.tell(d)?;
                        br.trace.push(BranchStep {
                            round,
                            renders: s.renders(),
                            job_renders: 0,
                            best: s.best_distance(),
                        });
                        if s.finished() {
                            br.state = CandidateState::Done;
                        }
                        if self.cfg.stop_at.is_some_and(|g| s.best_distance() <= g) {
                            goal = true;
                        }
                    }
                }
                let used = self.used;
                for br in &mut self.branches {
                    if let Some(step) = br.trace.last_mut()
                        && step.round == round
                    {
                        step.job_renders = used;
                    }
                }
                let rung = match self.phase {
                    Phase::Rung(r) => Some(r),
                    _ => None,
                };
                self.rounds.push(RoundInfo {
                    round,
                    renders: used,
                    rung,
                    live,
                });
                if goal {
                    self.finish(CreateStop::Goal);
                } else if self.last_round || self.used >= self.cfg.budget {
                    self.finish(CreateStop::Budget);
                }
            }
        }
        Ok(())
    }

    /// The job between rounds.
    #[must_use]
    pub fn progress(&self) -> Progress {
        Progress {
            rounds: self.rounds.len(),
            renders: self.used,
            budget: self.cfg.budget,
            rung: match self.phase {
                Phase::Rung(r) => Some(r),
                _ => None,
            },
            stop: self.stop,
            branches: (0..self.branches.len())
                .map(|b| {
                    let br = &self.branches[b];
                    BranchView {
                        branch: b,
                        start: br.start,
                        provenance: self.starts[br.start].provenance.clone(),
                        state: br.state,
                        distance: self.best_of(b),
                        renders: br.search.as_ref().map_or(0, Search::renders),
                    }
                })
                .collect(),
        }
    }

    /// Every branch as a [`Candidate`] (its best patch so far): what a shell posts between rounds.
    #[must_use]
    pub fn candidates(&self) -> Vec<Candidate> {
        (0..self.branches.len())
            .map(|b| {
                let br = &self.branches[b];
                let start = &self.starts[br.start];
                let (patch, distance, renders, runs, start_distance) = match &br.search {
                    Some(s) if s.renders() > 0 => {
                        let r = s.result();
                        let sd = r.start_distance;
                        (
                            r.best,
                            r.distance,
                            r.renders,
                            r.runs,
                            sd.is_finite().then_some(sd),
                        )
                    }
                    _ => (start.patch.clone(), self.best_of(b), 0, 0, None),
                };
                Candidate {
                    branch: b,
                    start: br.start,
                    provenance: start.provenance.clone(),
                    proposal: start.patch.clone(),
                    proposal_distance: start.screen_distance.or(start_distance),
                    patch,
                    distance,
                    state: br.state,
                    rung: br.rung,
                    renders,
                    runs,
                    trace: br.trace.clone(),
                }
            })
            .collect()
    }

    /// What has been found: every start, the K candidates, the renders and rounds, why it ended. Called before the
    /// job is over it is a snapshot, reported as [`CreateStop::Cancelled`].
    #[must_use]
    pub fn result(&self) -> CreateResult {
        CreateResult {
            starts: self.starts.clone(),
            candidates: self.candidates(),
            renders: self.used,
            budget: self.cfg.budget,
            rounds: self.rounds.clone(),
            stop: self.stop.unwrap_or(CreateStop::Cancelled),
            notes: self.notes.clone(),
        }
    }
}
