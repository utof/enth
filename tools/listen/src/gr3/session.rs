//! The owner's session: the reference, then A, then B, and "which is closer to the reference?".
//!
//! Resumable (an answer is saved after each one; `q` quits), with the other tools' replay commands: `rr`, `ra`, `rb`
//! replay one sound, `r` all three. The session shows only the trial's number: nothing that names an item, a stratum
//! or an arm (the key stays closed until every listener is done).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::design::{GAP_S, Manifest, OWNER, SCHEMA_ANSWERS, clips_dir};
use crate::Res;
use crate::player::Player;
use crate::session::{Answer, AnswerFile, AskSpec, LineSource, Reply, ask, load_answers, record};
use crate::wav::{Wav, decode};

/// The owner's answers file.
#[must_use]
pub fn answers_path(dir: &Path) -> PathBuf {
    dir.join("answers.json")
}

/// How the session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    /// Every trial is answered.
    Finished,
    /// The owner quit; the answers so far are saved.
    Quit,
}

/// Minutes a session takes: each trial's audio once and a fifth more for replays, plus 7 s to answer.
#[must_use]
pub fn estimate_minutes(m: &Manifest, audience: &str) -> f64 {
    let secs: f64 = m
        .trials_of(audience)
        .iter()
        .map(|t| t.seconds.iter().sum::<f64>() * 1.2 + 2.0 * f64::from(GAP_S) + 7.0)
        .sum();
    secs / 60.0
}

fn print_instructions(out: &mut dyn Write, m: &Manifest) -> std::io::Result<()> {
    let trials = m.trials_of(OWNER).len();
    writeln!(out)?;
    writeln!(
        out,
        "GR3 listening: {trials} trials, about {:.0} minutes.",
        estimate_minutes(m, OWNER)
    )?;
    writeln!(
        out,
        "  - Use headphones. Set the volume once and keep it there: every clip is level-matched."
    )?;
    writeln!(
        out,
        "  - Each trial plays three sounds: R (the reference), then A, then B. A and B are two attempts to copy R."
    )?;
    writeln!(
        out,
        "    Say which is closer to R:  a  or  b.  Go with your first impression; there is no \"same\" answer."
    )?;
    writeln!(
        out,
        "  - A few trials are checks (two identical sounds, or one clearly wrong sound): answer them like the rest."
    )?;
    writeln!(
        out,
        "  - rr / ra / rb replay one sound, r replays all three, q quits (progress is saved after each answer;"
    )?;
    writeln!(out, "    run the same command to resume).")?;
    writeln!(
        out,
        "  - Do not open manifest.json: it holds which sound is which."
    )
}

/// Run the owner's trials. `player` plays each trial assembled into one scratch file.
///
/// # Errors
/// A missing clip, a failing player, or an answers file that is not this design's.
pub fn run_session(
    dir: &Path,
    m: &Manifest,
    player: &mut dyn Player,
    lines: &mut dyn LineSource,
    out: &mut dyn Write,
) -> Res<SessionEnd> {
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
    let trials = m.trials_of(OWNER);
    if let Some(unknown) = file
        .answers
        .iter()
        .find(|a| !trials.iter().any(|t| t.id == a.id))
    {
        return Err(format!(
            "{} belongs to another design (trial {:?} is not in this key); use the folder it was made in",
            path.display(),
            unknown.id
        )
        .into());
    }
    print_instructions(out, m)?;
    if !file.answers.is_empty() {
        writeln!(
            out,
            "  - Resuming after {} answered trials.",
            file.answers.len()
        )?;
    }
    let mut cache: HashMap<String, Wav> = HashMap::new();
    let mut load = |stem: &str| -> Res<Wav> {
        if let Some(w) = cache.get(stem) {
            return Ok(w.clone());
        }
        let p = clips_dir(dir).join(format!("{stem}.wav"));
        let w = decode(&std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?)
            .ok_or_else(|| format!("{} is not a clip this tool wrote", p.display()))?;
        cache.insert(stem.to_string(), w.clone());
        Ok(w)
    };
    let scratch = dir.join(".gr3-play.wav");
    let choices = [("a", "a"), ("b", "b")];
    for t in &trials {
        if file.has(&t.id) {
            continue;
        }
        let (r, a, b) = (load(&t.clips[0])?, load(&t.clips[1])?, load(&t.clips[2])?);
        let spec = AskSpec {
            header: format!("[{}/{}]", t.n, trials.len()),
            clips: vec![("R", &r), ("A", &a), ("B", &b)],
            gap_s: GAP_S,
            prompt: "which is closer to R?  a / b   (rr ra rb r = replay, q = quit): ",
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
        "Done: {} answers saved to {}.",
        file.answers.len(),
        path.display()
    )?;
    Ok(SessionEnd::Finished)
}
