//! The agent baseline: what an agent, a no-op and a CMA-ES search score on the same codes the humans played.
//!
//! The report reads **attempts** from JSONL files of schema `enth.gr1-baseline/1`, one line per attempt, produced by
//! whoever ran the agent (`cargo xtask agent-eval`'s target track, or any script that plays the codes through the
//! `enth` CLI); the report runs nothing and spends nothing. `docs/gr1.md` has the field table. Arms are free labels:
//! `noop` and `cmaes` are the two floors and print as "no-op floor" and "CMA-ES floor"; any other label (`agent`,
//! `agent:sonnet-5-5`) is an agent and prints as written.
//!
//! Rates are pooled single-attempt rates (pass@1): passes over attempts. Attempts on one code are not independent, so
//! the Wilson intervals are a rough guide, as report 02 §2.10 notes (10 of 20 is [0.30, 0.70]).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::code;
use super::logs::{Problem, Round};
use super::schema::Severity;
use super::stats::{median, wilson};

/// The baseline file's schema.
pub const BASELINE_SCHEMA: &str = "enth.gr1-baseline/1";

/// The arm label of the no-op floor (the untouched start patch scored against the target).
pub const ARM_NOOP: &str = "noop";

/// The arm label of the CMA-ES floor (`enth match` on every parameter).
pub const ARM_CMAES: &str = "cmaes";

pub(super) const FIELDS: &[&str] = &[
    "schema",
    "arm",
    "code",
    "attempt",
    "passed",
    "sound",
    "structure",
    "combined",
    "pass_mark",
    "renders",
    "tokens",
    "cost_usd",
    "wall_s",
    "model",
    "note",
];

/// One attempt of one arm on one code.
#[derive(Clone, Debug, PartialEq)]
pub struct Attempt {
    /// The arm label.
    pub arm: String,
    /// The code as written.
    pub code: String,
    /// The code's comparison form.
    pub code_norm: String,
    /// The level the code names.
    pub level: Option<u8>,
    /// The attempt passed.
    pub passed: bool,
    /// Sound % of the final patch, when given.
    pub sound: Option<f64>,
    /// Renders spent.
    pub renders: Option<f64>,
    /// Tokens spent.
    pub tokens: Option<f64>,
    /// Cost in US dollars (an estimate).
    pub cost_usd: Option<f64>,
}

/// Read one baseline file's text. `label` names the file in messages.
pub fn read(label: &str, text: &str) -> (Vec<Attempt>, Vec<Problem>) {
    let mut attempts = Vec::new();
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
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                say(Severity::Error, format!("not JSON ({e})"));
                continue;
            }
        };
        let Some(obj) = value.as_object() else {
            say(Severity::Error, "not a JSON object".to_owned());
            continue;
        };
        match obj.get("schema").and_then(Value::as_str) {
            Some(BASELINE_SCHEMA) => {}
            Some(other) => {
                say(
                    Severity::Warn,
                    format!(
                        "schema `{other}`: this report reads `{BASELINE_SCHEMA}`, line skipped"
                    ),
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
        let (Some(arm), Some(code_text)) = (
            obj.get("arm")
                .and_then(Value::as_str)
                .filter(|a| !a.trim().is_empty()),
            obj.get("code").and_then(Value::as_str),
        ) else {
            say(Severity::Error, "needs `arm` and `code` strings".to_owned());
            continue;
        };
        let number = |key: &str| number_field(obj, key);
        let sound = number("sound").filter(|s| (0.0..=100.0).contains(s));
        let pass_mark = number("pass_mark");
        let passed = if let Some(p) = obj.get("passed").and_then(Value::as_bool) {
            p
        } else if let (Some(s), Some(m)) = (sound, pass_mark) {
            s >= m
        } else {
            say(
                Severity::Error,
                "needs `passed`, or `sound` with `pass_mark`".to_owned(),
            );
            continue;
        };
        let info = code::decode(code_text);
        if info.is_none() {
            say(
                Severity::Warn,
                "the code cannot be read, so the level is unknown".to_owned(),
            );
        }
        attempts.push(Attempt {
            arm: arm.trim().to_owned(),
            code: code_text.to_owned(),
            code_norm: code::normalize(code_text),
            level: info.and_then(|i| i.level),
            passed,
            sound,
            renders: number("renders"),
            tokens: number("tokens"),
            cost_usd: number("cost_usd"),
        });
    }
    (attempts, problems)
}

fn number_field(obj: &Map<String, Value>, key: &str) -> Option<f64> {
    obj.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
}

/// Passes, the rate and the median sound over a set of attempts or rounds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    /// Attempts (or rounds).
    pub n: usize,
    /// Of which passed.
    pub passes: usize,
    /// `passes / n`.
    pub rate: Option<f64>,
    /// The 95 % Wilson interval of the rate.
    pub ci: Option<(f64, f64)>,
    /// The median sound %.
    pub median_sound: Option<f64>,
}

