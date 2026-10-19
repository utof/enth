//! Instruction budget for the reverb module (P1a lane FXW), via gungraun (callgrind): a held chord of 1 and of 16
//! voices through the reverb at its catalogue defaults, one second at 48 kHz each. Setup (compile, `prepare`, the
//! 1.6 MB of delay lines) is outside the measured function; the reverb never goes idle here, so every segment
//! runs the core. Subtract the matching `engine` bench (same voices, no extra module) to read the module's cost:
//! the wrapper's `sanitize`, `set_params` and the idle bookkeeping on top of the core's own ~2,400 instructions per
//! frame (docs/dsp/reverb.md, "Module wrapper").
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests (`tests/reverb.rs`) stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as
//! the gungraun crate (`cargo xtask bench --bench reverb`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::ModuleKind;
use gungraun::{library_benchmark, library_benchmark_group, main};

fn chord(voices: usize) -> common::Prepared {
    common::prepare(&common::patch_with(ModuleKind::Reverb), voices, 1.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-19: 117,371,741 and 158,321,211 instructions,
// 2,445 and 3,298 per frame in all), plus 10 %. Minus the `engine` bench measured in the same tree (3,984,758 and
// 44,866,745) that is 2,362 and 2,364 instructions per frame for the reverb module: the core and the wrapper's
// per-segment `set_params` (about 100 instructions in all). Lanes D1 and D2 move the engine baseline when they
// replace the reference saw and envelope; the module's own share stays the same.
#[library_benchmark]
#[bench::reverb_1v(args = (1,), setup = chord, config = common::budget(129_200_000))]
#[bench::reverb_16v(args = (16,), setup = chord, config = common::budget(174_200_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = reverb, benchmarks = [render]);
main!(library_benchmark_groups = reverb);
