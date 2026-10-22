//! **Owner: lane SK3** (`enth create` and its engine, freeze point F2s), then **CLC** (the flags finished, the typed
//! `enth.create/1` ([`crate::schema::CreateOut`], `enth schema create`), the snapshots, docs/agent-cli.md's section;
//! all additive to SK3's output).
//!
//! ```text
//! enth create --target kick.wav [--candidates 8] [--seconds 60] [--budget N] [--stop-at D] [--score-seconds 3]
//!             [--bank enth-bank-v1.bin] [--perf …|--note A3 --vel 100 --hold 0.4s] [--seed 1] [--workers N]
//!             [--out best.patch.json] [--record run.json]
//! ```
//!
//! "Create from sound" for agents and GR3 (docs/r3.md, "`enth create`"):
//!
//! 1. **Decode** the file: WAV (`hound`), AIFF and AIFF-C (our own reader), FLAC (`claxon`), Ogg Vorbis (`lewton`),
//!    MP3 (`nanomp3`), through the same decoder file as the plugin (`enth-plugin/src/create/decode.rs`, included by
//!    path), so both shells get the same bits; 192 kHz and above are decimated exactly 2:1. `symphonia` (MPL-2.0)
//!    waits for the owner's OK.
//! 2. **The intake** (`enth_analysis::intake::Intake`, with the decoder's codec hint), then
//!    `Target::from_intake_scored` (the first `--score-seconds` of the clip, 3 s by default: `create::SCORE_S`);
//!    `--perf`, `--note`, `--vel` and `--hold` override the key, the velocity and the hold the intake inferred (the
//!    onset and the clip stay the intake's).
//! 3. **The portfolio**: the heuristic start, the templates (`content/templates/`) and, with `--bank`, the k-NN
//!    neighbours; without a bank the job says so and runs on the rest.
//! 4. **`create`**: K branches refined by successive halving on the CLI's rayon pool, scored by lane OBJ's robust
//!    distance (`logmel-pool85-robust/1`, `RobustMetricTarget::from_intake_scored`, one `RobustWorker` per thread;
//!    recorded as the record's `settings.objective`), distances by index, so the result is the same for any
//!    `--workers`. `--budget` caps the renders (the schedule never depends on it); `--stop-at` ends the job once a
//!    candidate is that close; the wall-clock cap (`--seconds`, at most 300) is checked before every render, and a
//!    batch it interrupts is dropped: the run is then the same job with its renders as the budget (`replay_cfg`), on
//!    any machine.
//!
//! **Output:** `enth.create/1` ([`enth_solver::create::record::CreateRecord`]): the intake report whole, every start
//! as a patch file, the K candidates with their provenance and trace, every round stamped with its seconds, the
//! decoded audio's and the input features' hashes. **Printed exactly** (no 4-digit rounding, unlike other commands):
//! the patches and seeds replay the run. The CLI adds `decoded`, `elapsed_s`, `cap_s`, `score_s`, `played` (the
//! performance as `enth.perf/1`, for `--perf` elsewhere), `wrote` and `hint` (the record reads leniently, so they
//! are additive within `/1`). A terminal shows the K candidates as a table. Exit 0 whatever stopped the job once a
//! candidate exists; the stop is in `stop` and a note. A cap that passes before the first round ends leaves no
//! candidate: that is `time_cap` (exit 4), not an empty result.
//!
//! **Errors** (all with a `hint`): exit 2 `bad_candidates`, `bad_seconds`, `bad_budget`, `bad_score_seconds`,
//! `bad_stop_at`, `bad_workers`, `bad_out`, `bad_perf`, `dur_fixed`; exit 3 `unknown_format`, `bad_file`,
//! `sample_rate`, `no_audio`, `intake`, `not_a_bank` and the bank's other codes; exit 4 `time_cap`, `no_candidate`,
//! `write_failed`; exit 5 `file_not_found` (the sound or the bank).

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
use enth_solver::create::record::{CandidateRecord, CreateRecord, RecordContext};
use enth_solver::create::{
    CreateCfg, CreateJob, CreateStop, Inputs, Portfolio, Provenance, SCORE_S, Target, Template,
};
use enth_solver::nearest::Bank;
use enth_solver::robust::RobustMetricTarget;
use rayon::prelude::*;
use serde_json::Value;

