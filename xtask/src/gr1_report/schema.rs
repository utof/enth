//! The round log, schema `enth.round-log/1`: one JSON object per line, appended by the plugin when a round ends or
//! the player disputes a score (`docs/gr1.md` has the field table; the writer is `enth-session`'s `RoundLogEntry`,
//! the result inside it is `enth-challenge`'s `RoundResult`, `enth.round-result/1`).
//!
//! This is a **tolerant reader**, not a deserializer. It works on `serde_json::Value`, so that a line from a newer
//! or older build never fails the report, and so that every deviation is named:
//!
//! | Severity | Meaning | What the report does |
//! | --- | --- | --- |
//! | [`Severity::Error`] | The line is not usable (not an object, no `schema`, no `event`, no `round`, no `code`) | skips the line |
//! | [`Severity::Warn`] | A field is the wrong type or out of range, or two fields disagree | reads the line, drops the field |
//! | [`Severity::Note`] | Something a newer build may add: an unknown field, an unknown enum value, a field an older build lacked | reads the line, ignores the extra |
//!
//! A line with another schema version (`enth.round-log/2`) is skipped with a warning: a version bump is the writer's
//! promise that the layout changed.

use serde_json::{Map, Value};

/// The only round-log layout this reader knows.
pub const ROUND_LOG_SCHEMA: &str = "enth.round-log/1";

/// The result's schema.
pub const ROUND_RESULT_SCHEMA: &str = "enth.round-result/1";

/// The blind rating's top (1 is "nothing like it", 5 "can't tell them apart").
pub const RATING_MAX: u8 = 5;

/// How bad a deviation is (the module docs have the table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// A newer or older build's difference, ignored.
    Note,
    /// A bad field, dropped.
    Warn,
    /// An unusable line, skipped.
    Error,
}

impl Severity {
    /// For tables and JSON.
    pub fn name(self) -> &'static str {
        match self {
            Severity::Note => "note",
            Severity::Warn => "warning",
            Severity::Error => "error",
        }
    }
}

/// One deviation found while reading a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// How bad.
    pub severity: Severity,
    /// What, without the line number (the caller adds the place).
    pub message: String,
}

fn issue(severity: Severity, message: impl Into<String>) -> Issue {
    Issue {
        severity,
        message: message.into(),
    }
}

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $(#[doc = concat!("`", $text, "`")] $variant,)+
            /// A value from a newer build, kept as written.
            Unknown(String),
        }

        impl $name {
            fn parse(text: &str) -> Self {
                match text {
                    $($text => $name::$variant,)+
                    other => $name::Unknown(other.to_owned()),
                }
            }

            /// The value as written in the log.
            #[allow(dead_code, reason = "not every enum is printed")]
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$variant => $text,)+
                    $name::Unknown(s) => s,
                }
            }

            /// A value this reader knows.
            pub fn is_known(&self) -> bool {
                !matches!(self, $name::Unknown(_))
            }
        }
    };
}

string_enum! {
    /// Why a line was written.
    Event { End => "end", Dispute => "dispute" }
}

string_enum! {
    /// What ended the round.
    EndedBy { Restore => "restore", Keep => "keep", NewRound => "new_round", Host => "host" }
}

string_enum! {
    /// How the round ended.
    Outcome { Passed => "passed", NotPassed => "not_passed", GaveUp => "gave_up" }
}

string_enum! {
    /// The player's vote on the score.
    Agreement { TooLow => "too_low", AboutRight => "about_right", TooHigh => "too_high" }
}

/// A round's identity within one player's logs: the instance's serial and the start timestamp (`RoundId`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RoundKey {
    /// Rounds started in the plugin instance.
    pub serial: u64,
    /// The shell's timestamp at the start.
    pub nonce: u64,
}

/// A score report as far as the GR1 numbers need it: the three percentages and the pass.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreView {
    /// Sound %, 0 to 100.
    pub sound: f64,
    /// Structure %.
    pub structure: f64,
    /// Combined %.
    pub combined: f64,
    /// The pass mark in sound %.
    pub pass_mark: Option<f64>,
    /// Whether this submit passed.
    pub passed: bool,
    /// The scorer ids (`sound/1`, `structure/1`, `combined/1`), when the report names them.
    pub scorers: [Option<String>; 3],
    /// The engine that rendered the scored audio.
    pub engine: Option<String>,
}

