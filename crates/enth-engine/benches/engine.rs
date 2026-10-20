//! Instruction budget for the engine skeleton (P1a lane S2a), via gungraun (callgrind): the segment loop, the FIFO,
//! the parameter tables, the voice sum and the pan, with the reference saw and envelope. Module lanes subtract these
//! numbers from their own benches (same voices) to read a module's cost; see `benches/common/mod.rs`.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate
//! (`cargo xtask bench --bench engine`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use gungraun::{library_benchmark, library_benchmark_group, main};

fn saw(voices: usize) -> common::Prepared {
    common::prepare(&common::base_patch(), voices, 1.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain, plus 10 %, rounded up to 1,000. 1 s at 48 kHz per case.
// Every patch holds an `osc` and an `adsr`, so a lane that changes either re-measures these two budgets in the same
// commit. With D1's Plaits saw, D2's ADSR and S2b's parameter pass merged (2026-10-19): 3,288,125 and 34,919,329
// instructions (69 and 727 per frame).
#[library_benchmark]
#[bench::saw_1v(args = (1,), setup = saw, config = common::budget(3_898_000))]
#[bench::saw_16v(args = (16,), setup = saw, config = common::budget(40_573_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = engine, benchmarks = [render]);
main!(library_benchmark_groups = engine);
