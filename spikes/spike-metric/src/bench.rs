//! `bench`: cost per evaluation and scaling with the number of workers.

use crate::util::{Res, host_threads, load_avg, parse_list, write_json};
use clap::Args as ClapArgs;
use serde::Serialize;
use spike_metric::distance::{Dist, Scale, distance, median};
use spike_metric::features::{AnalysisCfg, Analyzer};
use spike_metric::par::ParEval;
use spike_metric::render::{N_PARAMS, RenderCfg, render_unit};
use spike_metric::rng::{Rng, hash_f64};
use spike_metric::search::BatchEval;
use spike_metric::yin::Yin;
use std::path::PathBuf;
use std::time::Instant;

#[derive(ClapArgs)]
pub struct Args {
    /// Repetitions for the single-thread component costs.
    #[arg(long, default_value_t = 100)]
    reps: usize,
    /// Evaluations per scaling measurement.
    #[arg(long, default_value_t = 640)]
    evals: usize,
    /// Worker counts for the scaling table.
    #[arg(long, default_value = "1,2,4,8")]
    workers: String,
    /// Batch sizes (the CMA-ES population) for the scaling table.
    #[arg(long, default_value = "16,40")]
    batches: String,
    #[arg(long)]
    json: Option<PathBuf>,
    /// 0.5 s clips instead of 2 s (for tests).
    #[arg(long, hide = true)]
    quick: bool,
}

#[derive(Serialize)]
struct Costs {
    render_ms: f64,
    loudness_ms: f64,
    features_hop_n2_ms: f64,
    features_hop_n4_ms: f64,
    yin_f0_ms: f64,
    distance_a_ms: f64,
    distance_b_ms: f64,
    distance_c_ms: f64,
    evaluation_ms: f64,
    evaluations_per_s: f64,
    load_before: String,
    load_after: String,
}

#[derive(Serialize)]
struct Scaling {
    workers: usize,
    batch: usize,
    evals_per_s: f64,
    speedup: f64,
    efficiency: f64,
    identical_to_one_worker: bool,
    checksum: f64,
    load: String,
}

#[derive(Serialize)]
struct Output {
    schema: &'static str,
    host_threads: usize,
    costs: Costs,
    scaling: Vec<Scaling>,
}

fn ms(f: impl FnOnce()) -> f64 {
    let t = Instant::now();
    f();
    t.elapsed().as_secs_f64() * 1e3
}

fn single_thread_costs(args: &Args, render: &RenderCfg) -> Costs {
    let load_before = load_avg();
    let mut rng = Rng::new(42);
    let ps: Vec<Vec<f64>> = (0..args.reps)
        .map(|_| (0..N_PARAMS).map(|_| rng.uniform()).collect())
        .collect();
    let cfg2 = AnalysisCfg {
        sample_rate: render.sample_rate,
        hop_div: 2,
        ..AnalysisCfg::default()
    };
    let cfg4 = AnalysisCfg { hop_div: 4, ..cfg2 };
    let (mut a2, mut a4) = (Analyzer::new(cfg2), Analyzer::new(cfg4));
    let mut yin = Yin::new(render.sample_rate, 2048, 50.0, 2000.0);
    let sc = Scale::DEFAULT;
    let mut buf = Vec::new();
    // warm up
    for p in ps.iter().take(5) {
        render_unit(render, p, &mut buf);
        let _ = a2.features(&buf);
        let _ = a4.features(&buf);
    }
    let (mut r, mut l, mut f2, mut f4, mut y, mut feats) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    for p in &ps {
        r.push(ms(|| render_unit(render, p, &mut buf)));
        l.push(ms(|| {
            let _ = a2.loudness(&buf);
        }));
        let mut f = None;
        f2.push(ms(|| f = Some(a2.features(&buf))));
        feats.push(f.take().expect("features"));
        f4.push(ms(|| {
            let _ = a4.features(&buf);
        }));
        y.push(ms(|| {
            let _ = yin.track(&buf, 1024);
        }));
    }
    let mut d = [0.0; 3];
    let mut acc = 0.0;
    for (k, kind) in Dist::ALL.iter().enumerate() {
        let t = Instant::now();
        for i in 0..feats.len() {
            acc += distance(*kind, &feats[i], &feats[(i + 1) % feats.len()], &sc);
        }
        d[k] = t.elapsed().as_secs_f64() * 1e3 / feats.len() as f64;
    }
    // end-to-end evaluation: render + features + distance A against a cached target
    let target = feats[0].clone();
    let mut e = vec![];
    for p in &ps {
        e.push(ms(|| {
            render_unit(render, p, &mut buf);
            let f = a2.features(&buf);
            acc += distance(Dist::A, &target, &f, &sc);
        }));
    }
    let evaluation_ms = median(&mut e);
    assert!(acc.is_finite());
    Costs {
        render_ms: median(&mut r),
        loudness_ms: median(&mut l),
        features_hop_n2_ms: median(&mut f2),
        features_hop_n4_ms: median(&mut f4),
        yin_f0_ms: median(&mut y),
        distance_a_ms: d[0],
        distance_b_ms: d[1],
        distance_c_ms: d[2],
        evaluation_ms,
        evaluations_per_s: 1e3 / evaluation_ms,
        load_before,
        load_after: load_avg(),
    }
}