/// One scored submit (`SubmitRecord`): scores in hundredths of a percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmitScore {
    /// Active time when Submit was pressed.
    pub at_ms: u64,
    /// This submit passed.
    pub passed: bool,
    /// Sound % x 100.
    pub sound: u16,
    /// Structure % x 100.
    pub structure: u16,
    /// Combined % x 100.
    pub combined: u16,
}

/// The help a round used (`AssistsUsed`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Assists {
    /// Hints revealed.
    pub hints: u64,
    /// The live meter was on.
    pub live_meter: bool,
    /// The target was revealed.
    pub revealed: bool,
    /// The `dim_unused` assist was on (lane HINT, FU1: written only when true, so an older line reads as false).
    pub dim_unused: bool,
}

impl Assists {
    /// Any help at all.
    pub fn any(&self) -> bool {
        self.hints > 0 || self.live_meter || self.revealed || self.dim_unused
    }
}

/// The round's result (`RoundResult`, `enth.round-result/1`).
#[derive(Clone, Debug, PartialEq)]
pub struct ResultView {
    /// How it ended.
    pub outcome: Outcome,
    /// The recorded score: the first submit's report.
    pub first_submit: Option<ScoreView>,
    /// How many times the player submitted.
    pub submits: u64,
    /// Active time to the first passing submit.
    pub time_to_pass_ms: Option<u64>,
    /// Active time of the whole round.
    pub active_ms: u64,
    /// The help used.
    pub assists: Assists,
    /// The vote, if given.
    pub agreement: Option<Agreement>,
    /// The blind rating of the first submit, 1 to 5.
    pub rating: Option<u8>,
}

/// One line of the round log, as far as the report reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Why it was written.
    pub event: Event,
    /// The round.
    pub round: RoundKey,
    /// The code text as logged.
    pub code: String,
    /// What ended the round (`None` on a dispute line).
    pub ended_by: Option<EndedBy>,
    /// The round could not continue.
    pub broken: bool,
    /// The result; `None` when the code did not decode (or the result was unreadable).
    pub result: Option<ResultView>,
    /// Every scored submit.
    pub submits: Vec<SubmitScore>,
    /// The dispute note.
    pub dispute: Option<String>,
    /// Edits made.
    pub edits: u64,
    /// The line holds the last scored submit's patch.
    pub has_patch: bool,
    /// The target's fingerprint, when the build wrote it.
    pub fingerprint: Option<String>,
    /// The engine of the build that played the round.
    pub engine: Option<String>,
    /// Entries in the edit log.
    pub edit_log_len: usize,
}

/// What reading one line gave.
#[derive(Debug, Default)]
pub struct Parsed {
    /// The line, unless it is blank, unusable or from another schema version.
    pub entry: Option<Entry>,
    /// Every deviation found.
    pub issues: Vec<Issue>,
    /// The line has another schema version (skipped; counted by the caller).
    pub other_schema: bool,
}

pub(super) const TOP_FIELDS: &[&str] = &[
    "schema",
    "event",
    "round",
    "code",
    "ended_by",
    "broken",
    "result",
    "submits",
    "dispute",
    "edits",
    "patch",
    "report",
    "fingerprint",
    "engine",
    "edit_log",
];
pub(super) const RESULT_FIELDS: &[&str] = &[
    "schema",
    "code",
    "outcome",
    "first_submit",
    "submits",
    "time_to_pass_ms",
    "active_ms",
    "assists",
    "agreement",
    "rating",
];
pub(super) const ASSIST_FIELDS: &[&str] = &["hints", "live_meter", "revealed", "dim_unused"];
const ROUND_FIELDS: &[&str] = &["serial", "nonce"];

/// Collects issues while a line is read.
struct Reader {
    issues: Vec<Issue>,
}

impl Reader {
    fn note(&mut self, m: impl Into<String>) {
        self.issues.push(issue(Severity::Note, m));
    }