use super::{Reply, Res, game};
use crate::error::{self, CliError, EXIT_OK};
use crate::perf::{Perf, PerfArgs, note_name};
use crate::schema::{NoteRef, PerfFile, PerfNote, ids};
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
    /// (`RobustMetricTarget::from_intake_scored(&target, &intake.report, score_s)`): one `RobustWorker` per thread.
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
    /// The sound: a WAV, AIFF, FLAC, Ogg Vorbis or MP3 file (192 kHz and above are decimated 2:1).
    #[arg(long, value_name = "FILE")]
    pub target: PathBuf,
    /// K: the candidates refined at once (1 to 16). Fewer come back when the portfolio holds fewer starts (the
    /// heuristic and the templates are 4; `--bank` adds up to 16 neighbours).
    #[arg(long, value_name = "K", default_value_t = 8)]
    pub candidates: usize,
    /// The wall-clock cap in seconds (above 0, at most 300), from decoding to the last candidate. A capped run depends
    /// on the machine's speed; the record's renders replay it exactly.
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 60.0,
        allow_negative_numbers = true
    )]
    pub seconds: f64,
    /// The most renders (default 4000): only a cap, the schedule never depends on it.
    #[arg(long, value_name = "N")]
    pub budget: Option<usize>,
    /// Stop the job as soon as a candidate's distance is at or below this (`stop` reads `goal`). The distance is the
    /// objective's, `settings.objective` (`logmel-pool85-robust/1`: 0 is identical, and the patch that made a clean
    /// sound scores under 0.02), not `enth compare`'s.
    #[arg(long, value_name = "D", allow_negative_numbers = true)]
    pub stop_at: Option<f64>,
    /// The scored window in seconds: the first part of the clip (the intake keeps up to 5 s) that every candidate is
    /// rendered for and compared over. A shorter window is cheaper per render, a longer one hears more of the decay.
    /// Default 3, the product's setting.
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = SCORE_S,
        allow_negative_numbers = true
    )]
    pub score_seconds: f64,
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
    /// Write the best candidate's patch here (named after the file).
    #[arg(long, value_name = "PATCH")]
    pub out: Option<PathBuf>,
    /// Write the `enth.create/1` record here as well (what is printed).
    #[arg(long, value_name = "FILE")]
    pub record: Option<PathBuf>,
    /// Override the performance the intake inferred. A file, inline JSON (`enth schema perf`) or `-` (stdin) is a whole
    /// performance and replaces the key, velocity and hold of its one note (several notes: the first by note-on);
    /// `note=A3,hold=0.4s` replaces only what it names, the rest staying the intake's. The onset and the clip stay the
    /// intake's.
    #[arg(long, value_name = "PERF")]
    pub perf: Option<String>,
    /// Override only the note played (`A3`, `C#4`, or a MIDI number).
    #[arg(long, value_name = "NOTE")]
    pub note: Option<String>,
    /// Override only the MIDI velocity, 1 to 127.
    #[arg(long, value_name = "VEL")]
    pub vel: Option<String>,
    /// Override only how long the note is held (`0.4s`, `400ms`, or seconds).
    #[arg(long, value_name = "TIME")]
    pub hold: Option<String>,
    /// Refused (`dur_fixed`): the clip's length is the file's; `--score-seconds` sets the scored window.
    #[arg(long, value_name = "TIME", hide = true)]
    pub dur: Option<String>,
}

impl Args {
    /// The performance flags as the other commands read them (without `--dur`, which `create` refuses).
    fn perf_args(&self) -> PerfArgs {
        PerfArgs {
            perf: self.perf.clone(),
            note: self.note.clone(),
            vel: self.vel.clone(),
            hold: self.hold.clone(),
            dur: None,
        }
    }
}

