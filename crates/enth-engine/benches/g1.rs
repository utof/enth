//! Instruction budget for the G1 reference patch (spine lanes S2b and E): 16 voices, every P1a module, reverb and
//! compressor on, one 2 s render, via gungraun (callgrind).
//!
//! G1's CPU budget (P1a summary): at most 12,000 instructions per frame, re-based to the first product measurement
//! plus 10 %. Placeholder written by lane S0 so that `cargo xtask bench` and the CI budget job run from the start.
//! Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

use std::hint::black_box;

use gungraun::{
    Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main,
};

/// Absolute instruction budget: the measured value plus 10 % (docs/benchmarks.md).
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

library_benchmark_group!(name = g1, benchmarks = [placeholder]);
main!(library_benchmark_groups = g1);
