//! Instruction budget for the noise module (P1a lane D1), via gungraun (callgrind): the engine skeleton of
//! `benches/engine.rs` (one reference saw and the envelope) plus a `noise2`, white and pink, so a case minus the
//! matching `engine` case is the cost of one `noise`. 1 s at 48 kHz per case.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate
//! (`cargo xtask bench --bench noise`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::ModuleKind;
use enth_patch::catalogue::{NoiseColor, NoiseParam};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn setup(color: NoiseColor, voices: usize) -> common::Prepared {
    let mut p = common::patch_with(ModuleKind::Noise);
    let noise = p
        .generators
        .iter_mut()
        .find(|m| m.kind == ModuleKind::Noise)
        .expect("patch_with added a noise module");
    noise.values[NoiseParam::Color.index()] = color.index() as f32;
    common::prepare(&p, voices, 1.0)
}

fn white_1v() -> common::Prepared {
    setup(NoiseColor::White, 1)
}
fn white() -> common::Prepared {
    setup(NoiseColor::White, 16)
}
fn pink() -> common::Prepared {
    setup(NoiseColor::Pink, 16)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (see docs/dsp/noise.md), plus 10 %.
#[library_benchmark]
#[bench::white_1v(setup = white_1v, config = common::budget(5_710_000))]
#[bench::white_16v(setup = white, config = common::budget(65_660_000))]
#[bench::pink_16v(setup = pink, config = common::budget(96_400_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = noise, benchmarks = [render]);
main!(library_benchmark_groups = noise);
