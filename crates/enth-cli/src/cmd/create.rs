//! **Owner: lane SK3** (`enth create` and its engine, freeze point F2s), then **CLC** (the full flag set, the schema
//! snapshots and docs/agent-cli.md's section, additive).
//!
//! ```text
//! enth create --target kick.wav [--candidates 8] [--seconds 60] [--budget N] [--bank enth-bank-v1.bin]
//!             [--perf …|--note A3 --vel 100 --hold 0.4s] [--seed 1] [--workers N] [--out best.patch.json]
//!             [--record run.json]
//! ```
//!
//! "Create from sound" for agents and GR3 (docs/r3.md, "`enth create`"):
//!
//! 1. **Decode** the file: WAV (`hound`), FLAC (`claxon`), Ogg Vorbis (`lewton`), MP3 (`nanomp3`), through the same
//!    decoder file as the plugin (`enth-plugin/src/create/decode.rs`, included by path), so both shells get the same
//!    bits; 192 kHz and above are decimated exactly 2:1. `symphonia` (MPL-2.0) waits for the owner's OK.
//! 2. **The intake** (`enth_analysis::intake::Intake`, with the decoder's codec hint), then
//!    `Target::from_intake` (the scored 3 s window); `--perf`, `--note`, `--vel` and `--hold` override the key, the
//!    velocity and the hold the intake inferred (the onset and the clip stay the intake's).
//! 3. **The portfolio**: the heuristic start, the templates (`content/templates/`) and, with `--bank`, the k-NN
//!    neighbours; without a bank the job says so and runs on the rest.
//! 4. **`create`**: K branches refined by successive halving on the CLI's rayon pool, scored by lane OBJ's robust
//!    distance (`logmel-pool85-robust/1`, `RobustMetricTarget::from_intake`, one `RobustWorker` per thread; recorded
//!    as the record's `settings.objective`), distances by index, so the result is the same for any `--workers`. `--budget` caps the renders (the schedule never depends on it); the
//!    wall-clock cap (`--seconds`, at most 300) is checked before every render, and a batch it interrupts is dropped:
//!    the run is then the same job with its renders as the budget (`replay_cfg`), on any machine.
//!
//! **Output:** `enth.create/1` ([`enth_solver::create::record::CreateRecord`]): the intake report whole, every start
//! as a patch file, the K candidates with their provenance and trace, every round stamped with its seconds, the
//! decoded audio's and the input features' hashes. **Printed exactly** (no 4-digit rounding, unlike other commands):
//! the patches and seeds replay the run. The CLI adds `decoded`, `elapsed_s` and `cap_s` (the record reads leniently).
//! A terminal shows the K candidates as a table. Exit 0 whatever stopped the job; the stop is in `stop` and a note.

#[path = "../../../enth-plugin/src/create/decode.rs"]
#[allow(
    dead_code,
    reason = "the plugin's decoder file, shared by path: the CLI uses part of its API"
)]
mod decode;

use std::path::{Path, PathBuf};
use std::time::Instant;

use enth_analysis::intake::{Intake, IntakeCfg};
use enth_analysis::robust::METRIC_ROBUST;
use enth_patch::Patch;
use enth_render::DEFAULT_SEED;
use enth_solver::create::record::{CreateRecord, RecordContext};
use enth_solver::create::{CreateCfg, CreateJob, CreateStop, Inputs, Portfolio, Target, Template};
use enth_solver::nearest::Bank;
use enth_solver::robust::RobustMetricTarget;
use rayon::prelude::*;
use serde_json::Value;

use super::{Reply, Res, game};
use crate::error::{self, CliError, EXIT_OK};
use crate::perf::{Perf, PerfArgs};
use crate::{patchio, templates};

/// The longest wall-clock cap (s): every result arrives in under 5 minutes (GR3, clause 3).
const MAX_SECONDS: f64 = 300.0;
/// The most candidates.
const MAX_CANDIDATES: usize = 16;

/// One pool thread's scorer: a candidate's distance to the target (lower is better).
type Scorer<'a> = Box<dyn FnMut(&Patch) -> f64 + Send + 'a>;

/// What the job scores with: the id `enth.create/1` records and a scorer per thread. **A parameter of the search**,
/// built where [`run`] builds it: lane OBJ's robust distance since main merged OBJ (the coordinator, 2026-10-21; it
/// was metric v1, `logmel-l1/1`). The plugin's `create.rs` has the same seam.
struct Objective<'a> {
    id: &'static str,
    scorer: Box<dyn Fn() -> Scorer<'a> + Sync + 'a>,
}

