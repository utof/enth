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
    writeln!(f, "{text}").map_err(|e| format!("cannot append to {}: {e}", path.display()))?;
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