/// A cell over `(passed, sound)` pairs.
pub fn cell(items: &[(bool, Option<f64>)]) -> Cell {
    let n = items.len();
    let passes = items.iter().filter(|(p, _)| *p).count();
    let sounds: Vec<f64> = items.iter().filter_map(|(_, s)| *s).collect();
    Cell {
        n,
        passes,
        rate: (n > 0).then(|| passes as f64 / n as f64),
        ci: wilson(passes, n),
        median_sound: median(&sounds),
    }
}

/// One row of the baseline tables: a group (humans or an arm) overall and by level.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The label printed.
    pub label: String,
    /// The arm label for an arm, `humans` for the people.
    pub id: String,
    /// Distinct codes.
    pub codes: usize,
    /// All attempts.
    pub overall: Cell,
    /// By level 1 to 5 (levels with no attempts are left out), and `None` for codes with no readable level.
    pub by_level: Vec<(Option<u8>, Cell)>,
    /// Median renders per attempt.
    pub median_renders: Option<f64>,
    /// Median tokens per attempt.
    pub median_tokens: Option<f64>,
    /// Median cost per attempt, in US dollars.
    pub median_cost_usd: Option<f64>,
    /// Total cost, in US dollars.
    pub total_cost_usd: Option<f64>,
}

fn by_level(items: &[(Option<u8>, bool, Option<f64>)]) -> Vec<(Option<u8>, Cell)> {
    let levels: BTreeSet<Option<u8>> = items.iter().map(|(l, _, _)| *l).collect();
    // `None` sorts first in an Option's order; the report wants it last.
    let mut order: Vec<Option<u8>> = levels.iter().copied().flatten().map(Some).collect();
    if levels.contains(&None) {
        order.push(None);
    }
    order
        .into_iter()
        .map(|level| {
            let rows: Vec<(bool, Option<f64>)> = items
                .iter()
                .filter(|(l, _, _)| *l == level)
                .map(|(_, p, s)| (*p, *s))
                .collect();
            (level, cell(&rows))
        })
        .collect()
}

fn group_of_attempts(id: &str, attempts: &[&Attempt]) -> Group {
    let items: Vec<(Option<u8>, bool, Option<f64>)> = attempts
        .iter()
        .map(|a| (a.level, a.passed, a.sound))
        .collect();
    let pairs: Vec<(bool, Option<f64>)> = items.iter().map(|(_, p, s)| (*p, *s)).collect();
    let costs: Vec<f64> = attempts.iter().filter_map(|a| a.cost_usd).collect();
    let pick = |f: fn(&Attempt) -> Option<f64>| {
        median(&attempts.iter().filter_map(|a| f(a)).collect::<Vec<_>>())
    };
    Group {
        label: arm_label(id),
        id: id.to_owned(),
        codes: attempts
            .iter()
            .map(|a| &a.code_norm)
            .collect::<BTreeSet<_>>()
            .len(),
        overall: cell(&pairs),
        by_level: by_level(&items),
        median_renders: pick(|a| a.renders),
        median_tokens: pick(|a| a.tokens),
        median_cost_usd: median(&costs),
        total_cost_usd: (!costs.is_empty()).then(|| costs.iter().sum()),
    }
}

