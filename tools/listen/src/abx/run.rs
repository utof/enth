//! The ABX command: prepare (design once, then reuse), run sessions, analyse, dry-run.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use spike_metric::render::RenderCfg;
use spike_metric::rng::Rng;

use super::analyze::analyze;
use super::design::{design, raw_clip};
use super::{
    Config, GAP_S, Manifest, SCHEMA_ANSWERS, SCHEMA_MANIFEST, SCHEMA_REPORT, Side, StopAt,
    TrialKind,
};
use crate::Res;
use crate::files::{read_json, write_json};
use crate::player::{CommandPlayer, DEFAULT_PLAYER, Player};
use crate::session::{Answer, AnswerFile, AskSpec, LineSource, Reply, ask, load_answers, record};
use crate::wav::{Wav, decode, encode};

/// Which sessions to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionSel {
    /// One session.
    One(u32),
    /// A range, inclusive (`1-4`).
    Range(u32, u32),
    /// Every session in turn.
    All,
    /// The first session that is not finished.
    Next,
}

impl FromStr for SessionSel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let bad =
            || format!("{s:?} is not a session: use a number (3), a range (1-4), `next` or `all`");
        match s.trim().to_lowercase().as_str() {
            "all" => Ok(Self::All),
            "next" => Ok(Self::Next),
            t => {
                if let Some((a, b)) = t.split_once('-') {
                    let (a, b): (u32, u32) =
                        (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?);
                    if a == 0 || b < a {
                        return Err(bad());
                    }
                    Ok(Self::Range(a, b))
                } else {
                    let n: u32 = t.parse().map_err(|_| bad())?;
                    if n == 0 { Err(bad()) } else { Ok(Self::One(n)) }
                }
            }
        }
    }
}

/// Arguments of `enth-listen abx`.
#[derive(clap::Args, Clone, Debug)]
pub struct AbxArgs {
    /// Where the clips, manifest and answers live.
    #[arg(long, default_value = "abx-test")]
    pub dir: PathBuf,
    /// Run session(s): a number (3), a range (1-4), `next` (the first unfinished one) or `all`.
    #[arg(long)]
    pub session: Option<SessionSel>,
    /// Report per target (same / different), the G0 count and the controls, from the answer files.
    #[arg(long)]
    pub analyze: bool,
    /// Design in DIR/dry-run, answer every session with synthetic answers, analyse. Plays nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Command that plays one WAV file (the path is appended).
    #[arg(long, default_value = DEFAULT_PLAYER)]
    pub player: String,
    /// Seeds the target list and the trial design. A directory keeps the seed it was designed with.
    #[arg(long, default_value_t = 20_261_019)]
    pub seed: u64,
    /// Number of targets (the G0 gate asks for 20).
    #[arg(long, default_value_t = 20)]
    pub targets: usize,
    /// Trials per target, which is also the number of sessions.
    #[arg(long, default_value_t = 8)]
    pub trials: usize,
    /// Search workers (the G0 gate asks for 8).
    #[arg(long, default_value_t = 8)]
    pub workers: usize,
    /// Wall-clock cap per target, in seconds (the G0 gate asks for 60).
    #[arg(long, default_value_t = 60.0)]
    pub time_cap: f64,
    /// Evaluation cap per search.
    #[arg(long, default_value_t = 5000)]
    pub budget: usize,
    /// Where each search stops: the strict proxy threshold (ADR-0007), the loose one, or only at the caps.
    #[arg(long, value_enum, default_value = "t1")]
    pub stop_at: StopAt,
    /// Significance level of the per-target test (one-sided).
    #[arg(long, default_value_t = 0.05)]
    pub alpha: f64,
    /// 0.5 s clips and a tiny search budget (for tests).
    #[arg(long, hide = true)]
    pub quick: bool,
}

impl AbxArgs {
    fn config(&self) -> Config {
        let render = if self.quick {
            RenderCfg::short()
        } else {
            RenderCfg::default()
        };
        Config {
            seed: self.seed,
            n_targets: self.targets,
            trials_per_target: self.trials,
            n_null: 2,
            n_diff: 2,
            workers: self.workers.max(1),
            time_cap_s: self.time_cap,
            budget: if self.quick {
                self.budget.min(64)
            } else {
                self.budget
            },
            stop_at: self.stop_at,
            lambda: spike_metric::search::lambda_for_workers(self.workers.max(1)),
            sigma_start: 0.15,
            search_seed: 1000,
            render,
        }
    }
}