    fn warn(&mut self, m: impl Into<String>) {
        self.issues.push(issue(Severity::Warn, m));
    }

    fn unknown_fields(&mut self, obj: &Map<String, Value>, known: &[&str], place: &str) {
        for key in obj.keys() {
            if !known.contains(&key.as_str()) {
                self.note(format!("unknown field `{place}{key}` ignored"));
            }
        }
    }

    /// A non-negative integer field. A missing or null field is `None` without a complaint; a field of another type
    /// is a warning.
    fn uint(&mut self, obj: &Map<String, Value>, key: &str, place: &str) -> Option<u64> {
        match obj.get(key) {
            None | Some(Value::Null) => None,
            Some(v) => {
                let n = v.as_u64().or_else(|| {
                    v.as_f64()
                        .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f < 9.0e15)
                        .map(|f| f as u64)
                });
                if n.is_none() {
                    self.warn(format!("`{place}{key}` is not a non-negative integer"));
                }
                n
            }
        }
    }

    fn string<'a>(
        &mut self,
        obj: &'a Map<String, Value>,
        key: &str,
        place: &str,
    ) -> Option<&'a str> {
        match obj.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s),
            Some(_) => {
                self.warn(format!("`{place}{key}` is not a string"));
                None
            }
        }
    }

    fn boolean(&mut self, obj: &Map<String, Value>, key: &str, place: &str) -> Option<bool> {
        match obj.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::Bool(b)) => Some(*b),
            Some(_) => {
                self.warn(format!("`{place}{key}` is not true or false"));
                None
            }
        }
    }
}

/// Read one line of a round log.
pub fn parse_line(text: &str) -> Parsed {
    let mut out = Parsed::default();
    if text.trim().is_empty() {
        return out;
    }
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            out.issues
                .push(issue(Severity::Error, format!("not JSON ({e})")));
            return out;
        }
    };
    let Value::Object(obj) = value else {
        out.issues.push(issue(Severity::Error, "not a JSON object"));
        return out;
    };
    match obj.get("schema").and_then(Value::as_str) {
        Some(ROUND_LOG_SCHEMA) => {}
        Some(other) => {
            out.other_schema = true;
            out.issues.push(issue(
                Severity::Warn,
                format!("schema `{other}`: this report reads `{ROUND_LOG_SCHEMA}`, line skipped"),
            ));
            return out;
        }
        None => {
            out.issues
                .push(issue(Severity::Error, "no `schema` string"));
            return out;
        }
    }
    let mut r = Reader { issues: Vec::new() };
    let entry = read_entry(&obj, &mut r);
    out.issues = r.issues;
    out.entry = entry;
    out
}

