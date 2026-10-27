//! `agent-eval audit`: the honesty checks of the goal set (R1/S1 lane EV2; docs/agent-evals.md). No model, no
//! wrapper: `enth` runs directly in a scratch directory per goal.
//!
//! Per goal:
//! - the witness, replayed with the designer's list and again with its own `self_expect`, passes, and the judge
//!   (`enth check --no-fix` with the designer's list and the goal's performance) passes both final patches with no
//!   `thin` item;
//! - the untouched starting patch (`enth new --template <goal.template>`) fails the designer's list;
//! - with `--cross`, every other goal's witness patch is judged against this goal's list and performance (reported,
//!   not gated: a patch played on another note can be a fair answer; the sound only, not the goal's `requires`).
//!
//! A goal with `requires` (eval version 4) is judged on the sound **and** the structure, as the trial judge does; the
//! starting-patch rule and `--cross` look at the sound alone.
//!
//! After the table the audit prints what an agent can find through the CLI's own surfaces (`enth modules` and
//! `enth params <kind>`) for the kinds the goals require: a reserved kind that `enth modules` hides is reported, and
//! `--require-discoverable` (INT-S2a, after it clears the hide flags) turns that into a failure. A kind that
//! `enth params` cannot describe is always a failure.
//!
//! It exits with an error when any goal breaks one of the first two rules.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::goal::{Goal, fill};
use super::run::{self, Opts};
use super::structure::{self, What};
use super::util;

/// What one judged `enth check` says, in the terms the audit gates on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Verdict {
    pub pass: bool,
    pub failing: Vec<String>,
    pub thin: Vec<String>,
    /// The smallest `margin_pct` among passing Hz and ms items, with the item.
    pub lowest_pct: Option<(f64, String)>,
}

/// Read an `enth.check/2` object (exit 0 or 1) into a [`Verdict`].
pub fn verdict(check: &Value) -> Verdict {
    let mut v = Verdict {
        pass: check.get("pass").and_then(Value::as_bool).unwrap_or(false),
        ..Verdict::default()
    };
    for r in check
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let expect = r
            .get("expect")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned();
        let pass = r.get("pass").and_then(Value::as_bool).unwrap_or(false);
        if !pass {
            v.failing.push(expect.clone());
        }
        if r.get("thin").and_then(Value::as_bool).unwrap_or(false) {
            v.thin.push(expect.clone());
        }
        if let (true, Some(pct)) = (pass, r.get("margin_pct").and_then(Value::as_f64))
            && v.lowest_pct.as_ref().is_none_or(|(low, _)| pct < *low)
        {
            v.lowest_pct = Some((pct, expect));
        }
    }
    v
}

struct Audit {
    enth: PathBuf,
    base: PathBuf,
}

impl Audit {
    fn enth(&self, dir: &Path, args: &[String]) -> Result<(i32, String), String> {
        let out = Command::new(&self.enth)
            .current_dir(dir)
            .args(args)
            .output()
            .map_err(|e| format!("cannot run {}: {e}", self.enth.display()))?;
        Ok((
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ))
    }

    /// The sound and the structure: what the trial judge decides (`requires` unmet make the verdict fail).
    fn judge_built(&self, dir: &Path, patch: &str, goal: &Goal) -> Result<Verdict, String> {
        let mut v = self.judge(dir, patch, goal)?;
        let unmet = structure::check(&self.enth, dir, Path::new(patch), &goal.requirements())?;
        if !unmet.is_empty() {
            v.pass = false;
            v.failing
                .extend(unmet.into_iter().map(|u| format!("requires {u}")));
        }
        Ok(v)
    }

    /// The sound alone: `enth check` of the patch with the goal's performance and designer list.
    fn judge(&self, dir: &Path, patch: &str, goal: &Goal) -> Result<Verdict, String> {
        let args = [
            "check",
            patch,
            "--no-fix",
            "--perf",
            &goal.perf.json(),
            "--expect",
            &goal.expect,
        ]
        .map(str::to_owned);
        let (code, out) = self.enth(dir, &args)?;
        if code > 1 {
            return Err(format!(
                "{}: enth check exited {code}: {}",
                goal.id,
                out.trim()
            ));
        }
        let json: Value =
            serde_json::from_str(&out).map_err(|e| format!("{}: check output: {e}", goal.id))?;
        Ok(verdict(&json))
    }