impl<'a> Objective<'a> {
    /// Lane OBJ's robust distance (`logmel-pool85-robust/1`) against `scoring`
    /// (`RobustMetricTarget::from_intake(&target, &intake.report)`): one `RobustWorker` per thread.
    fn robust(scoring: &'a RobustMetricTarget) -> Self {
        Objective {
            id: METRIC_ROBUST,
            scorer: Box::new(move || {
                let mut w = scoring.worker();
                Box::new(move |p: &Patch| w.distance(p))
            }),
        }
    }
}

/// `enth create`'s arguments.
#[derive(Clone, Debug, PartialEq, clap::Args)]
pub struct Args {
    /// The sound: a WAV, FLAC, Ogg Vorbis or MP3 file (192 kHz and above are decimated 2:1).
    #[arg(long, value_name = "FILE")]
    pub target: PathBuf,
    /// K: the candidates refined at once (1 to 16).
    #[arg(long, value_name = "K", default_value_t = 8)]
    pub candidates: usize,
    /// The wall-clock cap in seconds (above 0, at most 300), from decoding to the last candidate. A capped run depends
    /// on the machine's speed; the record's renders replay it exactly.
    #[arg(long, value_name = "SECONDS", default_value_t = 60.0)]
    pub seconds: f64,
    /// The most renders (default 4000): only a cap, the schedule never depends on it.
    #[arg(long, value_name = "N")]
    pub budget: Option<usize>,
    /// The k-NN bank (`enth-bank-v1.bin`, from `cargo xtask bank`). Without it the job runs on the heuristic and
    /// the templates.
    #[arg(long, value_name = "FILE")]
    pub bank: Option<PathBuf>,
    /// The search's seed. The same seed, file and renders repeat a run exactly.
    #[arg(long, value_name = "SEED", default_value_t = 1)]
    pub seed: u64,
    /// Worker threads (default: every core). The output is the same for any number at the same renders.
    #[arg(long, value_name = "N")]
    pub workers: Option<usize>,
    /// Write the best candidate's patch here.
    #[arg(long, value_name = "PATCH")]
    pub out: Option<PathBuf>,
    /// Write the `enth.create/1` record here as well.
    #[arg(long, value_name = "FILE")]
    pub record: Option<PathBuf>,
    /// Override the performance the intake inferred: the key (`--note`), the velocity (`--vel`) and the hold
    /// (`--hold`), or all three from `--perf`'s first note. The onset and the clip stay the intake's.
    #[command(flatten)]
    pub perf: PerfArgs,
}

fn check(args: &Args) -> Result<(), CliError> {
    if !(1..=MAX_CANDIDATES).contains(&args.candidates) {
        return Err(error::usage(
            "bad_candidates",
            format!(
                "--candidates {}: K must be 1 to {MAX_CANDIDATES}",
                args.candidates
            ),
            "leave it out for 8",
        ));
    }
    if !(args.seconds.is_finite() && args.seconds > 0.0 && args.seconds <= MAX_SECONDS) {
        return Err(error::usage(
            "bad_seconds",
            format!(
                "--seconds {}: the cap must be above 0 and at most {MAX_SECONDS}",
                args.seconds
            ),
            "every result arrives in under 5 minutes; leave it out for 60",
        ));
    }
    if args.budget == Some(0) {
        return Err(error::usage(
            "bad_budget",
            "--budget 0: the render cap must be at least 1",
            "leave it out for 4000",
        ));
    }
    game::check_workers(args.workers)?;
    Ok(())
}

fn from_decode(path: &Path, e: &decode::DecodeError) -> CliError {
    let shown = patchio::show(path);
    match e.code {
        "io" => CliError::new(
            error::EXIT_NOT_FOUND,
            "file_not_found",
            e.message.clone(),
            e.hint.clone(),
        )
        .in_file(&shown),
        "cancelled" => error::failed(
            "time_cap",
            "the --seconds cap passed while the file was decoded",
            "give it more --seconds, or a shorter file",
        ),
        code => error::invalid(code, e.message.clone(), e.hint.clone()).in_file(&shown),
    }
}

/// The intake's performance with the flags' key, velocity and hold.
fn override_perf(
    target: &mut Target,
    args: &PerfArgs,
    stdin: &str,
) -> Result<Vec<String>, CliError> {
    let Some(perf) = Perf::from_args(args, stdin)? else {
        return Ok(Vec::new());
    };
    let Some(first) = perf.notes.first() else {
        return Ok(Vec::new());
    };
    let whole = args.perf.is_some();
    let mut notes = Vec::new();
    if (whole || args.note.is_some())
        && let Some(key) = first.key
    {
        target.perf.key = key;
    }
    if (whole || args.vel.is_some())
        && let Some(vel) = first.vel
    {
        target.perf.velocity = f32::from(vel) / 127.0;
    }
    if (whole || args.hold.is_some())
        && let Some(off) = first.off_s
    {
        target.perf.hold_s = (off - first.on_s).max(0.0);
    }
    if args.dur.is_some() || whole {
        notes.push(
            "the clip's length and onset are the intake's: --dur and --perf's times other than the hold are not used"
                .to_owned(),
        );
    }
    notes.push(format!(
        "performance overridden: key {}, velocity {:.0}, hold {:.3} s",
        target.perf.key,
        f64::from(target.perf.velocity) * 127.0,
        target.perf.hold_s
    ));
    Ok(notes)
}

