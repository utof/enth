//! Aggregation: per goal and track the solve rate over the trials (pass@k and pass^k), median and maximum renders,
//! tokens, cost (`total_cost_usd`, a client-side estimate), the invalid-command rate, wall time and the final margins;
//! then the G1 gate reading. `summary.json` in a run directory is exactly this value.

use std::path::Path;

use serde_json::{Value, json};

use super::goal::{EvalSet, Goal};
use super::run::{Track, TrialResult};
use super::util::{median, round4};

/// Strictly more than half of the trials: a goal passes when its median run passes.
fn majority(passes: usize, trials: usize) -> bool {
    passes * 2 > trials
}

fn stats(values: &[f64]) -> Value {
    json!({
        "median": median(values).map(round4),
        "max": values.iter().copied().max_by(f64::total_cmp).map(round4),
        "min": values.iter().copied().min_by(f64::total_cmp).map(round4),
    })
}

/// Everything about one goal on one track.
fn entry(goal: &Goal, track: Track, rs: &[&TrialResult]) -> Value {
    let n = rs.len();
    let passes = rs.iter().filter(|r| r.pass).count();
    let f = |g: &dyn Fn(&TrialResult) -> f64| -> Vec<f64> { rs.iter().map(|r| g(r)).collect() };
    let calls: u32 = rs.iter().map(|r| r.calls).sum();
    let invalid: u32 = rs.iter().map(|r| r.invalid_calls).sum();
    // The trial that stands for the goal: the passing run with the median renders, else the closest-to-passing run.
    let mut order: Vec<&&TrialResult> = rs.iter().collect();
    order.sort_by_key(|r| (!r.pass, r.failed.len(), r.renders, r.trial));
    let stand_in = order
        .get(if passes > 0 { (passes - 1) / 2 } else { 0 })
        .copied();
    let tokens_median =
        |g: &dyn Fn(&TrialResult) -> u64| median(&f(&|r| g(r) as f64)).map(|x| x.round() as u64);
    let mut v = json!({
        "goal": goal.id,
        "track": track.name(),
        "difficulty": goal.difficulty,
        "brief": goal.brief,
        "trials": n,
        "passes": passes,
        "solve_rate": round4(passes as f64 / n as f64),
        "pass_at_k": passes > 0,
        "pass_pow_k": passes == n,
        "majority_pass": majority(passes, n),
        "renders": stats(&f(&|r| f64::from(r.renders))),
        "tokens": {
            "median_total": tokens_median(&|r| r.tokens.total),
            "median_input": tokens_median(&|r| r.tokens.input),
            "median_output": tokens_median(&|r| r.tokens.output),
            "median_cache_read": tokens_median(&|r| r.tokens.cache_read),
            "median_cache_write": tokens_median(&|r| r.tokens.cache_write),
        },
        "cost_usd": {
            "median": median(&rs.iter().filter_map(|r| r.cost_usd).collect::<Vec<_>>()).map(round4),
            "total": round4(rs.iter().filter_map(|r| r.cost_usd).sum::<f64>()),
            "is_estimate": true,
        },
        "invalid_rate": if calls == 0 { Value::Null } else { json!(round4(f64::from(invalid) / f64::from(calls))) },
        "refused_renders": rs.iter().map(|r| r.refused_calls).sum::<u32>(),
        "wall_s": stats(&f(&|r| r.wall_s)),
        "statuses": rs.iter().map(|r| r.status.as_str()).collect::<Vec<_>>(),
    });
    if let Some(r) = stand_in {
        v["final"] = json!({
            "trial": r.trial,
            "pass": r.pass,
            "thin": r.thin,
            "failed": r.failed,
            "margins": r.margins.iter().map(|m| json!({
                "expect": m.expect, "pass": m.pass, "got": m.got, "margin": m.margin,
                "margin_pct": m.margin_pct, "unit": m.unit, "thin": m.thin,
            })).collect::<Vec<_>>(),
            "patch": r.final_patch,
            "wav": r.final_wav,
        });
    }
    if track == Track::SelfWritten {
        let own = rs.iter().filter(|r| r.own_pass == Some(true)).count();
        v["own_list_pass_rate"] = json!(round4(own as f64 / n as f64));
        v["own_lists"] = json!(rs.iter().map(|r| r.own_expect.clone()).collect::<Vec<_>>());
    }
    v
}

fn gate(entries: &[Value], track: Track) -> Value {
    let mine: Vec<&Value> = entries
        .iter()
        .filter(|e| e["track"] == track.name())
        .collect();
    let count = |key: &str| mine.iter().filter(|e| e[key] == true).count();
    json!({
        "goals": mine.len(),
        "majority_pass": count("majority_pass"),
        "any_pass": count("pass_at_k"),
        "all_pass": count("pass_pow_k"),
        "threshold": 5,
        "met": track == Track::Given && count("majority_pass") >= 5,
        "gated": track == Track::Given,
    })
}

/// The most common value among `items`.
fn mode(items: impl Iterator<Item = String>) -> Option<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for item in items {
        match counts.iter_mut().find(|(k, _)| *k == item) {
            Some((_, n)) => *n += 1,
            None => counts.push((item, 1)),
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(k, _)| k)
}

