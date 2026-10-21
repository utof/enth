//! **Owner: lane LM** (progress v1 and its merge, freeze point F1l); **REV** adds the review field, additively. The
//! reference is docs/learn.md, "Progress and the saved state" and "The lesson model (lane LM)".
//!
//! Per-user progress, the content of `<Documents>/enth/Learn/progress.json` (a file the shell owns), keyed by
//! [`LessonKey`] (`pack/lesson`) and [`StepId`], which are append-only, so pack updates never reset it:
//!
//! ```json
//! {"schema":"enth.progress/1","lessons":{"basics/cutoff":{"steps":{"hear":{"done":true,"clean":true},"try":{"done":true,"best":97}}}}}
//! ```
//!
//! A step is `done` (passed, or read), `clean` (passed at least once with no help) and has a `best` (the best
//! sound % of a Recreate step's submits). Each is a monotone fact, so [`Progress::merge`] is a join: the union of
//! lessons and steps, `done` and `clean` or-ed, the maximum of bests. It is commutative, associative and idempotent,
//! and **never shrinks** either side (proptests in `tests/progress.rs`). The shell writes by re-reading the file,
//! merging, writing a temporary file and renaming it, so two instances never lose each other's progress.
//!
//! **A lenient output.** A newer version is read, not refused. A field this build does not know, at any level, and
//! an entry it cannot decode (a lesson key or a step in a newer form) are **kept verbatim** and written back, so an
//! older build that merges and rewrites a newer file loses nothing of it. Where both sides hold different unknown
//! values under one key, the merge keeps the greater by canonical JSON text (deterministic, and still a join); where
//! one side can read an entry the other cannot, the readable one wins. Only text that is not a JSON object with an
//! `enth.progress/<N>` schema is refused: the shell must keep such a file aside, never overwrite it.

use std::collections::BTreeMap;

use serde::Serialize;
use serde::ser::SerializeMap;
use serde_json::Value;

use crate::error::{Doc, LearnError};
use crate::lesson::{Lesson, StepKind};
use crate::pack::{LessonKey, StepId, schema_version, syntax};
use crate::player::StepRecord;

/// The progress version this build writes.
pub const PROGRESS_VERSION: u32 = 1;
/// The `schema` string this build writes: `enth.progress/1`.
pub const PROGRESS_SCHEMA: &str = "enth.progress/1";

/// Fields kept verbatim: unknown to this build, written back as they were.
type Extra = BTreeMap<String, Value>;

/// One step's progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepProgress {
    /// Passed (a Read or Listen step: left forward).
    pub done: bool,
    /// Passed at least once with no hint, Show, reveal or wrong pick.
    pub clean: bool,
    /// The best sound % of a Recreate step's submits.
    pub best: Option<u8>,
    /// Fields from a newer build, kept.
    pub extra: Extra,
}

impl StepProgress {
    /// The join of two.
    pub fn merge(&mut self, other: &StepProgress) {
        self.done |= other.done;
        self.clean |= other.clean;
        self.best = self.best.max(other.best);
        merge_extra(&mut self.extra, &other.extra);
    }

    fn read(value: &Value) -> Option<StepProgress> {
        let obj = value.as_object()?;
        let mut p = StepProgress::default();
        for (k, v) in obj {
            match k.as_str() {
                "done" => p.done = v.as_bool()?,
                "clean" => p.clean = v.as_bool()?,
                "best" => {
                    p.best = match v {
                        Value::Null => None,
                        _ => Some(u8::try_from(v.as_u64()?).ok().filter(|b| *b <= 100)?),
                    }
                }
                _ => {
                    p.extra.insert(k.clone(), v.clone());
                }
            }
        }
        p.done |= p.clean;
        Some(p)
    }
}

impl Serialize for StepProgress {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("done", &self.done)?;
        if self.clean {
            m.serialize_entry("clean", &true)?;
        }
        if let Some(b) = self.best {
            m.serialize_entry("best", &b)?;
        }
        for (k, v) in &self.extra {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

/// One lesson's progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LessonProgress {
    /// Its steps by id.
    pub steps: BTreeMap<StepId, StepProgress>,
    /// Step entries this build could not decode, kept (never under a key `steps` also holds).
    pub unread: Extra,
    /// Fields from a newer build, kept.
    pub extra: Extra,
}

impl LessonProgress {
    /// The join of two.
    pub fn merge(&mut self, other: &LessonProgress) {
        for (id, s) in &other.steps {
            self.steps.entry(id.clone()).or_default().merge(s);
        }
        merge_extra(&mut self.unread, &other.unread);
        self.unread
            .retain(|k, _| !self.steps.keys().any(|id| id.as_str() == k));
        merge_extra(&mut self.extra, &other.extra);
    }

