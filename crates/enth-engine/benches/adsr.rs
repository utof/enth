//! Instruction budget for the ADSR envelope module (P1a lane D2), via gungraun (callgrind).
//!
//! Placeholder written by lane S0 so that `cargo xtask bench` and the CI budget job run from the start. The lane
//! replaces `placeholder` with a render of its module (setup outside the measured function, the way
//! `spikes/spike-engine/benches/instr.rs` does) and sets `hard_limits` to the measured count plus 10 %.
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

use std::hint::black_box;

use gungraun::{
    Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main,
};

/// Absolute instruction budget: the measured value plus 10 % (docs/benchmarks.md). Absolute limits need no stored
/// baseline, so they work in CI.
fn budget(ir: u64) -> LibraryBenchmarkConfig {
    LibraryBenchmarkConfig::default()
        .tool(Callgrind::default().hard_limits([(EventKind::Ir, ir)]))
        .clone()
}

#[library_benchmark]
#[bench::placeholder(args = (1_u32,), config = budget(10_000))]
fn placeholder(x: u32) -> u32 {
    black_box(x).wrapping_add(1)
}

library_benchmark_group!(name = adsr, benchmarks = [placeholder]);
main!(library_benchmark_groups = adsr);
