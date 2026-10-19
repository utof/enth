//! The reverb A/B command: prepare (design once, then reuse), run the session, analyse, dry-run.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use spike_metric::rng::Rng;

use super::analyze::analyze;
use super::design::design;
use super::{Candidate, Config, GAP_S, Manifest, SCHEMA_ANSWERS, SCHEMA_MANIFEST, SCHEMA_REPORT};
use crate::Res;
use crate::files::{read_json, write_json};
use crate::player::{CommandPlayer, DEFAULT_PLAYER, Player};
use crate::session::{Answer, AnswerFile, AskSpec, LineSource, Reply, ask, load_answers, record};
use crate::wav::{Wav, decode, encode};

/// Arguments of `enth-listen reverb-ab`.
#[derive(clap::Args, Clone, Debug)]
pub struct ReverbArgs {
    /// Where the clips, manifest and answers live.
    #[arg(long, default_value = "reverb-ab-test")]
    pub dir: PathBuf,
    /// Report preference with confidence from the answer file instead of listening.
    #[arg(long)]
    pub analyze: bool,
    /// Design in DIR/dry-run, answer with synthetic answers, analyse. Plays nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Command that plays one WAV file (the path is appended).
    #[arg(long, default_value = DEFAULT_PLAYER)]
    pub player: String,
    /// Seeds the engine renders and the trial design. A directory keeps the seed it was designed with.
    #[arg(long, default_value_t = 20_261_019)]
    pub seed: u64,
    /// The decay all three reverbs are calibrated to (RT60, seconds).
    #[arg(long, default_value_t = 2.0)]
    pub rt60: f64,
    /// Our reverb's wet level in dB (the catalogue default); the fundsp tails are matched to it in loudness.
    #[arg(long, default_value_t = -6.0, allow_negative_numbers = true)]
    pub wet_db: f64,
    /// Repeats of every source and pair of reverbs.
    #[arg(long, default_value_t = 4)]
    pub repeats: usize,
    /// 16 kHz and shorter sources (for tests, where fundsp runs unoptimised).
    #[arg(long, hide = true)]
    pub quick: bool,
}

impl ReverbArgs {
    fn config(&self) -> Config {
        Config {
            seed: self.seed,
            sample_rate: if self.quick { 16_000 } else { 48_000 },
            rt60_s: self.rt60,
            wet_db: self.wet_db,
            repeats: self.repeats,
            tail_rt60s: if self.quick { 1.0 } else { 1.5 },
        }
    }
}

fn clip_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("clips").join(format!("{id}.wav"))
}

fn answers_path(dir: &Path) -> PathBuf {
    dir.join("answers-session1.json")
}

fn prepare(dir: &Path, cfg: &Config, quick: bool, out: &mut dyn Write) -> Res<Manifest> {
    std::fs::create_dir_all(dir)?;
    let mp = dir.join("manifest.json");
    if mp.exists() {
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
            "using the stimuli designed earlier in {} (RT60 {} s, {} trials)",
            dir.display(),
            m.config.rt60_s,
            m.trials.len()
        )?;
        let missing = m
            .clips
            .iter()
            .filter(|c| !clip_path(dir, &c.id).exists())
            .count();
        if missing > 0 {
            return Err(format!(
                "{missing} clip file(s) are missing from {}; delete the folder to design again",
                dir.join("clips").display()
            )
            .into());
        }
        return Ok(m);
    }
    let d = design(cfg, quick, out)?;
    std::fs::create_dir_all(dir.join("clips"))?;
    for (id, wav) in &d.clips {
        std::fs::write(clip_path(dir, id), encode(wav))?;
    }
    write_json(&mp, &d.manifest)?;
    writeln!(
        out,
        "designed {} clips and {} trials; clips in {}",
        d.manifest.clips.len(),
        d.manifest.trials.len(),
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
    Ok(d.manifest)
}