pub fn run(args: &Args) -> Res<()> {
    let render = if args.quick {
        RenderCfg::short()
    } else {
        RenderCfg::default()
    };
    println!(
        "spike-metric bench: host threads {}, load {}",
        host_threads(),
        load_avg()
    );
    let c = single_thread_costs(args, &render);
    println!(
        "single thread, {} reps, {:.1} s clip at {:.0} Hz (medians; load {} -> {}):",
        args.reps, render.duration_s, render.sample_rate, c.load_before, c.load_after
    );
    println!("  render                       {:7.3} ms", c.render_ms);
    println!("  loudness (ebur128, gated)    {:7.3} ms", c.loudness_ms);
    println!(
        "  features, hop n/2 (default)  {:7.3} ms   (includes loudness normalisation)",
        c.features_hop_n2_ms
    );
    println!(
        "  features, hop n/4            {:7.3} ms",
        c.features_hop_n4_ms
    );
    println!("  YIN f0 track (extra)         {:7.3} ms", c.yin_f0_ms);
    println!(
        "  distance A / B / C           {:7.4} / {:7.4} / {:7.4} ms (mean, cached features)",
        c.distance_a_ms, c.distance_b_ms, c.distance_c_ms
    );
    println!(
        "  one evaluation               {:7.3} ms   = {:.1} evaluations/s on one thread",
        c.evaluation_ms, c.evaluations_per_s
    );

    let workers: Vec<usize> = parse_list(&args.workers)?;
    let batches: Vec<usize> = parse_list(&args.batches)?;
    let analysis = AnalysisCfg {
        sample_rate: render.sample_rate,
        ..AnalysisCfg::default()
    };
    let mut rng = Rng::new(9);
    let cands: Vec<Vec<f64>> = (0..args.evals)
        .map(|_| (0..N_PARAMS).map(|_| rng.uniform()).collect())
        .collect();
    let mut scaling = Vec::new();
    println!(
        "scaling: {} evaluations per row, distance A, work-queue evaluator",
        args.evals
    );
    println!(
        "  {:>7} {:>6} {:>10} {:>8} {:>10} {:>9}  load",
        "workers", "batch", "evals/s", "speedup", "efficiency", "identical"
    );
    for &batch in &batches {
        let mut base_rate = 0.0;
        let mut base_results: Vec<f64> = Vec::new();
        for &w in &workers {
            let mut pe = ParEval::new(w, render, analysis, Dist::A, Scale::DEFAULT);
            pe.set_target(&cands[0]);
            let _ = pe.eval(&cands[..batch.min(cands.len())]); // warm up
            let t = Instant::now();
            let mut all = Vec::with_capacity(cands.len());
            for chunk in cands.chunks(batch) {
                all.extend(pe.eval(chunk));
            }
            let rate = cands.len() as f64 / t.elapsed().as_secs_f64();
            if base_results.is_empty() {
                base_rate = rate;
                base_results.clone_from(&all);
            }
            let identical = all
                .iter()
                .zip(&base_results)
                .all(|(a, b)| a.to_bits() == b.to_bits());
            let row = Scaling {
                workers: w,
                batch,
                evals_per_s: rate,
                speedup: rate / base_rate,
                efficiency: rate / base_rate / w as f64 * workers[0] as f64,
                identical_to_one_worker: identical,
                checksum: all.iter().sum(),
                load: load_avg(),
            };
            println!(
                "  {:>7} {:>6} {:>10.1} {:>7.2}x {:>9.0}% {:>9}  {}   (hash {:016x})",
                row.workers,
                row.batch,
                row.evals_per_s,
                row.speedup,
                100.0 * row.efficiency,
                row.identical_to_one_worker,
                row.load,
                hash_f64(&all)
            );
            scaling.push(row);
        }
    }
    if let Some(path) = &args.json {
        write_json(
            path,
            &Output {
                schema: "enth.spike-metric.bench/1",
                host_threads: host_threads(),
                costs: c,
                scaling,
            },
        )?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
