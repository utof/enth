//! The lines a run appends (`enth.gr3-run/1`): `run` first, one `target` (or `target_failed`) per target, `run_end`
//! last. `cargo xtask gr3-report` reads them; docs/gr3.md describes every field. Written once, never rewritten.

use enth_solver::nearest::Bank;
use serde::Serialize;

use super::arm::Proposer;
use super::job::{Arm, CfgJob, Job, Mode, TargetJob};
use super::machine::{Conditions, MachineInfo, now_utc};
use super::target::ScoreOut;

/// Every line's schema id.
pub const SCHEMA: &str = "enth.gr3-run/1";

/// The run's first line.
#[derive(Debug, Serialize)]
pub struct RunHeader<'a> {
    schema: &'static str,
    record: &'static str,
    run_id: &'a str,
    date: String,
    arm: &'static str,
    mode: Mode,
    seconds: Option<f64>,
    budget: Option<usize>,
    workers: usize,
    split: &'a str,
    targets: usize,
    cfg: CfgJob,
    bank: Option<BankOut>,
    proposer: Option<String>,
    engine: &'static str,
    machine: MachineInfo,
    /// What xtask added: the build, the set, the command, the note.
    header: &'a serde_json::Value,
}

#[derive(Debug, Serialize)]
struct BankOut {
    path: String,
    entries: usize,
    sha256: String,
}

impl<'a> RunHeader<'a> {
    /// The header of `job`.
    pub fn new(job: &'a Job, bank: Option<&Bank>, proposer: Option<&dyn Proposer>) -> Self {
        RunHeader {
            schema: SCHEMA,
            record: "run",
            run_id: &job.run_id,
            date: now_utc(),
            arm: job.arm.name(),
            mode: job.mode,
            seconds: job.seconds,
            budget: job.budget,
            workers: job.workers,
            split: &job.split,
            targets: job.targets.len(),
            cfg: job.cfg,
            bank: bank.map(|b| BankOut {
                path: job
                    .bank
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                entries: b.len(),
                sha256: enth_solver::create::record::hex(&b.sha256()),
            }),
            proposer: proposer.map(Proposer::id),
            engine: enth_patch::file::ENGINE_VERSION,
            machine: MachineInfo::read(),
            header: &job.header,
        }
    }
}

/// The intake's reading of the upload, as recorded.
#[derive(Clone, Debug, Serialize)]
pub struct IntakeOut {
    /// Milliseconds the intake took.
    pub ms: f64,
    /// The inferred key.
    pub key: u8,
    /// The note-on (clip s).
    pub on_s: f64,
    /// The hold (s).
    pub hold_s: f64,
    /// The scored window's length (s).
    pub duration_s: f64,
    /// FE's pitch reading (Hz).
    pub f0_hz: Option<f64>,
    /// The detected bandwidth (Hz).
    pub bandwidth_hz: Option<f64>,
    /// The warnings' kinds (`phrase`, `notes`, `chord`, `unpitched`, `capped`, `channels_cancel`).
    pub warnings: Vec<String>,
}

/// The best candidate at a checkpoint.
#[derive(Clone, Debug, Serialize)]
pub struct Checkpoint {
    /// The mark: seconds (wall mode) or renders (render mode).
    pub mark: f64,
    /// When the round it reports ended (s from the clock's start, the intake included).
    pub t_s: f64,
    /// The job's renders then: a render-equivalent twin's budget.
    pub renders: usize,
    /// Rounds then.
    pub rounds: usize,
    /// The best candidate's objective distance (the arm's own objective, metric v1 on the scored window).
    pub objective: f64,
    /// Its branch.
    pub branch: usize,
    /// Its start's provenance kind (`heuristic`, `template`, `knn`, `model`).
    pub provenance: String,
    /// SHA-256 of its patch (`Patch::to_json`).
    pub patch_sha256: String,
    /// Its scores against the target patch, judged as played at the note create inferred (moved by the target's
    /// `register_shift` semitones first): the endpoint.
    pub score: Option<ScoreOut>,
    /// The scores of the patch as it stands, when the register shift is not 0 (otherwise the same as `score`).
    pub raw_score: Option<ScoreOut>,
}

/// The job's end.
#[derive(Clone, Debug, Serialize)]
pub struct FinalOut {
    /// Renders.
    pub renders: usize,
    /// Rounds.
    pub rounds: usize,
    /// Why it stopped (`budget`, `cancelled`, `exhausted`, `goal`).
    pub stop: String,
    /// Starts in the portfolio, and the K branches.
    pub starts: usize,
    /// Branches.
    pub branches: usize,
    /// The provenance kinds of the starts kept as branches, in branch order.
    pub branch_provenance: Vec<String>,
    /// Seconds from the clock's start to the job's end (the last round may end after the cap).
    pub t_s: f64,
    /// Renders per second over the timed window (the job's renders over its seconds).
    pub renders_per_s: f64,
}

/// One target's line.
#[derive(Clone, Debug, Serialize)]
pub struct TargetRecord {
    schema: &'static str,
    record: &'static str,
    /// The run.
    pub run_id: String,
    /// When the target started.
    pub date: String,
    /// The arm.
    pub arm: &'static str,
    /// The mode.
    pub mode: Mode,
    /// The cap: seconds (wall) or renders (render mode).
    pub cap: f64,
    /// The pool's threads.
    pub workers: usize,
    /// The target: id, split, kind, level, phrase, core, stratum.
    pub target: TargetOut,
    /// The machine over the timed window.
    pub machine: MachineOut,
    /// The intake.
    pub intake: IntakeOut,
    /// Seconds charged to the clock for inference (B and C), measured or declared.
    pub inference_s: f64,
    /// The intake's key less the key the target's performance starts on (semitones): what every candidate is moved
    /// by before it is scored (see `score`).
    pub register_shift: i32,
    /// The job's end.
    #[serde(rename = "final")]
    pub fin: FinalOut,
    /// The checkpoints, in mark order; the last is at the cap.
    pub checkpoints: Vec<Checkpoint>,
    /// Notes (the portfolio's, the scorer's).
    pub notes: Vec<String>,
}

