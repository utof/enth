//! The arms and the timed loop: one target from its upload to its record (see the [parent](super) for the steps).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use enth_analysis::intake::Intake;
use enth_patch::Patch;
use enth_render::DEFAULT_SEED;
use enth_solver::create::{
    Analysis, CreateJob, Inputs, Portfolio, Provenance, Start, Target, Template, run as drive,
};
use enth_solver::nearest::Bank;
use enth_solver::search::{MetricTarget, MetricWorker, Objective};
use rayon::prelude::*;
use serde::Deserialize;

use super::job::{Arm, Job, Mode, TargetJob};
use super::machine::{QuietLock, Window, now_utc};
use super::record::{Checkpoint, FinalOut, IntakeOut, MachineOut, PatchesLine, TargetRecord};
use super::target::{Gr3Target, ScoreOut, patch_sha256, transpose};

/// What every target of a run shares.
pub struct Ctx<'a> {
    /// The heuristic start's base (`content/templates/init`).
    pub init: &'a Patch,
    /// The templates `create` proposes.
    pub templates: &'a [Template],
    /// The k-NN bank, if any.
    pub bank: Option<&'a Bank>,
    /// The pool every batch runs on.
    pub pool: &'a rayon::ThreadPool,
    /// The model's proposer (arms B and C).
    pub proposer: Option<&'a dyn Proposer>,
}

/// **Where arm B plugs in.** A model's proposals for one target, computed inside the clock (its time is the arm's
/// time: B's clock includes intake, features and inference). Lane `EVb` implements it over `enth_solver::propose`
/// once lane ONX's kernels and a model file exist; until then [`FileProposer`] reads proposals from a file for dry
/// runs of the B and C paths.
pub trait Proposer {
    /// The model's id (recorded in the run's first line).
    fn id(&self) -> String;
    /// The starts for `target_id` (as [`Provenance::Model`]), in the model's order (by confidence).
    fn propose(
        &self,
        target_id: &str,
        target: &Target,
        analysis: &Analysis,
    ) -> Result<Vec<Start>, String>;
    /// Seconds charged to the clock on top of the time `propose` took (a file declares the inference it stands for).
    fn charged_s(&self, _target_id: &str) -> f64 {
        0.0
    }
}

/// The proposals file's schema id.
pub const PROPOSALS_SCHEMA: &str = "enth.gr3-proposals/1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalsFile {
    schema: String,
    model: String,
    targets: BTreeMap<String, FileTarget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTarget {
    /// The inference time these starts stand for (s), charged to the clock.
    #[serde(default)]
    charge_s: f64,
    starts: Vec<FileStart>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileStart {
    /// A patch file's JSON.
    patch: serde_json::Value,
    #[serde(default)]
    confidence: Option<f64>,
}

/// One target's proposals in a file: the seconds charged, then each start and its confidence.
type FileStarts = (f64, Vec<(Patch, Option<f64>)>);

/// Proposals read from a file (`enth.gr3-proposals/1`): a dry run of arms B and C, never a GR3 result.
pub struct FileProposer {
    model: String,
    targets: BTreeMap<String, FileStarts>,
}

impl FileProposer {
    /// Read `path`.
    pub fn load(path: &Path) -> Result<FileProposer, String> {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("cannot read proposals {}: {e}", path.display()))?;
        let f: ProposalsFile = serde_json::from_str(&text)
            .map_err(|e| format!("proposals {}: {e}", path.display()))?;
        if f.schema != PROPOSALS_SCHEMA {
            return Err(format!(
                "proposals {}: schema {}, not {PROPOSALS_SCHEMA}",
                path.display(),
                f.schema
            ));
        }
        let mut targets = BTreeMap::new();
        for (id, t) in f.targets {
            let mut starts = Vec::new();
            for (i, s) in t.starts.into_iter().enumerate() {
                let patch = enth_patch::file::load_value(s.patch)
                    .map_err(|e| format!("proposals for {id}, start {i}: {e}"))?
                    .patch;
                starts.push((patch, s.confidence));
            }
            targets.insert(id, (t.charge_s.max(0.0), starts));
        }
        Ok(FileProposer {
            model: f.model,
            targets,
        })
    }
}

impl Proposer for FileProposer {
    fn id(&self) -> String {
        format!("file:{}", self.model)
    }

    fn propose(&self, target_id: &str, _: &Target, _: &Analysis) -> Result<Vec<Start>, String> {
        let (_, starts) = self
            .targets
            .get(target_id)
            .ok_or_else(|| format!("the proposals file has nothing for {target_id}"))?;
        Ok(starts
            .iter()
            .enumerate()
            .map(|(hypothesis, (patch, confidence))| Start {
                patch: patch.clone(),
                provenance: Provenance::Model {
                    model: self.model.clone(),
                    hypothesis,
                    confidence: *confidence,
                },
            })
            .collect())
    }

    fn charged_s(&self, target_id: &str) -> f64 {
        self.targets.get(target_id).map_or(0.0, |t| t.0)
    }
}

