//! From log text to rounds: parse each line, then merge one player's lines into **rounds**.
//!
//! - One round is one `RoundKey` (`round.serial` and `round.nonce`) with one code. Its lines are an optional `dispute`
//!   line and one or more `end` lines. A project restored by the host can log a round again after its first `end`
//!   (docs/session.md, "The round log"), so **the last `end` line of a round, in file order, wins**; earlier ones only
//!   raise the round's `end_lines`.
//! - Several files of one player (two sittings, two plugin instances, a file sent twice) merge the same way, in the
//!   order they are given.
//! - Disputes are listed once per round and note, whether they came on a `dispute` line, on the `end` line, or both.
//! - Nothing here reads a file or a clock: [`load`] takes text.

use std::collections::{BTreeMap, BTreeSet};

use super::code;
use super::privacy;
use super::schema::{
    self, Agreement, Assists, EndedBy, Entry, Event, Outcome, ResultView, RoundKey, ScoreView,
    Severity, SubmitScore,
};

/// One log's text and whose it is.
pub struct Source {
    /// The player's anonymous id.
    pub player: String,
    /// What to call the file in messages: its base name, never its path (a path can hold a user name).
    pub label: String,
    /// The text.
    pub text: String,
}

/// A deviation, with its place (`p1.jsonl:12`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// How bad.
    pub severity: Severity,
    /// File label and line number.
    pub place: String,
    /// What.
    pub message: String,
}

/// What reading the files did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadStats {
    /// Files read.
    pub files: usize,
    /// Non-blank lines.
    pub lines: usize,
    /// Lines read into entries.
    pub entries: usize,
    /// Lines skipped as unusable.
    pub unusable: usize,
    /// Lines skipped for another schema version.
    pub other_schema: usize,
}

/// Where a round stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// The player tried it: a scored submit, or a reveal.
    Played,
    /// Ended without a submit and without a reveal.
    Skipped,
    /// The round could not continue.
    Broken,
    /// An `end` line with no readable result.
    NoResult,
}

/// One round of one player.
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    /// The player.
    pub player: String,
    /// The round.
    pub key: RoundKey,
    /// The code as logged.
    pub code: String,
    /// The code's comparison form.
    pub code_norm: String,
    /// The level the code names, if it can be read.
    pub level: Option<u8>,
    /// A long code with hand-set amounts.
    pub custom: bool,
    /// What ended it (of the last `end` line).
    pub ended_by: Option<EndedBy>,
    /// The round could not continue.
    pub broken: bool,
    /// The result.
    pub result: Option<ResultView>,
    /// Every scored submit.
    pub submits: Vec<SubmitScore>,
    /// How many `end` lines the round had: more than one means a host restore logged it again.
    pub end_lines: usize,
    /// The target's fingerprint.
    pub fingerprint: Option<String>,
    /// The engine that played it.
    pub engine: Option<String>,
    /// Edits made.
    pub edits: u64,
}

impl Round {
    /// Where the round stands.
    pub fn status(&self) -> Status {
        if self.broken {
            return Status::Broken;
        }
        let Some(res) = &self.result else {
            return Status::NoResult;
        };
        if res.submits > 0
            || !self.submits.is_empty()
            || res.assists.revealed
            || res.outcome == Outcome::GaveUp
        {
            Status::Played
        } else {
            Status::Skipped
        }
    }

    /// The round was played (not skipped, broken or unreadable).
    pub fn played(&self) -> bool {
        self.status() == Status::Played
    }

    /// A submit passed.
    pub fn passed(&self) -> bool {
        self.result.as_ref().is_some_and(|r| match r.outcome {
            Outcome::Passed => true,
            Outcome::Unknown(_) => r.time_to_pass_ms.is_some(),
            _ => false,
        })
    }

    /// The round used a hint, the live meter or the reveal.
    pub fn assisted(&self) -> bool {
        self.result.as_ref().is_some_and(|r| r.assists.any())
    }

    /// The assists used.
    pub fn assists(&self) -> Assists {
        self.result.as_ref().map(|r| r.assists).unwrap_or_default()
    }

    /// The recorded score (the first submit's report).
    pub fn first(&self) -> Option<&ScoreView> {
        self.result.as_ref().and_then(|r| r.first_submit.as_ref())
    }