/// Build the summary of a run from its trial results.
pub fn summarize(header: &Value, results: &[TrialResult], set: &EvalSet) -> Value {
    let mut entries = Vec::new();
    for goal in &set.goals {
        for track in [Track::Given, Track::SelfWritten] {
            let rs: Vec<&TrialResult> = results
                .iter()
                .filter(|r| r.goal == goal.id && r.track == track)
                .collect();
            if !rs.is_empty() {
                entries.push(entry(goal, track, &rs));
            }
        }
    }
    let gates = json!({
        "given": gate(&entries, Track::Given),
        "self": gate(&entries, Track::SelfWritten),
        "blind_listen": "pending: the owner's blind check, at least 7 of 10 final sounds match their text",
    });
    let cost: f64 = results.iter().filter_map(|r| r.cost_usd).sum();
    let mut statuses = serde_json::Map::new();
    for r in results {
        let c = statuses.entry(r.status.clone()).or_insert(json!(0));
        *c = json!(c.as_u64().unwrap_or(0) + 1);
    }
    let mut out = header.clone();
    out["schema"] = json!("enth.agent-eval.summary/1");
    out["model"] = json!(mode(results.iter().filter_map(|r| r.model.clone())));
    out["claude_code_version"] = json!(results.iter().find_map(|r| r.claude_code_version.clone()));
    out["results"] = json!(entries);
    out["gate"] = gates;
    out["totals"] = json!({
        "trials": results.len(),
        "cost_usd": round4(cost),
        "cost_usd_per_trial": if results.is_empty() { Value::Null } else { json!(round4(cost / results.len() as f64)) },
        "cost_is_estimate": true,
        "estimate_to_beat_usd_per_trial": 0.14,
        "tokens": results.iter().map(|r| r.tokens.total).sum::<u64>(),
        "calls": results.iter().map(|r| u64::from(r.calls)).sum::<u64>(),
        "statuses": statuses,
        "warnings": results.iter().filter(|r| !r.warnings.is_empty()).count(),
    });
    out
}

/// Print the summary as tables.
pub fn print(summary: &Value) {
    let model = summary["model"].as_str().unwrap_or("?");
    println!(
        "\nagent-eval {} | model {model} | Claude Code {} | enth {} | eval v{}",
        summary["date"].as_str().unwrap_or("?"),
        summary["claude_code_version"].as_str().unwrap_or("-"),
        summary["enth_git"]
            .as_str()
            .map_or("?", |h| &h[..h.len().min(10)]),
        summary["eval_version"],
    );
    if summary["dry_run"] == true {
        println!("DRY RUN: scripted agent, no model was called; tokens and cost are synthetic.");
    }
    for track in ["given", "self"] {
        let rows: Vec<&Value> = summary["results"]
            .as_array()
            .map(|a| a.iter().filter(|e| e["track"] == track).collect())
            .unwrap_or_default();
        if rows.is_empty() {
            continue;
        }
        println!(
            "\ntrack {track} ({})",
            if track == "given" {
                "the designer's list is given; gated"
            } else {
                "the agent writes its own list; reported, not gated"
            }
        );
        println!(
            "{:<22} {:>1} {:>5} {:>9} {:>6} {:>5} {:>9} {:>8} {:>7} {:>7}{}",
            "goal",
            "d",
            "pass",
            "renders",
            "max",
            "thin",
            "tokens",
            "cost",
            "invalid",
            "wall s",
            if track == "self" { "  own-list" } else { "" }
        );
        for e in rows {
            println!(
                "{:<22} {:>1} {:>2}/{:<2} {:>9} {:>6} {:>5} {:>9} {:>8} {:>7} {:>7}{}",
                e["goal"].as_str().unwrap_or("?"),
                e["difficulty"],
                e["passes"],
                e["trials"],
                num(&e["renders"]["median"], 1),
                num(&e["renders"]["max"], 0),
                e["final"]["thin"],
                e["tokens"]["median_total"]
                    .as_u64()
                    .map_or_else(|| "-".to_owned(), |t| t.to_string()),
                e["cost_usd"]["median"]
                    .as_f64()
                    .map_or_else(|| "-".to_owned(), |c| format!("${c:.3}")),
                e["invalid_rate"]
                    .as_f64()
                    .map_or_else(|| "-".to_owned(), |r| format!("{:.0}%", r * 100.0)),
                num(&e["wall_s"]["median"], 1),
                if track == "self" {
                    format!("  {}", num(&e["own_list_pass_rate"], 2))
                } else {
                    String::new()
                },
            );
        }
        let g = &summary["gate"][track];
        println!(
            "goals passed (median run): {} of {} (any run: {}, all runs: {}){}",
            g["majority_pass"],
            g["goals"],
            g["any_pass"],
            g["all_pass"],
            if track == "given" {
                format!(
                    " | G1 needs >= 5: {}",
                    if g["met"] == true { "met" } else { "NOT met" }
                )
            } else {
                String::new()
            }
        );
    }
    let t = &summary["totals"];
    println!(
        "\n{} trials, {} enth calls, {} tokens, estimated cost ${:.3} (${:.3} per trial; the plan estimated $0.14)",
        t["trials"],
        t["calls"],
        t["tokens"],
        t["cost_usd"].as_f64().unwrap_or(0.0),
        t["cost_usd_per_trial"].as_f64().unwrap_or(0.0),
    );
    println!("blind listen: pending (owner)");
}