/// The file `path` names, resolved as far as it exists (an output that is not written yet resolves through its
/// directory), so two spellings of one file compare equal.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => {
            let dir = if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            };
            std::fs::canonicalize(dir).map_or_else(|_| path.to_path_buf(), |d| d.join(name))
        }
        _ => path.to_path_buf(),
    }
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
    let intake = IntakeCfg::default();
    let window = intake.min_len_s..=intake.max_len_s;
    if !(args.score_seconds.is_finite() && window.contains(&args.score_seconds)) {
        return Err(error::usage(
            "bad_score_seconds",
            format!(
                "--score-seconds {}: the scored window must be {} to {} seconds",
                args.score_seconds,
                window.start(),
                window.end()
            ),
            format!(
                "the intake keeps at most {} s of a file, so a longer window scores the same clip; leave it out for {SCORE_S}",
                window.end()
            ),
        ));
    }
    if let Some(d) = args.stop_at
        && !(d.is_finite() && d >= 0.0)
    {
        return Err(error::usage(
            "bad_stop_at",
            format!("--stop-at {d}: a distance is zero or more"),
            "the objective's distance is 0 for an identical sound; the patch that made a clean sound scores under 0.02",
        ));
    }
    if args.dur.is_some() {
        return Err(error::usage(
            "dur_fixed",
            "--dur: the clip's length is the file's; create does not take one",
            "--score-seconds sets how much of the clip is scored; --perf, --note, --vel and --hold change what is played",
        ));
    }
    game::check_workers(args.workers)?;
    let target = resolved(&args.target);
    if let Some(out) = &args.out
        && let Some(record) = &args.record
        && resolved(out) == resolved(record)
    {
        return Err(error::usage(
            "bad_out",
            format!("--out and --record are both {}", patchio::show(out)),
            "--out takes the best candidate's patch file, --record the whole enth.create/1 record: name two files",
        ));
    }
    for (flag, path) in [("--out", &args.out), ("--record", &args.record)] {
        if let Some(path) = path
            && resolved(path) == target
        {
            return Err(error::usage(
                "bad_out",
                format!(
                    "{flag} {} is the --target file: it would overwrite the sound",
                    patchio::show(path)
                ),
                "write the result to another file",
            ));
        }
    }
    Ok(())
}

/// The `--seconds` cap passed before there was a candidate to give (while decoding, or before the first round ended).
fn time_cap(cap: f64) -> CliError {
    error::failed(
        "time_cap",
        format!("the --seconds cap ({cap} s) passed before the first candidate was made"),
        "give it more --seconds: decoding, the intake and the first round (every start rendered once) take a few seconds on a long file or few cores",
    )
}

fn from_decode(path: &Path, e: &decode::DecodeError, cap: f64) -> CliError {
    let shown = patchio::show(path);
    match e.code {
        "io" => CliError::new(
            error::EXIT_NOT_FOUND,
            "file_not_found",
            e.message.clone(),
            e.hint.clone(),
        )
        .in_file(&shown),
        "cancelled" => time_cap(cap),
        "unknown_format" if looks_like_json(path) => error::invalid(
            e.code,
            format!("{shown} is JSON, not a sound file"),
            "`enth create` takes a recording (WAV, AIFF, FLAC, Ogg Vorbis or MP3); to search for a patch that matches another patch, use `enth match --target PATCH`",
        )
        .in_file(&shown),
        code => error::invalid(code, e.message.clone(), e.hint.clone()).in_file(&shown),
    }
}

/// Whether the file opens with `{` or `[` (after any whitespace): a patch or another JSON document, which agents hand
/// to `--target` by mistake.
fn looks_like_json(path: &Path) -> bool {
    use std::io::Read as _;
    let mut head = [0_u8; 64];
    let Ok(n) = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)) else {
        return false;
    };
    head[..n]
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| matches!(b, b'{' | b'['))
}

/// A bank file that cannot be read: exit 5 with the bank's own hint.
fn bank_unreadable(path: &Path, why: &std::io::Error) -> CliError {
    let mut e = error::not_found(&patchio::show(path), why);
    "build it with `cargo xtask bank` (it writes enth-bank-v1.bin), or leave --bank out to run on the heuristic and the templates"
        .clone_into(&mut e.0.hint);
    e
}

/// What `--perf`, `--note`, `--vel` and `--hold` replace of the intake's performance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent facts about one flag set: which of key, velocity and hold it names, and whether it gave unused times"
)]
struct Named {
    key: bool,
    vel: bool,
    hold: bool,
    /// `--perf` also gave times that `create` does not use (a note-on, the clip's length).
    times: bool,
}

impl Named {
    fn of(args: &Args) -> Named {
        let mut n = Named {
            key: args.note.is_some(),
            vel: args.vel.is_some(),
            hold: args.hold.is_some(),
            times: false,
        };
        let Some(text) = args.perf.as_deref().map(str::trim) else {
            return n;
        };
        // The short form (`note=A3,hold=0.4s`, `perf::parse_perf_arg`'s rule) replaces only what it names, the rest
        // staying the intake's; a file, inline JSON or stdin is a whole performance and replaces all three.
        let short = text != "-"
            && !text.starts_with('{')
            && text.contains('=')
            && !Path::new(text).exists();
        if short {
            for key in text.split(',').filter_map(|pair| {
                pair.split_once('=')
                    .map(|(k, _)| k.trim().to_ascii_lowercase())
            }) {
                match key.as_str() {
                    "note" => n.key = true,
                    "vel" | "velocity" => n.vel = true,
                    "hold" => n.hold = true,
                    _ => n.times = true,
                }
            }
        } else {
            (n.key, n.vel, n.hold, n.times) = (true, true, true, true);
        }
        n
    }
}

