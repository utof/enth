//! **Owner: lane DUI** (the duel history, feature `duel`; additive after lane SKD's shell).
//!
//! The duel history file (docs/duels.md, "The duel history" and "Privacy"): a local list of duels, so **a duel survives
//! closing the editor, the plugin and the project**. Above all it keeps **the duel's salt with your codes** (the F1e
//! review's M6: a salt lost before the answer is sent makes a rebuilt answer miss its commitment, which looks like
//! cheating), written before **Copy** is offered. It is the shell's file, like Learn's progress file; the session saves
//! nothing of a duel in the project.
//!
//! # Where
//!
//! `$ENTH_DUELS_DIR/duels.json` when the variable is set and not empty, else `<Documents>/enth/Duels/duels.json`
//! (beside Presets, Packs and Learn; [`duels_dir`]). The folder is made at the first write. With no home folder there
//! is no file: duels then last as long as the plugin instance, and the card says so.
//!
//! # What it holds (`enth.duel-history/1`)
//!
//! One [`Entry`] per duel, newest last, at most [`MAX_ENTRIES`] (the oldest go first):
//!
//! | Field | What |
//! | --- | --- |
//! | `id` | a number in this file (the largest plus one); it means nothing outside it |
//! | `code` | the challenge code |
//! | `day` | the UTC day you played (days since 1970-01-01): **a date, never a time of day** |
//! | `salt` | the duel's salt (16 hex digits), drawn once per duel from OS randomness by the shell |
//! | `own` | your result ([`Own`]: the compared submit's scores, the submits, the times in milliseconds, the assists, the engine) and your codes: the share message, the scorecard and the answer code, which carries your canonical patch |
//! | `card`, `answer` | their scorecard and their answer code, as pasted (an answer still sealed stays in the file; the card shows nothing of it until you have played) |
//! | `checked` | what the last check on this computer measured ([`Checked`]: both re-scores, whether it was "as reported", whether their answer was the committed one) |
//! | `name` | the friend's name, **only if you typed one** (at most 40 characters) |
//!
//! **No personal data:** no account, machine id, file path, user name or time of day, and no patch `meta` (the codes
//! strip it, and no patch is stored but through the answer codes). The only name is one you type. The file leaves the
//! machine only if you send it.
//!
//! # Writes
//!
//! [`HistoryFile::update`] takes a lock file (`duels.json.lock`), re-reads the file, applies the change, keeps the newest
//! [`MAX_ENTRIES`], writes a temporary dot file beside it and renames it over the old one: a crash mid-write leaves the
//! old file or none, and two instances (in one host or two) never lose each other's entries. A file that is not a
//! history file is set aside (`duels.json.unreadable-<seconds>`), never overwritten; one written by a newer enth
//! (another schema) is read, and never written.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use enth_challenge::answer::AnswerCode;
use enth_challenge::result::Scorecard;
use enth_challenge::{AssistsUsed, EngineVersion, Outcome};
use enth_session::duel::{DuelChecked, DuelOwn, DuelResult, MAX_CODE_TEXT, Rescored};
use enth_session::game::ScoreSummary;
use enth_ui::editor::duel::{HistoryRow, ShareCodes};
use serde::{Deserialize, Serialize};

use crate::presets::{Environment, Os, documents_dir};

/// The file's schema.
pub const SCHEMA: &str = "enth.duel-history/1";
/// The file's name, in [`duels_dir`].
pub const HISTORY_FILE: &str = "duels.json";
/// The most duels the file keeps (the newest).
pub const MAX_ENTRIES: usize = 200;
/// The largest file read: 200 entries of a few kilobytes each fit with room to spare.
pub const MAX_FILE_BYTES: u64 = 4 << 20;
/// The most characters of a friend's name kept.
pub const MAX_NAME_CHARS: usize = enth_ui::editor::duel::state::NAME_CHARS;

/// The Duels folder: `$ENTH_DUELS_DIR` if set and not empty, else `<Documents>/enth/Duels`.
pub fn duels_dir(env: &dyn Environment, os: Os) -> Option<PathBuf> {
    env.var("ENTH_DUELS_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| documents_dir(env, os).map(|d| d.join("enth").join("Duels")))
}

/// Today as the history counts days: whole days since 1970-01-01 in UTC, from the system clock (a date only).
pub fn today() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    u32::try_from(secs / 86_400).unwrap_or(u32::MAX)
}