/// The human group over the given rounds, scored by the best sound % of each.
pub fn group_of_rounds(label: &str, rounds: &[&Round]) -> Group {
    let items: Vec<(Option<u8>, bool, Option<f64>)> = rounds
        .iter()
        .map(|r| (r.level, r.passed(), r.best_sound()))
        .collect();
    let pairs: Vec<(bool, Option<f64>)> = items.iter().map(|(_, p, s)| (*p, *s)).collect();
    Group {
        label: label.to_owned(),
        id: "humans".to_owned(),
        codes: rounds
            .iter()
            .map(|r| &r.code_norm)
            .collect::<BTreeSet<_>>()
            .len(),
        overall: cell(&pairs),
        by_level: by_level(&items),
        median_renders: None,
        median_tokens: None,
        median_cost_usd: None,
        total_cost_usd: None,
    }
}

/// What an arm is called in the tables.
pub fn arm_label(arm: &str) -> String {
    match arm {
        ARM_NOOP => "no-op floor".to_owned(),
        ARM_CMAES => "CMA-ES floor".to_owned(),
        other => other.to_owned(),
    }
}

/// The baseline beside the humans.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    /// Distinct codes in the baseline files.
    pub codes: usize,
    /// Of those, codes at least one human played.
    pub codes_played_by_humans: usize,
    /// Every human round (played ones), by best sound %.
    pub humans_all: Group,
    /// The human rounds on the baseline's codes, when any.
    pub humans_same_codes: Option<Group>,
    /// The agents, then the no-op and CMA-ES floors, in that order.
    pub arms: Vec<Group>,
}

impl Comparison {
    /// The first arm that is neither floor: the agent whose solve rate GR1 records.
    pub fn has_agent(&self) -> bool {
        self.arms
            .iter()
            .any(|g| g.id != ARM_NOOP && g.id != ARM_CMAES)
    }

    /// Whether the arm is present.
    pub fn has_arm(&self, id: &str) -> bool {
        self.arms.iter().any(|g| g.id == id)
    }
}