    /// What an agent finds through `enth modules` and `enth params <kind>` for the kinds the goals require. Prints a
    /// report and returns the problems: a kind `enth params` cannot describe, and (with `strict`) a kind
    /// `enth modules` does not list.
    fn discovery(&self, goals: &[&Goal], strict: bool) -> Result<Vec<String>, String> {
        let needed = required_kinds(goals);
        if needed.is_empty() {
            return Ok(Vec::new());
        }
        let dir = self.dir("_discovery", "modules")?;
        let (code, out) = self.enth(&dir, &["modules".to_owned()])?;
        if code != 0 {
            return Err(format!("enth modules exited {code}: {}", out.trim()));
        }
        let listed: BTreeSet<String> = serde_json::from_str::<Value>(&out)
            .map_err(|e| format!("enth modules output: {e}"))?
            .get("modules")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| m.get("kind").and_then(Value::as_str).map(str::to_owned))
            .collect();
        println!("\ndiscovery: the kinds the goals require, through the CLI's own surfaces");
        println!(
            "{:<10} {:<8} summary (`enth params <kind>`)",
            "kind", "modules"
        );
        let mut problems = Vec::new();
        let mut hidden = Vec::new();
        for kind in &needed {
            let (code, out) = self.enth(&dir, &["params".to_owned(), kind.clone()])?;
            let summary = (code == 0)
                .then(|| serde_json::from_str::<Value>(&out).ok())
                .flatten()
                .and_then(|v| {
                    let params = v
                        .get("params")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    let text = v.get("summary").and_then(Value::as_str)?.to_owned();
                    (params > 0 && !text.trim().is_empty()).then_some(text)
                });
            let is_listed = listed.contains(kind);
            if !is_listed {
                hidden.push(kind.as_str());
            }
            println!(
                "{:<10} {:<8} {}",
                kind,
                if is_listed { "listed" } else { "hidden" },
                summary.as_deref().unwrap_or("NOT DESCRIBED")
            );
            if summary.is_none() {
                problems.push(format!(
                    "discovery: `enth params {kind}` does not describe a kind the goals require"
                ));
            }
        }
        if hidden.is_empty() {
            println!("every required kind is in `enth modules`");
        } else {
            println!(
                "{} of {} required kinds are hidden from `enth modules` (a reserved S2a kind, \
                 ModuleSpec::hidden): an agent that does not already know {} cannot find them by listing; INT-S2a clears the flag",
                hidden.len(),
                needed.len(),
                hidden.join(", ")
            );
            if strict {
                problems.push(format!(
                    "discovery: `enth modules` does not list {}",
                    hidden.join(", ")
                ));
            }
        }
        Ok(problems)
    }

    fn dir(&self, goal: &str, what: &str) -> Result<PathBuf, String> {
        let d = self.base.join(goal).join(what);
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        Ok(d)
    }

    /// Replay the witness with `list` in place of `{expect}`; the last call must pass its own list.
    fn replay(
        &self,
        goal: &Goal,
        steps: &[Vec<String>],
        list: &str,
        dir: &Path,
    ) -> Result<(), String> {
        let perf = goal.perf.json();
        let mut last = (0, String::new());
        for step in steps {
            let args: Vec<String> = step
                .iter()
                .map(|a| fill(a, &[("perf", &perf), ("expect", list)]))
                .collect();
            last = self.enth(dir, &args)?;
            if last.0 >= 2 {
                return Err(format!(
                    "a witness call exited {}: {}",
                    last.0,
                    last.1.trim()
                ));
            }
        }
        if last.0 == 0 {
            Ok(())
        } else {
            Err("the witness's last call does not pass its list".to_owned())
        }
    }
}

