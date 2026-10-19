//! Speed driver. `engine-speed <template> <path> <reps> [stats]`
//!
//! templates: `simple1`, `simple16` (osc -> filter -> amp), `p1` (3 osc, 2 env, LFO, 8 voices, FDN reverb)
//! paths: `scalar`, `x4`, `x8`; each rep renders 2 s of audio. Without `stats` it only renders (for hyperfine and
//! valgrind: instructions per render = count(reps 2) - count(reps 1)); with `stats` it prints x-realtime min/median.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "command-line driver"
)]

use std::time::Instant;

use spike_engine::render::{frames, hash_samples, held_chord};
use spike_engine::{Engine, Flush, Patch, Path, SAMPLE_RATE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let template = args.get(1).map_or("simple1", String::as_str);
    let path = match args.get(2).map_or("scalar", String::as_str) {
        "scalar" => Path::Scalar,
        "x4" => Path::X4,
        "x8" => Path::X8,
        other => {
            eprintln!("unknown path {other:?}");
            std::process::exit(2);
        }
    };
    let reps: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    let stats = args.get(4).is_some_and(|s| s == "stats");
    let (patch, voices) = match template {
        "simple1" => (Patch::simple(), 1),
        "simple16" => (Patch::simple(), 16),
        "p1" => (Patch::p1_like(), 8),
        other => {
            eprintln!("unknown template {other:?}");
            std::process::exit(2);
        }
    };
    let seconds = 2.0;
    let notes = held_chord(voices, seconds);
    let n = frames(seconds);
    let mut out = vec![0.0f32; n];
    let mut times = Vec::with_capacity(reps);
    for _ in 0..reps {
        // Engine::new (allocation, libm set-up) is included in the timing on purpose: it is part of a render.
        let t0 = Instant::now();
        let mut engine = Engine::new(&patch, &notes, 1, path, Flush::On);
        for block in out.chunks_mut(spike_engine::BLOCK) {
            engine.process(block);
        }
        times.push(t0.elapsed().as_secs_f64());
    }
    println!("{}", hash_samples(&out));
    if stats {
        times.sort_by(f64::total_cmp);
        let audio = f64::from(SAMPLE_RATE).recip() * n as f64;
        let min = times[0];
        let median = times[times.len() / 2];
        println!(
            "{template} {path:?} reps={reps}: min {:.3} ms = {:.0}x realtime, median {:.3} ms = {:.0}x realtime",
            min * 1e3,
            audio / min,
            median * 1e3,
            audio / median
        );
    }
}
