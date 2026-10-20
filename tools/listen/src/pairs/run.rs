//! The `pairs` command: design once (then reuse), run a session, show the status, dry-run, and `report`.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::design::{design, write_clips};
use super::fit::rating_model;
use super::plan::{Rng, make_plan};
use super::report::{analyze, fit_rows};
use super::{
    Config, DEFAULT_PAIRS, DEFAULT_SEED, GAP_S, Manifest, PairAnswer, PairRole, ResultsFile,
    SCHEMA_FIT, SCHEMA_MANIFEST, SCHEMA_REPORT, SCHEMA_RESULTS, SESSIONS, Side, bin_label,
};
use crate::Res;
use crate::files::{host_threads, read_json, write_json};
use crate::player::{CommandPlayer, DEFAULT_PLAYER, Player};
use crate::session::{AskSpec, LineSource, Reply, ask, prompt};
use crate::wav::{Wav, decode};

/// What to do besides designing and playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum PairsAction {
    /// Agreement per distance bin, the checks and a proposed curve; writes pairs-report.json and pairs-fit.json.
    Report,
}

/// Arguments of `enth-listen pairs`.
#[derive(clap::Args, Clone, Debug)]
pub struct PairsArgs {
    /// `report`: read the results and print the agreement per distance bin. Without it: design once, then run
    /// `--session` or show the status.
    #[arg(value_enum)]
    pub action: Option<PairsAction>,
    /// Where the clips, manifest and results live (the files start with `pairs-`, so spike-metric's listen directory
    /// can hold them too).
    #[arg(long, default_value = "listen-test")]
    pub dir: PathBuf,
    /// Play one session: 1, or 2 for the retest (the same pairs in another order, ideally on another day).
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=2))]
    pub session: Option<u32>,
    /// Design in DIR/dry-run, answer both sessions with synthetic answers, report. Plays nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Command that plays one WAV file (the path is appended).
    #[arg(long, default_value = DEFAULT_PLAYER)]
    pub player: String,
    /// Seeds the plan. A directory keeps the seed it was designed with.
    #[arg(long, default_value_t = DEFAULT_SEED)]
    pub seed: u64,
    /// Test pairs (the four checks come on top).
    #[arg(long, default_value_t = DEFAULT_PAIRS)]
    pub pairs: usize,
    /// Threads for the design (0 = up to 8).
    #[arg(long, default_value_t = 0)]
    pub workers: usize,
    /// Bootstrap resamples behind the report's intervals.
    #[arg(long, default_value_t = 2000)]
    pub bootstrap: usize,
}

impl PairsArgs {
    /// Defaults for `dir` (what clap gives with only `--dir`).
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            action: None,
            dir: dir.to_path_buf(),
            session: None,
            dry_run: false,
            player: DEFAULT_PLAYER.to_string(),
            seed: DEFAULT_SEED,
            pairs: DEFAULT_PAIRS,
            workers: 0,
            bootstrap: 2000,
        }
    }
}

/// The manifest's path.
#[must_use]
pub fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("pairs-manifest.json")
}

/// The clips' directory.
#[must_use]
pub fn clips_dir(dir: &Path) -> PathBuf {
    dir.join("pairs-clips")
}

/// Session `s`'s results file.
#[must_use]
pub fn results_path(dir: &Path, session: u32) -> PathBuf {
    dir.join(format!("pairs-session{session}.json"))
}

fn load_manifest(dir: &Path) -> Res<Option<Manifest>> {
    let mp = manifest_path(dir);
    if !mp.exists() {
        return Ok(None);
    }
    let m: Manifest = read_json(&mp)?;
    if m.schema != SCHEMA_MANIFEST {
        return Err(format!(
            "{} has schema {}, expected {SCHEMA_MANIFEST}",
            mp.display(),
            m.schema
        )
        .into());
    }
    Ok(Some(m))
}