fn num(v: &Value, decimals: usize) -> String {
    v.as_f64()
        .map_or_else(|| "-".to_owned(), |x| format!("{x:.decimals$}"))
}

/// `agent-eval report <dir>`: rebuild the summary from the trial results under a run directory.
pub fn report(dir: &Path, set: &EvalSet) -> Result<(), String> {
    let header: Value = {
        let p = dir.join("header.json");
        let t = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        serde_json::from_str(&t).map_err(|e| format!("{}: {e}", p.display()))?
    };
    let mut results: Vec<TrialResult> = Vec::new();
    for goal in std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
    {
        if !goal.path().is_dir() {
            continue;
        }
        for trial in std::fs::read_dir(goal.path())
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
        {
            let p = trial.path().join("result.json");
            if let Ok(text) = std::fs::read_to_string(&p) {
                results.push(
                    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?,
                );
            }
        }
    }
    if results.is_empty() {
        return Err(format!(
            "{}: no result.json files; is it an agent-eval run directory?",
            dir.display()
        ));
    }
    results.sort_by(|a, b| {
        (&a.goal, a.track.name(), a.trial).cmp(&(&b.goal, b.track.name(), b.trial))
    });
    let summary = summarize(&header, &results, set);
    super::util::write_json(&dir.join("summary.json"), &summary)?;
    print(&summary);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_eval::run::{Margin, TokensOut};

    fn trial(goal: &str, track: Track, n: u32, pass: bool, renders: u32) -> TrialResult {
        TrialResult {
            schema: "enth.agent-eval.trial/1".into(),
            eval_version: 1,
            goal: goal.into(),
            track,
            trial: n,
            runner: "fake:witness".into(),
            model: Some("m".into()),
            date: "2026-10-19T00:00:00Z".into(),
            claude_code_version: None,
            status: "ok".into(),
            tokens: TokensOut {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                total: 15,
            },
            cost_usd: Some(0.1),
            cost_is_estimate: true,
            turns: Some(3),
            wall_s: 2.0,
            calls: 10,
            invalid_calls: u32::from(n == 2),
            refused_calls: 0,
            renders,
            pass,
            thin: 0,
            margins: vec![Margin {
                expect: "attack < 10".into(),
                pass,
                got: json!(5),
                margin: Some(5.0),
                margin_pct: Some(50.0),
                unit: Some("ms".into()),
                thin: false,
            }],
            failed: if pass {
                vec![]
            } else {
                vec!["attack < 10".into()]
            },
            own_expect: None,
            own_pass: None,
            own_error: None,
            final_patch: None,
            final_wav: None,
            patch_source: "answer".into(),
            produced_by_enth: true,
            dir: String::new(),
            warnings: vec![],
        }
    }

    #[test]
    fn a_goal_passes_on_a_strict_majority_of_its_trials() {
        assert!(majority(2, 3));
        assert!(!majority(1, 3));
        assert!(majority(1, 1));
        assert!(!majority(1, 2));
    }

    #[test]
    fn entries_count_pass_at_k_pass_pow_k_and_the_invalid_rate() {
        let goal: Goal = toml::from_str(
            r#"schema = "enth.agent-eval/1"
id = "g"
added = "2026-10-19"
difficulty = 2
tags = []
brief = "b"
template = "init"
expect = "attack < 10"
rationale = "r"
engine_min = "0.1.0"
perf = { notes = [{ note = "A3", vel = 100, on_s = 0.0, off_s = 1.5 }], dur_s = 2.0 }
budget = { renders = 20, turns = 60, usd = 1.0 }"#,
        )
        .unwrap();
        let t = [
            trial("g", Track::Given, 1, true, 4),
            trial("g", Track::Given, 2, false, 20),
            trial("g", Track::Given, 3, true, 6),
        ];
        let refs: Vec<&TrialResult> = t.iter().collect();
        let e = entry(&goal, Track::Given, &refs);
        assert_eq!(e["passes"], 2);
        assert_eq!(e["pass_at_k"], true);
        assert_eq!(e["pass_pow_k"], false);
        assert_eq!(e["majority_pass"], true);
        assert_eq!(e["renders"]["median"], 6.0);
        assert_eq!(e["renders"]["max"], 20.0);
        assert_eq!(e["invalid_rate"], 0.03333);
        assert_eq!(
            e["final"]["trial"], 1,
            "the passing run with the lower median renders stands in"
        );
    }

    #[test]
    fn the_mode_picks_the_most_common_model() {
        let m = mode(["a", "b", "b"].into_iter().map(str::to_owned));
        assert_eq!(m.as_deref(), Some("b"));
        assert!(mode(std::iter::empty()).is_none());
    }
}