    /// Whether every step of `lesson` is done.
    pub fn completed(&self, lesson: &Lesson) -> bool {
        lesson
            .steps
            .iter()
            .all(|s| self.steps.get(&s.id).is_some_and(|p| p.done))
    }

    /// Whether the lesson's mastery is shown: every step done, and every Recreate and Choose step clean.
    pub fn mastered(&self, lesson: &Lesson) -> bool {
        lesson.steps.iter().all(|s| {
            self.steps.get(&s.id).is_some_and(|p| {
                p.done
                    && (p.clean
                        || !matches!(s.kind, StepKind::Recreate { .. } | StepKind::Choose { .. }))
            })
        })
    }

    fn read(value: &Value) -> Option<LessonProgress> {
        let obj = value.as_object()?;
        let mut p = LessonProgress::default();
        for (k, v) in obj {
            if k == "steps" {
                for (id, s) in v.as_object()? {
                    match (StepId::new(id), StepProgress::read(s)) {
                        (Ok(id), Some(s)) => {
                            p.steps.insert(id, s);
                        }
                        _ => {
                            p.unread.insert(id.clone(), s.clone());
                        }
                    }
                }
            } else {
                p.extra.insert(k.clone(), v.clone());
            }
        }
        Some(p)
    }
}

impl Serialize for LessonProgress {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("steps", &Merged(&self.steps, &self.unread))?;
        for (k, v) in &self.extra {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

/// Per-user progress v1. See the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// The lessons by key.
    pub lessons: BTreeMap<LessonKey, LessonProgress>,
    /// Lesson entries this build could not decode, kept (never under a key `lessons` also holds).
    pub unread: Extra,
    /// Fields from a newer build, kept.
    pub extra: Extra,
}

impl Progress {
    /// No progress.
    pub fn new() -> Progress {
        Progress::default()
    }

    /// Read a progress file leniently (see the [module docs](self)). Refused only when it is not JSON
    /// (`bad_json`), not an object with an `enth.progress/<N>` schema (`missing_schema`, `bad_schema`), or its
    /// `lessons` is not an object (`bad_progress`): the shell keeps such a file aside.
    pub fn from_json(text: &str) -> Result<Progress, LearnError> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| syntax(Doc::Progress, e.to_string()))?;
        schema_version(Doc::Progress, &value)?;
        let mut p = Progress::default();
        for (k, v) in value.as_object().into_iter().flatten() {
            match k.as_str() {
                "schema" => {}
                "lessons" => {
                    let lessons = v.as_object().ok_or_else(|| LearnError::BadFields {
                        doc: Doc::Progress,
                        path: None,
                        message: "lessons is not an object".to_owned(),
                    })?;
                    for (key, l) in lessons {
                        match (key.parse::<LessonKey>(), LessonProgress::read(l)) {
                            (Ok(key), Some(l)) => {
                                p.lessons.insert(key, l);
                            }
                            _ => {
                                p.unread.insert(key.clone(), l.clone());
                            }
                        }
                    }
                }
                _ => {
                    p.extra.insert(k.clone(), v.clone());
                }
            }
        }
        Ok(p)
    }

    /// The JSON text: compact, schema first, keys in order, byte-deterministic.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The JSON text, indented.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// The join with `other`: the union of lessons and steps, `done` and `clean` or-ed, the maximum of bests, kept
    /// fields united. Commutative, associative, idempotent, and never shrinks either side.
    pub fn merge(&mut self, other: &Progress) {
        for (key, l) in &other.lessons {
            self.lessons.entry(key.clone()).or_default().merge(l);
        }
        merge_extra(&mut self.unread, &other.unread);
        self.unread
            .retain(|k, _| !self.lessons.keys().any(|key| key.to_string() == *k));
        merge_extra(&mut self.extra, &other.extra);
    }