/// Where a clip lives.
#[must_use]
pub fn clip_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("clips").join(format!("{id}.wav"))
}

fn answers_path(dir: &Path, session: u32) -> PathBuf {
    dir.join(format!("answers-session{session}.json"))
}

/// Re-render any clip file that is missing (deterministic: the manifest holds the parameters and the gains).
///
/// # Errors
/// A file that cannot be written.
pub fn ensure_clips(dir: &Path, m: &Manifest) -> Res<usize> {
    std::fs::create_dir_all(dir.join("clips"))?;
    let sr = m.config.render.sample_rate as u32;
    let mut made = 0;
    for c in &m.clips {
        let p = clip_path(dir, &c.id);
        if p.exists() {
            continue;
        }
        let item = &m.items[c.item];
        let params = match c.role {
            super::Role::Target => &item.target,
            super::Role::Solution => &item.solution,
        };
        let audio = raw_clip(&m.config.render, params);
        let wav = Wav::mono(
            sr,
            audio
                .iter()
                .map(|v| (f64::from(*v) * c.gain) as f32)
                .collect(),
        );
        std::fs::write(&p, encode(&wav))?;
        made += 1;
    }
    Ok(made)
}

/// Load the design in `dir`, or make it (solve, clips, plan) and save it.
fn prepare(dir: &Path, cfg: &Config, out: &mut dyn Write) -> Res<Manifest> {
    std::fs::create_dir_all(dir)?;
    let mp = dir.join("manifest.json");
    let manifest: Manifest = if mp.exists() {
        let m: Manifest = read_json(&mp)?;
        if m.schema != SCHEMA_MANIFEST {
            return Err(format!(
                "{} has schema {}, expected {SCHEMA_MANIFEST}",
                mp.display(),
                m.schema
            )
            .into());
        }
        writeln!(
            out,
            "using the stimuli designed earlier in {} ({} targets, seed {}, {} sessions)",
            dir.display(),
            m.items.len(),
            m.config.seed,
            m.sessions()
        )?;
        m
    } else {
        let d = design(cfg, out)?;
        std::fs::create_dir_all(dir.join("clips"))?;
        for (id, wav) in &d.clips {
            std::fs::write(clip_path(dir, id), encode(wav))?;
        }
        write_json(&mp, &d.manifest)?;
        writeln!(
            out,
            "designed {} targets in {:.0} s; {} sessions of {} trials; clips in {}",
            d.manifest.items.len(),
            d.manifest.design_wall_s,
            d.manifest.sessions(),
            d.manifest.session_trials(1).len(),
            dir.join("clips").display()
        )?;
        let flagged = d
            .manifest
            .clips
            .iter()
            .filter(|c| !c.flags.is_empty())
            .count();
        if flagged > 0 {
            writeln!(
                out,
                "{flagged} clip(s) have click or discontinuity flags (kept; listed in the report)"
            )?;
        }
        d.manifest
    };
    let remade = ensure_clips(dir, &manifest)?;
    if remade > 0 {
        writeln!(out, "re-rendered {remade} missing clip file(s)")?;
    }
    Ok(manifest)
}

fn print_instructions(
    out: &mut dyn Write,
    session: u32,
    total: u32,
    trials: usize,
) -> std::io::Result<()> {
    writeln!(out)?;
    writeln!(
        out,
        "ABX, session {session} of {total}: {trials} trials, about {} minutes.",
        (trials * 15).div_ceil(60)
    )?;
    writeln!(
        out,
        "  - Use headphones. Set the volume once and keep it the same in every session."
    )?;
    writeln!(
        out,
        "  - Each trial plays three sounds: A, B, then X. X is an exact copy of A or of B."
    )?;
    writeln!(out, "    Decide which one X is:  a = X is A,  b = X is B.")?;
    writeln!(
        out,
        "  - Some trials have nothing to hear (A and B are the same sound): guess. That is expected."
    )?;
    writeln!(
        out,
        "  - ra / rb / rx replay one sound, r replays all three, q quits (progress is saved;"
    )?;
    writeln!(out, "    run the same command to resume).")?;
    writeln!(out, "  - Do not open manifest.json: it holds the answers.")
}