/// A score as the session keeps it: hundredths of a percent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Score {
    /// Sound %, × 100.
    pub sound: u16,
    /// Combined %, × 100.
    pub combined: u16,
    /// Sound decided a pass.
    pub passed: bool,
}

impl From<Rescored> for Score {
    fn from(r: Rescored) -> Self {
        Score {
            sound: r.sound,
            combined: r.combined,
            passed: r.passed,
        }
    }
}

/// Your result and your codes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Own {
    /// How the round stood.
    pub outcome: Option<Outcome>,
    /// The compared submit's number (1 = the first).
    pub number: Option<u32>,
    /// The compared submit's scores.
    pub score: Option<ScoreSummary>,
    /// Scored submits in the round.
    pub submits: u32,
    /// The time to pass in milliseconds (reported).
    pub time_ms: Option<u64>,
    /// The round's active time in milliseconds.
    pub active_ms: u64,
    /// The help used.
    pub assists: AssistsUsed,
    /// The engine that scored it (`0.2.0`).
    pub engine: String,
    /// The share message (the two human lines and the scorecard).
    pub message: String,
    /// The scorecard.
    pub scorecard: String,
    /// The answer code (it carries the salt and your canonical patch).
    pub answer: String,
}

impl Own {
    /// Your result and the codes made from it.
    pub fn of(own: &DuelOwn, codes: &ShareCodes) -> Own {
        Own {
            outcome: Some(own.outcome),
            number: own.number,
            score: own.score,
            submits: own.submits,
            time_ms: own.time_ms,
            active_ms: own.active_ms,
            assists: own.assists,
            engine: own.engine.to_string(),
            message: codes.message.clone(),
            scorecard: codes.scorecard.clone(),
            answer: codes.answer.clone(),
        }
    }

    /// The codes, as the card shows them.
    pub fn codes(&self, code: &str) -> ShareCodes {
        ShareCodes {
            code: code.to_owned(),
            message: self.message.clone(),
            scorecard: self.scorecard.clone(),
            answer: self.answer.clone(),
            scorecard_symbols: symbols(&self.scorecard),
            answer_symbols: symbols(&self.answer),
        }
    }

    /// The result to restore in the session (`DuelEdit::duel_set_own`): the fields above, the scorers named by the
    /// scorecard, and the canonical patch the answer code rebuilds (none when this build cannot rebuild it: then the
    /// codes are still here as text).
    pub fn result(&self, code: &str) -> DuelResult {
        let engine = EngineVersion::parse(&self.engine).unwrap_or(EngineVersion::CURRENT);
        let mut own = DuelOwn::new(
            code,
            self.outcome.unwrap_or(Outcome::Unknown),
            self.submits,
            self.active_ms,
        )
        .with_assists(self.assists)
        .with_engine(engine);
        if let (Some(n), Some(s)) = (self.number, self.score) {
            own = own.with_score(n, s);
        }
        if let Some(t) = self.time_ms {
            own = own.with_time(t);
        }
        if let Ok(card) = Scorecard::decode(&self.scorecard)
            && let Some(scoring) = card
                .fields()
                .scorers
                .scoring(card.challenge().spec().difficulty)
        {
            own = own.with_scoring(scoring);
        }
        let patch = AnswerCode::decode(&self.answer)
            .ok()
            .map(|a| a.answer().clone());
        DuelResult::new(own, patch)
    }
}

/// What the last check on this computer measured.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Checked {
    /// Their patch, re-scored here.
    pub theirs: Score,
    /// Yours, on the same check.
    pub own: Option<Score>,
    /// Their re-score gave their claimed numbers ("as reported"); none without their scorecard.
    pub as_reported: Option<bool>,
    /// Their answer is the one their scorecard fixed; none without their scorecard.
    pub commitment_ok: Option<bool>,
}

impl Checked {
    /// From the session's check.
    pub fn of(c: &DuelChecked) -> Checked {
        Checked {
            theirs: c.theirs.into(),
            own: c.own.map(Into::into),
            as_reported: c.matches_claim(),
            commitment_ok: c.commitment_ok(),
        }
    }
}

