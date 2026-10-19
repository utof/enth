//! The interactive trial loop and the answer files, shared by both tools.
//!
//! One trial is a few labelled clips played in order (`A`, `B`, and for ABX `X`) with a gap between them, then a
//! prompt. The owner can replay everything (`r`) or one clip (`ra`, `rb`, `rx`), answer, or quit (`q`); progress is
//! saved after every answer, so the same command resumes. Playback goes through a [`Player`]; the clips are
//! assembled in memory and written to one scratch file per play.

use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::Res;
use crate::files::{read_json, write_json};
use crate::player::Player;
use crate::wav::{Wav, assemble, encode};

/// One answered trial.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    /// 1-based position in the session.
    pub n: usize,
    /// The trial's random id.
    pub id: String,
    /// What the owner chose: `a`, `b` (or `=` in the reverb test).
    pub choice: String,
    /// How many times a sound was replayed.
    pub replays: u32,
    /// Milliseconds from the first playback to the answer.
    pub ms: u64,
}

/// The answers of one session, saved after every answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerFile {
    /// Schema id, per tool.
    pub schema: String,
    /// Session number (1-based).
    pub session: u32,
    /// True for the `--dry-run` answers (generated, never the owner's).
    pub synthetic: bool,
    /// The answers, in the order given.
    pub answers: Vec<Answer>,
}

impl AnswerFile {
    /// An empty file.
    #[must_use]
    pub fn new(schema: &str, session: u32, synthetic: bool) -> Self {
        Self {
            schema: schema.to_string(),
            session,
            synthetic,
            answers: Vec::new(),
        }
    }

    /// Whether trial `id` has an answer.
    #[must_use]
    pub fn has(&self, id: &str) -> bool {
        self.answers.iter().any(|a| a.id == id)
    }
}

/// Load an answer file, or `None` when it does not exist yet.
///
/// # Errors
/// A file that cannot be read or parsed, for the wrong schema or the wrong session.
pub fn load_answers(path: &Path, schema: &str, session: u32) -> Res<Option<AnswerFile>> {
    if !path.exists() {
        return Ok(None);
    }
    let f: AnswerFile = read_json(path)?;
    if f.schema != schema {
        return Err(format!(
            "{} has schema {}, expected {schema}",
            path.display(),
            f.schema
        )
        .into());
    }
    if f.session != session {
        return Err(format!(
            "{} is for session {}, not {session}",
            path.display(),
            f.session
        )
        .into());
    }
    Ok(Some(f))
}

/// A source of typed lines: stdin for the owner, a script for tests.
pub trait LineSource {
    /// The next line, trimmed and lower-cased; `None` at the end of input.
    fn read_line(&mut self) -> Option<String>;
}

/// Lines from standard input.
#[derive(Debug, Default)]
pub struct StdinLines;

impl LineSource for StdinLines {
    fn read_line(&mut self) -> Option<String> {
        let mut s = String::new();
        match std::io::stdin().lock().read_line(&mut s) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(s.trim().to_lowercase()),
        }
    }
}

/// Lines from a fixed script.
#[derive(Debug, Default)]
pub struct Scripted(pub std::collections::VecDeque<String>);

impl Scripted {
    /// A script from the given lines.
    #[must_use]
    pub fn new(lines: &[&str]) -> Self {
        Self(lines.iter().map(|s| s.trim().to_lowercase()).collect())
    }
}

impl LineSource for Scripted {
    fn read_line(&mut self) -> Option<String> {
        self.0.pop_front()
    }
}

/// One trial to present.
#[derive(Debug)]
pub struct AskSpec<'a> {
    /// Shown before playing, e.g. `[3/24]`.
    pub header: String,
    /// The clips in play order with their labels (`A`, `B`, `X`).
    pub clips: Vec<(&'a str, &'a Wav)>,
    /// Silence between clips.
    pub gap_s: f32,
    /// The prompt text, ending where the owner types.
    pub prompt: &'a str,
    /// Accepted answers: what is typed, and what is stored.
    pub answers: &'a [(&'a str, &'a str)],
}

/// How a trial ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    /// An answer.
    Answer {
        /// The stored value.
        value: String,
        /// Replays used.
        replays: u32,
        /// Milliseconds from the first playback.
        ms: u64,
    },
    /// The owner quit (or the input ended).
    Quit,
}

fn play_clips(
    clips: &[&Wav],
    gap_s: f32,
    player: &mut dyn Player,
    scratch: &Path,
) -> Result<(), String> {
    let wav = assemble(clips, gap_s);
    std::fs::write(scratch, encode(&wav)).map_err(|e| format!("{}: {e}", scratch.display()))?;
    player.play(scratch)
}

