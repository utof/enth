//! **Owner: lane TP** (the schedule's first version, freeze point F1s); **MS** adds the model's proposals (R3b).
//! Pre-declared by lane S0s (R3).
//!
//! [`CreateJob`]: K branches refined by successive halving (docs/r3.md, "K branches and successive halving"), as an
//! ask/tell state machine like [`Search`], so the shell's orchestrator thread drives it, posts candidates between
//! rounds and cancels it.
//!
//! 1. **Screening (round 0).** One batch renders every start once (at most the budget's worth, in portfolio
//!    order), so the first K proposals have distances after a single batch. The heuristic start is always kept and
//!    the best K − 1 others (by distance, ties to the earlier start) become the branches, in start order.
//! 2. **Branches.** Each branch is its own [`Search`] from its start, over the parameters the held phrase reveals
//!    (`enth_challenge::random::observable`, as `enth match --free observable`) less the envelopes' release times
//!    when the target's note-off falls at or after the clip's end, with the target's heuristic and a seed derived
//!    from the job's seed and its start's index (so a replay from recorded starts draws the same). A branch's search
//!    has no render cap of its own, so what it draws never depends on the budget. Its first batch begins with its
//!    start, already rendered in the screening round: that distance is reused rather than rendered again.
//! 3. **Rounds.** Each round asks every eligible branch for its next batch, in branch order, concatenates them into
//!    one batch for the caller's objective, and splits the distances back to each branch's `tell` by index: the
//!    result is the same for any worker count, and 8 branches × λ 16 keep a pool busy.
//! 4. **Successive halving, in each branch's renders and independent of the budget.** Rung `r` ends at the
//!    milestone [`CreateCfg::milestone`]`(r)` = `first_rung · eta^r` renders per branch (50, 100, 200, … by
//!    default): a branch asks for batches until it has reached it (a batch boundary, so it can run over by less than
//!    one batch) and then waits; when every live branch has reached it or ended, the branches are ranked by their
//!    best distance (ties to the lower branch) and the best `⌈live / eta⌉` survive, at least
//!    [`CreateCfg::finalists`]. The finalists then continue through the growing rungs until the cap: from 8
//!    branches, 4 survive their 50th render, 2 their 100th, and those 2 go on.
//! 5. **The budget is only a hard cap.** A round whose batches would pass it keeps the whole batches that fit, in
//!    branch order up to the first that does not, and is the last round. Nothing else depends on the budget, and
//!    nothing here depends on time: the shell's progress callback is the only clock, and stopping there truncates
//!    the run. **A run capped by its budget or its callback is an exact prefix of a longer one**: the evaluated
//!    patches, in order, are the longer run's first ones, so the render-equivalent twin of a timed run is the same
//!    job with `budget` = its renders.

use enth_dsp::util::rng::derive;
use enth_patch::Patch;

use super::candidate::{
    BranchStep, Candidate, CandidateState, CreateResult, CreateStop, Provenance, RoundInfo,
    StartOutcome,
};
use super::portfolio::Portfolio;
use super::{Analysis, CreateCfg, CreateError, Target, TargetPerf, free_set};
use crate::heuristic::Heuristic;
use crate::search::{FAILED, Search, SearchCfg, SearchError};

/// The stream a branch's search seed derives from (the job's seed, then this, then the start's index).
const BRANCH_STREAM: u64 = 0x4352_4541_5445_4252; // "CREATEBR"

/// A branch's search has no cap of its own: the job's budget is the only one (see the module docs, point 2).
const UNCAPPED: usize = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Screen,
    Rung(usize),
    Done,
}

/// One branch's part of a round's batch.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slice {
    branch: usize,
    /// The patches of the batch this branch renders.
    rendered: usize,
    /// The screening distance of the branch's start, which leads its first batch and is not rendered again.
    reused: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
enum Pending {
    None,
    Screen,
    Branches(Vec<Slice>),
}