/// Put the attempts beside the humans' played rounds.
pub fn compare(attempts: &[Attempt], rounds: &[Round]) -> Option<Comparison> {
    if attempts.is_empty() {
        return None;
    }
    let mut arms: BTreeMap<&str, Vec<&Attempt>> = BTreeMap::new();
    for a in attempts {
        arms.entry(a.arm.as_str()).or_default().push(a);
    }
    let mut groups: Vec<Group> = Vec::new();
    for (arm, list) in &arms {
        if *arm != ARM_NOOP && *arm != ARM_CMAES {
            groups.push(group_of_attempts(arm, list));
        }
    }
    for floor in [ARM_NOOP, ARM_CMAES] {
        if let Some(list) = arms.get(floor) {
            groups.push(group_of_attempts(floor, list));
        }
    }
    let baseline_codes: BTreeSet<&str> = attempts.iter().map(|a| a.code_norm.as_str()).collect();
    let played: Vec<&Round> = rounds.iter().filter(|r| r.played()).collect();
    let same: Vec<&Round> = played
        .iter()
        .copied()
        .filter(|r| baseline_codes.contains(r.code_norm.as_str()))
        .collect();
    let played_codes: BTreeSet<&str> = same.iter().map(|r| r.code_norm.as_str()).collect();
    Some(Comparison {
        codes: baseline_codes.len(),
        codes_played_by_humans: played_codes.len(),
        humans_all: group_of_rounds("humans (all rounds)", &played),
        humans_same_codes: (!same.is_empty())
            .then(|| group_of_rounds("humans (the baseline's codes)", &same)),
        arms: groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(arm: &str, code: &str, passed: bool, sound: f64) -> String {
        json!({"schema": BASELINE_SCHEMA, "arm": arm, "code": code, "passed": passed, "sound": sound,
               "renders": 40, "tokens": 100_000, "cost_usd": 0.25})
        .to_string()
    }

    fn l2(seed: u64) -> String {
        code::encode_short(2, 1, seed)
    }

    #[test]
    fn attempts_read_with_their_level_and_optional_fields() {
        let text = format!(
            "{}\n{}\n",
            line("agent", &l2(1), true, 93.5),
            json!({"schema": BASELINE_SCHEMA, "arm": "noop", "code": code::encode_short(4, 1, 9), "sound": 12.0, "pass_mark": 90})
        );
        let (attempts, problems) = read("b.jsonl", &text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(attempts.len(), 2);
        assert_eq!(
            (attempts[0].level, attempts[0].passed, attempts[0].renders),
            (Some(2), true, Some(40.0))
        );
        // Without `passed`, sound against pass_mark decides.
        assert_eq!(
            (attempts[1].level, attempts[1].passed, attempts[1].renders),
            (Some(4), false, None)
        );
    }

    #[test]
    fn bad_lines_are_reported_and_skipped_and_unknown_fields_are_notes() {
        let good = json!({"schema": BASELINE_SCHEMA, "arm": "agent", "code": l2(2), "passed": true, "extra": 1}).to_string();
        let text = [
            "{oops".to_owned(),
            json!({"arm": "x"}).to_string(),
            json!({"schema": "enth.gr1-baseline/2", "arm": "x"}).to_string(),
            json!({"schema": BASELINE_SCHEMA, "code": "1"}).to_string(),
            json!({"schema": BASELINE_SCHEMA, "arm": "a", "code": l2(3)}).to_string(),
            good,
        ]
        .join("\n");
        let (attempts, problems) = read("b.jsonl", &text);
        assert_eq!(attempts.len(), 1);
        let sev: Vec<Severity> = problems.iter().map(|p| p.severity).collect();
        assert_eq!(
            sev,
            vec![
                Severity::Error,
                Severity::Error,
                Severity::Warn,
                Severity::Error,
                Severity::Error,
                Severity::Note
            ]
        );
        assert_eq!(problems[5].place, "b.jsonl:6");
    }

    #[test]
    fn a_group_pools_attempts_into_a_rate_a_median_and_costs() {
        let text = [
            line("agent", &l2(1), true, 95.0),
            line("agent", &l2(1), false, 70.0),
            line("agent", &l2(2), true, 92.0),
            line("agent", &l2(2), true, 91.0),
            line("noop", &l2(1), false, 5.0),
            line("noop", &l2(2), false, 8.0),
            line("cmaes", &l2(1), true, 90.5),
        ]
        .join("\n");
        let (attempts, problems) = read("b.jsonl", &text);
        assert!(problems.is_empty(), "{problems:?}");
        let cmp = compare(&attempts, &[]).unwrap();
        let ids: Vec<&str> = cmp.arms.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["agent", "noop", "cmaes"],
            "agents first, then the no-op and CMA-ES floors"
        );
        assert_eq!(cmp.arms[1].label, "no-op floor");
        assert_eq!(cmp.arms[2].label, "CMA-ES floor");
        let agent = &cmp.arms[0];
        assert_eq!(
            (agent.codes, agent.overall.n, agent.overall.passes),
            (2, 4, 3)
        );
        assert_eq!(agent.overall.rate, Some(0.75));
        assert_eq!(agent.overall.median_sound, Some(91.5));
        assert_eq!(agent.by_level.len(), 1);
        assert_eq!(
            (
                agent.median_renders,
                agent.median_tokens,
                agent.median_cost_usd
            ),
            (Some(40.0), Some(100_000.0), Some(0.25))
        );
        assert_eq!(agent.total_cost_usd, Some(1.0));
        assert!(cmp.has_agent() && cmp.has_arm("noop") && cmp.has_arm("cmaes"));
        assert_eq!(cmp.codes, 2);
        assert!(cmp.humans_same_codes.is_none() && cmp.humans_all.overall.n == 0);
    }

    #[test]
    fn floors_alone_are_not_an_agent() {
        let (attempts, _) = read("b.jsonl", &line("noop", &l2(1), false, 3.0));
        let cmp = compare(&attempts, &[]).unwrap();
        assert!(!cmp.has_agent());
        assert!(compare(&[], &[]).is_none());
    }

    #[test]
    fn cells_have_wilson_intervals() {
        let c = cell(&[
            (true, Some(90.0)),
            (false, Some(10.0)),
            (true, None),
            (false, None),
        ]);
        assert_eq!((c.n, c.passes, c.rate), (4, 2, Some(0.5)));
        assert_eq!(c.median_sound, Some(50.0));
        let (lo, hi) = c.ci.unwrap();
        assert!(lo < 0.5 && 0.5 < hi);
        assert_eq!(cell(&[]).rate, None);
    }
}
