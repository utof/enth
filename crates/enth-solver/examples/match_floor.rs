//! Lane MT's measurement: the "CMA-ES floor" on randomizer targets (`docs/dsp/match.md`).
//!
//! For each level preset asked for, generate targets from fixed seeds with `enth_challenge::random`, then search from
//! the level's starting patch over the parameters the generator drew, with the heuristic start and CMA-ES, and
//! report the metric v1 distance before (the start patch) and after, and the renders used.
//!
//! ```text
//! cargo run --release -p enth-solver --example match_floor -- [--levels 1,2] [--targets 5] [--budget 1500]
//!     [--workers 4] [--lambda 0] [--no-heuristic] [--seed 1] [--free drawn|observable]
//! ```
//!
//! Run it under `flock -x /tmp/enth-quiet.lock` when timing matters. Plays no audio and writes no files.
#![allow(
    clippy::disallowed_methods,
    clippy::print_stdout,
    reason = "a measurement tool: it reads the clock for wall time and prints its table"
)]

#[path = "support/pool.rs"]
mod pool;

use std::time::Instant;

use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::random;
use enth_challenge::spec::PRESETS;
use enth_render::RenderSpec;
use enth_solver::heuristic::Heuristic;
use enth_solver::search::{FreeSet, MetricTarget, SearchCfg, search};

fn arg<T: std::str::FromStr>(name: &str, default: T) -> T {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        f64::NAN
    } else if v.len() % 2 == 1 {
        v[v.len() / 2]
    } else {
        f64::midpoint(v[v.len() / 2 - 1], v[v.len() / 2])
    }
}

fn main() {
    let levels: String = arg("--levels", "1,2".to_owned());
    let targets: u64 = arg("--targets", 5);
    let budget: usize = arg("--budget", 1500);
    let workers: usize = arg("--workers", 4);
    let lambda: usize = arg("--lambda", 0);
    let seed: u64 = arg("--seed", 1);
    let free_mode: String = arg("--free", "drawn".to_owned());
    let use_heuristic = !flag("--no-heuristic");
    let verbose = flag("--verbose");
    let stop_at: f64 = arg("--stop-at", 0.0);
    let spec = RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED);

    println!(
        "budget {budget}, lambda {lambda} (0 = default), workers {workers}, heuristic {use_heuristic}, free set {free_mode}"
    );
    println!(
        "{:>3} {:>10} {:>3} {:>5} {:>8} {:>8} {:>8} {:>7} {:>5} {:>6} {:>7}",
        "lvl", "seed", "n", "free", "before", "heur", "after", "renders", "runs", "secs", "drawn"
    );
    let (mut befores, mut afters, mut renders_all, mut heurs) = (vec![], vec![], vec![], vec![]);
    for level in levels.split(',').filter_map(|l| l.trim().parse::<usize>().ok()) {
        let preset = &PRESETS[level - 1];
        let (mut b_l, mut a_l, mut r_l) = (vec![], vec![], vec![]);
        for i in 0..targets {
            let rspec = preset.spec(1, 0x4D54_0000 + i);
            let generated = random::target(&rspec).expect("preset specs generate");
            let start = random::start(rspec.template).expect("template 0 exists");
            let perf = generated.phrase().performance();
            let target = MetricTarget::from_patch(&generated.patch, &perf, &spec)
                .expect("generated targets render");
            let addrs: Vec<_> = if free_mode == "observable" {
                start_mask_addrs(&start, &generated)
            } else {
                generated.params.clone()
            };
            let free = FreeSet::new(&start, &addrs).expect("drawn parameters exist in the start");
            let heuristic = target
                .descriptors()
                .map(|d| Heuristic::new(d, target.performance()));
            let cfg = SearchCfg {
                seed: seed + i,
                budget,
                lambda,
                use_heuristic,
                stop_at: (stop_at > 0.0).then_some(stop_at),
                ..SearchCfg::default()
            };
            let mut objective = pool::Pool::new(&target, workers);
            let t0 = Instant::now();
            let r = search(&start, &free, heuristic.as_ref(), &mut objective, &cfg)
                .expect("search runs");
            let secs = t0.elapsed().as_secs_f64();
            println!(
                "{:>3} {:>10x} {:>3} {:>5} {:>8.4} {:>8} {:>8.4} {:>7} {:>5} {:>6.1} {:>7}",
                level,
                rspec.seed,
                i,
                free.len(),
                r.start_distance,
                r.heuristic_distance
                    .map_or_else(|| "-".to_owned(), |h| format!("{h:.4}")),
                r.distance,
                r.renders,
                r.runs,
                secs,
                generated.params.len(),
            );
            if verbose {
                for a in free.addrs() {
                    println!(
                        "      {:<18} target {:>10.4}  start {:>10.4}  found {:>10.4}",
                        a.to_string(),
                        generated.patch.value(a).unwrap_or(f32::NAN),
                        start.value(a).unwrap_or(f32::NAN),
                        r.best.value(a).unwrap_or(f32::NAN)
                    );
                }
                println!(
                    "      suggestions {:?}",
                    r.suggestions
                        .iter()
                        .map(|s| format!("{}={:.3}", s.addr, s.value))
                        .collect::<Vec<_>>()
                );
                let reach = |thr: f64| {
                    r.trace
                        .iter()
                        .find(|t| t.best <= thr)
                        .map_or_else(|| "-".to_owned(), |t| t.renders.to_string())
                };
                println!(
                    "      renders to reach 0.1: {}, 0.05: {}, 0.02: {}, 0.005: {}",
                    reach(0.1),
                    reach(0.05),
                    reach(0.02),
                    reach(0.005)
                );
            }
            b_l.push(r.start_distance);
            a_l.push(r.distance);
            r_l.push(r.renders as f64);
            heurs.push(r.heuristic_distance.unwrap_or(r.start_distance));
        }
        println!(
            "level {level}: median before {:.4}, after {:.4}, renders {:.0}",
            median(&mut b_l.clone()),
            median(&mut a_l.clone()),
            median(&mut r_l.clone())
        );
        befores.extend(b_l);
        afters.extend(a_l);
        renders_all.extend(r_l);
    }
    println!(
        "ALL: median before {:.4}, heuristic start {:.4}, after {:.4}, renders {:.0}, targets {}",
        median(&mut befores),
        median(&mut heurs),
        median(&mut afters),
        median(&mut renders_all),
        afters.len()
    );
}

/// Every observable parameter of the target's mask, as the "agent does not know what was drawn" free set.
fn start_mask_addrs(
    start: &enth_patch::Patch,
    generated: &random::Generated,
) -> Vec<enth_patch::ParamAddr> {
    let mask = enth_challenge::random::observable(start, generated.phrase());
    mask.observable_params().cloned().collect()
}
