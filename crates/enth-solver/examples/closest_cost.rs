//! Lane CLS's measurement: what "How close am I?" costs on lesson-like starts (`docs/dsp/match.md`, "Closest").
//!
//! A lesson's Recreate step is a generated round whose every other control is already set and whose editable set is
//! 1 to 4 parameters. So each row here is a generator v1 round (a level preset and a seed): the target patch with `k`
//! of its randomized parameters taken back out of place. Which `k`: the first of the order a course meets the
//! controls in (cutoff, resonance, attack, release, pitch, …) that the round moves, or with `--by-effect` the ones
//! whose reset hurts the sound most. Only knobs by default (`--mixed` allows a waveform or a filter mode). Two kinds
//! of start:
//!
//! - **untouched**: the `k` parameters at the template's values (the step as it opens);
//! - **partial**: each moved half-way to the target's value on its normalised axis (a try that is not there yet).
//!
//! A round counts only if its start fails the round (a lesson's start always does) and its target differs from the
//! rounds taken before it; a (round, `k`, start) whose start passes anyway costs one render and is listed apart.
//! `tgtdist` is the knob distance to the target's own settings, where a pass exists: the nearest passing patch should
//! not be farther.
//!
//! The same job runs on every worker count asked for (`--workers 1,8`); the answers must be identical (the run stops
//! if not) and the seconds are wall time per count.
//!
//! ```text
//! cargo run --release -p enth-solver --example closest_cost -- [--levels 2,3] [--rounds 5] [--ks 1,2,3,4]
//!     [--starts untouched,partial] [--workers 1,8] [--budget 300] [--lambda 0] [--sigma 0.1] [--patience 3]
//!     [--tol 0.03] [--choices 3] [--flip-cost 1] [--seed 1] [--from 0] [--scan 60] [--by-effect] [--mixed]
//!     [--list] [--verbose] [--quiet]
//! ```
//!
//! Run it under `flock -x /tmp/enth-quiet.lock` when timing matters. Plays no audio and writes no files.
#![allow(
    clippy::disallowed_methods,
    clippy::print_stdout,
    reason = "a measurement tool: it reads the clock for wall time and prints its table"
)]

#[path = "support/round_pool.rs"]
mod round_pool;

use std::time::Instant;

use enth_challenge::spec::PRESETS;
use enth_challenge::{PreparedRound, prepare_round};
use enth_patch::{Op, ParamAddr, Patch, apply};
use enth_solver::closest::{Closest, ClosestCfg, ClosestJob, ClosestStop, closest, reading};
use enth_solver::search::FreeSet;

fn arg<T: std::str::FromStr>(name: &str, default: T) -> T {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn list<T: std::str::FromStr>(name: &str, default: &str) -> Vec<T> {
    arg::<String>(name, default.to_owned())
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => f64::NAN,
        n if n % 2 == 1 => v[n / 2],
        n => f64::midpoint(v[n / 2 - 1], v[n / 2]),
    }
}

fn set(p: &mut Patch, a: &ParamAddr, value: f32) {
    apply(
        p,
        Op::SetParam {
            addr: a.clone(),
            value,
        },
    )
    .expect("a randomized parameter takes its own values");
}

/// One lesson-like step: the editable parameters, and the two kinds of start.
struct Step {
    free: Vec<ParamAddr>,
    untouched: Patch,
    partial: Patch,
}

/// The order a course meets the controls in (docs/learn.md, PLAN.md: the filter first, then the envelope, then the
/// oscillator): the free set of a step is the first `k` of these the round actually moves.
const LESSON_ORDER: [&str; 10] = [
    "filter1.cutoff",
    "filter1.resonance",
    "env1.attack",
    "env1.release",
    "osc1.semi",
    "env1.decay",
    "env1.sustain",
    "osc1.pw",
    "env2.decay",
    "env2.sustain",
];

