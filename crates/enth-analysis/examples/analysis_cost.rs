//! Cost of the scalar FFT path against the dispatched one (`docs/dsp/analysis.md`, "Cost").
//!
//! ```text
//! cargo build --release -p enth-analysis --example analysis_cost
//! target/release/examples/analysis_cost                       # wall-clock table, information only
//! valgrind --tool=callgrind target/release/examples/analysis_cost describe scalar 10   # instruction counts
//! ```
//!
//! With arguments it runs one workload `reps` times and prints nothing else, so two callgrind runs with different
//! `reps` give the instructions per call. Wall-clock numbers are information only (AGENTS.md); run them under
//! `flock -x /tmp/enth-quiet.lock` on a quiet machine.

#![allow(
    clippy::disallowed_methods,
    clippy::print_stdout,
    reason = "a benchmark example: reads the clock and the arguments, prints a table"
)]

use enth_analysis::descriptors::{Describer, NoteTiming};
use enth_analysis::features::{FeatureCfg, FeatureExtractor};
use enth_analysis::fft::{FftPath, FftPlanner};
use enth_analysis::test_signals::{SR, ToyPatch, ToyPerf, render_toy};
use realfft::num_complex::Complex32;
use std::hint::black_box;
use std::time::Instant;

fn clip(seconds: f64) -> Vec<f32> {
    let p = ToyPatch {
        detune_ct: 20.0,
        cutoff_hz: 4000.0,
        sustain_db: -6.0,
        ..ToyPatch::default()
    };
    let perf = ToyPerf {
        note_hz: 220.0,
        dur_s: seconds,
        hold_s: seconds - 1.0,
    };
    render_toy(SR, &p, &perf, (0.0, 0.37), 0.0)
}

/// Run `workload` on `path` `reps` times.
fn run(workload: &str, path: FftPath, reps: usize) {
    match workload {
        "fft2048" => {
            let f = FftPlanner::new(path).forward(2048);
            let mut x: Vec<f32> = (0..2048)
                .map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5)
                .collect();
            let mut out = vec![Complex32::new(0.0, 0.0); f.bins()];
            let mut scratch = f.make_scratch();
            for _ in 0..reps {
                f.forward(black_box(&mut x), &mut out, &mut scratch);
                black_box(&out);
            }
        }
        "features" => {
            let x = clip(2.0);
            let mut fx = FeatureExtractor::new(FeatureCfg {
                fft: path,
                ..FeatureCfg::default()
            });
            for _ in 0..reps {
                black_box(fx.extract(black_box(&x)));
            }
        }
        "describe" => {
            let x = clip(3.0);
            let mut d = Describer::with_path(SR, path);
            for _ in 0..reps {
                black_box(d.describe(black_box(&x), NoteTiming::held(2.0)).unwrap());
            }
        }
        _ => panic!("workloads: fft2048, features, describe"),
    }
}

fn path_of(s: &str) -> FftPath {
    match s {
        "scalar" => FftPath::Scalar,
        "dispatched" => FftPath::Dispatched,
        _ => panic!("paths: scalar, dispatched"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 {
        run(&args[1], path_of(&args[2]), args[3].parse().expect("reps"));
        return;
    }
    println!("workload            | scalar        | dispatched    | scalar / dispatched");
    for (workload, reps, unit, scale) in [
        ("fft2048", 20_000, "us", 1e6),
        ("features", 40, "ms", 1e3),
        ("describe", 40, "ms", 1e3),
    ] {
        let mut t = [0.0f64; 2];
        for (i, path) in [FftPath::Scalar, FftPath::Dispatched]
            .into_iter()
            .enumerate()
        {
            run(workload, path, reps / 10); // warm up: plans, caches
            let mut best = f64::INFINITY;
            for _ in 0..5 {
                let start = Instant::now();
                run(workload, path, reps);
                best = best.min(start.elapsed().as_secs_f64() / reps as f64);
            }
            t[i] = best * scale;
        }
        println!(
            "{:19} | {:9.3} {unit:2} | {:9.3} {unit:2} | {:.2}x",
            match workload {
                "fft2048" => "one real FFT, 2048",
                "features" => "features, 2 s clip",
                _ => "describe, 3 s clip",
            },
            t[0],
            t[1],
            t[0] / t[1]
        );
    }
}