/// A target's identity in a record.
#[derive(Clone, Debug, Serialize)]
pub struct TargetOut {
    /// Its id.
    pub id: String,
    /// `test` or `dev`.
    pub split: String,
    /// `generator` or `preset`.
    pub kind: super::job::Kind,
    /// Its level (generator).
    pub level: Option<u8>,
    /// Its phrase (generator).
    pub phrase: Option<u16>,
    /// In the core.
    pub core: bool,
    /// `gen-1note`, `gen-2note`, `preset`.
    pub stratum: &'static str,
}

/// The machine over a target's timed window.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct MachineOut {
    /// The window.
    #[serde(flatten)]
    pub conditions: Conditions,
    /// Whether the quiet lock was held exclusively over it.
    pub locked: bool,
    /// Seconds spent waiting for the lock.
    pub lock_wait_s: f64,
}

impl TargetRecord {
    /// A target's line.
    #[allow(
        clippy::too_many_arguments,
        reason = "a record's parts, each built by its own step"
    )]
    pub fn new(
        job: &Job,
        t: &TargetJob,
        stratum: &'static str,
        date: String,
        machine: MachineOut,
        intake: IntakeOut,
        inference_s: f64,
        register_shift: i32,
        fin: FinalOut,
        checkpoints: Vec<Checkpoint>,
        notes: Vec<String>,
    ) -> Self {
        let cap = match job.mode {
            Mode::Wall => job.seconds.unwrap_or(f64::NAN),
            Mode::Renders => job.budget_of(t) as f64,
        };
        TargetRecord {
            schema: SCHEMA,
            record: "target",
            run_id: job.run_id.clone(),
            date,
            arm: job.arm.name(),
            mode: job.mode,
            cap,
            workers: job.workers,
            target: TargetOut {
                id: t.id.clone(),
                split: t.split.clone(),
                kind: t.kind,
                level: t.level,
                phrase: t.phrase,
                core: t.core,
                stratum,
            },
            machine,
            intake,
            inference_s,
            register_shift,
            fin,
            checkpoints,
            notes,
        }
    }

    /// The score at the cap (the last checkpoint).
    pub fn final_score(&self) -> Option<&ScoreOut> {
        self.checkpoints.last().and_then(|c| c.score.as_ref())
    }

    /// One line for the log.
    pub fn summary_line(&self, i: usize, n: usize) -> String {
        let at_cap = self.checkpoints.last();
        let (pct, jnd, renders) = at_cap.map_or((f64::NAN, f64::NAN, 0), |c| {
            c.score
                .as_ref()
                .map_or((f64::NAN, f64::NAN, c.renders), |s| {
                    (s.percent, s.jnd, c.renders)
                })
        });
        format!(
            "gr3 [{i}/{n}] {} ({}): {pct:.1} % ({jnd:.2} JND) at the cap, {renders} renders, {:.1} s, others {:.1} cores{}",
            self.target.id,
            self.target.stratum,
            self.fin.t_s,
            self.machine.conditions.others_cores.unwrap_or(f64::NAN),
            if pct >= super::target::PASS_MARK {
                ", PASS"
            } else {
                ""
            }
        )
    }
}

/// A target that could not run: recorded, never dropped.
#[derive(Debug, Serialize)]
pub struct TargetFailure {
    schema: &'static str,
    record: &'static str,
    run_id: String,
    date: String,
    arm: &'static str,
    target: String,
    error: String,
}

impl TargetFailure {
    /// The failure of `t`.
    pub fn new(job: &Job, t: &TargetJob, error: &str) -> Self {
        TargetFailure {
            schema: SCHEMA,
            record: "target_failed",
            run_id: job.run_id.clone(),
            date: now_utc(),
            arm: job.arm.name(),
            target: t.id.clone(),
            error: error.to_owned(),
        }
    }
}

/// The run's last line.
#[derive(Debug, Serialize)]
pub struct RunEnd {
    schema: &'static str,
    record: &'static str,
    run_id: String,
    date: String,
    arm: &'static str,
    targets_done: usize,
    targets_failed: usize,
    passed_at_cap: usize,
    seconds: f64,
    load1: Option<f64>,
}

impl RunEnd {
    /// The end of `job`.
    pub fn new(job: &Job, done: usize, failed: usize, passed: usize, seconds: f64) -> Self {
        RunEnd {
            schema: SCHEMA,
            record: "run_end",
            run_id: job.run_id.clone(),
            date: now_utc(),
            arm: job.arm.name(),
            targets_done: done,
            targets_failed: failed,
            passed_at_cap: passed,
            seconds,
            load1: super::machine::load1(),
        }
    }
}

/// A target's checkpoint patches (the sidecar file, not committed): a twin replay rebuilds them, this saves the
/// replay for listening kits and inspection.
#[derive(Debug, Serialize)]
pub struct PatchesLine {
    /// The run.
    pub run_id: String,
    /// The arm.
    pub arm: Arm,
    /// The target.
    pub target: String,
    /// Each checkpoint's mark and patch (`Patch::to_json`, as a JSON value).
    pub checkpoints: Vec<(f64, serde_json::Value)>,
}