/// Load session `s`'s results, or `None` when there are none yet.
///
/// # Errors
/// A file that cannot be read, or one for another schema, session or design.
pub fn load_results(dir: &Path, m: &Manifest, session: u32) -> Res<Option<ResultsFile>> {
    let p = results_path(dir, session);
    if !p.exists() {
        return Ok(None);
    }
    let f: ResultsFile = read_json(&p)?;
    if f.schema != SCHEMA_RESULTS {
        return Err(format!(
            "{} has schema {}, expected {SCHEMA_RESULTS}",
            p.display(),
            f.schema
        )
        .into());
    }
    if f.session != session {
        return Err(format!(
            "{} is for session {}, not {session}",
            p.display(),
            f.session
        )
        .into());
    }
    if f.plan_hash != m.plan_hash {
        return Err(format!(
            "{} belongs to another design (plan {}, this one is {}); use the directory it was made in",
            p.display(),
            f.plan_hash,
            m.plan_hash
        )
        .into());
    }
    Ok(Some(f))
}

/// Re-render any missing clip file (deterministic: the manifest holds every pair's move and step count).
fn ensure_clips(dir: &Path, m: &Manifest) -> Res<usize> {
    let cd = clips_dir(dir);
    std::fs::create_dir_all(&cd)?;
    let mut made = 0;
    for p in &m.pairs {
        made += write_clips(p, &cd, false)?.len();
    }
    Ok(made)
}

/// Load the design in `dir`, or make it and save it.
fn prepare(dir: &Path, args: &PairsArgs, out: &mut dyn Write) -> Res<Manifest> {
    std::fs::create_dir_all(dir)?;
    let m = if let Some(m) = load_manifest(dir)? {
        writeln!(
            out,
            "using the pairs designed earlier in {} ({} test pairs, seed {}, engine {})",
            dir.display(),
            m.config.pairs,
            m.config.seed,
            m.engine
        )?;
        if m.engine != enth_patch::file::ENGINE_VERSION {
            writeln!(
                out,
                "note: this build's engine is {}; the clips and distances are the {} ones in the manifest",
                enth_patch::file::ENGINE_VERSION,
                m.engine
            )?;
        }
        m
    } else {
        let workers = if args.workers > 0 {
            args.workers
        } else {
            host_threads().min(8)
        };
        let cfg = Config {
            seed: args.seed,
            pairs: args.pairs,
            workers,
        };
        let plan = make_plan(cfg.seed, cfg.pairs)?;
        writeln!(
            out,
            "designing {} test pairs and {} checks on {workers} threads (sound/1 on each answer's ladder of step counts; about a minute) ...",
            cfg.pairs,
            plan.pairs.len() - cfg.pairs
        )?;
        let d = design(&plan, &cfg, &clips_dir(dir))?;
        write_json(&manifest_path(dir), &d.manifest)?;
        let m = d.manifest;
        writeln!(
            out,
            "designed in {:.0} s (plan {}); clips in {}",
            m.design_wall_s,
            m.plan_hash,
            clips_dir(dir).display()
        )?;
        let mut counts = vec![0usize; super::BIN_LOWER.len()];
        for p in m.pairs.iter().filter(|p| p.plan.role == PairRole::Test) {
            counts[p.bin] += 1;
        }
        let row: Vec<String> = counts
            .iter()
            .enumerate()
            .map(|(b, c)| format!("{}: {c}", bin_label(b)))
            .collect();
        writeln!(out, "test pairs per distance bin (JND): {}", row.join(", "))?;
        for w in &d.warnings {
            writeln!(out, "note: {w}")?;
        }
        m
    };
    let made = ensure_clips(dir, &m)?;
    if made > 0 {
        writeln!(out, "re-rendered {made} missing clip file(s)")?;
    }
    Ok(m)
}

