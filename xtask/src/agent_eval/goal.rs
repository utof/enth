//! The eval set on disk: goal files (`goals/<id>.toml`), hand-picked witnesses for the dry run (`witness/<id>.toml`)
//! and the set's version (`eval.toml`). Loading validates what a typo would otherwise hide.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::json;

use super::structure;

/// The schema string of a goal file whose sound is the whole check (eval versions 1 to 3, and the older goals since).
pub const GOAL_SCHEMA: &str = "enth.agent-eval/1";
/// The schema string of a goal file that also lists `requires` (eval version 4): additive over `/1`, which still
/// loads and means the same.
pub const GOAL_SCHEMA_V2: &str = "enth.agent-eval/2";
/// The schema string every witness file carries.
pub const WITNESS_SCHEMA: &str = "enth.agent-eval-witness/1";
/// The schema string of `eval.toml`.
pub const SET_SCHEMA: &str = "enth.agent-eval-set/1";

/// One note of a goal's performance.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerfNote {
    pub note: String,
    pub vel: u8,
    pub on_s: f64,
    pub off_s: f64,
}

/// The performance the judge forces (`enth.perf/1`, without the schema key).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Perf {
    pub notes: Vec<PerfNote>,
    pub dur_s: f64,
}

impl Perf {
    /// The inline JSON `enth ... --perf` takes.
    pub fn json(&self) -> String {
        let notes: Vec<_> = self
            .notes
            .iter()
            .map(|n| json!({"note": n.note, "vel": n.vel, "on_s": n.on_s, "off_s": n.off_s}))
            .collect();
        json!({"notes": notes, "dur_s": self.dur_s}).to_string()
    }

    /// The short form (`note=A3,vel=100,hold=1.5s,dur=2s`) for a single note that starts at 0.
    pub fn short(&self) -> Option<String> {
        match self.notes.as_slice() {
            [n] if n.on_s == 0.0 => Some(format!(
                "note={},vel={},hold={}s,dur={}s",
                n.note, n.vel, n.off_s, self.dur_s
            )),
            _ => None,
        }
    }
}

/// What one trial may spend.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Renders (`render`, `step`, and `describe` or `check` of a patch); the wrapper refuses one more.
    pub renders: u32,
    /// Agent turns (`--max-turns`).
    pub turns: u32,
    /// Estimated dollars (`--max-budget-usd`).
    pub usd: f64,
}

/// A goal: a brief, the performance the judge forces, and the expectation list a sound designer would write.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    dead_code,
    reason = "format fields (`added`, `tags`, `engine_min`) are for people and tools that read the goal files"
)]
pub struct Goal {
    pub schema: String,
    pub id: String,
    pub added: String,
    /// 1 (easy) to 5 (hard).
    pub difficulty: u8,
    pub tags: Vec<String>,
    /// The only text the agent gets on the self-written track.
    pub brief: String,
    /// The starting patch: a template name.
    pub template: String,
    /// The designer's expectation list: what the judge runs.
    pub expect: String,
    /// For maintainers; never shown to the agent.
    pub rationale: String,
    pub engine_min: String,
    pub perf: Perf,
    pub budget: Budget,
    /// What the final patch must be made of, beside passing `expect` (`enth.agent-eval/2` only; see
    /// [`super::structure`]): `["wt x2", "wt -> wt.pm"]`. Never shown to the agent.
    #[serde(default)]
    pub requires: Vec<String>,
}

impl Goal {
    /// The parsed `requires`; [`load`] has already checked that they parse.
    pub fn requirements(&self) -> Vec<structure::Requirement> {
        self.requires
            .iter()
            .filter_map(|r| structure::parse(r).ok())
            .collect()
    }
}

/// One scripted `enth` call of a witness.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessStep {
    /// The arguments after `enth`; `{perf}` and `{expect}` are replaced.
    pub args: Vec<String>,
}

/// A hand-picked solution: the scripted fake agent's moves, and the proof that the goal is reachable.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub schema: String,
    pub id: String,
    /// The fake agent's own expectation list on the self-written track.
    pub self_expect: String,
    pub step: Vec<WitnessStep>,
}