#[derive(Debug)]
struct Branch {
    start: usize,
    /// `None` when the start has no parameter the target reveals: the branch keeps its start.
    search: Option<Search>,
    state: CandidateState,
    rung: usize,
    /// Renders the job spent on this branch (its start's screening render is the job's, and is not counted again).
    renders: usize,
    /// The start's screening distance, until the first batch reuses it.
    reuse: Option<f64>,
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
///     if cancelled() || past_the_cap() { job.cancel(); }
/// }
/// let result = job.result();
/// ```
#[derive(Debug)]
pub struct CreateJob {
    cfg: CreateCfg,
    perf: TargetPerf,
    starts: Vec<StartOutcome>,
    notes: Vec<String>,
    heuristic: Option<Heuristic>,
    branches: Vec<Branch>,
    phase: Phase,
    used: usize,
    pending: Pending,
    rounds: Vec<RoundInfo>,
    stop: Option<CreateStop>,
    last_round: bool,
}

impl CreateJob {
    /// A job over `portfolio`'s starts for `target` (measured as `analysis`).
    ///
    /// # Errors
    /// [`CreateError::BadCfg`] for a setting out of range, [`CreateError::NoStarts`] for an empty portfolio.
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
        Ok(CreateJob {
            cfg: cfg.clone(),
            perf: target.perf,
            starts,
            notes: portfolio.notes,
            heuristic,
            branches: Vec::new(),
            phase: Phase::Screen,
            used: 0,
            pending: Pending::None,
            rounds: Vec::new(),
            stop: None,
            last_round: false,
        })
    }

    /// The settings it runs under (fixed for the job's life: the record replays them).
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

    /// The starts the screening round evaluates: all of them, or the budget's worth.
    fn screened(&self) -> usize {
        self.starts.len().min(self.cfg.budget)
    }

    fn make_branches(&mut self, kept: &[usize]) -> Result<(), CreateError> {
        for (b, &si) in kept.iter().enumerate() {
            let patch = &self.starts[si].patch;
            let scfg = SearchCfg {
                seed: derive(derive(self.cfg.seed, BRANCH_STREAM), si as u64),
                budget: UNCAPPED,
                lambda: self.cfg.lambda,
                stop_at: self.cfg.stop_at,
                ..SearchCfg::default()
            };
            let search = match free_set(patch, &self.perf) {
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
                renders: 0,
                reuse: self.starts[si].screen_distance,
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

    fn live(&self) -> Vec<usize> {
        (0..self.branches.len())
            .filter(|&b| self.branches[b].live())
            .collect()
    }

    fn begin_rung(&mut self, r: usize) {
        let live = self.live();
        if live.is_empty() {
            self.finish(CreateStop::Exhausted);
            return;
        }
        for &b in &live {
            self.branches[b].rung = r;
        }
        self.phase = Phase::Rung(r);
    }

    /// Every live branch has reached rung `r`'s milestone: keep the best `⌈live / eta⌉` (at least the finalists).
    fn end_rung(&mut self, r: usize) {
        let live = self.live();
        if live.is_empty() {
            self.finish(CreateStop::Exhausted);
            return;
        }
        let keep = live
            .len()
            .div_ceil(self.cfg.eta)
            .max(self.cfg.finalists)
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

    /// Stop now (the caller's cap or cancel): what has been found so far is the result. Between rounds, after a
    /// [`tell`](Self::tell); a shell cancelling while a batch is out drops that batch (it never calls `tell`
    /// with it), so a cancelled run stays a prefix of the full one.
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
                    let n = self.screened();
                    return self.starts[..n].iter().map(|s| s.patch.clone()).collect();
                }
                Phase::Rung(r) => {
                    if self.used >= self.cfg.budget {
                        self.finish(CreateStop::Budget);
                        continue;
                    }
                    let milestone = self.cfg.milestone(r);
                    let eligible: Vec<usize> = (0..self.branches.len())
                        .filter(|&b| {
                            self.branches[b].live() && self.branches[b].renders < milestone
                        })
                        .collect();
                    if eligible.is_empty() {
                        self.end_rung(r);
                        continue;
                    }
                    let mut batches: Vec<(Slice, Vec<Patch>)> = Vec::new();
                    for b in eligible {
                        let start = self.branches[b].start;
                        let br = &mut self.branches[b];
                        let mut batch = match br.search.as_mut() {
                            Some(s) => s.ask(),
                            None => Vec::new(),
                        };
                        if batch.is_empty() {
                            br.state = CandidateState::Done;
                            continue;
                        }
                        // The first batch begins with the start itself, which the screening round rendered.
                        let reuse = br.reuse.take();
                        let reused = reuse
                            .filter(|_| batch.len() > 1 && batch[0] == self.starts[start].patch);
                        if reused.is_some() {
                            batch.remove(0);
                        }
                        batches.push((
                            Slice {
                                branch: b,
                                rendered: batch.len(),
                                reused,
                            },
                            batch,
                        ));
                    }
                    if batches.is_empty() {
                        continue;
                    }
                    let room = self.cfg.budget - self.used;
                    let total: usize = batches.iter().map(|(s, _)| s.rendered).sum();
                    if total > room {
                        // The hard cap: the whole batches that fit, in branch order up to the first that does not.
                        let mut sum = 0usize;
                        let fit = batches
                            .iter()
                            .take_while(|(s, _)| {
                                sum += s.rendered;
                                sum <= room
                            })
                            .count();
                        for (s, _) in &batches[fit..] {
                            self.branches[s.branch].state = CandidateState::Done;
                        }
                        batches.truncate(fit);
                        self.last_round = true;
                        if batches.is_empty() {
                            self.finish(CreateStop::Budget);
                            continue;
                        }
                    }
                    for (s, _) in &batches {
                        self.branches[s.branch].state = CandidateState::Searching;
                    }
                    self.pending = Pending::Branches(batches.iter().map(|(s, _)| *s).collect());
                    return batches.into_iter().flat_map(|(_, p)| p).collect();
                }
            }
        }
    }

