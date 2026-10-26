//! Instruction budget for the `ott` module (S2a lane OTT), via gungraun (callgrind): a held chord of 1 and of 16
//! voices through the effect at its catalogue defaults, at full depth and at `mix` 0, one second at 48 kHz each. Setup
//! is outside the measured function. Subtract the matching `engine` bench (same voices, no extra module) to read the
//! module's own cost, recorded with the budgets in `docs/dsp/ott.md` ("Cost").
//!
//! The module's cost per frame does not depend on the settings (all three bands, both curves and the mix branch run
//! every frame, a `mix` of 0 included, so that the state is the same whether or not the engine skips an idle module);
//! the three variants measure that. Instruction counts are exact and independent of runner load; they under-report
//! subnormal penalties, so the subnormal tests (`tests/ott.rs`, `enth-dsp/tests/ott.rs`) stay mandatory. Needs
//! valgrind and a `gungraun-runner` of the same version as the gungraun crate (`cargo xtask bench --bench ott`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::ModuleKind;
use enth_patch::catalogue::OttParam;
use gungraun::{library_benchmark, library_benchmark_group, main};

fn chord(voices: usize) -> common::Prepared {
    common::prepare(&common::patch_with(ModuleKind::Ott), voices, 1.0)
}

fn chord_with(voices: usize, param: OttParam, value: f32) -> common::Prepared {
    let mut p = common::patch_with(ModuleKind::Ott);
    p.fx.last_mut().expect("the ott").values[param.index()] = value;
    common::prepare(&p, voices, 1.0)
}

fn full_depth(voices: usize) -> common::Prepared {
    chord_with(voices, OttParam::Depth, 100.0)
}

fn mix_zero(voices: usize) -> common::Prepared {
    chord_with(voices, OttParam::Mix, 0.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-26, main 704f41d plus this lane): 50,492,504 and
// 83,802,537 instructions for 1 and 16 voices at the defaults (1,052 and 1,746 per frame in all), 83,809,734 at full
// depth and 82,746,537 at `mix` 0, each plus 10 %, rounded up to 1,000. Minus the `engine` bench measured in the same
// tree (3,610,682 and 37,290,522) that is 977 and 969 instructions per frame for the module (947 at `mix` 0: the
// branch skips the blend), the same at any depth.
#[library_benchmark]
#[bench::ott_1v(args = (1,), setup = chord, config = common::budget(55_542_000))]
#[bench::ott_16v(args = (16,), setup = chord, config = common::budget(92_183_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

#[library_benchmark]
#[bench::ott_full_depth_16v(args = (16,), setup = full_depth, config = common::budget(92_191_000))]
fn render_full_depth(p: common::Prepared) -> f32 {
    common::render(p)
}

#[library_benchmark]
#[bench::ott_mix_zero_16v(args = (16,), setup = mix_zero, config = common::budget(91_022_000))]
fn render_mix_zero(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(
    name = ott,
    benchmarks = [render, render_full_depth, render_mix_zero]
);
main!(library_benchmark_groups = ott);