/// One duel.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// Its number in this file.
    pub id: u64,
    /// The challenge code.
    pub code: String,
    /// The UTC day you played (or the day it was first kept).
    pub day: u32,
    /// The duel's salt, 16 hex digits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salt: Option<String>,
    /// Your result and codes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own: Option<Own>,
    /// Their scorecard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    /// Their answer code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    /// The last check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<Checked>,
    /// The friend's name you typed.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl Entry {
    /// The salt as a number.
    pub fn salt(&self) -> Option<u64> {
        self.salt
            .as_deref()
            .and_then(|s| u64::from_str_radix(s, 16).ok())
    }

    /// Keep `salt`.
    pub fn set_salt(&mut self, salt: u64) {
        self.salt = Some(format!("{salt:016x}"));
    }

    /// Keep `name`, trimmed to [`MAX_NAME_CHARS`].
    pub fn set_name(&mut self, name: &str) {
        self.name = name.trim().chars().take(MAX_NAME_CHARS).collect();
    }

    /// The duel is checked: a later duel on its code starts afresh unless reopened from the history.
    pub fn finished(&self) -> bool {
        self.checked.is_some()
    }

    /// The card's row.
    pub fn row(&self) -> HistoryRow {
        let claimed = self
            .card
            .as_deref()
            .and_then(|t| Scorecard::decode(t).ok())
            .map(|c| c.fields().sound_tenths);
        HistoryRow {
            id: self.id,
            code: self.code.clone(),
            day: self.day,
            own: self.own.as_ref().and_then(|o| o.score).map(|s| s.sound),
            claimed,
            checked: self.checked.as_ref().map(|c| c.theirs.sound),
            as_reported: self.checked.as_ref().and_then(|c| c.as_reported),
            name: self.name.clone(),
        }
    }

    /// Drop what no history writes: oversized code texts, a long name.
    fn sanitize(&mut self) {
        let fits = |t: &Option<String>| t.as_ref().is_none_or(|t| t.len() <= MAX_CODE_TEXT);
        if !fits(&self.card) {
            self.card = None;
        }
        if !fits(&self.answer) {
            self.answer = None;
        }
        if self.name.chars().count() > MAX_NAME_CHARS {
            let name = self.name.clone();
            self.set_name(&name);
        }
    }
}

/// The file's whole content.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    /// [`SCHEMA`].
    pub schema: String,
    /// The duels, oldest first.
    pub entries: Vec<Entry>,
}

/// Symbols in a code's text (hyphens and spaces not counted).
pub fn symbols(text: &str) -> usize {
    text.chars().filter(char::is_ascii_alphanumeric).count()
}

/// The next id: the largest plus one.
pub fn next_id(entries: &[Entry]) -> u64 {
    entries.iter().map(|e| e.id).max().unwrap_or(0) + 1
}

/// What reading the file found.
enum Read {
    /// No file.
    Missing,
    /// A history file of this schema.
    Ours(Vec<Entry>),
    /// A newer enth's: its entries, read tolerantly; never written.
    Newer(Vec<Entry>),
}

/// The duel history file (see the [module docs](self)).
#[derive(Clone, Debug, Default)]
pub struct HistoryFile {
    path: Option<PathBuf>,
}

impl HistoryFile {
    /// The file at `path` (none: no file, nothing kept).
    pub fn new(path: Option<PathBuf>) -> HistoryFile {
        HistoryFile { path }
    }

    /// The file where `env` puts it ([`duels_dir`]).
    pub fn from_env(env: &dyn Environment, os: Os) -> HistoryFile {
        HistoryFile::new(duels_dir(env, os).map(|d| d.join(HISTORY_FILE)))
    }

    /// Where it is.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The duels it holds, oldest first: none when it is missing or not a history file (which is then set aside).
    pub fn load(&self) -> Vec<Entry> {
        match self.path.as_deref().map(read) {
            Some(Read::Ours(e) | Read::Newer(e)) => e,
            _ => Vec::new(),
        }
    }

