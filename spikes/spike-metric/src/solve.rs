//! `solve`: recover random targets with CMA-ES under each distance and report solve rate, evaluations and time.
//!
//! "Solved" is a per-distance proxy: the best distance to the target is at or below T1 (strict) or T3 (loose), the
//! median distance from the target to 16 renders with every continuous parameter jittered by +-1 % (T1) or +-3 % (T3)
//! of its range. Every final result is also re-judged by all three distances ("common judge"). Ears decide in the
//! listening test; these are proxies.

use crate::util::{Res, host_threads, load_avg, write_json};
use clap::{Args as ClapArgs, ValueEnum};
use serde::Serialize;
use spike_metric::distance::{Dist, Scale, median};
use spike_metric::features::{AnalysisCfg, Analyzer};
use spike_metric::heuristic::heuristic;
use spike_metric::par::ParEval;
use spike_metric::render::{N_PARAMS, RenderCfg, Wave, render_unit};
use spike_metric::rng::Rng;
use spike_metric::search::{Restarts, Search, SearchCfg, drive, lambda_for_workers};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Copy, ValueEnum)]
enum DistArg {
    A,
    B,
    C,
    All,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum StartArg {
    Heuristic,
    Random,
    /// Heuristic and random searches side by side in one combined batch; the best result wins.
    Both,
    /// Run each of the three configurations in turn.
    All,
}

#[derive(Clone, Copy, ValueEnum)]
enum RestartArg {
    Ipop,
    Bipop,
}

#[derive(ClapArgs)]
pub struct Args {
    /// Distance to search with (A: log-mel L1, B: A + envelope + centroid, C: MFCC).
    #[arg(long, value_enum, ignore_case = true, default_value = "a")]
    distance: DistArg,
    #[arg(long, value_enum, default_value = "heuristic")]
    start: StartArg,
    /// Number of random targets (the same seeded list every run).
    #[arg(long, default_value_t = 20)]
    targets: usize,
    /// Evaluation cap per search.
    #[arg(long, default_value_t = 5000)]
    budget: usize,
    #[arg(long, default_value_t = 8)]
    workers: usize,
    /// Population size of the first CMA-ES run; 0 = Hansen's default rounded up to a multiple of the workers.
    #[arg(long, default_value_t = 0)]
    lambda: usize,
    /// Initial step size around the heuristic start.
    #[arg(long, default_value_t = 0.15)]
    sigma_start: f64,
    #[arg(long, value_enum, default_value = "ipop")]
    restarts: RestartArg,
    /// Wall-clock cap per solve in seconds (0 = none). Enforced here, not in the solver.
    #[arg(long, default_value_t = 0.0)]
    time_cap: f64,
    /// Base seed for the searches (target i uses seed + i).
    #[arg(long, default_value_t = 1000)]
    seed: u64,
    /// Write all results as JSON.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Print only the summaries.
    #[arg(long)]
    quiet: bool,
    /// 0.5 s clips instead of 2 s (for tests).
    #[arg(long, hide = true)]
    quick: bool,
}

#[derive(Serialize, Clone)]
struct Judged {
    distance: Dist,
    value: f64,
    strict: bool,
    loose: bool,
}

#[derive(Serialize)]
struct TargetResult {
    target: usize,
    /// Thresholds of the searched distance.
    t1: f64,
    t3: f64,
    start_distance: Option<f64>,
    best: f64,
    evals: usize,
    restarts: usize,
    evals_to_t1: Option<usize>,
    evals_to_t3: Option<usize>,
    wall_to_t1_s: Option<f64>,
    wall_s: f64,
    wave_ok: bool,
    param_err: f64,
    judged: Vec<Judged>,
}

#[derive(Serialize)]
struct CellSummary {
    distance: Dist,
    start: StartArg,
    workers: usize,
    lambda: usize,
    budget: usize,
    sigma_start: f64,
    restarts: &'static str,
    targets: usize,
    solved_t1: usize,
    solved_t3: usize,
    median_evals_t1: f64,
    median_evals_t3: f64,
    median_wall_to_t1_s: f64,
    median_wall_s: f64,
    max_wall_s: f64,
    total_wall_s: f64,
    wave_ok: usize,
    median_param_err: f64,
    checksum: f64,
    /// Results re-judged by A, B, C (strict / loose counts), whatever distance found them.
    judged_strict: [usize; 3],
    judged_loose: [usize; 3],
    load_start: String,
    load_end: String,
}

#[derive(Serialize)]
struct Cell {
    summary: CellSummary,
    results: Vec<TargetResult>,
}

#[derive(Serialize)]
struct Output {
    schema: &'static str,
    host_threads: usize,
    scale: Scale,
    cells: Vec<Cell>,
}

/// The same seeded target list as the researcher's probe (so numbers compare): random patches that are audible.
fn make_targets(n: usize, cfg: &RenderCfg) -> Vec<Vec<f64>> {
    let mut buf = Vec::new();
    let mut v = Vec::new();
    let mut rng = Rng::new(20_261_019);
    while v.len() < n {
        let p: Vec<f64> = (0..N_PARAMS).map(|_| rng.uniform()).collect();
        render_unit(cfg, &p, &mut buf);
        let rms = (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt();
        if rms > 1e-3 {
            v.push(p);
        }
    }
    v
}

/// T1/T3 per distance: medians over 16 renders jittered by +-1 % and 16 by +-3 % (waveform kept).
fn thresholds(pe: &ParEval, tp: &[f64], ti: usize, scale: &Scale) -> [(f64, f64); 3] {
    let mut rng = Rng::new(ti as u64 * 7919 + 1);
    let batch: Vec<Vec<f64>> = (0..32)
        .map(|rep| {
            let pert = if rep < 16 { 0.01 } else { 0.03 };
            tp.iter()
                .enumerate()
                .map(|(i, v)| {
                    if i == 0 {
                        *v
                    } else {
                        (v + pert * (2.0 * rng.uniform() - 1.0)).clamp(0.0, 1.0)
                    }
                })
                .collect()
        })
        .collect();
    let comps = pe.components(&batch);
    let mut out = [(0.0, 0.0); 3];
    for (k, kind) in Dist::ALL.iter().enumerate() {
        let mut d1: Vec<f64> = comps[..16]
            .iter()
            .map(|c| c.distance(*kind, scale))
            .collect();
        let mut d3: Vec<f64> = comps[16..]
            .iter()
            .map(|c| c.distance(*kind, scale))
            .collect();
        out[k] = (median(&mut d1), median(&mut d3));
    }
    out
}

/// (total evaluations, seconds) at which the best distance first crossed a threshold.
type Milestone = (usize, f64);

struct TargetInfo {
    params: Vec<f64>,
    audio: Vec<f32>,
    thr: [(f64, f64); 3],
}

fn dist_index(d: Dist) -> usize {
    Dist::ALL.iter().position(|x| *x == d).unwrap_or(0)
}

#[allow(
    clippy::too_many_arguments,
    reason = "one experiment cell: all of its knobs are separate CLI settings"
)]
fn run_cell(
    args: &Args,
    pe: &mut ParEval,
    render: &RenderCfg,
    scale: &Scale,
    infos: &[TargetInfo],
    kind: Dist,
    start: StartArg,
    lambda: usize,
) -> Cell {
    let ki = dist_index(kind);
    pe.set_kind(kind, *scale);
    let load_start = load_avg();
    let cell_t0 = Instant::now();
    let mut results = Vec::new();
    let mut checksum = 0.0;
    for (ti, info) in infos.iter().enumerate() {
        pe.set_target(&info.params);
        let (t1, t3) = info.thr[ki];
        let cfg_for = |heur: bool, seed: u64| SearchCfg {
            seed,
            start: heur.then(|| heuristic(render, &info.audio)),
            sigma_start: args.sigma_start,
            sigma_random: 0.3,
            lambda0: lambda,
            budget: args.budget,
            stop_at: Some(t1),
            restarts: match args.restarts {
                RestartArg::Ipop => Restarts::Ipop,
                RestartArg::Bipop => Restarts::Bipop,
            },
        };
        let mut searches: Vec<Search> = match start {
            StartArg::Heuristic => vec![Search::new(cfg_for(true, args.seed + ti as u64))],
            StartArg::Random => vec![Search::new(cfg_for(false, args.seed + ti as u64))],
            _ => vec![
                Search::new(cfg_for(true, args.seed + ti as u64)),
                Search::new(cfg_for(false, args.seed + 100_000 + ti as u64)),
            ],
        };
        let t0 = Instant::now();
        let mut e1: Option<Milestone> = None;
        let mut e3: Option<Milestone> = None;
        drive(&mut searches, pe, |ss| {
            let best = ss
                .iter()
                .map(Search::best_fitness)
                .fold(f64::INFINITY, f64::min);
            let total: usize = ss.iter().map(Search::evals).sum();
            let t = t0.elapsed().as_secs_f64();
            if e3.is_none() && best <= t3 {
                e3 = Some((total, t));
            }
            if e1.is_none() && best <= t1 {
                e1 = Some((total, t));
            }
            args.time_cap <= 0.0 || t < args.time_cap
        });
        let wall = t0.elapsed().as_secs_f64();
        let win = searches
            .iter()
            .min_by(|a, b| a.best_fitness().total_cmp(&b.best_fitness()))
            .expect("at least one search");
        let (evals, restarts) = (
            searches.iter().map(Search::evals).sum::<usize>(),
            searches.iter().map(Search::restarts).sum::<usize>(),
        );
        let bx = win.best_x().to_vec();
        let comps = pe.components(std::slice::from_ref(&bx))[0];
        let judged: Vec<Judged> = Dist::ALL
            .iter()
            .enumerate()
            .map(|(k, d)| {
                let v = comps.distance(*d, scale);
                Judged {
                    distance: *d,
                    value: v,
                    strict: v <= info.thr[k].0,
                    loose: v <= info.thr[k].1,
                }
            })
            .collect();
        let param_err = (1..N_PARAMS)
            .map(|i| (info.params[i] - bx[i]).abs())
            .sum::<f64>()
            / (N_PARAMS - 1) as f64;
        let wave_ok = Wave::from_unit(info.params[0]) == Wave::from_unit(bx[0]);
        checksum += win.best_fitness() + evals as f64;
        let r = TargetResult {
            target: ti,
            t1,
            t3,
            start_distance: searches
                .iter()
                .map(Search::start_fitness)
                .find(|v| v.is_finite()),
            best: win.best_fitness(),
            evals,
            restarts,
            evals_to_t1: e1.map(|e| e.0),
            evals_to_t3: e3.map(|e| e.0),
            wall_to_t1_s: e1.map(|e| e.1),
            wall_s: wall,
            wave_ok,
            param_err,
            judged,
        };
        if !args.quiet {
            println!(
                "{kind:?} {start:?} t{ti:02} T1 {:.4} T3 {:.4} start {} best {:.4} evals {} e3 {:?} e1 {:?} restarts {} wave {} perr {:.3} wall {:.2}s",
                r.t1,
                r.t3,
                r.start_distance
                    .map_or("-".to_string(), |v| format!("{v:.4}")),
                r.best,
                r.evals,
                r.evals_to_t3,
                r.evals_to_t1,
                r.restarts,
                r.wave_ok,
                r.param_err,
                r.wall_s
            );
        }
        results.push(r);
    }
    let count = |f: &dyn Fn(&TargetResult) -> bool| results.iter().filter(|r| f(r)).count();
    let med = |mut it: Vec<f64>| median(&mut it);
    let mut judged_strict = [0; 3];
    let mut judged_loose = [0; 3];
    for r in &results {
        for (k, j) in r.judged.iter().enumerate() {
            judged_strict[k] += usize::from(j.strict);
            judged_loose[k] += usize::from(j.loose);
        }
    }
    let summary = CellSummary {
        distance: kind,
        start,
        workers: pe.n_workers(),
        lambda,
        budget: args.budget,
        sigma_start: args.sigma_start,
        restarts: match args.restarts {
            RestartArg::Ipop => "ipop",
            RestartArg::Bipop => "bipop",
        },
        targets: results.len(),
        solved_t1: count(&|r| r.evals_to_t1.is_some()),
        solved_t3: count(&|r| r.evals_to_t3.is_some()),
        median_evals_t1: med(results
            .iter()
            .filter_map(|r| r.evals_to_t1)
            .map(|e| e as f64)
            .collect()),
        median_evals_t3: med(results
            .iter()
            .filter_map(|r| r.evals_to_t3)
            .map(|e| e as f64)
            .collect()),
        median_wall_to_t1_s: med(results.iter().filter_map(|r| r.wall_to_t1_s).collect()),
        median_wall_s: med(results.iter().map(|r| r.wall_s).collect()),
        max_wall_s: results.iter().map(|r| r.wall_s).fold(0.0, f64::max),
        total_wall_s: cell_t0.elapsed().as_secs_f64(),
        wave_ok: count(&|r| r.wave_ok),
        median_param_err: med(results.iter().map(|r| r.param_err).collect()),
        checksum,
        judged_strict,
        judged_loose,
        load_start,
        load_end: load_avg(),
    };
    println!(
        "SUMMARY dist {:?} start {:?} workers {} lambda {} budget {} {}: T1 {}/{} (median evals {:.0}, median wall-to-T1 {:.2}s) | T3 {}/{} (median evals {:.0}) | wave {}/{} | median perr {:.3} | wall/solve median {:.2}s max {:.2}s | total {:.1}s | judged strict A/B/C {:?} loose {:?} | checksum {:.9} | load {} -> {}",
        summary.distance,
        summary.start,
        summary.workers,
        summary.lambda,
        summary.budget,
        summary.restarts,
        summary.solved_t1,
        summary.targets,
        summary.median_evals_t1,
        summary.median_wall_to_t1_s,
        summary.solved_t3,
        summary.targets,
        summary.median_evals_t3,
        summary.wave_ok,
        summary.targets,
        summary.median_param_err,
        summary.median_wall_s,
        summary.max_wall_s,
        summary.total_wall_s,
        summary.judged_strict,
        summary.judged_loose,
        summary.checksum,
        summary.load_start,
        summary.load_end
    );
    Cell { summary, results }
}