/// Run `enth create`.
pub fn run(args: &Args, stdin: &str) -> Res {
    check(args)?;
    let t0 = Instant::now();
    let cap = args.seconds;
    let late = || t0.elapsed().as_secs_f64() >= cap;

    // 1. Decode, with the cap.
    let decoded =
        decode::decode_path(&args.target, &late).map_err(|e| from_decode(&args.target, &e))?;
    let mut notes = decoded.notes();
    let described = decoded.describe();
    // 2. The intake, with the decoder's codec hint.
    let cfg = IntakeCfg {
        codec: Some(decoded.codec),
        ..IntakeCfg::default()
    };
    let intake = Intake::with_cfg(&decoded.interleaved, decoded.channels, decoded.sample_rate, &cfg)
        .map_err(|e| {
            error::invalid(
                "intake",
                format!("{}: {e}", patchio::show(&args.target)),
                "check the file holds a sound: it is silent, not finite, or at a rate the analysis refuses",
            )
        })?;
    drop(decoded);
    let mut target = Target::from_intake(&intake, DEFAULT_SEED);
    notes.extend(override_perf(&mut target, &args.perf, stdin)?);
    // 3. The starts and the bank.
    let bank = if let Some(path) = &args.bank {
        let bytes = patchio::read_bytes(path)?;
        let bank = Bank::parse(&bytes).map_err(|e| {
            error::invalid(
                e.code(),
                format!("{}: {e}", patchio::show(path)),
                "rebuild it with `cargo xtask bank`, or leave --bank out to run without k-NN",
            )
            .in_file(patchio::show(path))
        })?;
        Some(bank)
    } else {
        notes.push(
            "no --bank: running on the heuristic and the templates (k-NN needs enth-bank-v1.bin from `cargo xtask bank`)"
                .to_owned(),
        );
        None
    };
    let analysis = target
        .analyse()
        .map_err(|e| error::invalid(e.code(), e.to_string(), e.hint()))?;
    let init = game::init_patch();
    let tmpl: Vec<Template> = templates::TEMPLATES
        .iter()
        .filter(|t| t.name != "init")
        .map(|t| Template {
            name: t.name.to_owned(),
            patch: t.patch(),
        })
        .collect();
    let inputs = Inputs {
        init: &init,
        templates: &tmpl,
        bank: bank.as_ref(),
        proposals: &[],
    };
    let mut ccfg = CreateCfg {
        k: args.candidates,
        seed: args.seed,
        ..CreateCfg::default()
    };
    if let Some(b) = args.budget {
        ccfg.budget = b;
    }
    let portfolio = Portfolio::build(&target, &analysis, &inputs, &ccfg);
    let mut job = CreateJob::new(&target, &analysis, portfolio, &ccfg)
        .map_err(|e| error::invalid(e.code(), e.to_string(), e.hint()))?;
    // 4. The search on the CLI's pool, the cap checked before every render, scored by lane OBJ's robust distance from
    //    what the intake found ([`Objective`]).
    let scoring = RobustMetricTarget::from_intake(&target, &intake.report);
    let objective = Objective::robust(&scoring);
    let mut stamps: Vec<f64> = Vec::new();
    let capped = game::with_workers(args.workers, || {
        search(&mut job, &objective, &late, &mut || {
            stamps.push(t0.elapsed().as_secs_f64());
        })
    })?
    .map_err(|e| error::failed(e.code(), e.to_string(), e.hint()))?;
    let result = job.result();
    if capped {
        notes.push(format!(
            "stopped at the --seconds cap after {} renders; the record's replay_cfg (budget {}) repeats it on any machine",
            result.renders, result.renders
        ));
    }
    let names: Vec<String> = tmpl.iter().map(|t| t.name.clone()).collect();
    let mut record = CreateRecord::new(
        &result,
        &RecordContext {
            target: &target,
            analysis: &analysis,
            intake: Some(&intake.report),
            objective: objective.id,
            bank: bank.as_ref(),
            templates: &names,
        },
    );
    for (r, s) in record.rounds.iter_mut().zip(&stamps) {
        r.seconds = Some(*s);
    }
    record.notes.splice(0..0, notes);
    let elapsed = t0.elapsed().as_secs_f64();

    if let (Some(path), Some(best)) = (&args.out, result.best()) {
        patchio::write(path, &named(&best.patch, &args.target))?;
    }
    let mut value = serde_json::to_value(&record)
        .unwrap_or_else(|e| unreachable!("a create record always serialises: {e}"));
    if let Value::Object(m) = &mut value {
        m.insert("decoded".to_owned(), Value::from(described.clone()));
        m.insert("elapsed_s".to_owned(), Value::from(elapsed));
        m.insert("cap_s".to_owned(), Value::from(cap));
    }
    if let Some(path) = &args.record {
        let mut text = serde_json::to_string_pretty(&value)
            .unwrap_or_else(|e| unreachable!("a JSON value always prints: {e}"));
        text.push('\n');
        std::fs::write(path, text).map_err(|e| error::write_failed(&patchio::show(path), &e))?;
    }
    let mut stderr = String::new();
    for n in &record.notes {
        stderr.push_str("note: ");
        stderr.push_str(n);
        stderr.push('\n');
    }
    Ok(Reply {
        text: Some(table(&record, &described, &args.target, elapsed, cap)),
        value,
        code: EXIT_OK,
        stderr,
        columns: &[],
    })
}

