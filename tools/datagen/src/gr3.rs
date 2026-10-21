//! **Owner: lane EVK** (the GR3 kit, R3); **`EVb`** adds the model's proposer when R3b's model exists. Driven by
//! `cargo xtask gr3-run`, which stays light: it reads `content/gr3/synth.toml`, picks the targets, checks the freeze
//! and writes a job file; this command runs the arms in-process with the engine crates and appends the results.
//!
//! ```text
//! enth-datagen gr3 run --job JOB.json      # the arms over the job's targets; one JSON line per target appended
//! ```
//!
//! # What a run does, per target (docs/gr3.md has the protocol)
//!
//! 1. **The upload:** the target patch (a generator-v1 code regenerated and checked against its fingerprint, or a
//!    frozen preset file checked against its SHA-256) rendered through the target's own performance (the code's
//!    phrase; the preset's note) at 48 kHz with the default render seed, mono. Never audio from disk: renders are
//!    bit-exact.
//! 2. **The clock starts**, under `flock -x` on the quiet lock in wall-clock mode (per target, so other lanes wait
//!    at most one target): FE's intake, `Target::from_intake_scored` (the scored window), the analysis, the arm's
//!    portfolio (B and C: the proposer's inference, charged to the clock), then the create job, one merged batch per
//!    round on a pool of `workers` threads. Wall-clock mode stops the job from its progress callback at the cap;
//!    render mode caps it by its budget. Either way the job is an exact prefix of a longer one (docs/r3.md, the
//!    determinism contract, point 3).
//! 3. **Checkpoints:** after every round the kit keeps the best candidate by the arm's own objective; at each mark
//!    of the grid (seconds in wall mode, renders in render mode) it records the state of the last round that ended
//!    at or before the mark: its renders (the render-equivalent twin's budget), its objective distance and its patch.
//! 4. **The clock stops; the lock is released;** then, untimed, every checkpoint's patch is scored with `sound/1`
//!    against the target patch through the target's performance (pass at 90), `structure/1`, and the independent
//!    metric no arm optimises (MFCC mean squared error between the two scoring renders).
//! 5. **One JSON line** (`enth.gr3-run/1`, `record = "target"`) is appended to the job's `out` file and flushed,
//!    with the machine's conditions over the timed window (load, the CPU other processes used, the lock's wait).
//!    A run also writes a `run` line first and a `run_end` line last. Nothing is ever rewritten or dropped.
//!
//! # The arms (docs/r3.md, "GR3 and its hygiene"; the F1s judge's mapping)
//!
//! | Arm | Portfolio | Budget |
//! | --- | --- | --- |
//! | A | `create`'s own: the heuristic start, the templates, the k-NN neighbours | the cap |
//! | B | A's, plus the proposer's starts as [`Provenance::Model`] (`Inputs::proposals`); the proposer's inference is charged to the clock | the cap |
//! | C | the proposer's starts only | their count (a screening round, no search) |
//! | D | A's, with the templates replaced by the oracle template: the target's own structure (modules, routes) with every parameter its phrase reveals at its default | the cap |
//!
//! **Where B plugs in:** [`arm::Proposer`]. Today the only proposer reads a proposals file
//! (`enth.gr3-proposals/1`), for dry runs of the B and C paths; lane `EVb` adds one that calls
//! `enth_solver::propose` (lane ONX) on the target and the analysis, inside the clock, and nothing else changes.
//!
//! [`Provenance::Model`]: enth_solver::create::Provenance::Model
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "tooling: reads the job and the bank, appends the runs, reads the clock and /proc, runs a rayon pool"
)]

pub mod arm;
pub mod job;
pub mod machine;
pub mod record;
pub mod target;

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::time::Instant;

use enth_solver::create::Template;
use enth_solver::nearest::Bank;

use self::arm::{Ctx, FileProposer, Proposer};
use self::job::{Arm, Job};
use self::record::{RunEnd, RunHeader};

const USAGE: &str =
    "usage: enth-datagen gr3 run --job JOB.json   (run it through `cargo xtask gr3-run`)";