pub fn run(args: &Args) -> Res<()> {
    let render = if args.quick {
        RenderCfg::short()
    } else {
        RenderCfg::default()
    };
    let analysis = AnalysisCfg {
        sample_rate: render.sample_rate,
        ..AnalysisCfg::default()
    };
    let lambda = if args.lambda > 0 {
        args.lambda
    } else {
        lambda_for_workers(args.workers)
    };
    let mut an = Analyzer::new(analysis);
    let scale = Scale::calibrate(&mut an, &render, 42, 50);
    println!(
        "spike-metric solve: {} targets, {} workers (host threads {}), lambda {lambda}, budget {}, load {}",
        args.targets,
        args.workers,
        host_threads(),
        args.budget,
        load_avg()
    );
    println!(
        "scale for B (median component over random pairs): logmel {:.4} env {:.4} centroid {:.4}",
        scale.logmel, scale.env, scale.centroid
    );
    let mut pe = ParEval::new(args.workers, render, analysis, Dist::A, scale);
    let tparams = make_targets(args.targets, &render);
    let infos: Vec<TargetInfo> = tparams
        .into_iter()
        .enumerate()
        .map(|(ti, params)| {
            pe.set_target(&params);
            let thr = thresholds(&pe, &params, ti, &scale);
            let mut audio = Vec::new();
            render_unit(&render, &params, &mut audio);
            TargetInfo { params, audio, thr }
        })
        .collect();
    let kinds: Vec<Dist> = match args.distance {
        DistArg::A => vec![Dist::A],
        DistArg::B => vec![Dist::B],
        DistArg::C => vec![Dist::C],
        DistArg::All => Dist::ALL.to_vec(),
    };
    let starts: Vec<StartArg> = match args.start {
        StartArg::All => vec![StartArg::Heuristic, StartArg::Random, StartArg::Both],
        s => vec![s],
    };
    let mut cells = Vec::new();
    for kind in kinds {
        for start in &starts {
            cells.push(run_cell(
                args, &mut pe, &render, &scale, &infos, kind, *start, lambda,
            ));
        }
    }
    if let Some(path) = &args.json {
        write_json(
            path,
            &Output {
                schema: "enth.spike-metric.solve/1",
                host_threads: host_threads(),
                scale,
                cells,
            },
        )?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
