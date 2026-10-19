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
// Every patch holds an `osc` and an `adsr`, so lanes D1 and D2 re-measure these two budgets when they replace the
// reference saw and envelope, in the same commit. Lane D2 (real ADSR, S2a's reference saw still in place,
// 2026-10-19): 3,873,015 (81 per frame) and 43,370,997 (904 per frame). S2a's reference envelope measured
// 3,660,346 and 40,367,014 at its own commit; the same commit measures 3,984,758 and 44,866,745 on today's machine, so
// the S2a budget of 44.4 M for 16 voices no longer held at this base. Lane D1 re-measures them with the real
// oscillator.
#[library_benchmark]
#[bench::saw_1v(args = (1,), setup = saw, config = common::budget(4_261_000))]
#[bench::saw_16v(args = (16,), setup = saw, config = common::budget(47_709_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = engine, benchmarks = [render]);
main!(library_benchmark_groups = engine);