    /// Change the file with `change`: under the lock file, re-read, `change`, keep the newest [`MAX_ENTRIES`], a
    /// temporary file and a rename. Returns what `change` returned and the entries as written.
    ///
    /// # Errors
    /// No file to keep (no home folder), the folder or the file cannot be written, or the file is a newer enth's.
    pub fn update<R>(
        &self,
        change: impl FnOnce(&mut Vec<Entry>) -> R,
    ) -> io::Result<(R, Vec<Entry>)> {
        let Some(path) = self.path.as_deref() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "there is no folder for the duel history (no home folder; set ENTH_DUELS_DIR)",
            ));
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("json.lock"))?;
        lock.lock()?;
        let result = (|| {
            let mut entries = match read(path) {
                Read::Missing => Vec::new(),
                Read::Ours(e) => e,
                Read::Newer(_) => {
                    return Err(io::Error::other(
                        "the duel history was written by a newer enth: it is left as it is",
                    ));
                }
            };
            let r = change(&mut entries);
            for e in &mut entries {
                e.sanitize();
            }
            entries.sort_by_key(|e| e.id);
            if entries.len() > MAX_ENTRIES {
                let over = entries.len() - MAX_ENTRIES;
                entries.drain(..over);
            }
            let doc = History {
                schema: SCHEMA.to_owned(),
                entries,
            };
            let text = serde_json::to_string_pretty(&doc).map_err(io::Error::other)?;
            write_atomic(path, text.as_bytes())?;
            Ok((r, doc.entries))
        })();
        let _ = lock.unlock();
        result
    }
}

/// Read the file at `path`.
fn read(path: &Path) -> Read {
    let Ok(meta) = fs::metadata(path) else {
        return Read::Missing;
    };
    let parsed = (meta.is_file() && meta.len() <= MAX_FILE_BYTES)
        .then(|| fs::read_to_string(path).ok())
        .flatten()
        .and_then(|text| serde_json::from_str::<History>(&text).ok());
    match parsed {
        Some(h) if h.schema == SCHEMA => Read::Ours(h.entries),
        Some(h) if h.schema.starts_with("enth.duel-history/") => Read::Newer(h.entries),
        _ => {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let _ = fs::rename(path, path.with_extension(format!("json.unreadable-{secs}")));
            Read::Missing
        }
    }
}