/// `agent-eval audit [--goals a,b] [--cross] [--require-discoverable] [--enth PATH]`.
pub fn audit(args: &[String]) -> Result<(), String> {
    let mut opts = Opts::default();
    let mut cross = false;
    let mut require_discoverable = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--cross" => cross = true,
            "--require-discoverable" => require_discoverable = true,
            "--enth" => opts.enth = Some(PathBuf::from(it.next().ok_or("--enth needs a path")?)),
            "--goals" => {
                opts.goals = Some(
                    it.next()
                        .ok_or("--goals needs a list")?
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .collect(),
                );
            }
            other => return Err(format!("agent-eval audit: unknown argument `{other}`")),
        }
    }
    let meta = crate::metadata()?;
    let set = super::load_set(&meta.workspace_root)?;
    let goals: Vec<&Goal> = set
        .goals
        .iter()
        .filter(|g| opts.goals.as_ref().is_none_or(|w| w.contains(&g.id)))
        .collect();
    if goals.is_empty() {
        return Err("agent-eval audit: no goal matches --goals".to_owned());
    }
    let audit = Audit {
        enth: run::ensure_enth(&meta.workspace_root, &meta.target_directory, &opts)?,
        base: meta
            .target_directory
            .join("agent-eval")
            .join(format!("audit-{}", util::stamp(util::now_secs()))),
    };
    println!(
        "agent-eval audit: eval set version {}, {} goals, scratch in {}",
        set.meta.eval_version,
        goals.len(),
        audit.base.display()
    );
    println!(
        "{:<22} {:>1}  {:<6} {:<6} {:<5} {:<11} lowest margin (Hz/ms items)",
        "goal", "d", "given", "self", "thin", "start fails"
    );
    let mut problems = Vec::new();
    let mut patches = Vec::new();
    for goal in &goals {
        let witness = set.witness(&goal.id)?;
        let steps: Vec<Vec<String>> = witness.step.iter().map(|s| s.args.clone()).collect();
        let mut row = Vec::new();
        let mut thin = 0;
        let mut lowest = None;
        for (track, list) in [("given", &goal.expect), ("self", &witness.self_expect)] {
            let dir = audit.dir(&goal.id, track)?;
            let verdict = audit
                .replay(goal, &steps, list, &dir)
                .and_then(|()| audit.judge_built(&dir, super::fake::PATCH, goal));
            match verdict {
                Ok(v) => {
                    if !v.pass {
                        problems.push(format!(
                            "{} ({track}): the judge fails {:?}",
                            goal.id, v.failing
                        ));
                    }
                    if !v.thin.is_empty() {
                        problems.push(format!("{} ({track}): thin items {:?}", goal.id, v.thin));
                    }
                    thin += v.thin.len();
                    row.push(if v.pass { "pass" } else { "FAIL" });
                    if track == "given" {
                        lowest = v.lowest_pct;
                        patches.push((goal.id.clone(), dir.join(super::fake::PATCH)));
                    }
                }
                Err(e) => {
                    problems.push(format!("{} ({track}): {e}", goal.id));
                    row.push("ERROR");
                }
            }
        }
        let dir = audit.dir(&goal.id, "start")?;
        let new = [
            "new",
            "--template",
            &goal.template,
            "--out",
            "start.patch.json",
        ]
        .map(str::to_owned);
        let (code, out) = audit.enth(&dir, &new)?;
        if code != 0 {
            return Err(format!("{}: enth new failed: {}", goal.id, out.trim()));
        }
        let start = audit.judge(&dir, "start.patch.json", goal)?;
        if start.pass {
            problems.push(format!(
                "{}: the untouched `{}` template already passes the list",
                goal.id, goal.template
            ));
        }
        let items = count_items(&goal.expect);
        println!(
            "{:<22} {:>1}  {:<6} {:<6} {:<5} {:<11} {}",
            goal.id,
            goal.difficulty,
            row[0],
            row.get(1).copied().unwrap_or("-"),
            thin,
            format!("{}/{}", start.failing.len(), items),
            lowest.map_or_else(
                || "-".to_owned(),
                |(pct, item)| format!("{pct:.0} % ({item})")
            ),
        );
    }
    if cross {
        println!(
            "\ncross check: each goal's list on the other goals' witness patches (reported, not gated)"
        );
        for goal in &goals {
            let mut passed = Vec::new();
            for (other, patch) in &patches {
                if other == &goal.id {
                    continue;
                }
                let dir = patch.parent().unwrap_or(&audit.base);
                let path = patch.to_string_lossy();
                if audit.judge(dir, &path, goal)?.pass {
                    passed.push(other.as_str());
                }
            }
            println!(
                "{:<22} passed by {} of {}: {}",
                goal.id,
                passed.len(),
                patches.len().saturating_sub(1),
                passed.join(", ")
            );
        }
    }
    problems.extend(audit.discovery(&goals, require_discoverable)?);
    if problems.is_empty() {
        println!(
            "\nagent-eval audit: {} goals ok (witness passes both lists and its structure with no thin item; the start fails)",
            goals.len()
        );
        Ok(())
    } else {
        Err(format!(
            "agent-eval audit: {} problem(s):\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

/// The module kinds the goals' `requires` name (a route's source counts unless it is `any` or a control source).
fn required_kinds(goals: &[&Goal]) -> BTreeSet<String> {
    let mut kinds = BTreeSet::new();
    for goal in goals {
        for r in goal.requirements() {
            match r.what {
                What::Kind { kind, .. } | What::Param { kind, .. } => {
                    kinds.insert(kind);
                }
                What::Route { src, dst, .. } => {
                    let control = src == "any"
                        || src.starts_with("macro")
                        || ["velocity", "key", "modwheel"].contains(&src.as_str());
                    if !control {
                        kinds.insert(src);
                    }
                    kinds.insert(dst);
                }
            }
        }
    }
    kinds
}

/// Items in an expectation list: commas inside `[...]` belong to the item.
fn count_items(list: &str) -> usize {
    let mut depth = 0i32;
    let mut items = 1;
    for c in list.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            ',' | ';' if depth == 0 => items += 1,
            _ => {}
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_verdict_collects_failures_thin_items_and_the_lowest_margin() {
        let check = json!({"schema": "enth.check/2", "pass": false, "results": [
            {"expect": "envelope.attack_ms < 10", "pass": true, "margin_pct": 40.0, "thin": false},
            {"expect": "spectrum.centroid_hz > 2000", "pass": true, "margin_pct": 12.5, "thin": true},
            {"expect": "envelope.sustain_db < -30", "pass": false, "thin": false},
            {"expect": "loudness.clipping = false", "pass": true, "thin": false},
        ]});
        let v = verdict(&check);
        assert!(!v.pass);
        assert_eq!(v.failing, ["envelope.sustain_db < -30"]);
        assert_eq!(v.thin, ["spectrum.centroid_hz > 2000"]);
        assert_eq!(
            v.lowest_pct,
            Some((12.5, "spectrum.centroid_hz > 2000".to_owned()))
        );
    }

    #[test]
    fn items_are_counted_with_brackets_kept_whole() {
        assert_eq!(count_items("attack < 10ms"), 1);
        assert_eq!(
            count_items("pitch.note ~ A1, duration in [600ms, 1500ms]; peak < -1dB"),
            3
        );
    }
}