fn read_entry(obj: &Map<String, Value>, r: &mut Reader) -> Option<Entry> {
    r.unknown_fields(obj, TOP_FIELDS, "");
    let Some(event) = r.string(obj, "event", "") else {
        r.issues.push(issue(Severity::Error, "no `event` string"));
        return None;
    };
    let event = Event::parse(event);
    if !event.is_known() {
        r.note(format!(
            "unknown event `{}`: the line is kept but counts for nothing",
            event.as_str()
        ));
    }
    let round = if let Some(Value::Object(round)) = obj.get("round") {
        r.unknown_fields(round, ROUND_FIELDS, "round.");
        let mut part = |key: &str| {
            let v = r.uint(round, key, "round.");
            if v.is_none() {
                r.note(format!("`round.{key}` missing, counted as 0"));
            }
            v.unwrap_or(0)
        };
        RoundKey {
            serial: part("serial"),
            nonce: part("nonce"),
        }
    } else {
        r.issues.push(issue(Severity::Error, "no `round` object"));
        return None;
    };
    let Some(code) = r.string(obj, "code", "") else {
        r.issues.push(issue(Severity::Error, "no `code` string"));
        return None;
    };
    let code = code.to_owned();

    let ended_by = r.string(obj, "ended_by", "").map(|s| {
        let e = EndedBy::parse(s);
        if !e.is_known() {
            r.note(format!("unknown `ended_by` value `{s}`"));
        }
        e
    });
    match (&event, &ended_by) {
        (Event::End, None) => r.note("an `end` line without `ended_by`"),
        (Event::Dispute, Some(_)) => r.note("a `dispute` line with `ended_by`"),
        _ => {}
    }
    let broken = r.boolean(obj, "broken", "").unwrap_or_else(|| {
        r.note("`broken` missing, counted as false");
        false
    });
    let result = match obj.get("result") {
        None | Some(Value::Null) => {
            if event == Event::End && !broken {
                r.warn("an `end` line without a `result` (the code did not decode?)");
            }
            None
        }
        Some(Value::Object(res)) => read_result(res, &code, r),
        Some(_) => {
            r.warn("`result` is not an object, ignored");
            None
        }
    };
    let submits = read_submits(obj, r);
    let dispute = r.string(obj, "dispute", "").map(str::to_owned);
    if event == Event::Dispute && dispute.as_deref().is_none_or(|d| d.trim().is_empty()) {
        r.warn("a `dispute` line without a note");
    }
    let edits = r.uint(obj, "edits", "").unwrap_or(0);
    let has_patch = match obj.get("patch") {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => {
            r.warn("`patch` is not a string");
            false
        }
    };
    match obj.get("report") {
        None | Some(Value::Null | Value::Object(_)) => {}
        Some(_) => r.warn("`report` is not an object"),
    }
    let fingerprint = r.string(obj, "fingerprint", "").and_then(|f| {
        if f.len() == 64 && f.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some(f.to_ascii_lowercase())
        } else {
            r.warn("`fingerprint` is not 64 hex digits");
            None
        }
    });
    let engine = r.string(obj, "engine", "").and_then(|e| {
        if is_version(e) {
            Some(e.to_owned())
        } else {
            r.warn("`engine` is not a major.minor.patch version");
            None
        }
    });
    let edit_log_len = match obj.get("edit_log") {
        None | Some(Value::Null) => 0,
        Some(Value::Array(a)) => {
            let bad = a
                .iter()
                .filter(|e| {
                    !e.as_object()
                        .is_some_and(|o| o.get("t_ms").is_some_and(Value::is_number))
                })
                .count();
            if bad > 0 {
                r.warn(format!("{bad} `edit_log` entries have no `t_ms`"));
            }
            a.len()
        }
        Some(_) => {
            r.warn("`edit_log` is not an array");
            0
        }
    };
    if event == Event::End && !has_patch && result.as_ref().is_some_and(|x| x.submits > 0) {
        r.note("an `end` line with scored submits but no `patch` (over the size budget?)");
    }
    Some(Entry {
        event,
        round,
        code,
        ended_by,
        broken,
        result,
        submits,
        dispute,
        edits,
        has_patch,
        fingerprint,
        engine,
        edit_log_len,
    })
}

fn is_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
}

fn read_submits(obj: &Map<String, Value>, r: &mut Reader) -> Vec<SubmitScore> {
    let list = match obj.get("submits") {
        None | Some(Value::Null) => {
            r.note("`submits` missing, counted as none");
            return Vec::new();
        }
        Some(Value::Array(a)) => a,
        Some(_) => {
            r.warn("`submits` is not an array");
            return Vec::new();
        }
    };
    let mut out = Vec::with_capacity(list.len());
    let mut bad = 0;
    for item in list {
        let Some(o) = item.as_object() else {
            bad += 1;
            continue;
        };
        let hundredths = |key: &str| {
            o.get(key)
                .and_then(Value::as_u64)
                .filter(|v| *v <= 10_000)
                .map(|v| v as u16)
        };
        match (
            hundredths("sound"),
            hundredths("structure"),
            hundredths("combined"),
        ) {
            (Some(sound), Some(structure), Some(combined)) => out.push(SubmitScore {
                at_ms: o.get("at_ms").and_then(Value::as_u64).unwrap_or(0),
                passed: o.get("passed").and_then(Value::as_bool).unwrap_or(false),
                sound,
                structure,
                combined,
            }),
            _ => bad += 1,
        }
    }
    if bad > 0 {
        r.warn(format!("{bad} `submits` entries are not scores in hundredths of a percent (0 to 10000), dropped"));
    }
    out
}

