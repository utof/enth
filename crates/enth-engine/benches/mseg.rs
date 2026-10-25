//! Instruction budget for the `mseg` module (S2a lane MSEG), via gungraun (callgrind).
//!
//! Placeholder written by lane S0d (freeze F0m, docs/phase-s2.md) so that `cargo xtask bench` runs it from the start
//! and `enth-engine/Cargo.toml` is not a merge hotspot. Lane MSEG replaces `placeholder` with renders of its module
//! through `common::prepare(&common::patch_with(ModuleKind::Mseg), voices, 1.0)` (setup outside the measured function;
//! see `benches/svf.rs`), subtracts the matching `engine` bench to read the module's own cost, records both in
//! `docs/dsp/mseg.md`, and sets the budget to the measured count plus 10 %. Instruction counts are exact and
//! independent of runner load; they under-report subnormal penalties, so the subnormal tests stay mandatory. Needs
//! valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

#[library_benchmark]
#[bench::placeholder(args = (1_u32,), config = common::budget(10_000))]
fn placeholder(x: u32) -> u32 {
    black_box(x).wrapping_add(1)
}

library_benchmark_group!(name = mseg, benchmarks = [placeholder]);
main!(library_benchmark_groups = mseg);