    /// The distances of the batch the last [`CreateJob::ask`] returned, in the same order. A non-finite distance
    /// counts as failed. To cancel while a batch is out (a shell's pool checks its flag per job), drop the batch
    /// and call [`cancel`](Self::cancel) without telling it: the result is what the rounds before it found.
    ///
    /// # Errors
    /// [`CreateError::ObjectiveCount`] if `distances` does not match the batch (nothing changes), or
    /// [`CreateError::Search`] if a branch refuses its share (a bug).
    pub fn tell(&mut self, distances: &[f64]) -> Result<(), CreateError> {
        let expected = match &self.pending {
            Pending::None => 0,
            Pending::Screen => self.screened(),
            Pending::Branches(s) => s.iter().map(|x| x.rendered).sum(),
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
                // Starts past the budget are never evaluated, and never kept.
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
                let heuristic = self.starts[..distances.len()]
                    .iter()
                    .position(|s| matches!(s.provenance, Provenance::Heuristic));
                let mut others: Vec<usize> = (0..distances.len())
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
                for slice in slices {
                    let d = &distances[at..at + slice.rendered];
                    at += slice.rendered;
                    self.used += slice.rendered;
                    let br = &mut self.branches[slice.branch];
                    br.renders += slice.rendered;
                    if let Some(s) = br.search.as_mut() {
                        match slice.reused {
                            Some(first) => {
                                let mut all = Vec::with_capacity(d.len() + 1);
                                all.push(first);
                                all.extend_from_slice(d);
                                s.tell(&all)?;
                            }
                            None => s.tell(d)?,
                        }
                        br.trace.push(BranchStep {
                            round,
                            renders: br.renders,
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
                        renders: br.renders,
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
                let (patch, distance, runs, start_distance) = match &br.search {
                    Some(s) if s.renders() > 0 => {
                        let r = s.result();
                        let sd = r.start_distance;
                        (r.best, r.distance, r.runs, sd.is_finite().then_some(sd))
                    }
                    _ => (start.patch.clone(), self.best_of(b), 0, None),
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
                    renders: br.renders,
                    runs,
                    trace: br.trace.clone(),
                }
            })
            .collect()
    }

    /// What has been found: every start, the K candidates, the renders and rounds, the settings, why it ended.
    /// Called before the job is over it is a snapshot, reported as [`CreateStop::Cancelled`].
    #[must_use]
    pub fn result(&self) -> CreateResult {
        CreateResult {
            starts: self.starts.clone(),
            candidates: self.candidates(),
            renders: self.used,
            cfg: self.cfg.clone(),
            rounds: self.rounds.clone(),
            stop: self.stop.unwrap_or(CreateStop::Cancelled),
            notes: self.notes.clone(),
        }
    }
}