/// Metric v1 over a pool of workers, distances by index (what `enth match` and the plugin's create pool do), so the
/// result never depends on the worker count.
struct PoolObjective<'a> {
    target: &'a MetricTarget,
    pool: &'a rayon::ThreadPool,
}

impl Objective for PoolObjective<'_> {
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64> {
        let target = self.target;
        self.pool.install(|| {
            candidates
                .par_iter()
                .map_init(
                    || target.worker(),
                    |w: &mut MetricWorker<'_>, p| w.distance(p),
                )
                .collect()
        })
    }
}

/// The best candidate after a round.
#[derive(Clone)]
struct Snap {
    t_s: f64,
    renders: usize,
    rounds: usize,
    objective: f64,
    branch: usize,
    provenance: String,
    patch: Patch,
}

fn snap(job: &CreateJob, t_s: f64) -> Option<Snap> {
    let cands = job.candidates();
    let best = cands.iter().min_by(|a, b| {
        a.distance
            .total_cmp(&b.distance)
            .then(a.branch.cmp(&b.branch))
    })?;
    Some(Snap {
        t_s,
        renders: job.renders(),
        rounds: job.progress().rounds,
        objective: best.distance,
        branch: best.branch,
        provenance: best.provenance.kind().to_owned(),
        patch: best.patch.clone(),
    })
}

/// The checkpoint marks and the state each one reports: the last round that ended at or before it.
struct Tracker {
    marks: Vec<f64>,
    next: usize,
    prev: Option<Snap>,
    taken: Vec<(f64, Option<Snap>)>,
}

impl Tracker {
    fn new(marks: Vec<f64>) -> Tracker {
        Tracker {
            marks,
            next: 0,
            prev: None,
            taken: Vec::new(),
        }
    }

    /// A round ended at `axis` (seconds or renders) with `now` as its state.
    fn round(&mut self, axis: f64, now: Option<Snap>) {
        while self.next < self.marks.len() && self.marks[self.next] < axis {
            self.taken.push((self.marks[self.next], self.prev.clone()));
            self.next += 1;
        }
        self.prev = now;
    }

    /// The job ended: every mark left reports the last state.
    fn finish(mut self) -> Vec<(f64, Option<Snap>)> {
        while self.next < self.marks.len() {
            self.taken.push((self.marks[self.next], self.prev.clone()));
            self.next += 1;
        }
        self.taken
    }
}

fn warning_kinds(intake: &Intake) -> Vec<String> {
    intake
        .report
        .warnings
        .iter()
        .map(|w| {
            serde_json::to_value(w)
                .ok()
                .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned())
        })
        .collect()
}