/// Write `bytes` to `target` through a temporary dot file in the same folder and a rename, so a crash mid-write leaves
/// the old file or none. The temporary name is unique within the process (two instances in one host).
fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let temp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, target)
    })();
    if result.is_err() {
        // The temporary file is ours; do not leave it behind.
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
    use enth_challenge::{ChallengeCode, prepare_round, score_round};

    use super::*;

    /// A fresh folder under the system's temporary folder, removed when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "enth-dui-{label}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn file(&self) -> HistoryFile {
            HistoryFile::new(Some(self.0.join("Duels").join(HISTORY_FILE)))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Env(Vec<(&'static str, &'static str)>);

    impl Environment for Env {
        fn var(&self, name: &str) -> Option<std::ffi::OsString> {
            self.0
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).into())
        }

        fn read_text(&self, _path: &Path) -> Option<String> {
            None
        }
    }

    #[test]
    fn the_folder_is_the_variable_else_documents() {
        let env = Env(vec![("HOME", "/home/u"), ("ENTH_DUELS_DIR", "/tmp/d")]);
        assert_eq!(duels_dir(&env, Os::Linux), Some(PathBuf::from("/tmp/d")));
        let env = Env(vec![("HOME", "/home/u"), ("ENTH_DUELS_DIR", "")]);
        assert_eq!(
            duels_dir(&env, Os::Linux),
            Some(PathBuf::from("/home/u/Documents/enth/Duels"))
        );
        assert_eq!(duels_dir(&Env(vec![]), Os::Linux), None);
        assert!(
            HistoryFile::new(None).update(|_| ()).is_err(),
            "no folder, nothing kept"
        );
    }

    #[test]
    fn the_first_write_makes_the_file_and_a_reread_gives_it_back() {
        let dir = TempDir::new("first");
        let file = dir.file();
        assert_eq!(file.load(), Vec::new(), "a missing file is empty");
        let (id, entries) = file
            .update(|e| {
                let id = next_id(e);
                let mut entry = Entry {
                    id,
                    code: "1080-1000-0001-AB".to_owned(),
                    day: 20_751,
                    ..Entry::default()
                };
                entry.set_salt(0x0123_4567_89ab_cdef);
                entry.set_name("  Sam with a name far longer than forty characters in all  ");
                e.push(entry);
                id
            })
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(file.load(), entries);
        let e = &entries[0];
        assert_eq!(e.salt(), Some(0x0123_4567_89ab_cdef));
        assert_eq!(e.salt.as_deref(), Some("0123456789abcdef"));
        assert_eq!(e.name.chars().count(), MAX_NAME_CHARS);
        assert!(e.name.starts_with("Sam"));
        let text = fs::read_to_string(file.path().unwrap()).unwrap();
        assert!(text.contains(SCHEMA));
        // No temporary file is left beside it.
        let left: Vec<_> = fs::read_dir(file.path().unwrap().parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                Path::new(n)
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("tmp"))
            })
            .collect();
        assert_eq!(left, Vec::<String>::new());
    }

    /// Two writers (two instances) at once lose nothing: each write re-reads under the lock file. The newest
    /// [`MAX_ENTRIES`] are kept.
    #[test]
    fn writers_merge_under_the_lock_and_the_newest_are_kept() {
        let dir = TempDir::new("merge");
        let path = dir.file().path().unwrap().to_path_buf();
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let file = HistoryFile::new(Some(path));
                    for i in 0..25 {
                        file.update(|e| {
                            let id = next_id(e);
                            e.push(Entry {
                                id,
                                code: format!("T{t}-{i}"),
                                ..Entry::default()
                            });
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let file = HistoryFile::new(Some(path));
        let entries = file.load();
        assert_eq!(entries.len(), 100, "every writer's entries are there");
        let mut ids: Vec<u64> = entries.iter().map(|e| e.id).collect();
        ids.dedup();
        assert_eq!(ids, (1..=100).collect::<Vec<_>>(), "unique, in order");
        let ((), entries) = file
            .update(|e| {
                for _ in 0..150 {
                    let id = next_id(e);
                    e.push(Entry {
                        id,
                        ..Entry::default()
                    });
                }
            })
            .unwrap();
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].id, 51, "the oldest went first");
        assert_eq!(entries.last().unwrap().id, 250);
    }

    #[test]
    fn a_file_that_is_not_a_history_is_set_aside_and_a_newer_one_is_never_written() {
        let dir = TempDir::new("aside");
        let file = dir.file();
        let path = file.path().unwrap().to_path_buf();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "my notes, not a history").unwrap();
        assert_eq!(file.load(), Vec::new());
        let aside: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("duels.json.unreadable-"))
            .collect();
        assert_eq!(aside.len(), 1, "set aside, not overwritten");
        let newer =
            r#"{"schema":"enth.duel-history/2","entries":[{"id":4,"code":"X","future":1}]}"#;
        fs::write(&path, newer).unwrap();
        assert_eq!(file.load().len(), 1, "a newer file's entries are read");
        assert!(file.update(|_| ()).is_err(), "and never written");
        assert_eq!(fs::read_to_string(&path).unwrap(), newer);
    }

    /// The level-2 friend's codes of SKD's tests: a passing answer whose patch carries a real-looking name in its meta.
    fn codes() -> (ChallengeCode, DuelOwn, ShareCodes, u64) {
        let challenge =
            ChallengeCode::short(SpecPreset::by_id(2).unwrap().spec(LATEST_GENERATOR, 77)).unwrap();
        let round = prepare_round(challenge.spec()).unwrap();
        let mut answer = round.target().clone();
        "Somebody Real".clone_into(&mut answer.meta.author);
        "My Secret Name".clone_into(&mut answer.meta.name);
        let salt = 0x5eed_0000_1234_abcd;
        let code = AnswerCode::new(challenge, salt, &answer).unwrap();
        let report = score_round(&round, code.answer()).unwrap();
        let assists = AssistsUsed {
            hints: 1,
            ..AssistsUsed::default()
        };
        let card = Scorecard::new(
            challenge,
            &report,
            &round.challenge().scoring,
            2,
            95_500,
            &assists,
            code.commitment(),
        )
        .unwrap();
        let summary = enth_session::game::summary(&report);
        let own = DuelOwn::new(challenge.to_string(), Outcome::Passed, 3, 120_000)
            .with_score(2, summary)
            .with_time(95_500)
            .with_assists(assists)
            .with_scoring(round.challenge().scoring.clone());
        let [a, b] = card.share_lines();
        let codes = ShareCodes {
            code: challenge.to_string(),
            message: format!("{a}\n{b}\n{}", card.encode()),
            scorecard: card.encode(),
            answer: code.encode(),
            scorecard_symbols: symbols(&card.encode()),
            answer_symbols: code.symbols(),
        };
        (challenge, own, codes, salt)
    }

    /// Your result goes through the file and comes back as the same result, with the canonical patch the answer code
    /// rebuilds and the scorers its scorecard names.
    #[test]
    fn your_result_comes_back_from_the_file_as_it_went_in() {
        let dir = TempDir::new("own");
        let file = dir.file();
        let (challenge, own, codes, salt) = codes();
        file.update(|e| {
            let mut entry = Entry {
                id: next_id(e),
                code: challenge.to_string(),
                day: 20_751,
                own: Some(Own::of(&own, &codes)),
                ..Entry::default()
            };
            entry.set_salt(salt);
            e.push(entry);
        })
        .unwrap();
        let entry = file.load().remove(0);
        assert_eq!(entry.salt(), Some(salt));
        let back = entry.own.as_ref().unwrap();
        assert_eq!(
            back.codes(&entry.code),
            codes,
            "the same codes, sizes included"
        );
        let restored = back.result(&entry.code);
        assert_eq!(restored.own.code, own.code);
        assert_eq!(restored.own.outcome, own.outcome);
        assert_eq!(restored.own.number, own.number);
        assert_eq!(restored.own.score, own.score);
        assert_eq!(restored.own.submits, own.submits);
        assert_eq!(restored.own.time_ms, own.time_ms);
        assert_eq!(restored.own.active_ms, own.active_ms);
        assert_eq!(restored.own.assists, own.assists);
        assert_eq!(restored.own.engine, own.engine);
        assert_eq!(
            restored.own.scoring, own.scoring,
            "the scorecard's scorer set"
        );
        assert!(restored.own.can_share_answer());
        let canonical = AnswerCode::decode(&codes.answer).unwrap();
        assert_eq!(restored.answer.as_deref(), Some(canonical.answer()));
        let row = entry.row();
        assert_eq!(row.own, own.score.map(|s| s.sound));
        assert_eq!(row.day, 20_751);
    }

    /// **No personal data in the file**: a patch whose meta holds a name, a share, their codes and a check give a file
    /// with no name but the one typed, no path, no time of day, and only the documented keys.
    #[test]
    fn the_file_holds_no_personal_data() {
        fn walk(v: &serde_json::Value, keys: &mut Vec<String>) {
            match v {
                serde_json::Value::Object(m) => {
                    for (k, v) in m {
                        keys.push(k.clone());
                        walk(v, keys);
                    }
                }
                serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, keys)),
                _ => {}
            }
        }
        let dir = TempDir::new("privacy");
        let file = dir.file();
        let (challenge, own, codes, salt) = codes();
        file.update(|e| {
            let mut entry = Entry {
                id: next_id(e),
                code: challenge.to_string(),
                day: today(),
                own: Some(Own::of(&own, &codes)),
                card: Some(codes.scorecard.clone()),
                answer: Some(codes.answer.clone()),
                checked: Some(Checked {
                    theirs: Score {
                        sound: 10_000,
                        combined: 9_000,
                        passed: true,
                    },
                    own: None,
                    as_reported: Some(true),
                    commitment_ok: Some(true),
                }),
                ..Entry::default()
            };
            entry.set_salt(salt);
            entry.set_name("Sam");
            e.push(entry);
        })
        .unwrap();
        let text = fs::read_to_string(file.path().unwrap()).unwrap();
        for banned in [
            "Somebody Real",
            "My Secret Name",
            "/tmp",
            "/home",
            "Documents",
        ] {
            assert!(
                !text.contains(banned),
                "{banned:?} in the history file:\n{text}"
            );
        }
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys = Vec::new();
        walk(&json, &mut keys);
        keys.sort();
        keys.dedup();
        let allowed = [
            "active_ms",
            "answer",
            "as_reported",
            "assists",
            "card",
            "checked",
            "code",
            "combined",
            "commitment_ok",
            "day",
            "engine",
            "entries",
            "hints",
            "id",
            "live_meter",
            "message",
            "name",
            "number",
            "outcome",
            "own",
            "passed",
            "revealed",
            "salt",
            "schema",
            "score",
            "scorecard",
            "sound",
            "structure",
            "submits",
            "theirs",
            "time_ms",
        ];
        for k in &keys {
            assert!(allowed.contains(&k.as_str()), "an undocumented key {k:?}");
        }
        // The only name is the one typed; the day is a day number, not a time.
        assert_eq!(json["entries"][0]["name"], "Sam");
        assert!(json["entries"][0]["day"].as_u64().unwrap() < 100_000);
    }
}