/// Seconds of one session's audio played once, and its trials.
fn session_seconds(m: &Manifest, session: u32) -> f64 {
    m.session_trials(session)
        .iter()
        .map(|t| {
            let p = &m.pairs[t.pair].plan;
            let len = |id: &str| m.clip(id).map_or(3.0, |c| c.seconds);
            let (a, b) = (len(&p.target_clip), len(&p.answer_clip));
            a + b + a.max(b) + 2.0 * f64::from(GAP_S)
        })
        .sum()
}

fn print_instructions(out: &mut dyn Write, m: &Manifest, session: u32) -> std::io::Result<()> {
    let trials = m.session_trials(session).len();
    // the audio once, about 7 s of answering per trial, and a replay now and then
    let minutes = (session_seconds(m, session) * 1.2 + 7.0 * trials as f64) / 60.0;
    writeln!(out)?;
    writeln!(
        out,
        "Real-engine pairs, session {session} of {SESSIONS}: {trials} trials, about {minutes:.0} minutes."
    )?;
    writeln!(
        out,
        "  - Use headphones. Set the volume once and keep it the same in every session."
    )?;
    writeln!(
        out,
        "  - Each trial plays three sounds: A, B, then X. X is an exact copy of A or of B."
    )?;
    writeln!(out, "    1. Say which one X is:  a = X is A,  b = X is B.")?;
    writeln!(
        out,
        "    2. Rate how similar A and B sound:  1 = very different   2   3   4   5 = indistinguishable."
    )?;
    writeln!(
        out,
        "  - Some trials play the same sound twice: guess X and rate what you hear. That is expected."
    )?;
    writeln!(
        out,
        "  - ra / rb / rx replay one sound, r replays all three, q quits (progress is saved after each trial;"
    )?;
    writeln!(out, "    run the same command to resume).")?;
    writeln!(
        out,
        "  - Do not open pairs-manifest.json or the results: they hold the metric's distances."
    )
}

/// Whether a session ended or the owner quit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    /// Every trial is answered.
    Finished,
    /// The owner quit; the answers so far are saved.
    Quit,
}

fn run_session(
    dir: &Path,
    m: &Manifest,
    session: u32,
    player: &mut dyn Player,
    lines: &mut dyn LineSource,
    out: &mut dyn Write,
) -> Res<SessionEnd> {
    let trials = m.session_trials(session);
    let path = results_path(dir, session);
    let mut file =
        load_results(dir, m, session)?.unwrap_or_else(|| ResultsFile::new(m, session, false));
    if file.synthetic {
        return Err(format!(
            "{} holds synthetic answers; use a different --dir",
            path.display()
        )
        .into());
    }
    print_instructions(out, m, session)?;
    if !file.answers.is_empty() {
        writeln!(
            out,
            "  - Resuming after {} answered trials.",
            file.answers.len()
        )?;
    }
    let mut cache: HashMap<String, Wav> = HashMap::new();
    let mut load = |id: &str| -> Res<Wav> {
        if let Some(w) = cache.get(id) {
            return Ok(w.clone());
        }
        let p = clips_dir(dir).join(format!("{id}.wav"));
        let w = decode(&std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?)
            .ok_or_else(|| format!("{} is not a clip this tool wrote", p.display()))?;
        cache.insert(id.to_string(), w.clone());
        Ok(w)
    };
    let scratch = dir.join(".pairs-play.wav");
    let sides = [("a", "a"), ("b", "b")];
    let ratings = [("1", "1"), ("2", "2"), ("3", "3"), ("4", "4"), ("5", "5")];
    for t in &trials {
        if file.has(&t.id) {
            continue;
        }
        let pair = &m.pairs[t.pair];
        let (target, answer) = (load(&pair.plan.target_clip)?, load(&pair.plan.answer_clip)?);
        let (a, b) = if t.target_first {
            (target, answer)
        } else {
            (answer, target)
        };
        let x = match t.x_is {
            Side::A => a.clone(),
            Side::B => b.clone(),
        };
        let header = format!("[{}/{}]", t.n, trials.len());
        let mut spec = AskSpec {
            header,
            clips: vec![("A", &a), ("B", &b), ("X", &x)],
            gap_s: GAP_S,
            prompt: "X is A or B?  a / b   (ra rb rx r = replay, q = quit): ",
            answers: &sides,
        };
        let started = Instant::now();
        let quit = |file: &ResultsFile, out: &mut dyn Write| -> Res<SessionEnd> {
            writeln!(
                out,
                "saved {} answers; run the same command to resume.",
                file.answers.len()
            )?;
            Ok(SessionEnd::Quit)
        };
        let (x_said, r1) = match ask(&spec, player, &scratch, lines, out)? {
            Reply::Quit => return quit(&file, out),
            Reply::Answer { value, replays, .. } => (value, replays),
        };
        spec.prompt = "How similar are A and B?  1 = very different .. 5 = indistinguishable  (r ra rb rx, q): ";
        spec.answers = &ratings;
        let (rating, r2, ms) = match prompt(&spec, started, player, &scratch, lines, out)? {
            Reply::Quit => return quit(&file, out),
            Reply::Answer { value, replays, ms } => (value.parse::<u8>().unwrap_or(0), replays, ms),
        };
        file.answers.push(PairAnswer {
            n: t.n,
            id: t.id.clone(),
            rating,
            x_correct: (pair.plan.role != PairRole::Same).then(|| x_said == t.x_is.key()),
            x: x_said,
            replays: r1 + r2,
            ms,
            pair: t.pair,
            role: pair.plan.role,
            level: pair.plan.level,
            kind: pair.kind,
            distance: pair.measured.distance,
            jnd: pair.measured.jnd,
            percent: pair.measured.percent,
        });
        write_json(&path, &file)?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "Session {session} done: {} answers saved to {}.",
        file.answers.len(),
        path.display()
    )?;
    Ok(SessionEnd::Finished)
}