/// The best patch named after the file (as the plugin's "Use this" names it).
fn named(patch: &Patch, path: &Path) -> Patch {
    let mut p = patch.clone();
    if let Some(stem) = path
        .file_stem()
        .map(|s| s.to_string_lossy().trim().to_owned())
        && !stem.is_empty()
    {
        p.meta.name = stem;
    }
    p
}

/// The rounds of `job` (each batch's distances by index, a render skipped once `late` says so, then the batch
/// dropped and the job cancelled), calling `round` after each told round. Returns whether the cap stopped it.
fn search(
    job: &mut CreateJob,
    objective: &Objective<'_>,
    late: &(dyn Fn() -> bool + Sync),
    round: &mut dyn FnMut(),
) -> Result<bool, enth_solver::create::CreateError> {
    let workers: Vec<std::sync::Mutex<Scorer<'_>>> = (0..rayon::current_num_threads().max(1))
        .map(|_| std::sync::Mutex::new((objective.scorer)()))
        .collect();
    loop {
        if late() {
            job.cancel();
            return Ok(true);
        }
        let batch = job.ask();
        if batch.is_empty() {
            return Ok(false);
        }
        let distances: Vec<Option<f64>> = batch
            .par_iter()
            .map(|patch| {
                if late() {
                    return None;
                }
                let i = rayon::current_thread_index().unwrap_or(0) % workers.len();
                let mut score = workers[i]
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Some((*score)(patch))
            })
            .collect();
        if distances.iter().any(Option::is_none) {
            job.cancel();
            return Ok(true);
        }
        let distances: Vec<f64> = distances.into_iter().flatten().collect();
        job.tell(&distances)?;
        round();
    }
}

fn stop_name(s: CreateStop) -> &'static str {
    match s {
        CreateStop::Budget => "budget",
        CreateStop::Cancelled => "time cap",
        CreateStop::Exhausted => "exhausted",
        CreateStop::Goal => "goal",
        _ => "unknown",
    }
}

/// What a terminal shows: the file, the stop, and the K candidates from best to worst.
fn table(record: &CreateRecord, described: &str, path: &Path, elapsed: f64, cap: f64) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let p = &record.input.perf;
    let pitch = p
        .f0_hz
        .map_or_else(|| "unpitched".to_owned(), |f| format!("{f:.1} Hz"));
    let _ = writeln!(
        s,
        "{} ({described}): note {} ({pitch}), held {:.2} s, {:.2} s scored",
        patchio::show(path),
        crate::perf::note_name(p.key),
        p.hold_s,
        p.duration_s
    );
    let _ = writeln!(
        s,
        "stop: {} after {} renders in {elapsed:.1} s (cap {cap:.0} s)",
        stop_name(record.stop),
        record.renders
    );
    let _ = writeln!(
        s,
        "rank  branch  start                  state      distance  proposal"
    );
    let mut order: Vec<&enth_solver::create::record::CandidateRecord> =
        record.candidates.iter().collect();
    order.sort_by(|a, b| {
        a.distance
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.distance.unwrap_or(f64::INFINITY))
            .then(a.branch.cmp(&b.branch))
    });
    for (rank, c) in order.iter().enumerate() {
        let start = match &c.provenance {
            enth_solver::create::Provenance::Template { name } => format!("template {name}"),
            enth_solver::create::Provenance::Knn { rank, .. } => format!("k-NN {}", rank + 1),
            other => other.kind().to_owned(),
        };
        let fmt = |d: Option<f64>| d.map_or_else(|| "-".to_owned(), |d| format!("{d:.4}"));
        let state = serde_json::to_value(c.state)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let _ = writeln!(
            s,
            "{:<5} {:<7} {:<22} {:<10} {:<9} {}",
            rank + 1,
            c.branch,
            start,
            state,
            fmt(c.distance),
            fmt(c.proposal_distance)
        );
    }
    s
}