    /// Active time to the first passing submit.
    pub fn time_to_pass_ms(&self) -> Option<u64> {
        self.result
            .as_ref()
            .and_then(|r| r.time_to_pass_ms)
            .filter(|_| self.passed())
    }

    /// The best sound % over every submit (the first submit's when the line lists none).
    pub fn best_sound(&self) -> Option<f64> {
        let best = self
            .submits
            .iter()
            .map(|s| f64::from(s.sound) / 100.0)
            .max_by(f64::total_cmp);
        best.or_else(|| self.first().map(|f| f.sound))
    }

    /// The vote, if the player gave one this reader understands.
    pub fn vote(&self) -> Option<&Agreement> {
        self.result.as_ref().and_then(|r| r.agreement.as_ref())
    }

    /// The blind rating, 1 to 5.
    pub fn rating(&self) -> Option<u8> {
        self.result.as_ref().and_then(|r| r.rating)
    }

    /// "Another round?" was answered yes.
    pub fn wanted_more(&self) -> bool {
        self.ended_by == Some(EndedBy::NewRound)
    }
}

/// A score dispute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispute {
    /// The player.
    pub player: String,
    /// The round.
    pub key: RoundKey,
    /// The code as logged.
    pub code: String,
    /// The note.
    pub note: String,
    /// The round has an `end` line (a crash leaves a dispute alone).
    pub has_end: bool,
}

/// Everything the files held.
#[derive(Debug, Default)]
pub struct Loaded {
    /// Player ids, sorted.
    pub players: Vec<String>,
    /// File labels per player, in the order given.
    pub files: BTreeMap<String, Vec<String>>,
    /// Rounds, per player in order of first appearance.
    pub rounds: Vec<Round>,
    /// Disputes, deduplicated, in the order found.
    pub disputes: Vec<Dispute>,
    /// Every deviation, in file order.
    pub problems: Vec<Problem>,
    /// Where something that looks like personal data was found: place and kind, never the text.
    pub privacy: Vec<Problem>,
    /// What reading did.
    pub stats: ReadStats,
}

/// A code the players played: what the agent run must play too.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CodeRow {
    /// The code as the first player logged it.
    pub code: String,
    /// The level the code names, if it can be read.
    pub level: Option<u8>,
    /// Players who played it.
    pub players: usize,
}

impl Loaded {
    /// The distinct codes of played rounds, by level (unreadable levels last) and code.
    pub fn played_codes(&self) -> Vec<CodeRow> {
        // Sorted by (level, comparison form); the first round's text is the one shown.
        let mut first: BTreeMap<(u8, &str), &Round> = BTreeMap::new();
        let mut players: BTreeMap<(u8, &str), BTreeSet<&str>> = BTreeMap::new();
        for r in self.rounds.iter().filter(|r| r.played()) {
            let key = (r.level.unwrap_or(u8::MAX), r.code_norm.as_str());
            first.entry(key).or_insert(r);
            players.entry(key).or_default().insert(r.player.as_str());
        }
        first
            .into_iter()
            .map(|(key, r)| CodeRow {
                code: r.code.clone(),
                level: r.level,
                players: players[&key].len(),
            })
            .collect()
    }
}

/// Read every source and merge each player's lines into rounds.
pub fn load(sources: &[Source]) -> Loaded {
    let mut out = Loaded::default();
    // (player, round, normalized code) -> index into `out.rounds`.
    let mut index: BTreeMap<(String, RoundKey, String), usize> = BTreeMap::new();
    // (player, round, normalized code, note) of every dispute seen.
    let mut seen_disputes: BTreeMap<(String, RoundKey, String, String), usize> = BTreeMap::new();
    for source in sources {
        out.stats.files += 1;
        out.files
            .entry(source.player.clone())
            .or_default()
            .push(source.label.clone());
        for (i, line) in source.text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let place = format!("{}:{}", source.label, i + 1);
            out.stats.lines += 1;
            for kind in privacy::scan(line) {
                out.privacy.push(Problem {
                    severity: Severity::Warn,
                    place: place.clone(),
                    message: format!("looks like {kind}"),
                });
            }
            let parsed = schema::parse_line(line);
            for iss in parsed.issues {
                out.problems.push(Problem {
                    severity: iss.severity,
                    place: place.clone(),
                    message: iss.message,
                });
            }
            if parsed.other_schema {
                out.stats.other_schema += 1;
            }
            let Some(entry) = parsed.entry else {
                if !parsed.other_schema && !parsed.not_a_game_round {
                    out.stats.unusable += 1;
                }
                continue;
            };
            out.stats.entries += 1;
            merge(
                &mut out,
                &mut index,
                &mut seen_disputes,
                &source.player,
                entry,
            );
        }
    }
    out.players = out.files.keys().cloned().collect();
    for d in &mut out.disputes {
        let norm = code::normalize(&d.code);
        d.has_end = index.contains_key(&(d.player.clone(), d.key, norm));
    }
    out
}