fn run_session(
    dir: &Path,
    m: &Manifest,
    player: &mut dyn Player,
    lines: &mut dyn LineSource,
    out: &mut dyn Write,
) -> Res<()> {
    let path = answers_path(dir);
    let mut file = load_answers(&path, SCHEMA_ANSWERS, 1)?
        .unwrap_or_else(|| AnswerFile::new(SCHEMA_ANSWERS, 1, false));
    if file.synthetic {
        return Err(format!(
            "{} holds synthetic answers; use a different --dir",
            path.display()
        )
        .into());
    }
    let total = m.trials.len();
    writeln!(out)?;
    writeln!(
        out,
        "Reverb A/B: {total} trials, about {} minutes.",
        (total * 25).div_ceil(60)
    )?;
    writeln!(
        out,
        "  - Use headphones. Set the volume once and do not touch it again."
    )?;
    writeln!(
        out,
        "  - Each trial plays the same sound twice, A then B, with a different reverb on each."
    )?;
    writeln!(
        out,
        "    Say which reverb you prefer:  a  or  b.  If you truly cannot choose: =  (use it sparingly)."
    )?;
    writeln!(
        out,
        "  - Judge the reverb, not the loudness: the clips are level-matched. Go with your first impression."
    )?;
    writeln!(
        out,
        "  - ra / rb replay one sound, r replays both, q quits (progress is saved; run the same command to resume)."
    )?;
    writeln!(out, "  - Do not open manifest.json: it holds the answers.")?;
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
    let choices = [("a", "a"), ("b", "b"), ("=", "="), ("n", "=")];
    for t in &m.trials {
        if file.has(&t.id) {
            continue;
        }
        let (a, b) = (load(&t.a)?, load(&t.b)?);
        let spec = AskSpec {
            header: format!("[{}/{total}]", t.n),
            clips: vec![("A", &a), ("B", &b)],
            gap_s: GAP_S,
            prompt: "prefer A or B?  a / b / = (no preference)   (ra rb r = replay, q = quit): ",
            answers: &choices,
        };
        match ask(&spec, player, &scratch, lines, out)? {
            Reply::Quit => {
                writeln!(
                    out,
                    "saved {} answers; run the same command to resume.",
                    file.answers.len()
                )?;
                return Ok(());
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
        "Done: {} answers saved to {}. Next: `enth-listen reverb-ab --dir {} --analyze`",
        file.answers.len(),
        path.display(),
        dir.display()
    )?;
    Ok(())
}

/// Plausible answers for `--dry-run`: a hidden quality per reverb (ours 0, reverb2 -0.35, reverb4 +0.25) plus
/// noise decides each trial; close calls become "=".
#[must_use]
pub fn synthetic_answers(m: &Manifest, seed: u64) -> AnswerFile {
    let mut rng = Rng::new(seed ^ 0x7e7e_0000_0000_0001);
    let quality = |c: Candidate| match c {
        Candidate::Ours => 0.0,
        Candidate::Reverb2 => -0.35,
        Candidate::Reverb4 => 0.25,
    };
    let mut f = AnswerFile::new(SCHEMA_ANSWERS, 1, true);
    for t in &m.trials {
        let d = quality(t.a_candidate) - quality(t.b_candidate) + 0.8 * rng.normal();
        let choice = if d.abs() < 0.15 {
            "="
        } else if d > 0.0 {
            "a"
        } else {
            "b"
        };
        f.answers.push(Answer {
            n: t.n,
            id: t.id.clone(),
            choice: choice.into(),
            replays: 0,
            ms: 0,
        });
    }
    f
}

fn run_analysis(dir: &Path, m: &Manifest, out: &mut dyn Write) -> Res<()> {
    let Some(file) = load_answers(&answers_path(dir), SCHEMA_ANSWERS, 1)? else {
        return Err("no answers yet; run `enth-listen reverb-ab --dir <dir>` first".into());
    };
    let report = analyze(m, &[file]);
    write!(out, "{}", report.to_text())?;
    let p = dir.join("reverb-report.json");
    write_json(&p, &report)?;
    writeln!(out, "wrote {} ({})", p.display(), SCHEMA_REPORT)?;
    Ok(())
}

/// Run `enth-listen reverb-ab`. `player` replaces the command player (tests); `lines` is where answers are typed.
///
/// # Errors
/// A bad directory or file, a failing player, a render failure.
pub fn run(
    args: &ReverbArgs,
    out: &mut dyn Write,
    lines: &mut dyn LineSource,
    player: Option<&mut dyn Player>,
) -> Res<()> {
    if args.repeats == 0 || !(args.rt60.is_finite() && args.rt60 > 0.05) {
        return Err("--repeats must be at least 1 and --rt60 a positive time in seconds".into());
    }
    let dir = if args.dry_run {
        args.dir.join("dry-run")
    } else {
        args.dir.clone()
    };
    let m = prepare(&dir, &args.config(), args.quick, out)?;
    if args.dry_run {
        writeln!(
            out,
            "dry run: nothing is played; the session is answered with synthetic answers"
        )?;
        write_json(&answers_path(&dir), &synthetic_answers(&m, m.config.seed))?;
        return run_analysis(&dir, &m, out);
    }
    if args.analyze {
        return run_analysis(&dir, &m, out);
    }
    let mut own;
    let player: &mut dyn Player = if let Some(p) = player {
        p
    } else {
        own = CommandPlayer::parse(&args.player)?;
        writeln!(out, "playing with: {}", own.command_line())?;
        &mut own
    };
    run_session(&dir, &m, player, lines, out)
}