fn read_result(res: &Map<String, Value>, code: &str, r: &mut Reader) -> Option<ResultView> {
    match res.get("schema").and_then(Value::as_str) {
        Some(ROUND_RESULT_SCHEMA) => {}
        Some(other) => {
            r.warn(format!("`result.schema` is `{other}`, this report reads `{ROUND_RESULT_SCHEMA}`: result ignored"));
            return None;
        }
        None => r.note("`result.schema` missing"),
    }
    r.unknown_fields(res, RESULT_FIELDS, "result.");
    let outcome = if let Some(s) = r.string(res, "outcome", "result.") {
        let o = Outcome::parse(s);
        if !o.is_known() {
            r.note(format!("unknown `result.outcome` value `{s}`"));
        }
        o
    } else {
        r.warn("`result.outcome` missing");
        Outcome::Unknown(String::new())
    };
    let first_submit = match res.get("first_submit") {
        None | Some(Value::Null) => None,
        Some(Value::Object(report)) => read_score(report, r),
        Some(_) => {
            r.warn("`result.first_submit` is not an object");
            None
        }
    };
    let submits = r.uint(res, "submits", "result.").unwrap_or(0);
    let time_to_pass_ms = r.uint(res, "time_to_pass_ms", "result.");
    let active_ms = r.uint(res, "active_ms", "result.").unwrap_or(0);
    let assists = match res.get("assists") {
        Some(Value::Object(a)) => {
            r.unknown_fields(a, ASSIST_FIELDS, "result.assists.");
            Assists {
                hints: r.uint(a, "hints", "result.assists.").unwrap_or(0),
                live_meter: r
                    .boolean(a, "live_meter", "result.assists.")
                    .unwrap_or(false),
                revealed: r.boolean(a, "revealed", "result.assists.").unwrap_or(false),
                dim_unused: r
                    .boolean(a, "dim_unused", "result.assists.")
                    .unwrap_or(false),
            }
        }
        None | Some(Value::Null) => {
            r.note("`result.assists` missing, counted as none");
            Assists::default()
        }
        Some(_) => {
            r.warn("`result.assists` is not an object");
            Assists::default()
        }
    };
    let agreement = r.string(res, "agreement", "result.").map(|s| {
        let a = Agreement::parse(s);
        if !a.is_known() {
            r.note(format!(
                "unknown `result.agreement` value `{s}`, counted as unknown"
            ));
        }
        a
    });
    let rating = match res.get("rating") {
        None | Some(Value::Null) => None,
        Some(v) => {
            if let Some(n) = v
                .as_u64()
                .filter(|n| (1..=u64::from(RATING_MAX)).contains(n))
            {
                Some(n as u8)
            } else {
                r.warn(format!("`result.rating` is not 1 to {RATING_MAX}, ignored"));
                None
            }
        }
    };
    // The invariants `RoundResult` enforces when it is written; a line that breaks them is still read.
    if outcome.is_known() && (outcome == Outcome::Passed) != time_to_pass_ms.is_some() {
        r.warn("`result.time_to_pass_ms` and a `passed` outcome disagree");
    }
    if (submits > 0) != first_submit.is_some()
        && !matches!(res.get("first_submit"), Some(Value::Object(_)))
    {
        r.warn("`result.submits` and `result.first_submit` disagree");
    }
    if let Some(rc) = res.get("code").and_then(Value::as_str)
        && super::code::normalize(rc) != super::code::normalize(code)
    {
        r.warn("`result.code` differs from `code`");
    }
    Some(ResultView {
        outcome,
        first_submit,
        submits,
        time_to_pass_ms,
        active_ms,
        assists,
        agreement,
        rating,
    })
}

fn percent(report: &Map<String, Value>, part: &str, r: &mut Reader) -> Option<f64> {
    let p = report.get(part)?.get("percent")?.as_f64();
    match p {
        Some(p) if p.is_finite() && (0.0..=100.0).contains(&p) => Some(p),
        _ => {
            r.warn(format!("`first_submit.{part}.percent` is not a percentage"));
            None
        }
    }
}

