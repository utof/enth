//! Instruction counts (callgrind via gungraun): the stable regression gate for the engine.
//! Wall-clock numbers jitter 15-35% on a shared machine; instruction counts do not.
//! They under-report denormal penalties about 18x, so the subnormal tests (`tests/subnormals.rs`) stay mandatory.
//!
//! Run (needs valgrind and a `gungraun-runner` of the same version as the `gungraun` crate):
//!   GUNGRAUN_RUNNER=/path/to/gungraun-runner cargo bench -p spike-engine --bench instr
#![allow(missing_docs, reason = "benchmark harness")]

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};
use spike_engine::render::{frames, held_chord};
use spike_engine::{BLOCK, Engine, Flush, Patch, Path};

fn setup(template: &str, voices: usize, path: Path) -> (Engine, Vec<f32>) {
    let patch = if template == "simple" {
        Patch::simple()
    } else {
        Patch::p1_like()
    };
    let notes = held_chord(voices, 2.0);
    (
        Engine::new(&patch, &notes, 1, path, Flush::On),
        vec![0.0f32; frames(2.0)],
    )
}

fn render_2s((mut engine, mut out): (Engine, Vec<f32>)) -> f32 {
    for block in out.chunks_mut(BLOCK) {
        engine.process(black_box(block));
    }
    black_box(out[1000])
}

#[library_benchmark]
#[bench::simple_1v_scalar(args = ("simple", 1, Path::Scalar), setup = setup)]
#[bench::simple_16v_scalar(args = ("simple", 16, Path::Scalar), setup = setup)]
#[bench::simple_16v_x4(args = ("simple", 16, Path::X4), setup = setup)]
#[bench::simple_16v_x8(args = ("simple", 16, Path::X8), setup = setup)]
#[bench::p1_8v_scalar(args = ("p1", 8, Path::Scalar), setup = setup)]
#[bench::p1_8v_x8(args = ("p1", 8, Path::X8), setup = setup)]
fn render(input: (Engine, Vec<f32>)) -> f32 {
    render_2s(input)
}

library_benchmark_group!(name = engine, benchmarks = [render]);
main!(library_benchmark_groups = engine);