/// The intake's performance with the flags' key, velocity and hold. `perf` is what the flags describe (read before the
/// file was decoded, so a bad `--perf` costs nothing), `None` when no flag was given.
fn override_perf(target: &mut Target, perf: Option<&Perf>, args: &Args) -> Vec<String> {
    let Some(perf) = perf else {
        return Vec::new();
    };
    // The note played is the earliest note-on, ties in order: the rule of `Perf::timing` and `measure`.
    let Some(first) = perf
        .notes
        .iter()
        .enumerate()
        .min_by(|(i, a), (j, b)| a.on_s.total_cmp(&b.on_s).then(i.cmp(j)))
        .map(|(_, n)| n)
    else {
        return Vec::new();
    };
    let named = Named::of(args);
    let mut notes = Vec::new();
    if named.key
        && let Some(key) = first.key
    {
        target.perf.key = key;
    }
    if named.vel
        && let Some(vel) = first.vel
    {
        target.perf.velocity = f32::from(vel) / 127.0;
    }
    if named.hold
        && let Some(off) = first.off_s
    {
        target.perf.hold_s = (off - first.on_s).max(0.0);
    }
    if named.times {
        if perf.notes.len() > 1 {
            notes.push(format!(
                "--perf holds {} notes: create plays one, the first by note-on",
                perf.notes.len()
            ));
        }
        notes.push(
            "the clip's length and onset are the intake's: --perf's times other than the hold are not used (--score-seconds sets the scored window)"
                .to_owned(),
        );
    }
    notes.push(format!(
        "performance overridden: key {}, velocity {:.0}, hold {:.3} s",
        note_name(target.perf.key),
        f64::from(target.perf.velocity) * 127.0,
        target.perf.hold_s
    ));
    notes
}

/// The performance a candidate was played with, as `enth.perf/1` (what `--perf` takes elsewhere).
fn played(record: &CreateRecord) -> PerfFile {
    let p = &record.input.perf;
    let vel = (f64::from(p.velocity) * 127.0).round().clamp(1.0, 127.0) as u8;
    PerfFile {
        schema: Some(ids::PERF.to_owned()),
        notes: vec![PerfNote {
            note: NoteRef::Name(note_name(p.key)),
            vel,
            on_s: p.on_s,
            off_s: p.on_s + p.hold_s,
        }],
        dur_s: p.duration_s,
    }
}

/// A start in a few words: `heuristic`, `template pluck_1osc`, `k-NN 3`.
fn start_label(p: &Provenance) -> String {
    match p {
        Provenance::Template { name } => format!("template {name}"),
        Provenance::Knn { rank, .. } => format!("k-NN {}", rank + 1),
        other => other.kind().to_owned(),
    }
}

/// What to do next, from the closest candidate: where it is, what its numbers mean, how to hear it.
fn next_hint(record: &CreateRecord, args: &Args, wrote_patch: bool) -> String {
    let Some(best) = record
        .best
        .and_then(|b| record.candidates.iter().find(|c| c.branch == b))
    else {
        return "no candidate has a distance yet: give it more --seconds or --budget".to_owned();
    };
    let at = format!("candidates[{}]", best.branch);
    let what = match best.distance {
        Some(d) => format!("{}, distance {d:.4}", start_label(&best.provenance)),
        None => start_label(&best.provenance),
    };
    let patch = match (&args.out, wrote_patch) {
        (Some(p), true) => format!(", written to {}", patchio::show(p)),
        _ => format!("; add --out best.patch.json to write its patch, or take {at}.patch"),
    };
    format!(
        "the closest is {at} ({what}){patch}; `played` is how every candidate was played: pass it as --perf to `enth render`, `enth describe` or `enth compare --target {}` (their distances are `compare`'s, not this objective's)",
        patchio::show(&args.target)
    )
}