/// The `k` randomized parameters of a step. Without `mixed` only continuous parameters are candidates (a lesson
/// usually teaches knobs; a waveform or a filter mode is the `--mixed` case). With `by_effect` they are the ones whose
/// reset hurts the sound most (ties in address order), otherwise the first `k` in [`LESSON_ORDER`] (then by effect).
fn step(round: &PreparedRound, k: usize, mixed: bool, by_effect: bool) -> Option<Step> {
    let (target, start) = (round.target(), round.start());
    let mut ranked: Vec<(usize, f64, ParamAddr)> = round
        .params()
        .iter()
        .filter(|a| target.value(a) != start.value(a))
        .filter(|a| {
            mixed
                || FreeSet::new(start, std::slice::from_ref(a))
                    .is_ok_and(|f| f.categorical().is_empty())
        })
        .map(|a| {
            let mut p = target.clone();
            set(&mut p, a, start.value(a).expect("the start has it"));
            let order = if by_effect {
                0
            } else {
                LESSON_ORDER
                    .iter()
                    .position(|n| *n == a.to_string())
                    .unwrap_or(LESSON_ORDER.len())
            };
            (
                order,
                reading(round, &p).map_or(100.0, |r| r.percent),
                a.clone(),
            )
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    if ranked.len() < k {
        return None;
    }
    let free: Vec<ParamAddr> = ranked.into_iter().take(k).map(|(_, _, a)| a).collect();
    let (mut untouched, mut partial) = (target.clone(), target.clone());
    for a in &free {
        let (t, s) = (
            target.value(a).expect("target has it"),
            start.value(a).expect("start has it"),
        );
        set(&mut untouched, a, s);
        let spec = start.spec(a).expect("spec");
        let half = f32::midpoint(spec.to_norm(t), spec.to_norm(s));
        set(&mut partial, a, spec.from_norm(half));
    }
    Some(Step {
        free,
        untouched,
        partial,
    })
}

/// The knob distance from the player's patch to the target's own settings on the free axes, in `closest`'s own metric
/// (a changed waveform or filter mode costs `flip_cost`): a pass exists there, so the nearest passing patch is no
/// farther.
fn target_distance(
    round: &PreparedRound,
    patch: &Patch,
    free: &[ParamAddr],
    flip_cost: f64,
) -> f64 {
    let set = FreeSet::new(patch, free).expect("a free set");
    let (from, to) = (set.point_of(patch), set.point_of(round.target()));
    let knobs: f64 = from
        .cont
        .iter()
        .zip(&to.cont)
        .map(|(a, b)| (a - b) * (a - b))
        .sum();
    let flips = from.cat.iter().zip(&to.cat).filter(|(a, b)| a != b).count();
    knobs.sqrt() + flip_cost * flips as f64
}

struct Row {
    k: usize,
    kind: &'static str,
    found: Closest,
    tgt: f64,
    secs: Vec<f64>,
}

fn main() {
    let levels: Vec<usize> = list("--levels", "2,3");
    let rounds: usize = arg("--rounds", 5);
    let from: u64 = arg("--from", 0);
    let scan: u64 = arg("--scan", 60);
    let ks: Vec<usize> = list("--ks", "1,2,3,4");
    let starts: Vec<String> = list("--starts", "untouched,partial");
    let workers: Vec<usize> = list("--workers", "1,8");
    let budget: usize = arg("--budget", 300);
    let base = ClosestCfg::default();
    let cfg = ClosestCfg {
        seed: arg("--seed", base.seed),
        lambda: arg("--lambda", base.lambda),
        sigma: arg("--sigma", base.sigma),
        patience: arg("--patience", base.patience),
        tol: arg("--tol", base.tol),
        choices: arg("--choices", base.choices),
        flip_cost: arg("--flip-cost", base.flip_cost),
    };
    let (verbose, quiet) = (flag("--verbose"), flag("--quiet"));
    let (mixed, by_effect) = (flag("--mixed"), flag("--by-effect"));
    println!("budget {budget}, workers {workers:?}, cfg {cfg:?}");
    if !quiet {
        println!(
            "{:>3} {:>6} {:>2} {:>9} {:>6} {:>6} {:>4} {:>7} {:>7} {:>6} {:>7} {:>9}  secs per worker count",
            "lvl",
            "seed",
            "k",
            "start",
            "from%",
            "to%",
            "pass",
            "dist",
            "tgtdist",
            "renders",
            "1st",
            "stop"
        );
    }
    let mut all: Vec<Row> = Vec::new();
    let mut start_passes = 0usize;
    for &level in &levels {
        let preset = &PRESETS[level - 1];
        let mut taken: Vec<Vec<u32>> = Vec::new();
        let mut accepted = 0usize;
        for i in from..from + scan {
            if accepted == rounds {
                break;
            }
            let spec = preset.spec(1, 0x434C_0000 + i);
            let Ok(round) = prepare_round(&spec) else {
                continue;
            };
            if flag("--list") {
                println!(
                    "level {level} seed {:x}: {:?}",
                    spec.seed,
                    round
                        .params()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                );
                accepted += 1;
                continue;
            }
            // A lesson's start fails, and no two rounds of the table share a target.
            let Some(first) = step(&round, 1, mixed, by_effect) else {
                continue;
            };
            if reading(&round, &first.untouched).is_none_or(|r| r.passed) {
                continue;
            }
            let signature: Vec<u32> = step(&round, 4, mixed, by_effect)
                .unwrap_or(first)
                .free
                .iter()
                .map(|a| round.target().value(a).map_or(0, f32::to_bits))
                .collect();
            if taken.contains(&signature) {
                continue;
            }
            taken.push(signature);
            accepted += 1;
            for &k in &ks {
                let Some(s) = step(&round, k, mixed, by_effect) else {
                    continue;
                };
                for kind in &starts {
                    let (patch, kind) = if kind == "partial" {
                        (&s.partial, "partial")
                    } else {
                        (&s.untouched, "untouched")
                    };
                    let job = ClosestJob {
                        round: &round,
                        patch,
                        free: &s.free,
                        budget,
                    };
                    let mut first: Option<Closest> = None;
                    let mut secs = Vec::new();
                    for &w in &workers {
                        let mut pool = round_pool::RoundPool::new(w);
                        let t0 = Instant::now();
                        let found = closest(&job, &mut pool, &cfg).expect("the job runs");
                        secs.push(t0.elapsed().as_secs_f64());
                        match &first {
                            None => first = Some(found),
                            Some(f) => assert_eq!(
                                f, &found,
                                "level {level} seed {:x} k {k} {kind}: {w} workers differ from {}",
                                spec.seed, workers[0]
                            ),
                        }
                    }
                    let found = first.expect("at least one worker count");
                    let tgt = target_distance(&round, patch, &s.free, cfg.flip_cost);
                    if !quiet {
                        println!(
                            "{:>3} {:>6x} {:>2} {:>9} {:>6.1} {:>6.1} {:>4} {:>7.4} {:>7.4} {:>6} {:>7} {:>9}  {}",
                            level,
                            spec.seed & 0xFFFF,
                            k,
                            kind,
                            found.start_percent,
                            found.percent,
                            if found.passed { "yes" } else { "NO" },
                            found.distance,
                            tgt,
                            found.renders,
                            found
                                .first_pass_renders
                                .map_or_else(|| "-".to_owned(), |r| r.to_string()),
                            found.stop.name(),
                            secs.iter()
                                .map(|s| format!("{s:.2}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                        if verbose {
                            println!(
                                "      free: {}",
                                s.free
                                    .iter()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                            for d in &found.deltas {
                                println!(
                                    "      {:<16} {:>10.3} -> {:>10.3}  ({:+.3} of the knob); the target has {:.3}",
                                    d.addr.to_string(),
                                    d.from,
                                    d.to,
                                    d.norm,
                                    round.target().value(&d.addr).unwrap_or(f32::NAN)
                                );
                            }
                        }
                    }
                    if found.stop == ClosestStop::StartPasses {
                        start_passes += 1;
                        continue;
                    }
                    all.push(Row {
                        k,
                        kind,
                        found,
                        tgt,
                        secs,
                    });
                }
            }
        }
    }
    println!();
    println!("starts that already pass (one render, left out below): {start_passes}");
    println!(
        "{:>2} {:>9} {:>3} {:>6} {:>8} {:>8} {:>7} {:>8} {:>9} {:>5}  secs median (max) per worker count",
        "k",
        "start",
        "n",
        "passed",
        "rend med",
        "rend max",
        "1st med",
        "dist med",
        "dist/tgt",
        "worse"
    );
    for &k in &ks {
        for kind in ["untouched", "partial"] {
            let rows: Vec<&Row> = all.iter().filter(|r| r.k == k && r.kind == kind).collect();
            if rows.is_empty() {
                continue;
            }
            let col = |f: &dyn Fn(&Row) -> f64| rows.iter().map(|r| f(r)).collect::<Vec<_>>();
            let renders = col(&|r| r.found.renders as f64);
            let firsts: Vec<f64> = rows
                .iter()
                .filter_map(|r| r.found.first_pass_renders.map(|v| v as f64))
                .collect();
            let dists = col(&|r| r.found.distance);
            let passed: Vec<&&Row> = rows.iter().filter(|r| r.found.passed).collect();
            // How near the answer is, against the target's own distance (< 1 is better than the target itself).
            let ratios: Vec<f64> = passed
                .iter()
                .filter(|r| r.tgt > 0.0)
                .map(|r| r.found.distance / r.tgt)
                .collect();
            let worse = ratios.iter().filter(|r| **r > 1.0).count();
            let secs: Vec<String> = (0..workers.len())
                .map(|w| {
                    let v = col(&|r| r.secs[w]);
                    format!(
                        "{:.2} ({:.2})",
                        median(&v),
                        v.iter().copied().fold(0.0, f64::max)
                    )
                })
                .collect();
            println!(
                "{:>2} {:>9} {:>3} {:>6} {:>8.0} {:>8.0} {:>7.0} {:>8.4} {:>9.2} {:>5}  {}",
                k,
                kind,
                rows.len(),
                format!("{}/{}", passed.len(), rows.len()),
                median(&renders),
                renders.iter().copied().fold(0.0, f64::max),
                median(&firsts),
                median(&dists),
                median(&ratios),
                worse,
                secs.join("  ")
            );
        }
    }
    let total: f64 = all.iter().map(|r| r.found.renders as f64).sum();
    let passed = all.iter().filter(|r| r.found.passed).count();
    println!("ALL: {passed}/{} passed, {total:.0} renders", all.len());
}
