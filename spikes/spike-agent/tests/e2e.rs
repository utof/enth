//! The scripted end-to-end run recorded in docs/spikes/agent-probe.md: `new` with a goal's performance, a failing
//! `check` of the neutral patch, one `step` with hand-picked values that passes the goal, and the judge call with
//! the performance forced. The values are a sound designer's first guess for a short bright pluck, not a search
//! result. Also a mechanical baseline (see `follower`): apply the top hint of every failing expectation and re-check.

#![allow(clippy::print_stdout, reason = "the baseline prints its table")]

use serde::Deserialize;
use serde_json::Value;
use spike_agent::commands::check_patch;
use spike_agent::patch::Patch;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
struct Goals {
    goal: Vec<Goal>,
}

#[derive(Deserialize)]
struct Goal {
    id: String,
    perf: String,
    expect: String,
}

fn goal(id: &str) -> Goal {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/goals.toml")).unwrap();
    toml::from_str::<Goals>(&text)
        .unwrap()
        .goal
        .into_iter()
        .find(|g| g.id == id)
        .unwrap()
}

fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_spike-agent"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .args(args)
        .output()
        .unwrap();
    let v = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("spike-agent-e2e-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn a_short_bright_pluck_by_hand_picked_values() {
    let g = goal("g1-pluck");
    let dir = tmp("pluck");
    let p = dir.join("pluck.json");

    // 1. a neutral start with the goal's performance
    let (code, _) = run(&["new", "--out", s(&p), "--perf", &g.perf]);
    assert_eq!(code, 0);

    // 2. the neutral patch fails the goal, and the failures come with measured hints
    let (code, v) = run(&["check", s(&p), "--expect", &g.expect]);
    assert_eq!(code, 1, "{v}");
    let failed = v["failed"].as_u64().unwrap();
    assert!(
        failed >= 3,
        "a neutral patch is not a pluck: {failed} failing"
    );
    for r in v["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["pass"] == false)
    {
        assert!(r["hint"].is_object(), "every failure carries a hint: {r}");
    }

    // 3. a pluck: instant attack, no sustain, a filter that opens and closes quickly, a bright saw
    let (code, v) = run(&[
        "step",
        s(&p),
        "--set",
        "amp.attack=2ms,amp.decay=120ms,amp.sustain=-60dB,amp.release=80ms,filter.cutoff=6kHz,filter.resonance=1.5,filter.env_amount=2oct,filter.env_decay=150ms",
        "--expect",
        &g.expect,
        "--out",
        s(&p),
    ]);
    assert_eq!(code, 0, "the hand-picked pluck passes its goal: {v}");
    assert_eq!(v["check"]["pass"], true);
    assert_eq!(v["check"]["passed"], 5);
    assert!(p.with_extension("wav").exists());

    // 4. the judge forces the goal's performance, so changing the performance cannot pass the goal
    let (code, v) = run(&["check", s(&p), "--perf", &g.perf, "--expect", &g.expect]);
    assert_eq!(code, 0, "{v}");
}

/// Apply the first hint of every failing expectation (in order), then re-check; stop at a pass or a stall.
fn follow_hints(perf: &str, expect: &str, max_steps: usize) -> (bool, usize, String) {
    let mut p = Patch::default();
    for item in perf.split(',') {
        let (k, v) = item.split_once('=').unwrap();
        p.set(&format!("perf.{k}"), v).unwrap();
    }
    let mut last = String::new();
    for step in 0..=max_steps {
        let (v, pass) = check_patch(&p, expect).unwrap();
        if pass {
            return (true, step, last);
        }
        let mut sets = Vec::new();
        for r in v["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["pass"] == false)
        {
            let h = &r["hint"];
            if let (Some(param), Some(to)) = (h["param"].as_str(), h["try"].as_str())
                && h["measured"] == true
            {
                sets.push(format!("{param}={to}"));
            }
        }
        let failing: Vec<&str> = v["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["pass"] == false)
            .filter_map(|r| r["expect"].as_str())
            .collect();
        last = format!(
            "failing {failing:?}, hints {sets:?}, patch {}",
            p.to_json()["params"]
        );
        if sets.is_empty() {
            return (false, step, last);
        }
        match p.apply_sets(&sets) {
            Ok((next, _)) if next != p => p = next,
            _ => return (false, step, last),
        }
    }
    (false, max_steps, last)
}

#[test]
#[ignore = "baseline printout; run on request: cargo test --release -p spike-agent --test e2e -- --ignored --nocapture"]
fn follower() {
    let goals: Goals = toml::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/goals.toml")).unwrap(),
    )
    .unwrap();
    for g in goals.goal {
        let (ok, steps, last) = follow_hints(&g.perf, &g.expect, 12);
        println!(
            "{}: hint-follower {} after {steps} steps; {last}",
            g.id,
            if ok { "PASSED" } else { "did not pass" }
        );
    }
}