fn read_score(report: &Map<String, Value>, r: &mut Reader) -> Option<ScoreView> {
    let sound = percent(report, "sound", r);
    let structure = percent(report, "structure", r);
    let combined = percent(report, "combined", r);
    let (Some(sound), Some(structure), Some(combined)) = (sound, structure, combined) else {
        r.warn("`result.first_submit` has no readable sound, structure and combined percentages, ignored");
        return None;
    };
    let scorer = |part: &str| {
        report
            .get(part)
            .and_then(|p| p.get("scorer"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Some(ScoreView {
        sound,
        structure,
        combined,
        pass_mark: report.get("pass_mark").and_then(Value::as_f64),
        passed: report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        scorers: [scorer("sound"), scorer("structure"), scorer("combined")],
        engine: report
            .get("engine")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A minimal valid `end` line, the shape `RoundLogEntry` writes.
    fn end_line() -> Value {
        json!({
            "schema": "enth.round-log/1",
            "event": "end",
            "round": {"serial": 3, "nonce": 1_760_000_000_000_u64},
            "code": "1080-1000-0001-AB",
            "ended_by": "new_round",
            "broken": false,
            "result": {
                "schema": "enth.round-result/1",
                "code": "1080-1000-0001-AB",
                "outcome": "passed",
                "first_submit": {
                    "schema": "enth.score-report/1",
                    "sound": {"scorer": "sound/1", "percent": 91.25},
                    "structure": {"scorer": "structure/1", "percent": 75.5},
                    "combined": {"scorer": "combined/1", "percent": 85.7},
                    "pass_mark": 90, "passed": true, "engine": "0.2.0"
                },
                "submits": 2, "time_to_pass_ms": 95_250, "active_ms": 120_000,
                "assists": {"hints": 1, "live_meter": false, "revealed": false},
                "agreement": "about_right", "rating": 4
            },
            "submits": [
                {"at_ms": 60_000, "passed": false, "sound": 8800, "structure": 7000, "combined": 8200},
                {"at_ms": 95_250, "passed": true, "sound": 9125, "structure": 7550, "combined": 8570}
            ],
            "dispute": null,
            "edits": 41,
            "patch": "{\"schema\":1}",
            "report": null,
            "fingerprint": "0123abcd".repeat(8),
            "engine": "0.2.0",
            "edit_log": [{"t_ms": 1200, "source": "editor", "ops": []}]
        })
    }

    fn parse(v: &Value) -> Parsed {
        parse_line(&v.to_string())
    }

    fn severities(p: &Parsed) -> Vec<Severity> {
        p.issues.iter().map(|i| i.severity).collect()
    }

    #[test]
    fn a_complete_line_reads_without_a_single_issue() {
        let p = parse(&end_line());
        assert!(p.issues.is_empty(), "{:?}", p.issues);
        let e = p.entry.unwrap();
        assert_eq!(e.event, Event::End);
        assert_eq!(
            e.round,
            RoundKey {
                serial: 3,
                nonce: 1_760_000_000_000
            }
        );
        assert_eq!(e.ended_by, Some(EndedBy::NewRound));
        assert_eq!(e.submits.len(), 2);
        assert_eq!(e.edit_log_len, 1);
        let res = e.result.unwrap();
        assert_eq!(res.outcome, Outcome::Passed);
        assert_eq!(res.rating, Some(4));
        assert_eq!(res.agreement, Some(Agreement::AboutRight));
        assert_eq!(res.time_to_pass_ms, Some(95_250));
        let first = res.first_submit.unwrap();
        assert_eq!(
            (first.sound, first.pass_mark, first.passed),
            (91.25, Some(90.0), true)
        );
        assert_eq!(first.scorers[0].as_deref(), Some("sound/1"));
        assert!(res.assists.any());
    }

    /// Lane HINT (FU1): the dim assist is a known field (no note) and marks the round assisted; a line without it reads
    /// as before.
    #[test]
    fn the_dim_assist_reads_without_an_issue_and_counts_as_an_assist() {
        let mut v = end_line();
        v["result"]["assists"] =
            json!({"hints": 0, "live_meter": false, "revealed": false, "dim_unused": true});
        let p = parse(&v);
        assert!(p.issues.is_empty(), "{:?}", p.issues);
        let a = p.entry.unwrap().result.unwrap().assists;
        assert!(a.dim_unused && a.any());
        v["result"]["assists"] = json!({"hints": 0, "live_meter": false, "revealed": false});
        let a = parse(&v).entry.unwrap().result.unwrap().assists;
        assert!(!a.dim_unused && !a.any());
    }

    #[test]
    fn blank_lines_are_not_issues() {
        let p = parse_line("   ");
        assert!(p.entry.is_none() && p.issues.is_empty());
    }

    #[test]
    fn unusable_lines_are_errors_and_give_no_entry() {
        for (text, what) in [
            ("{not json", "not JSON"),
            ("[1,2]", "not a JSON object"),
            (r#"{"event":"end"}"#, "no `schema`"),
            (r#"{"schema":"enth.round-log/1"}"#, "no `event`"),
            (
                r#"{"schema":"enth.round-log/1","event":"end","code":"x"}"#,
                "no `round`",
            ),
            (
                r#"{"schema":"enth.round-log/1","event":"end","round":{"serial":1,"nonce":2}}"#,
                "no `code`",
            ),
        ] {
            let p = parse_line(text);
            assert!(p.entry.is_none(), "{text}");
            assert_eq!(p.issues.len(), 1, "{text}: {:?}", p.issues);
            assert_eq!(p.issues[0].severity, Severity::Error);
            assert!(
                p.issues[0].message.starts_with(what),
                "{text}: {}",
                p.issues[0].message
            );
        }
    }

    #[test]
    fn another_schema_version_is_skipped_with_a_warning() {
        let mut v = end_line();
        v["schema"] = json!("enth.round-log/2");
        let p = parse(&v);
        assert!(p.entry.is_none() && p.other_schema);
        assert_eq!(severities(&p), vec![Severity::Warn]);
    }

    #[test]
    fn unknown_fields_and_values_from_a_newer_build_are_notes_and_the_line_still_reads() {
        let mut v = end_line();
        v["client"] = json!({"os": "linux"});
        v["ended_by"] = json!("timeout");
        v["result"]["future"] = json!(1);
        v["result"]["outcome"] = json!("timed_out");
        v["result"]["time_to_pass_ms"] = Value::Null;
        v["result"]["agreement"] = json!("much_too_high");
        v["result"]["assists"]["coach"] = json!(true);
        v["round"]["tab"] = json!(2);
        let p = parse(&v);
        assert!(
            severities(&p).iter().all(|s| *s == Severity::Note),
            "{:?}",
            p.issues
        );
        let msgs: Vec<&str> = p.issues.iter().map(|i| i.message.as_str()).collect();
        for want in [
            "`client`",
            "`result.future`",
            "`result.assists.coach`",
            "`round.tab`",
            "`timeout`",
            "`timed_out`",
            "`much_too_high`",
        ] {
            assert!(msgs.iter().any(|m| m.contains(want)), "{want} in {msgs:?}");
        }
        let e = p.entry.unwrap();
        assert_eq!(e.ended_by, Some(EndedBy::Unknown("timeout".to_owned())));
        let res = e.result.unwrap();
        assert_eq!(res.outcome, Outcome::Unknown("timed_out".to_owned()));
        assert_eq!(
            res.agreement,
            Some(Agreement::Unknown("much_too_high".to_owned()))
        );
        assert!(!res.agreement.unwrap().is_known());
    }

    #[test]
    fn fields_an_older_build_lacked_default_with_a_note_or_nothing() {
        // The F1r-review fields (`fingerprint`, `engine`, `edit_log`) default when missing, silently; `rating` too.
        let mut v = end_line();
        for key in ["fingerprint", "engine", "edit_log", "report", "dispute"] {
            v.as_object_mut().unwrap().remove(key);
        }
        v["result"].as_object_mut().unwrap().remove("rating");
        let p = parse(&v);
        assert!(p.issues.is_empty(), "{:?}", p.issues);
        let e = p.entry.unwrap();
        assert_eq!((e.fingerprint, e.engine, e.edit_log_len), (None, None, 0));
        assert_eq!(e.result.unwrap().rating, None);
    }

    #[test]
    fn bad_fields_are_warnings_and_are_dropped() {
        let mut v = end_line();
        v["result"]["rating"] = json!(7);
        v["fingerprint"] = json!("abc");
        v["engine"] = json!("zero.two");
        v["edits"] = json!("many");
        v["submits"][0]["sound"] = json!(12_000);
        let p = parse(&v);
        let msgs: Vec<&str> = p.issues.iter().map(|i| i.message.as_str()).collect();
        assert!(
            p.issues.iter().all(|i| i.severity == Severity::Warn),
            "{msgs:?}"
        );
        for want in [
            "`result.rating`",
            "`fingerprint`",
            "`engine`",
            "`edits`",
            "`submits` entries",
        ] {
            assert!(msgs.iter().any(|m| m.contains(want)), "{want} in {msgs:?}");
        }
        let e = p.entry.unwrap();
        assert_eq!(e.result.as_ref().unwrap().rating, None);
        assert_eq!((e.fingerprint, e.engine, e.edits), (None, None, 0));
        assert_eq!(e.submits.len(), 1, "the out-of-range submit is dropped");
    }

    #[test]
    fn percentages_outside_0_to_100_drop_the_first_submit() {
        let mut v = end_line();
        v["result"]["first_submit"]["sound"]["percent"] = json!(140.0);
        let p = parse(&v);
        assert!(p.issues.iter().any(|i| i.message.contains("first_submit")));
        assert!(p.entry.unwrap().result.unwrap().first_submit.is_none());
    }

    #[test]
    fn a_result_the_writer_could_not_have_made_is_read_with_a_warning() {
        let mut v = end_line();
        v["result"]["time_to_pass_ms"] = Value::Null; // passed without a time
        let p = parse(&v);
        assert!(
            p.issues.iter().any(|i| i.message.contains("disagree")),
            "{:?}",
            p.issues
        );
        assert!(p.entry.is_some());
    }

    #[test]
    fn a_result_from_another_schema_is_ignored_with_a_warning() {
        let mut v = end_line();
        v["result"]["schema"] = json!("enth.round-result/2");
        let p = parse(&v);
        assert!(p.entry.unwrap().result.is_none());
        assert!(
            p.issues
                .iter()
                .any(|i| i.severity == Severity::Warn && i.message.contains("round-result/2"))
        );
    }

    #[test]
    fn a_dispute_line_has_a_note_and_no_ended_by() {
        let mut v = end_line();
        v["event"] = json!("dispute");
        v["ended_by"] = Value::Null;
        v["dispute"] = json!("sounded the same to me");
        let p = parse(&v);
        assert!(p.issues.is_empty(), "{:?}", p.issues);
        let e = p.entry.unwrap();
        assert_eq!(e.event, Event::Dispute);
        assert_eq!(e.dispute.as_deref(), Some("sounded the same to me"));
        v["dispute"] = Value::Null;
        assert!(
            parse(&v)
                .issues
                .iter()
                .any(|i| i.message.contains("without a note"))
        );
    }

    #[test]
    fn a_broken_round_may_have_no_result() {
        let mut v = end_line();
        v["broken"] = json!(true);
        v["result"] = Value::Null;
        v["submits"] = json!([]);
        assert!(parse(&v).issues.is_empty(), "{:?}", parse(&v).issues);
        v["broken"] = json!(false);
        assert!(
            parse(&v)
                .issues
                .iter()
                .any(|i| i.message.contains("without a `result`"))
        );
    }

    #[test]
    fn integers_written_as_floats_are_accepted() {
        let mut v = end_line();
        v["round"]["serial"] = json!(3.0);
        let p = parse(&v);
        assert!(p.issues.is_empty(), "{:?}", p.issues);
        assert_eq!(p.entry.unwrap().round.serial, 3);
    }
}