fn answered(dir: &Path, m: &Manifest, session: u32) -> Res<usize> {
    Ok(load_results(dir, m, session)?.map_or(0, |f| f.answers.len()))
}

fn print_status(dir: &Path, m: &Manifest, out: &mut dyn Write) -> Res<()> {
    writeln!(out, "sessions ({} trials each):", m.session_trials(1).len())?;
    for s in 1..=SESSIONS {
        writeln!(
            out,
            "  session {s}: {}/{} answered{}",
            answered(dir, m, s)?,
            m.session_trials(s).len(),
            if s == 2 { " (optional retest)" } else { "" }
        )?;
    }
    let d = dir.display();
    let done = |s: u32| -> Res<bool> { Ok(answered(dir, m, s)? >= m.session_trials(s).len()) };
    if !done(1)? {
        writeln!(out, "Next: `enth-listen pairs --dir {d} --session 1`")?;
    } else if !done(2)? {
        writeln!(
            out,
            "Next: `enth-listen pairs report --dir {d}` (and, on another day, `--session 2` for the retest)"
        )?;
    } else {
        writeln!(
            out,
            "Both sessions are answered. Next: `enth-listen pairs report --dir {d}`"
        )?;
    }
    Ok(())
}

/// Plausible answers for `--dry-run`, from `sound/1`'s own curve on the rating scale (z50 = 5.11, so rating 4.5 at
/// the pass mark) with rating noise of 0.5; X is named right with probability `0.5 + 0.5 z² / (1 + z²)` (75 % at
/// one JND). Identical pairs are rated 5 and their X is a coin flip; obviously different ones are rated 1.
#[must_use]
pub fn synthetic_results(m: &Manifest, session: u32, seed: u64) -> ResultsFile {
    let mut rng = Rng::new(seed ^ (u64::from(session) << 40) ^ 0x5a_0000_0000);
    let c = m.curve_z50;
    let q = m.curve_k * libm::log(7.0) / libm::log(9.0);
    let mut f = ResultsFile::new(m, session, true);
    for t in m.session_trials(session) {
        let pair = &m.pairs[t.pair];
        let z = pair.measured.jnd;
        let normal = {
            let (u1, u2) = (rng.uniform().max(1e-12), rng.uniform());
            libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(2.0 * core::f64::consts::PI * u2)
        };
        let (rating, p_right) = match pair.plan.role {
            PairRole::Same => (5.0, 0.5),
            PairRole::Different => (1.0, 0.98),
            PairRole::Test => (
                rating_model(z, c, q) + 0.5 * normal,
                0.5 + 0.5 * z * z / (1.0 + z * z),
            ),
        };
        let right = rng.uniform() < p_right;
        let said = if right == (t.x_is == Side::A) {
            "a"
        } else {
            "b"
        };
        f.answers.push(PairAnswer {
            n: t.n,
            id: t.id.clone(),
            rating: rating.round().clamp(1.0, 5.0) as u8,
            x: said.to_string(),
            x_correct: (pair.plan.role != PairRole::Same).then_some(right),
            replays: 0,
            ms: 0,
            pair: t.pair,
            role: pair.plan.role,
            level: pair.plan.level,
            kind: pair.kind,
            distance: pair.measured.distance,
            jnd: z,
            percent: pair.measured.percent,
        });
    }
    f
}

