//! The optional end-of-session question: "Want more?". The round log already records "another round?" after every
//! round (`ended_by: new_round`); this is the plain answer a friend gives when they send their file, typed in by the
//! owner into a small JSONL file of schema `enth.gr1-survey/1`, one line per player:
//!
//! ```json
//! {"schema":"enth.gr1-survey/1","player":"p1","want_more":"yes","os":"linux","daw":"reaper","note":"..."}
//! ```
//!
//! `player` is the anonymous id the report uses for that player's log. `want_more` is `yes`, `maybe` or `no`; `os`,
//! `daw` and `note` are optional context. Read tolerantly like the log (unknown fields and values are notes).

use serde_json::Value;

use super::logs::Problem;
use super::schema::Severity;

/// The survey's schema.
pub const SURVEY_SCHEMA: &str = "enth.gr1-survey/1";

/// A player's answer to "want more?".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WantMore {
    /// Yes.
    Yes,
    /// Maybe.
    Maybe,
    /// No.
    No,
    /// An answer this reader does not know, as written.
    Other(String),
}

impl WantMore {
    /// As written.
    pub fn as_str(&self) -> &str {
        match self {
            WantMore::Yes => "yes",
            WantMore::Maybe => "maybe",
            WantMore::No => "no",
            WantMore::Other(s) => s,
        }
    }
}

/// One player's survey line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// The player's anonymous id.
    pub player: String,
    /// The answer.
    pub want_more: WantMore,
    /// The player's OS, if given.
    pub os: Option<String>,
    /// The player's DAW, if given.
    pub daw: Option<String>,
    /// A free note.
    pub note: Option<String>,
}

pub(super) const FIELDS: &[&str] = &["schema", "player", "want_more", "os", "daw", "note"];

/// Read a survey file's text; the last line of a player wins.
pub fn read(label: &str, text: &str) -> (Vec<Answer>, Vec<Problem>) {
    let mut answers: Vec<Answer> = Vec::new();
    let mut problems = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let place = format!("{label}:{}", i + 1);
        let mut say = |severity, message: String| {
            problems.push(Problem {
                severity,
                place: place.clone(),
                message,
            });
        };
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(line) else {
            say(Severity::Error, "not a JSON object".to_owned());
            continue;
        };
        match obj.get("schema").and_then(Value::as_str) {
            Some(SURVEY_SCHEMA) => {}
            Some(other) => {
                say(
                    Severity::Warn,
                    format!("schema `{other}`: this report reads `{SURVEY_SCHEMA}`, line skipped"),
                );
                continue;
            }
            None => {
                say(Severity::Error, "no `schema` string".to_owned());
                continue;
            }
        }
        for key in obj.keys().filter(|k| !FIELDS.contains(&k.as_str())) {
            say(Severity::Note, format!("unknown field `{key}` ignored"));
        }
        let (Some(player), Some(want)) = (
            obj.get("player")
                .and_then(Value::as_str)
                .filter(|p| !p.trim().is_empty()),
            obj.get("want_more").and_then(Value::as_str),
        ) else {
            say(
                Severity::Error,
                "needs `player` and `want_more` strings".to_owned(),
            );
            continue;
        };
        let want_more = match want.trim().to_ascii_lowercase().as_str() {
            "yes" => WantMore::Yes,
            "maybe" => WantMore::Maybe,
            "no" => WantMore::No,
            other => {
                say(
                    Severity::Note,
                    format!("unknown `want_more` value `{other}`"),
                );
                WantMore::Other(other.to_owned())
            }
        };
        let text_of = |key: &str| obj.get(key).and_then(Value::as_str).map(str::to_owned);
        let answer = Answer {
            player: player.trim().to_owned(),
            want_more,
            os: text_of("os"),
            daw: text_of("daw"),
            note: text_of("note"),
        };
        match answers.iter_mut().find(|a| a.player == answer.player) {
            Some(existing) => *existing = answer,
            None => answers.push(answer),
        }
    }
    (answers, problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn answers_read_and_the_last_line_of_a_player_wins() {
        let text = [
            json!({"schema": SURVEY_SCHEMA, "player": "p1", "want_more": "Maybe", "os": "linux", "daw": "reaper"}),
            json!({"schema": SURVEY_SCHEMA, "player": "p2", "want_more": "no", "future": 1}),
            json!({"schema": SURVEY_SCHEMA, "player": "p1", "want_more": "yes", "note": "more levels"}),
            json!({"schema": SURVEY_SCHEMA, "player": "p3", "want_more": "definitely"}),
        ]
        .map(|v| v.to_string())
        .join("\n");
        let (answers, problems) = read("s.jsonl", &text);
        assert_eq!(answers.len(), 3);
        assert_eq!(answers[0].want_more, WantMore::Yes, "p1's second line wins");
        assert_eq!(answers[0].note.as_deref(), Some("more levels"));
        assert_eq!(answers[0].os, None, "and replaces the whole answer");
        assert_eq!(
            answers[2].want_more,
            WantMore::Other("definitely".to_owned())
        );
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().all(|p| p.severity == Severity::Note));
    }

    #[test]
    fn bad_lines_are_errors() {
        let text = "nope\n{\"schema\":\"enth.gr1-survey/1\",\"player\":\"p1\"}\n{\"player\":\"p\"}";
        let (answers, problems) = read("s.jsonl", text);
        assert!(answers.is_empty(), "{answers:?}");
        assert_eq!(problems.len(), 3);
        assert!(problems.iter().all(|p| p.severity == Severity::Error));
    }
}