/// Run `enth create`.
pub fn run(args: &Args, stdin: &str) -> Res {
    check(args)?;
    // The performance flags are read before the file is decoded: a bad `--perf` costs nothing.
    let perf = Perf::from_args(&args.perf_args(), stdin)?;
    let t0 = Instant::now();
    let cap = args.seconds;
    let late = || t0.elapsed().as_secs_f64() >= cap;

    // 1. Decode, with the cap.
    let decoded =
        decode::decode_path(&args.target, &late).map_err(|e| from_decode(&args.target, &e, cap))?;
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
    let mut target = Target::from_intake_scored(&intake, DEFAULT_SEED, args.score_seconds);
    if target.perf.duration_s + 1e-9 < intake.report.perf.duration_s {
        notes.push(format!(
            "scored over the first {:.2} s of the {:.2} s clip (--score-seconds {})",
            target.perf.duration_s, intake.report.perf.duration_s, args.score_seconds
        ));
    }
    notes.extend(override_perf(&mut target, perf.as_ref(), args));
    // 3. The starts and the bank.
    let bank = if let Some(path) = &args.bank {
        let bytes = std::fs::read(path).map_err(|e| bank_unreadable(path, &e))?;
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
        stop_at: args.stop_at,
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
    let scoring =
        RobustMetricTarget::from_intake_scored(&target, &intake.report, args.score_seconds);
    let objective = Objective::robust(&scoring);
    let mut stamps: Vec<f64> = Vec::new();
    let capped = game::with_workers(args.workers, || {
        search(&mut job, &objective, &late, &mut || {
            stamps.push(t0.elapsed().as_secs_f64());
        })
    })?
    .map_err(|e| error::failed(e.code(), e.to_string(), e.hint()))?;
    let result = job.result();
    let Some(best) = result.best().filter(|c| c.distance.is_finite()) else {
        return Err(if capped {
            time_cap(cap)
        } else {
            error::failed(
                "no_candidate",
                format!(
                    "none of the {} starts rendered to a distance",
                    result.starts.len()
                ),
                "this is a bug in create: keep the file and the flags, and report it",
            )
        });
    };
    if capped {
        notes.push(format!(
            "stopped at the --seconds cap after {} renders; the record's replay_cfg (budget {}) repeats it on any machine",
            result.renders, result.renders
        ));
    }
    let (n, starts) = (result.candidates.len(), result.starts.len());
    if n < ccfg.k {
        notes.push(if starts < ccfg.k {
            format!(
                "K is {} but the portfolio held {starts} starts, so {n} candidates (a branch per start){}",
                ccfg.k,
                if bank.is_none() {
                    "; --bank adds up to 16 k-NN neighbours"
                } else {
                    ""
                }
            )
        } else {
            format!(
                "K is {} but only {n} candidates: the render budget ended the screening round ({} renders of {starts} starts); raise --budget",
                ccfg.k, result.renders
            )
        });
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

    if let Some(path) = &args.out {
        patchio::write(path, &named(&best.patch, &args.target))?;
    }
    let mut wrote: Vec<String> = Vec::new();
    wrote.extend(args.out.iter().map(|p| patchio::show(p)));
    wrote.extend(args.record.iter().map(|p| patchio::show(p)));
    let hint = next_hint(&record, args, args.out.is_some());
    let mut value = serde_json::to_value(&record)
        .unwrap_or_else(|e| unreachable!("a create record always serialises: {e}"));
    if let Value::Object(m) = &mut value {
        m.insert("decoded".to_owned(), Value::from(described.clone()));
        m.insert("elapsed_s".to_owned(), Value::from(elapsed));
        m.insert("cap_s".to_owned(), Value::from(cap));
        m.insert("score_s".to_owned(), Value::from(args.score_seconds));
        m.insert(
            "played".to_owned(),
            serde_json::to_value(played(&record))
                .unwrap_or_else(|e| unreachable!("a performance always serialises: {e}")),
        );
        m.insert("wrote".to_owned(), Value::from(wrote));
        m.insert("hint".to_owned(), Value::from(hint.clone()));
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
        text: Some(table(
            &record,
            &described,
            &args.target,
            elapsed,
            cap,
            &hint,
        )),
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
fn table(
    record: &CreateRecord,
    described: &str,
    path: &Path,
    elapsed: f64,
    cap: f64,
    hint: &str,
) -> String {
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
    let mut order: Vec<&CandidateRecord> = record.candidates.iter().collect();
    order.sort_by(|a, b| {
        a.distance
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.distance.unwrap_or(f64::INFINITY))
            .then(a.branch.cmp(&b.branch))
    });
    for (rank, c) in order.iter().enumerate() {
        let start = start_label(&c.provenance);
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
    let _ = writeln!(s, "hint: {hint}");
    s
}