/// `gr3`: run a job.
pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("run") => {
            let path = args
                .iter()
                .position(|a| a == "--job")
                .and_then(|i| args.get(i + 1))
                .ok_or(USAGE)?;
            let text =
                fs::read_to_string(path).map_err(|e| format!("cannot read the job {path}: {e}"))?;
            let job: Job =
                serde_json::from_str(&text).map_err(|e| format!("the job {path}: {e}"))?;
            run_job(&job)
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// The templates `create` proposes (`content/templates/`), compiled in so a run never reads a moved file.
pub fn templates() -> Result<(enth_patch::Patch, Vec<Template>), String> {
    let load = |text: &str| {
        enth_patch::file::load(text)
            .map(|l| l.patch)
            .map_err(|e| format!("a template does not load: {e}"))
    };
    let init = load(include_str!("../../../content/templates/init.patch.json"))?;
    let templates = vec![
        Template {
            name: "pluck_1osc".into(),
            patch: load(include_str!(
                "../../../content/templates/pluck_1osc.patch.json"
            ))?,
        },
        Template {
            name: "pad_2osc".into(),
            patch: load(include_str!(
                "../../../content/templates/pad_2osc.patch.json"
            ))?,
        },
        Template {
            name: "bass_sub".into(),
            patch: load(include_str!(
                "../../../content/templates/bass_sub.patch.json"
            ))?,
        },
    ];
    Ok((init, templates))
}

/// Load the job's bank, cut to its first `entries` when asked (the builder takes candidates in order, so the first
/// N entries of a bank are the bank `--entries N` builds).
pub fn load_bank(path: &Path, entries: Option<usize>) -> Result<Bank, String> {
    let bytes =
        fs::read(path).map_err(|e| format!("cannot read the bank {}: {e}", path.display()))?;
    let bank = Bank::parse(&bytes).map_err(|e| format!("the bank {}: {e}", path.display()))?;
    Ok(match entries {
        Some(n) if n < bank.len() => Bank::new(*bank.header(), bank.entries()[..n].to_vec()),
        _ => bank,
    })
}

/// Append one JSON line to `path` and flush it.
pub fn append(path: &Path, line: &impl serde::Serialize) -> Result<(), String> {
    let text =
        serde_json::to_string(line).map_err(|e| format!("cannot serialise a record: {e}"))?;
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    // One write of the whole line: concurrent runs appending to one file never interleave inside a line.
    let mut line = text;
    line.push('\n');
    f.write_all(line.as_bytes())
        .map_err(|e| format!("cannot append to {}: {e}", path.display()))?;
    f.flush()
        .map_err(|e| format!("cannot flush {}: {e}", path.display()))
}

/// Run every target of `job`, appending as it goes.
pub fn run_job(job: &Job) -> Result<(), String> {
    job.check()?;
    let (init, templates) = templates()?;
    let bank = match &job.bank {
        Some(p) => Some(load_bank(p, job.bank_entries)?),
        None => None,
    };
    let proposer: Option<Box<dyn Proposer>> = match (&job.proposals, job.arm) {
        (Some(p), _) => Some(Box::new(FileProposer::load(p)?)),
        (None, Arm::B | Arm::C) => {
            return Err(format!(
                "arm {} needs a model's proposals, and R3b's model does not exist yet: pass a proposals file \
                 (enth.gr3-proposals/1) for a dry run, or wait for lanes ONX, TR and MS (docs/gr3.md, \"Arm B\")",
                job.arm.name()
            ));
        }
        (None, _) => None,
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(job.workers.max(1))
        .thread_name(|i| format!("enth-gr3-{i}"))
        .build()
        .map_err(|e| format!("cannot start {} threads: {e}", job.workers))?;
    let ctx = Ctx {
        init: &init,
        templates: &templates,
        bank: bank.as_ref(),
        pool: &pool,
        proposer: proposer.as_deref(),
    };
    let started = Instant::now();
    append(
        &job.out,
        &RunHeader::new(job, bank.as_ref(), proposer.as_deref()),
    )?;
    let (mut done, mut failed, mut passed) = (0usize, 0usize, 0usize);
    for (i, t) in job.targets.iter().enumerate() {
        match arm::run_target(job, t, &ctx) {
            Ok((rec, patches)) => {
                if rec.final_score().is_some_and(|s| s.passed) {
                    passed += 1;
                }
                println!("{}", rec.summary_line(i + 1, job.targets.len()));
                append(&job.out, &rec)?;
                if let Some(p) = &job.patches_out {
                    append(p, &patches)?;
                }
                done += 1;
            }
            Err(e) => {
                // A failed target is recorded too: none is dropped.
                eprintln!("gr3: {}: {e}", t.id);
                append(&job.out, &record::TargetFailure::new(job, t, &e))?;
                failed += 1;
            }
        }
    }
    append(
        &job.out,
        &RunEnd::new(job, done, failed, passed, started.elapsed().as_secs_f64()),
    )?;
    println!(
        "gr3: run {} done: {done} targets, {passed} passed, {failed} failed, appended to {}",
        job.run_id,
        job.out.display()
    );
    if failed > 0 {
        return Err(format!(
            "{failed} target(s) failed (recorded in {})",
            job.out.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! The kit's own checks, on one cheap dev target (a level-1 generator code; tuning targets, never test ones):
    //! the result does not depend on the worker count, a checkpoint's renders are its twin's budget, arm C renders
    //! exactly its proposals, arm D's oracle keeps the structure and drops the values, and the bookkeeping of
    //! checkpoints, strata and the register shift.
    use std::path::PathBuf;

    use enth_challenge::random::observable;
    use enth_solver::create::Provenance;

    use super::arm::{self, Ctx, Proposer};
    use super::job::{Arm, CfgJob, Job, Kind, Mode, SCHEMA, TargetJob};
    use super::target::{Gr3Target, stratum, transpose};
    use super::*;

    /// `gen-d-l1-01` of content/gr3/synth.toml (dev).
    fn dev_target() -> TargetJob {
        TargetJob {
            id: "gen-d-l1-01".into(),
            split: "dev".into(),
            kind: Kind::Generator,
            level: Some(1),
            phrase: Some(1),
            core: false,
            code: Some("1040-1PD3-SPQ1-H4".into()),
            fingerprint: Some(
                "eb30ccd04b3edf5f6f5ad62c140e2613f8c33c685fced0afd518c144bdd2eac5".into(),
            ),
            file: None,
            sha256: None,
            perf: None,
            budget: None,
        }
    }

    fn job(arm: Arm, budget: usize, marks: Vec<f64>, workers: usize) -> Job {
        Job {
            schema: SCHEMA.into(),
            run_id: "test".into(),
            out: PathBuf::from("/dev/null"),
            patches_out: None,
            arm,
            mode: Mode::Renders,
            seconds: None,
            budget: Some(budget),
            marks,
            workers,
            lock: None,
            cfg: CfgJob {
                score_s: 1.0,
                ..CfgJob::default()
            },
            bank: None,
            bank_entries: None,
            proposals: None,
            split: "dev".into(),
            header: serde_json::Value::Null,
            targets: vec![dev_target()],
        }
    }

    fn run_one(
        j: &Job,
        proposer: Option<&dyn Proposer>,
    ) -> (record::TargetRecord, record::PatchesLine) {
        let (init, templates) = templates().unwrap();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(j.workers)
            .build()
            .unwrap();
        let ctx = Ctx {
            init: &init,
            templates: &templates,
            bank: None,
            pool: &pool,
            proposer,
        };
        arm::run_target(j, &j.targets[0], &ctx).unwrap()
    }

    fn hashes(r: &record::TargetRecord) -> Vec<(f64, usize, String)> {
        r.checkpoints
            .iter()
            .map(|c| (c.mark, c.renders, c.patch_sha256.clone()))
            .collect()
    }

    #[test]
    fn a_run_is_the_same_for_any_worker_count_and_a_checkpoint_is_its_twins_budget() {
        let one = run_one(&job(Arm::A, 90, vec![30.0], 1), None).0;
        let three = run_one(&job(Arm::A, 90, vec![30.0], 3), None).0;
        assert_eq!(hashes(&one), hashes(&three));
        assert_eq!(one.fin.renders, three.fin.renders);
        assert!(one.fin.renders <= 90);
        // Every checkpoint is scored, and the last is at the cap.
        assert!(one.checkpoints.iter().all(|c| c.score.is_some()));
        assert_eq!(one.checkpoints.last().unwrap().mark, 90.0);
        // The twin of the checkpoint at 30: a run with its renders as the budget ends on the same patch.
        let at30 = &one.checkpoints[0];
        assert_eq!(at30.mark, 30.0);
        assert!(at30.renders <= 30);
        let twin = run_one(&job(Arm::A, at30.renders.max(1), vec![], 2), None).0;
        let last = twin.checkpoints.last().unwrap();
        assert_eq!(last.renders, at30.renders);
        assert_eq!(last.patch_sha256, at30.patch_sha256);
    }

    struct Fixed(Vec<enth_patch::Patch>);

    impl Proposer for Fixed {
        fn id(&self) -> String {
            "fixed".into()
        }
        fn propose(
            &self,
            _: &str,
            _: &enth_solver::create::Target,
            _: &enth_solver::create::Analysis,
        ) -> Result<Vec<enth_solver::create::Start>, String> {
            Ok(self
                .0
                .iter()
                .enumerate()
                .map(|(i, p)| enth_solver::create::Start {
                    patch: p.clone(),
                    provenance: Provenance::Model {
                        model: "fixed".into(),
                        hypothesis: i,
                        confidence: None,
                    },
                })
                .collect())
        }
        fn charged_s(&self, _: &str) -> f64 {
            0.5
        }
    }

    #[test]
    fn arm_c_renders_its_proposals_once_and_arm_b_adds_them_to_the_portfolio() {
        let (init, templates) = templates().unwrap();
        // The true patch, moved into the frame create searches in: the intake hears this target 12 semitones up
        // (its oscillators sit an octave above the key it is played at), so the model would propose it 12 down, and
        // the kit's register shift moves it back before scoring.
        let t = Gr3Target::build(&dev_target()).unwrap();
        let intake = enth_analysis::intake::Intake::new(&t.upload().unwrap(), 1, 48_000).unwrap();
        let shift = i32::from(intake.report.perf.key) - i32::from(t.true_key());
        assert_eq!(shift, 12);
        let truth = transpose(&t.patch, -shift);
        let p = Fixed(vec![init.clone(), templates[1].patch.clone(), truth]);
        let c = run_one(&job(Arm::C, 1000, vec![], 1), Some(&p)).0;
        assert_eq!(c.fin.renders, 3);
        assert_eq!(c.fin.starts, 3);
        assert!(c.inference_s >= 0.5);
        // The true patch is among the proposals: arm C's best is it, and it passes.
        let last = c.checkpoints.last().unwrap();
        assert_eq!(last.provenance, "model");
        assert_eq!(c.register_shift, 12);
        assert!(last.score.unwrap().passed, "{:?}", last.score);
        // Arm B: the heuristic, the templates (one equals init and is skipped) and the three proposals (init again
        // is a duplicate of no start: the heuristic start is init with the heuristic applied).
        let b = run_one(&job(Arm::B, 60, vec![], 1), Some(&p)).0;
        assert!(b.fin.starts >= 6, "{}", b.fin.starts);
        assert!(b.fin.branch_provenance.iter().any(|k| k == "model"));
    }

    #[test]
    fn the_oracle_template_keeps_the_structure_and_drops_the_values() {
        let t = Gr3Target::build(&dev_target()).unwrap();
        let o = t.oracle_template().unwrap();
        let enabled = |p: &enth_patch::Patch| -> Vec<(String, bool)> {
            p.modules()
                .map(|(_, m)| (m.id.as_str().to_owned(), m.enabled))
                .collect()
        };
        assert_eq!(enabled(&o), enabled(&t.patch));
        assert_eq!(o.routes, t.patch.routes);
        let mask = observable(&t.patch, t.phrase);
        let mut moved = 0;
        for addr in mask.observable_params() {
            let spec = t.patch.spec(addr).unwrap();
            assert_eq!(o.value(addr), Some(spec.clamp(spec.default)), "{addr}");
            moved += usize::from(t.patch.value(addr) != o.value(addr));
        }
        assert!(moved > 0, "the target moved nothing off its defaults?");
        // Arm D runs with it as its only template.
        let d = run_one(&job(Arm::D, 40, vec![], 1), None).0;
        assert!(d.fin.branch_provenance.iter().any(|k| k == "template"));
    }

    #[test]
    fn strata_shift_and_marks() {
        assert_eq!(stratum(Kind::Generator, Some(1)), "gen-1note");
        assert_eq!(stratum(Kind::Generator, Some(2)), "gen-2note");
        assert_eq!(stratum(Kind::Generator, Some(3)), "gen-2note");
        assert_eq!(stratum(Kind::Preset, None), "preset");
        let t = Gr3Target::build(&dev_target()).unwrap();
        assert_eq!(t.true_key(), 48);
        let semi: enth_patch::ParamAddr = "osc1.semi".parse().unwrap();
        let before = t.patch.value(&semi).unwrap();
        let up = transpose(&t.patch, 12);
        assert_eq!(
            up.value(&semi),
            Some(t.patch.spec(&semi).unwrap().clamp(before + 12.0))
        );
        assert_eq!(transpose(&t.patch, 0), t.patch);
        // The marks: the grid below the cap, then the cap.
        let j = job(Arm::A, 300, vec![], 1);
        assert_eq!(j.marks_of(&j.targets[0]), vec![100.0, 250.0, 300.0]);
        let mut w = job(Arm::A, 300, vec![], 1);
        w.mode = Mode::Wall;
        w.seconds = Some(30.0);
        assert_eq!(
            w.marks_of(&w.targets[0]),
            vec![2.0, 5.0, 10.0, 15.0, 20.0, 30.0]
        );
        assert_eq!(w.budget_of(&w.targets[0]), job::UNREACHED);
    }

    #[test]
    fn a_moved_target_is_refused() {
        let mut t = dev_target();
        t.fingerprint = Some("00".repeat(32));
        assert!(Gr3Target::build(&t).err().unwrap().contains("fingerprint"));
        let mut t = dev_target();
        t.phrase = Some(2);
        assert!(Gr3Target::build(&t).err().unwrap().contains("phrase"));
    }
}