/// Run one target of `job` and build its record (and its checkpoint patches).
pub fn run_target(
    job: &Job,
    t: &TargetJob,
    ctx: &Ctx<'_>,
) -> Result<(TargetRecord, PatchesLine), String> {
    let target = Gr3Target::build(t)?;
    let upload = target.upload()?;
    let oracle = match job.arm {
        Arm::D => vec![Template {
            name: "oracle".to_owned(),
            patch: target.oracle_template()?,
        }],
        _ => Vec::new(),
    };
    let budget = job.budget_of(t);
    let marks = job.marks_of(t);
    let date = now_utc();

    // ---- the clock (and, in wall mode, the quiet lock) ----
    let lock = match (job.mode, &job.lock) {
        (Mode::Wall, Some(path)) => Some(QuietLock::take(path)?),
        _ => None,
    };
    let window = Window::start();
    let t0 = window.t0();
    let ti = Instant::now();
    let intake = Intake::new(&upload, 1, enth_challenge::model::SCORING_SAMPLE_RATE)
        .map_err(|e| format!("intake: {e}"))?;
    let intake_ms = ti.elapsed().as_secs_f64() * 1000.0;
    let ct = Target::from_intake_scored(&intake, DEFAULT_SEED, job.cfg.score_s);
    let analysis = ct.analyse().map_err(|e| format!("analyse: {e}"))?;
    let mut cfg = job.cfg.create_cfg(budget);
    let tp = Instant::now();
    let (proposals, charged) = match (job.arm, ctx.proposer) {
        (Arm::B | Arm::C, Some(p)) => (p.propose(&t.id, &ct, &analysis)?, p.charged_s(&t.id)),
        (Arm::B | Arm::C, None) => return Err("arms B and C need a proposer".to_owned()),
        _ => (Vec::new(), 0.0),
    };
    let portfolio = if job.arm == Arm::C {
        let mut p = Portfolio::default();
        for s in &proposals {
            p.push(s.clone());
        }
        cfg.budget = p.starts.len().max(1);
        p
    } else {
        let inputs = Inputs {
            init: ctx.init,
            templates: if job.arm == Arm::D {
                &oracle
            } else {
                ctx.templates
            },
            bank: ctx.bank,
            proposals: &proposals,
        };
        Portfolio::build(&ct, &analysis, &inputs, &cfg)
    };
    // The proposer's own time (already on the clock) plus what it declares on top.
    let inference_s = if matches!(job.arm, Arm::B | Arm::C) {
        tp.elapsed().as_secs_f64() + charged
    } else {
        0.0
    };
    let starts = portfolio.starts.len();
    let mut notes = portfolio.notes.clone();
    let mut cjob =
        CreateJob::new(&ct, &analysis, portfolio, &cfg).map_err(|e| format!("create: {e}"))?;
    let metric = ct.metric_target();
    let mut objective = PoolObjective {
        target: &metric,
        pool: ctx.pool,
    };
    let cap_s = job.seconds.unwrap_or(f64::INFINITY);
    let mut tracker = Tracker::new(marks);
    let wall = job.mode == Mode::Wall;
    {
        let mut progress = |j: &CreateJob| -> bool {
            let t_s = t0.elapsed().as_secs_f64() + charged;
            let axis = if wall { t_s } else { j.renders() as f64 };
            tracker.round(axis, snap(j, t_s));
            !wall || t_s < cap_s
        };
        drive(&mut cjob, &mut objective, &mut progress).map_err(|e| format!("create: {e}"))?;
    }
    let end_s = t0.elapsed().as_secs_f64() + charged;
    let conditions = window.end();
    let lock_wait_s = lock.as_ref().map_or(0.0, |l| l.waited_s);
    let locked = lock.is_some();
    drop(lock);
    // ---- the clock stopped; the lock is released ----

    let result = cjob.result();
    notes.extend(result.notes.iter().cloned());
    let taken = tracker.finish();
    let scorer = match target.scorer() {
        Ok(s) => Some(s),
        Err(e) => {
            notes.push(format!("not scored: {e}"));
            None
        }
    };
    let shift = i32::from(ct.perf.key) - i32::from(target.true_key());
    let mut cache: BTreeMap<String, (Option<ScoreOut>, Option<ScoreOut>)> = BTreeMap::new();
    let score_one = |p: &Patch, notes: &mut Vec<String>, mark: f64| -> Option<ScoreOut> {
        scorer.as_ref().and_then(|sc| match sc.score(p) {
            Ok(x) => Some(x),
            Err(e) => {
                notes.push(format!("checkpoint {mark}: not scored: {e}"));
                None
            }
        })
    };
    let mut checkpoints = Vec::new();
    let mut patches = Vec::new();
    for (mark, s) in taken {
        let Some(s) = s else {
            checkpoints.push(Checkpoint {
                mark,
                t_s: 0.0,
                renders: 0,
                rounds: 0,
                objective: f64::INFINITY,
                branch: 0,
                provenance: "none".to_owned(),
                patch_sha256: String::new(),
                score: None,
                raw_score: None,
            });
            continue;
        };
        let sha = patch_sha256(&s.patch);
        let (score, raw_score) = if let Some(sc) = cache.get(&sha) {
            *sc
        } else {
            let aligned = score_one(&transpose(&s.patch, shift), &mut notes, mark);
            let raw = if shift == 0 {
                None
            } else {
                score_one(&s.patch, &mut notes, mark)
            };
            cache.insert(sha.clone(), (aligned, raw));
            (aligned, raw)
        };
        patches.push((
            mark,
            serde_json::from_str(&s.patch.to_json()).unwrap_or(serde_json::Value::Null),
        ));
        checkpoints.push(Checkpoint {
            mark,
            t_s: s.t_s,
            renders: s.renders,
            rounds: s.rounds,
            objective: s.objective,
            branch: s.branch,
            provenance: s.provenance,
            patch_sha256: sha,
            score,
            raw_score,
        });
    }
    let rep = &intake.report;
    let intake_out = IntakeOut {
        ms: intake_ms,
        key: ct.perf.key,
        on_s: ct.perf.on_s,
        hold_s: ct.perf.hold_s,
        duration_s: ct.perf.duration_s,
        f0_hz: ct.perf.f0_hz,
        bandwidth_hz: rep.degradation.bandwidth_hz,
        warnings: warning_kinds(&intake),
    };
    let fin = FinalOut {
        renders: result.renders,
        rounds: result.rounds.len(),
        stop: serde_json::to_value(result.stop)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        starts,
        branches: result.candidates.len(),
        branch_provenance: result
            .candidates
            .iter()
            .map(|c| c.provenance.kind().to_owned())
            .collect(),
        t_s: end_s,
        renders_per_s: if conditions.seconds > 0.0 {
            result.renders as f64 / conditions.seconds
        } else {
            0.0
        },
    };
    let machine = MachineOut {
        conditions,
        locked,
        lock_wait_s,
    };
    let rec = TargetRecord::new(
        job,
        t,
        target.stratum,
        date,
        machine,
        intake_out,
        inference_s,
        shift,
        fin,
        checkpoints,
        notes,
    );
    let line = PatchesLine {
        run_id: job.run_id.clone(),
        arm: job.arm,
        target: t.id.clone(),
        checkpoints: patches,
    };
    Ok((rec, line))
}
