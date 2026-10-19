//! Instruction budget for the compressor module (P1a lane FXW), via gungraun (callgrind): a held chord of 1 and of
//! 16 voices through the compressor at its catalogue defaults, one second at 48 kHz each. Setup is outside the
//! measured function. Subtract the matching `engine` bench (same voices, no extra module) to read the module's
//! cost: the wrapper's `set_params` per segment on top of the core's own ~190 instructions per frame
//! (docs/dsp/compressor.md, "Module wrapper").
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests (`tests/comp.rs`) stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as
//! the gungraun crate (`cargo xtask bench --bench comp`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::ModuleKind;
use gungraun::{library_benchmark, library_benchmark_group, main};

fn chord(voices: usize) -> common::Prepared {
    common::prepare(&common::patch_with(ModuleKind::Comp), voices, 1.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-19: 13,184,583 and 54,146,473 instructions,
// 275 and 1,128 per frame in all), plus 10 %. Minus the `engine` bench measured in the same tree (3,984,758 and
// 44,866,745) that is 192 and 193 instructions per frame for the compressor module. Lanes D1 and D2 move the
// engine baseline when they replace the reference saw and envelope; the module's own share stays the same.
#[library_benchmark]
#[bench::comp_1v(args = (1,), setup = chord, config = common::budget(14_550_000))]
#[bench::comp_16v(args = (16,), setup = chord, config = common::budget(59_600_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = comp, benchmarks = [render]);
main!(library_benchmark_groups = comp);