/// Present one trial: play it, then read commands until an answer or a quit.
///
/// # Errors
/// The player failed or the scratch file could not be written.
pub fn ask(
    spec: &AskSpec<'_>,
    player: &mut dyn Player,
    scratch: &Path,
    lines: &mut dyn LineSource,
    out: &mut dyn Write,
) -> Result<Reply, String> {
    let io = |e: std::io::Error| e.to_string();
    let all: Vec<&Wav> = spec.clips.iter().map(|(_, w)| *w).collect();
    let started = Instant::now();
    let mut replays = 0u32;
    writeln!(out).map_err(io)?;
    writeln!(out, "{} playing ...", spec.header).map_err(io)?;
    play_clips(&all, spec.gap_s, player, scratch)?;
    loop {
        write!(out, "{}", spec.prompt).map_err(io)?;
        out.flush().map_err(io)?;
        let Some(line) = lines.read_line() else {
            return Ok(Reply::Quit);
        };
        if line == "q" {
            return Ok(Reply::Quit);
        }
        if let Some((_, stored)) = spec.answers.iter().find(|(typed, _)| *typed == line) {
            return Ok(Reply::Answer {
                value: (*stored).to_string(),
                replays,
                ms: started.elapsed().as_millis() as u64,
            });
        }
        if line == "r" {
            replays += 1;
            play_clips(&all, spec.gap_s, player, scratch)?;
            continue;
        }
        let one = line.strip_prefix('r').and_then(|l| {
            spec.clips
                .iter()
                .find(|(label, _)| label.eq_ignore_ascii_case(l))
        });
        if let Some((_, w)) = one {
            replays += 1;
            play_clips(&[*w], spec.gap_s, player, scratch)?;
        } else {
            let keys: Vec<&str> = spec.answers.iter().map(|(t, _)| *t).collect();
            writeln!(
                out,
                "type {}, r, one of the r-commands, or q",
                keys.join(", ")
            )
            .map_err(io)?;
        }
    }
}

/// Append an answer to the file and save it.
///
/// # Errors
/// The file cannot be written.
pub fn record(file: &mut AnswerFile, path: &Path, answer: Answer) -> Res<()> {
    file.answers.push(answer);
    write_json(path, file)
}

/// A recorder for tests and dry runs: remembers what was played and never makes a sound.
#[derive(Debug, Default)]
pub struct MockPlayer {
    /// The length in bytes of each file played.
    pub played: Vec<u64>,
}

impl Player for MockPlayer {
    fn play(&mut self, wav: &Path) -> Result<(), String> {
        let len = std::fs::metadata(wav).map_err(|e| e.to_string())?.len();
        self.played.push(len);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("enth-listen-session-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn spec<'a>(
        a: &'a Wav,
        b: &'a Wav,
        x: &'a Wav,
        answers: &'a [(&'a str, &'a str)],
    ) -> AskSpec<'a> {
        AskSpec {
            header: "[1/1]".into(),
            clips: vec![("A", a), ("B", b), ("X", x)],
            gap_s: 0.1,
            prompt: "> ",
            answers,
        }
    }

    #[test]
    fn replays_are_counted_and_the_answer_is_stored() {
        let d = dir("ask");
        let w = Wav::mono(8_000, vec![0.1; 800]);
        let answers = [("a", "a"), ("b", "b")];
        let mut player = MockPlayer::default();
        let mut out = Vec::new();
        let mut lines = Scripted::new(&["r", "rx", "RB", "huh", "b"]);
        let r = ask(
            &spec(&w, &w, &w, &answers),
            &mut player,
            &d.join("p.wav"),
            &mut lines,
            &mut out,
        )
        .unwrap();
        match r {
            Reply::Answer { value, replays, .. } => assert_eq!((value.as_str(), replays), ("b", 3)),
            Reply::Quit => panic!("expected an answer"),
        }
        // first play of all three (3 clips + 2 gaps), a replay of all, then two single clips
        let all = 44 + (800 * 3 + 2 * 800) * 3;
        let one = 44 + 800 * 3;
        assert_eq!(player.played, vec![all, all, one, one]);
        assert!(String::from_utf8(out).unwrap().contains("type a, b"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn quit_and_end_of_input_both_quit() {
        let d = dir("quit");
        let w = Wav::mono(8_000, vec![0.1; 80]);
        let answers = [("a", "a")];
        for script in [vec!["q"], vec![]] {
            let mut lines = Scripted::new(&script);
            let r = ask(
                &spec(&w, &w, &w, &answers),
                &mut MockPlayer::default(),
                &d.join("p.wav"),
                &mut lines,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(r, Reply::Quit);
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn answer_files_load_check_and_append() {
        let d = dir("files");
        let p = d.join("answers.json");
        assert!(load_answers(&p, "s/1", 1).unwrap().is_none());
        let mut f = AnswerFile::new("s/1", 1, false);
        record(
            &mut f,
            &p,
            Answer {
                n: 1,
                id: "k1".into(),
                choice: "a".into(),
                replays: 0,
                ms: 5,
            },
        )
        .unwrap();
        let back = load_answers(&p, "s/1", 1).unwrap().unwrap();
        assert!(back.has("k1") && !back.has("k2"));
        assert!(load_answers(&p, "s/2", 1).is_err());
        assert!(load_answers(&p, "s/1", 2).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