/// Whether a session ended or the owner quit in the middle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    /// Every trial of the session is answered.
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
    let path = answers_path(dir, session);
    let mut file = load_answers(&path, SCHEMA_ANSWERS, session)?
        .unwrap_or_else(|| AnswerFile::new(SCHEMA_ANSWERS, session, false));
    if file.synthetic {
        return Err(format!(
            "{} holds synthetic answers; use a different --dir",
            path.display()
        )
        .into());
    }
    print_instructions(out, session, m.sessions(), trials.len())?;
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
        let p = clip_path(dir, id);
        let w = decode(&std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?)
            .ok_or_else(|| format!("{} is not a clip this tool wrote", p.display()))?;
        cache.insert(id.to_string(), w.clone());
        Ok(w)
    };
    let scratch = dir.join(".play.wav");
    let choices = [("a", "a"), ("b", "b")];
    for t in &trials {
        if file.has(&t.id) {
            continue;
        }
        let (a, b) = (load(&t.a)?, load(&t.b)?);
        let x = match t.x_is {
            Side::A => a.clone(),
            Side::B => b.clone(),
        };
        let spec = AskSpec {
            header: format!("[{}/{}]", t.n, trials.len()),
            clips: vec![("A", &a), ("B", &b), ("X", &x)],
            gap_s: GAP_S,
            prompt: "X is A or B?  a / b   (ra rb rx r = replay, q = quit): ",
            answers: &choices,
        };
        match ask(&spec, player, &scratch, lines, out)? {
            Reply::Quit => {
                writeln!(
                    out,
                    "saved {} answers; run the same command to resume.",
                    file.answers.len()
                )?;
                return Ok(SessionEnd::Quit);
            }
            Reply::Answer { value, replays, ms } => {
                record(
                    &mut file,
                    &path,
                    Answer {
                        n: t.n,
                        id: t.id.clone(),
                        choice: value,
                        replays,
                        ms,
                    },
                )?;
            }
        }
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

fn answered_in(dir: &Path, m: &Manifest, session: u32) -> Res<usize> {
    let n = load_answers(&answers_path(dir, session), SCHEMA_ANSWERS, session)?
        .map_or(0, |f| f.answers.len());
    Ok(n.min(m.session_trials(session).len()))
}

fn print_status(dir: &Path, m: &Manifest, out: &mut dyn Write) -> Res<()> {
    writeln!(out, "sessions ({} trials each):", m.session_trials(1).len())?;
    let mut next = None;
    for s in 1..=m.sessions() {
        let (done, total) = (answered_in(dir, m, s)?, m.session_trials(s).len());
        if done < total && next.is_none() {
            next = Some(s);
        }
        writeln!(out, "  session {s}: {done}/{total} answered")?;
    }
    match next {
        Some(_) => writeln!(
            out,
            "Next: `enth-listen abx --dir {} --session next`",
            dir.display()
        )?,
        None => writeln!(
            out,
            "All sessions are answered. Next: `enth-listen abx --dir {} --analyze`",
            dir.display()
        )?,
    }
    Ok(())
}

/// Plausible answers for `--dry-run`: the chance of a correct answer rises from chance (0.52) at the strict
/// threshold T1 to 0.98 at three times T1; the null trials are coin flips and the obviously different ones are easy.
#[must_use]
pub fn synthetic_answers(m: &Manifest, session: u32, seed: u64) -> AnswerFile {
    let mut rng = Rng::new(seed ^ (u64::from(session) << 40) ^ 0xab_0000_0000);
    let mut f = AnswerFile::new(SCHEMA_ANSWERS, session, true);
    for t in m.session_trials(session) {
        let q = match t.kind {
            TrialKind::Real => {
                let it = &m.items[t.item];
                let r = it.distance / it.t1.max(1e-12);
                if r <= 1.0 {
                    0.52
                } else {
                    0.52 + 0.46 * (libm::log(r) / libm::log(3.0)).min(1.0)
                }
            }
            TrialKind::Null => 0.5,
            TrialKind::Different => 0.98,
        };
        let right = rng.uniform() < q;
        let choice = if right {
            t.x_is
        } else if t.x_is == Side::A {
            Side::B
        } else {
            Side::A
        };
        f.answers.push(Answer {
            n: t.n,
            id: t.id.clone(),
            choice: choice.key().to_string(),
            replays: 0,
            ms: 0,
        });
    }
    f
}

fn run_analysis(dir: &Path, m: &Manifest, alpha: f64, out: &mut dyn Write) -> Res<()> {
    let mut files = Vec::new();
    for s in 1..=m.sessions() {
        if let Some(f) = load_answers(&answers_path(dir, s), SCHEMA_ANSWERS, s)? {
            files.push(f);
        }
    }
    if files.is_empty() {
        return Err("no answer files yet; run a session first (--session next)".into());
    }
    let report = analyze(m, &files, alpha);
    write!(out, "{}", report.to_text())?;
    let p = dir.join("abx-report.json");
    write_json(&p, &report)?;
    writeln!(out, "wrote {} ({})", p.display(), SCHEMA_REPORT)?;
    Ok(())
}

/// Run `enth-listen abx`. `player` replaces the command player (tests); `lines` is where answers are typed.
///
/// # Errors
/// A bad directory or file, a failing player, or a request with nothing to do.
pub fn run(
    args: &AbxArgs,
    out: &mut dyn Write,
    lines: &mut dyn LineSource,
    player: Option<&mut dyn Player>,
) -> Res<()> {
    let cfg = args.config();
    if cfg.n_targets == 0 || cfg.trials_per_target == 0 {
        return Err("--targets and --trials must be at least 1".into());
    }
    let dir = if args.dry_run {
        args.dir.join("dry-run")
    } else {
        args.dir.clone()
    };
    if crate::stats::critical_correct(cfg.trials_per_target, args.alpha).is_none() {
        writeln!(
            out,
            "warning: with {} trials per target, no target can be called different at alpha {}; the G0 verdict will read INVALID (use --trials 6 or more)",
            cfg.trials_per_target, args.alpha
        )?;
    }
    let m = prepare(&dir, &cfg, out)?;
    if args.dry_run {
        writeln!(
            out,
            "dry run: nothing is played; every session is answered with synthetic answers"
        )?;
        for s in 1..=m.sessions() {
            write_json(
                &answers_path(&dir, s),
                &synthetic_answers(&m, s, m.config.seed),
            )?;
        }
        return run_analysis(&dir, &m, args.alpha, out);
    }
    if let Some(sel) = args.session {
        let all: Vec<u32> = (1..=m.sessions()).collect();
        let todo: Vec<u32> = match sel {
            SessionSel::One(n) => vec![n],
            SessionSel::Range(a, b) => (a..=b).collect(),
            SessionSel::All => all.clone(),
            SessionSel::Next => {
                let mut next = Vec::new();
                for s in &all {
                    if answered_in(&dir, &m, *s)? < m.session_trials(*s).len() {
                        next.push(*s);
                        break;
                    }
                }
                next
            }
        };
        if let Some(bad) = todo.iter().find(|s| !all.contains(s)) {
            return Err(format!(
                "session {bad} does not exist: this design has sessions 1 to {}",
                m.sessions()
            )
            .into());
        }
        if todo.is_empty() {
            writeln!(out, "every session is finished; run with --analyze")?;
        }
        let mut own;
        let player: &mut dyn Player = if let Some(p) = player {
            p
        } else {
            own = CommandPlayer::parse(&args.player)?;
            writeln!(out, "playing with: {}", own.command_line())?;
            &mut own
        };
        for s in todo {
            if run_session(&dir, &m, s, player, lines, out)? == SessionEnd::Quit {
                break;
            }
        }
    }
    if args.analyze {
        run_analysis(&dir, &m, args.alpha, out)?;
    }
    if args.session.is_none() && !args.analyze {
        print_status(&dir, &m, out)?;
    }
    Ok(())
}