fn merge(
    out: &mut Loaded,
    index: &mut BTreeMap<(String, RoundKey, String), usize>,
    seen_disputes: &mut BTreeMap<(String, RoundKey, String, String), usize>,
    player: &str,
    entry: Entry,
) {
    let norm = code::normalize(&entry.code);
    let key = (player.to_owned(), entry.round, norm.clone());
    if let Some(note) = entry
        .dispute
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        let dkey = (
            player.to_owned(),
            entry.round,
            norm.clone(),
            note.to_owned(),
        );
        seen_disputes.entry(dkey).or_insert_with(|| {
            out.disputes.push(Dispute {
                player: player.to_owned(),
                key: entry.round,
                code: entry.code.clone(),
                note: note.to_owned(),
                has_end: false,
            });
            out.disputes.len() - 1
        });
    }
    if entry.event != Event::End {
        return;
    }
    let info = code::decode(&entry.code);
    let round = Round {
        player: player.to_owned(),
        key: entry.round,
        code: entry.code,
        code_norm: norm,
        level: info.as_ref().and_then(|i| i.level),
        custom: info.as_ref().is_some_and(|i| i.custom),
        ended_by: entry.ended_by,
        broken: entry.broken,
        result: entry.result,
        submits: entry.submits,
        end_lines: 1,
        fingerprint: entry.fingerprint,
        engine: entry.engine,
        edits: entry.edits,
    };
    if let Some(&at) = index.get(&key) {
        // A host-restored round logged again: the last `end` line wins.
        let earlier = out.rounds[at].end_lines;
        out.rounds[at] = Round {
            end_lines: earlier + 1,
            ..round
        };
    } else {
        index.insert(key, out.rounds.len());
        out.rounds.push(round);
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use serde_json::{Value, json};

    fn end(serial: u64, outcome: &str, ended_by: &str) -> Value {
        let passed = outcome == "passed";
        json!({
            "schema": "enth.round-log/1", "event": "end",
            "round": {"serial": serial, "nonce": 5_000 + serial},
            "code": super::code::encode_short(2, 1, 100 + serial),
            "ended_by": ended_by, "broken": false,
            "result": {
                "schema": "enth.round-result/1", "outcome": outcome,
                "first_submit": {"sound": {"percent": if passed { 92.0 } else { 40.0 }},
                                 "structure": {"percent": 60.0}, "combined": {"percent": 70.0},
                                 "pass_mark": 90, "passed": passed},
                "submits": 1, "time_to_pass_ms": if passed { json!(60_000) } else { Value::Null },
                "active_ms": 90_000,
                "assists": {"hints": 0, "live_meter": false, "revealed": false},
                "agreement": "about_right", "rating": 4
            },
            "submits": [{"at_ms": 50_000, "passed": passed, "sound": if passed { 9200 } else { 4000 }, "structure": 6000, "combined": 7000}],
            "edits": 10
        })
    }

    fn src(player: &str, label: &str, lines: &[Value]) -> Source {
        Source {
            player: player.to_owned(),
            label: label.to_owned(),
            text: lines.iter().fold(String::new(), |mut out, l| {
                let _ = writeln!(out, "{l}");
                out
            }),
        }
    }

    #[test]
    fn the_last_end_line_of_a_restored_round_wins() {
        // Round 2 ended because the host loaded another state, then the project was reopened and the round was
        // finished and logged again: two `end` lines, one round, and the second one counts.
        let lines = [
            end(1, "passed", "new_round"),
            end(2, "not_passed", "host"),
            end(3, "passed", "keep"),
            end(2, "passed", "new_round"),
        ];
        let loaded = load(&[src("p1", "p1.jsonl", &lines)]);
        assert_eq!(loaded.rounds.len(), 3, "four end lines, three rounds");
        let two = loaded.rounds.iter().find(|r| r.key.serial == 2).unwrap();
        assert_eq!(two.end_lines, 2);
        assert!(two.passed(), "the later line, not the first, is the round");
        assert_eq!(two.ended_by, Some(EndedBy::NewRound));
        assert_eq!(loaded.rounds.iter().filter(|r| r.end_lines > 1).count(), 1);
        assert_eq!(loaded.stats.entries, 4);
    }

    #[test]
    fn played_codes_lists_each_code_once_with_its_players_and_leaves_skipped_rounds_out() {
        let mut skipped = end(5, "not_passed", "keep");
        skipped["result"]["submits"] = json!(0);
        skipped["result"]["first_submit"] = Value::Null;
        skipped["submits"] = json!([]);
        let a = [
            end(1, "passed", "keep"),
            end(2, "passed", "keep"),
            end(3, "passed", "keep"),
        ];
        let b = [
            end(2, "passed", "keep"),
            end(3, "not_passed", "keep"),
            end(4, "passed", "keep"),
            skipped,
        ];
        let loaded = load(&[src("a", "a.jsonl", &a), src("b", "b.jsonl", &b)]);
        let rows = loaded.played_codes();
        assert_eq!(
            rows.iter().map(|r| r.players).collect::<Vec<_>>(),
            [1, 2, 2, 1]
        );
        assert!(rows.iter().all(|r| r.level == Some(2)));
        let mut texts: Vec<&str> = rows.iter().map(|r| r.code.as_str()).collect();
        texts.dedup();
        assert_eq!(
            texts.len(),
            4,
            "four distinct codes, five rounds skipped or not"
        );
    }

    #[test]
    fn the_later_line_wins_even_when_it_is_worse() {
        let lines = [end(1, "passed", "keep"), end(1, "not_passed", "host")];
        let loaded = load(&[src("p1", "a.jsonl", &lines)]);
        assert_eq!(loaded.rounds.len(), 1);
        assert!(
            !loaded.rounds[0].passed(),
            "file order decides, not the better result"
        );
    }

    #[test]
    fn files_of_one_player_merge_and_a_file_sent_twice_adds_nothing() {
        let first = [
            end(1, "passed", "new_round"),
            end(2, "not_passed", "new_round"),
        ];
        let second = [end(2, "passed", "keep"), end(3, "passed", "keep")];
        let loaded = load(&[
            src("p1", "sitting1.jsonl", &first),
            src("p1", "sitting2.jsonl", &second),
            src("p1", "sitting1-again.jsonl", &first),
        ]);
        // Round 2 appears in sitting 1, sitting 2 and again in the resent copy of sitting 1, which is last.
        assert_eq!(loaded.rounds.len(), 3);
        assert_eq!(loaded.files["p1"].len(), 3);
        assert!(
            !loaded
                .rounds
                .iter()
                .find(|r| r.key.serial == 2)
                .unwrap()
                .passed()
        );
        assert_eq!(
            loaded
                .rounds
                .iter()
                .find(|r| r.key.serial == 2)
                .unwrap()
                .end_lines,
            3
        );
    }

    #[test]
    fn players_never_share_rounds_even_with_the_same_ids_and_codes() {
        let lines = [end(1, "passed", "keep")];
        let loaded = load(&[src("p1", "a.jsonl", &lines), src("p2", "b.jsonl", &lines)]);
        assert_eq!(loaded.rounds.len(), 2);
        assert_eq!(loaded.players, vec!["p1", "p2"]);
    }

    #[test]
    fn the_same_serial_with_another_code_is_another_round() {
        // Two plugin instances can both be on round 1 at the same start timestamp; the code tells them apart.
        let mut other = end(1, "passed", "keep");
        other["code"] = json!(super::code::encode_short(3, 1, 999));
        let loaded = load(&[src("p1", "a.jsonl", &[end(1, "passed", "keep"), other])]);
        assert_eq!(loaded.rounds.len(), 2);
    }

    #[test]
    fn disputes_are_listed_once_whether_on_a_dispute_line_the_end_line_or_both() {
        let mut dispute = end(4, "not_passed", "keep");
        dispute["event"] = json!("dispute");
        dispute["ended_by"] = Value::Null;
        dispute["dispute"] = json!("sounded the same");
        let mut with_note = end(4, "not_passed", "keep");
        with_note["dispute"] = json!("sounded the same");
        let mut crashed = dispute.clone();
        crashed["round"] = json!({"serial": 9, "nonce": 5_009});
        crashed["code"] = json!(super::code::encode_short(1, 1, 109));
        crashed["dispute"] = json!("too harsh");
        let loaded = load(&[src("p1", "a.jsonl", &[dispute, with_note, crashed])]);
        assert_eq!(loaded.disputes.len(), 2);
        assert!(loaded.disputes[0].has_end && loaded.disputes[0].note == "sounded the same");
        assert!(
            !loaded.disputes[1].has_end,
            "a crash leaves a dispute with no end line"
        );
        assert_eq!(loaded.rounds.len(), 1, "a dispute line is not a round");
    }

    #[test]
    fn status_tells_played_from_skipped_broken_and_unreadable() {
        let mut skipped = end(1, "not_passed", "new_round");
        skipped["result"]["submits"] = json!(0);
        skipped["result"]["first_submit"] = Value::Null;
        skipped["submits"] = json!([]);
        let mut gave_up = skipped.clone();
        gave_up["round"]["serial"] = json!(2);
        gave_up["result"]["outcome"] = json!("gave_up");
        gave_up["result"]["assists"]["revealed"] = json!(true);
        let mut broken = skipped.clone();
        broken["round"]["serial"] = json!(3);
        broken["broken"] = json!(true);
        broken["result"] = Value::Null;
        let mut no_result = skipped.clone();
        no_result["round"]["serial"] = json!(4);
        no_result["result"] = Value::Null;
        let loaded = load(&[src("p1", "a.jsonl", &[skipped, gave_up, broken, no_result])]);
        let status: Vec<Status> = loaded.rounds.iter().map(Round::status).collect();
        assert_eq!(
            status,
            vec![
                Status::Skipped,
                Status::Played,
                Status::Broken,
                Status::NoResult
            ]
        );
        assert!(loaded.rounds[1].assisted() && !loaded.rounds[1].passed());
    }

    #[test]
    fn bad_lines_are_skipped_counted_and_placed() {
        let text = format!(
            "{}\n{{truncated by a crash\n\n{}\n",
            end(1, "passed", "keep"),
            json!({"schema": "enth.round-log/2", "event": "end"})
        );
        let loaded = load(&[Source {
            player: "p1".into(),
            label: "x.jsonl".into(),
            text,
        }]);
        assert_eq!(loaded.rounds.len(), 1);
        assert_eq!(
            loaded.stats,
            ReadStats {
                files: 1,
                lines: 3,
                entries: 1,
                unusable: 1,
                other_schema: 1
            }
        );
        let places: Vec<&str> = loaded
            .problems
            .iter()
            .filter(|p| p.severity > Severity::Note)
            .map(|p| p.place.as_str())
            .collect();
        assert_eq!(places, vec!["x.jsonl:2", "x.jsonl:4"]);
    }

    #[test]
    fn privacy_findings_name_the_line_and_the_kind_only() {
        let mut leaky = end(1, "passed", "keep");
        leaky["dispute"] = json!("mail alice@example.com");
        let loaded = load(&[src("p1", "a.jsonl", &[leaky])]);
        assert_eq!(loaded.privacy.len(), 1);
        assert_eq!(loaded.privacy[0].place, "a.jsonl:1");
        assert_eq!(loaded.privacy[0].message, "looks like an email address");
    }

    #[test]
    fn levels_come_from_the_code() {
        let lines: Vec<Value> = (1..=5u64)
            .map(|level| {
                let mut v = end(level, "passed", "keep");
                v["code"] = json!(super::code::encode_short(level as u8, 1, 7 + level));
                v
            })
            .collect();
        let loaded = load(&[src("p1", "a.jsonl", &lines)]);
        let levels: Vec<Option<u8>> = loaded.rounds.iter().map(|r| r.level).collect();
        assert_eq!(levels, vec![Some(1), Some(2), Some(3), Some(4), Some(5)]);
    }
}