/// `eval.toml`: the version of the set. Changing a prompt or a goal list changes it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetMeta {
    pub schema: String,
    pub eval_version: u32,
    /// What changed in this version, newest first.
    #[allow(
        dead_code,
        reason = "read by people; the harness only checks that it parses"
    )]
    pub changelog: Vec<String>,
}

/// The whole set.
#[derive(Clone, Debug)]
pub struct EvalSet {
    pub dir: PathBuf,
    pub meta: SetMeta,
    pub goals: Vec<Goal>,
}

impl EvalSet {
    /// The prompt template named `name` (`system`, `given`, `self`).
    pub fn prompt(&self, name: &str) -> Result<String, String> {
        let path = self.dir.join("prompts").join(format!("{name}.md"));
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// A fingerprint of the three prompts, for the result header.
    pub fn prompt_fingerprint(&self) -> Result<String, String> {
        let mut all = String::new();
        for name in ["system", "given", "self"] {
            all.push_str(&self.prompt(name)?);
            all.push('\u{1}');
        }
        Ok(format!("{:016x}", super::util::fnv1a64(all.as_bytes())))
    }

    pub fn witness(&self, id: &str) -> Result<Witness, String> {
        let path = self.dir.join("witness").join(format!("{id}.toml"));
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let w: Witness = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if w.schema != WITNESS_SCHEMA {
            return Err(format!(
                "{}: schema must be {WITNESS_SCHEMA}",
                path.display()
            ));
        }
        if w.id != id {
            return Err(format!(
                "{}: id `{}` does not match the file name",
                path.display(),
                w.id
            ));
        }
        if w.step.is_empty() || w.self_expect.trim().is_empty() {
            return Err(format!("{}: needs steps and a self_expect", path.display()));
        }
        Ok(w)
    }
}

/// Replace `{key}` with the value for each pair, in order.
pub fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (key, value) in pairs {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

fn validate(goal: &Goal, stem: &str) -> Result<(), String> {
    let id_ok = !goal.id.is_empty()
        && goal
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !id_ok {
        return Err(format!(
            "{}: id must be lowercase letters, digits and `-`",
            goal.id
        ));
    }
    if goal.schema != GOAL_SCHEMA && goal.schema != GOAL_SCHEMA_V2 {
        return Err(format!(
            "{stem}: schema must be {GOAL_SCHEMA} or {GOAL_SCHEMA_V2}"
        ));
    }
    if goal.schema == GOAL_SCHEMA && !goal.requires.is_empty() {
        return Err(format!(
            "{stem}: `requires` needs schema {GOAL_SCHEMA_V2}, the version that added it"
        ));
    }
    for r in &goal.requires {
        structure::parse(r).map_err(|e| format!("{stem}: {e}"))?;
    }
    if goal.id != stem {
        return Err(format!(
            "{stem}: id `{}` does not match the file name",
            goal.id
        ));
    }
    if !(1..=5).contains(&goal.difficulty) {
        return Err(format!("{stem}: difficulty must be 1..=5"));
    }
    for (what, text) in [
        ("brief", &goal.brief),
        ("expect", &goal.expect),
        ("rationale", &goal.rationale),
        ("template", &goal.template),
    ] {
        if text.trim().is_empty() {
            return Err(format!("{stem}: {what} is empty"));
        }
    }
    if goal.perf.notes.is_empty() || goal.perf.dur_s <= 0.0 {
        return Err(format!("{stem}: perf needs a note and a positive dur_s"));
    }
    for n in &goal.perf.notes {
        if !(n.off_s > n.on_s && n.off_s <= goal.perf.dur_s) {
            return Err(format!(
                "{stem}: a note must end after it starts, inside dur_s"
            ));
        }
    }
    if goal.budget.renders == 0 || goal.budget.turns == 0 || goal.budget.usd <= 0.0 {
        return Err(format!(
            "{stem}: budget needs positive renders, turns and usd"
        ));
    }
    Ok(())
}

/// Load `eval.toml` and every `goals/*.toml` under `dir`, sorted by difficulty then id.
pub fn load(dir: &Path) -> Result<EvalSet, String> {
    let meta_path = dir.join("eval.toml");
    let meta_text =
        std::fs::read_to_string(&meta_path).map_err(|e| format!("{}: {e}", meta_path.display()))?;
    let meta: SetMeta =
        toml::from_str(&meta_text).map_err(|e| format!("{}: {e}", meta_path.display()))?;
    if meta.schema != SET_SCHEMA {
        return Err(format!(
            "{}: schema must be {SET_SCHEMA}",
            meta_path.display()
        ));
    }
    let goals_dir = dir.join("goals");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&goals_dir)
        .map_err(|e| format!("{}: {e}", goals_dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    let mut goals = Vec::new();
    for path in paths {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let goal: Goal = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        validate(&goal, &stem)?;
        goals.push(goal);
    }
    if goals.is_empty() {
        return Err(format!("{}: no goals", goals_dir.display()));
    }
    goals.sort_by(|a, b| (a.difficulty, &a.id).cmp(&(b.difficulty, &b.id)));
    for pair in goals.windows(2) {
        if pair[0].id == pair[1].id {
            return Err(format!("duplicate goal id {}", pair[0].id));
        }
    }
    Ok(EvalSet {
        dir: dir.to_owned(),
        meta,
        goals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Goal {
        toml::from_str(
            r#"
schema = "enth.agent-eval/1"
id = "pluck-bright-short"
added = "2026-10-19"
difficulty = 1
tags = ["pluck"]
brief = "a short bright pluck"
template = "init"
expect = "attack < 10ms"
rationale = "why"
engine_min = "0.1.0"
perf = { notes = [{ note = "A3", vel = 100, on_s = 0.0, off_s = 1.5 }], dur_s = 2.0 }
budget = { renders = 20, turns = 60, usd = 1.0 }
"#,
        )
        .expect("sample parses")
    }

    #[test]
    fn perf_renders_inline_json_and_the_short_form() {
        let g = sample();
        let v: serde_json::Value = serde_json::from_str(&g.perf.json()).unwrap();
        assert_eq!(v["notes"][0]["note"], "A3");
        assert_eq!(v["dur_s"], 2.0);
        assert_eq!(g.perf.short().unwrap(), "note=A3,vel=100,hold=1.5s,dur=2s");
    }

    #[test]
    fn validation_catches_a_mismatched_id_and_bad_timing() {
        let mut g = sample();
        assert!(validate(&g, "pluck-bright-short").is_ok());
        assert!(validate(&g, "other").is_err());
        g.perf.notes[0].off_s = 3.0;
        assert!(validate(&g, "pluck-bright-short").is_err());
        let mut g = sample();
        g.difficulty = 6;
        assert!(validate(&g, "pluck-bright-short").is_err());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let bad = r#"schema = "enth.agent-eval/1"
id = "x"
extra = 1"#;
        assert!(toml::from_str::<Goal>(bad).is_err());
    }

    #[test]
    fn a_version_one_goal_still_loads_and_needs_nothing_built() {
        // eval versions 1 to 3 wrote `enth.agent-eval/1` and no `requires`; they mean what they meant.
        let g = sample();
        assert_eq!(g.schema, GOAL_SCHEMA);
        assert!(g.requires.is_empty() && g.requirements().is_empty());
        assert!(validate(&g, "pluck-bright-short").is_ok());
    }

    #[test]
    fn requires_belongs_to_schema_two() {
        let mut g = sample();
        g.requires = vec!["wt x2".to_owned(), "wt -> wt.pm".to_owned()];
        assert!(
            validate(&g, "pluck-bright-short").is_err(),
            "schema 1 predates requires"
        );
        g.schema = GOAL_SCHEMA_V2.to_owned();
        assert!(validate(&g, "pluck-bright-short").is_ok());
        assert_eq!(g.requirements().len(), 2);
        g.requires.push("wt ->".to_owned());
        assert!(
            validate(&g, "pluck-bright-short").is_err(),
            "a requirement that does not parse is refused at load, not at judging"
        );
        g.requires.clear();
        g.schema = "enth.agent-eval/3".to_owned();
        assert!(validate(&g, "pluck-bright-short").is_err());
    }

    #[test]
    fn fill_replaces_every_placeholder() {
        assert_eq!(
            fill("{a} and {a} and {b}", &[("a", "1"), ("b", "2")]),
            "1 and 1 and 2"
        );
    }
}