fn run_report(dir: &Path, m: &Manifest, bootstrap: usize, out: &mut dyn Write) -> Res<()> {
    let mut files = Vec::new();
    for s in 1..=SESSIONS {
        if let Some(f) = load_results(dir, m, s)? {
            files.push(f);
        }
    }
    if files.is_empty() {
        return Err(format!(
            "no results in {} yet; play a session first (`enth-listen pairs --dir {} --session 1`)",
            dir.display(),
            dir.display()
        )
        .into());
    }
    let seed = m.config.seed ^ 0xb007;
    let report = analyze(m, &files, bootstrap, seed);
    write!(out, "{}", report.to_text())?;
    let rp = dir.join("pairs-report.json");
    write_json(&rp, &report)?;
    let fp = dir.join("pairs-fit.json");
    write_json(&fp, &fit_rows(m, &files, bootstrap, seed))?;
    writeln!(
        out,
        "wrote {} ({SCHEMA_REPORT}) and {} ({SCHEMA_FIT})",
        rp.display(),
        fp.display()
    )?;
    Ok(())
}

/// Run `enth-listen pairs`. `player` replaces the command player (tests); `lines` is where answers are typed.
///
/// # Errors
/// A bad directory or file, a failing player or design.
pub fn run(
    args: &PairsArgs,
    out: &mut dyn Write,
    lines: &mut dyn LineSource,
    player: Option<&mut dyn Player>,
) -> Res<()> {
    let dir = if args.dry_run {
        args.dir.join("dry-run")
    } else {
        args.dir.clone()
    };
    if args.action == Some(PairsAction::Report) {
        let m = load_manifest(&dir)?.ok_or_else(|| {
            format!(
                "no design in {}; run `enth-listen pairs --dir {}` first",
                dir.display(),
                args.dir.display()
            )
        })?;
        return run_report(&dir, &m, args.bootstrap, out);
    }
    let m = prepare(&dir, args, out)?;
    if args.dry_run {
        writeln!(
            out,
            "dry run: nothing is played; both sessions are answered with synthetic answers"
        )?;
        for s in 1..=SESSIONS {
            write_json(
                &results_path(&dir, s),
                &synthetic_results(&m, s, m.config.seed),
            )?;
        }
        return run_report(&dir, &m, args.bootstrap, out);
    }
    if let Some(s) = args.session {
        let mut own;
        let player: &mut dyn Player = if let Some(p) = player {
            p
        } else {
            own = CommandPlayer::parse(&args.player)?;
            writeln!(out, "playing with: {}", own.command_line())?;
            &mut own
        };
        if run_session(&dir, &m, s, player, lines, out)? == SessionEnd::Finished {
            print_status(&dir, &m, out)?;
        }
    } else {
        print_status(&dir, &m, out)?;
    }
    Ok(())
}