    /// Merge one step's outcome (what the player's [`Effect::Record`](crate::player::Effect::Record) carries).
    pub fn record(&mut self, r: &StepRecord) {
        self.lessons
            .entry(r.lesson.clone())
            .or_default()
            .steps
            .entry(r.step.clone())
            .or_default()
            .merge(&r.progress);
    }

    /// A lesson's progress.
    pub fn lesson(&self, key: &LessonKey) -> Option<&LessonProgress> {
        self.lessons.get(key)
    }

    /// A step's progress.
    pub fn step(&self, key: &LessonKey, step: &StepId) -> Option<&StepProgress> {
        self.lessons.get(key)?.steps.get(step)
    }
}

impl Serialize for Progress {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("schema", PROGRESS_SCHEMA)?;
        m.serialize_entry("lessons", &Merged(&self.lessons, &self.unread))?;
        for (k, v) in &self.extra {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

/// A map of readable entries and kept unreadable ones, written as one object in key-text order.
struct Merged<'a, K, V>(&'a BTreeMap<K, V>, &'a Extra);

impl<K: ToString, V: Serialize> Serialize for Merged<'_, K, V> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut entries: Vec<(String, Entry<'_, V>)> = self
            .0
            .iter()
            .map(|(k, v)| (k.to_string(), Entry::Read(v)))
            .chain(self.1.iter().map(|(k, v)| (k.clone(), Entry::Kept(v))))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut m = s.serialize_map(Some(entries.len()))?;
        for (k, v) in entries {
            match v {
                Entry::Read(v) => m.serialize_entry(&k, v)?,
                Entry::Kept(v) => m.serialize_entry(&k, v)?,
            }
        }
        m.end()
    }
}

enum Entry<'a, V> {
    Read(&'a V),
    Kept(&'a Value),
}

/// A kept value's canonical text, the order of conflicting kept values.
fn canonical(v: &Value) -> String {
    v.to_string()
}

/// Unite kept fields; on a conflict keep the greater by canonical JSON text, so the merge stays a join.
fn merge_extra(into: &mut Extra, other: &Extra) {
    for (k, v) in other {
        match into.get_mut(k) {
            Some(mine) if *mine != *v => {
                if canonical(v) > canonical(mine) {
                    mine.clone_from(v);
                }
            }
            Some(_) => {}
            None => {
                into.insert(k.clone(), v.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_file_reads_and_keeps_what_it_does_not_know() {
        let text = r#"{"schema":"enth.progress/3","review":{"boxes":{"svf.cutoff":2}},"lessons":{"basics/cutoff":{"steps":{"try":{"done":true,"best":97,"time_ms":4000},"hear":{"done":"yes"}},"votes":"new"},"Basics/X":{"steps":{}}}}"#;
        let p = Progress::from_json(text).unwrap();
        let key: LessonKey = "basics/cutoff".parse().unwrap();
        let step = p.step(&key, &StepId::new("try").unwrap()).unwrap();
        assert!(step.done && step.best == Some(97));
        assert_eq!(step.extra["time_ms"], 4000);
        assert!(p.lesson(&key).unwrap().unread.contains_key("hear"));
        assert!(p.unread.contains_key("Basics/X"));
        let again = Progress::from_json(&p.to_json()).unwrap();
        assert_eq!(again, p, "what was kept is written back");
        assert!(
            p.to_json()
                .starts_with(r#"{"schema":"enth.progress/1","lessons":"#)
        );
    }

    #[test]
    fn only_non_progress_text_is_refused() {
        assert_eq!(Progress::from_json("nope").unwrap_err().code(), "bad_json");
        assert_eq!(
            Progress::from_json("{}").unwrap_err().code(),
            "missing_schema"
        );
        assert_eq!(
            Progress::from_json(r#"{"schema":"enth.pack/1"}"#)
                .unwrap_err()
                .code(),
            "bad_schema"
        );
        assert_eq!(
            Progress::from_json(r#"{"schema":"enth.progress/1","lessons":[]}"#)
                .unwrap_err()
                .code(),
            "bad_progress"
        );
        assert!(Progress::from_json(r#"{"schema":"enth.progress/1"}"#).is_ok());
    }
}
