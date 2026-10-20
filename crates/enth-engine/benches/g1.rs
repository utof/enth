//! Instruction budget for the G1 reference patch (spine lanes S2b, then E): 16 voices, every P1a module kind in use
//! with modulation routes from every kind of source, compressor and reverb on, one 2 s render of a held 16-note
//! chord in 512-frame host calls, via gungraun (callgrind).
//!
//! G1's CPU budget (roadmap.md, P1a summary): at most 12,000 instructions per stereo frame on the scalar path, with the
//! hard limit re-based to the first full product measurement plus 10 %. Lane E did that on 2026-10-19 (the numbers
//! are in the comment at the budget below), so the hard limit is now `G1_BUDGET_IR`, not the 12,000 cap. A module
//! change that adds more than that margin fails this bench; re-base it deliberately, with a new measurement in the
//! comment and in docs/dsp/engine.md, and never above the cap.
//! Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate (`cargo xtask bench --bench g1`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

#[path = "common/g1.rs"]
mod g1_patch;

use g1_patch::g1_patch;
use gungraun::{library_benchmark, library_benchmark_group, main};

/// Frames per G1 render: 2 s at 48 kHz.
const FRAMES: u64 = 96_000;

/// G1's cap: 12,000 instructions per stereo frame.
const G1_PER_FRAME: u64 = 12_000;

/// The hard limit: the first full product measurement (590,803,521) plus 10 %, rounded up to a round number.
const G1_BUDGET_IR: u64 = 650_000_000;

// The re-based budget must stay inside G1's cap (12,000 per frame = 1,152,000,000 per render).
const _: () = assert!(G1_BUDGET_IR <= FRAMES * G1_PER_FRAME);

fn g1(voices: usize) -> common::Prepared {
    common::prepare(&g1_patch(), voices, FRAMES as f32 / common::SR)
}

// Budget: the first full product measurement plus 10 %, 650,000,000 instructions per 2 s render (6,771 per frame),
// within G1's 12,000 per frame (1,152,000,000).
// Measured on x86_64 Linux (Intel i7-9750H, valgrind/callgrind via gungraun 0.20.0) with the pinned toolchain:
// - S2b, 2026-10-19 (stub osc/noise/svf/lfo/reverb/comp, the reference envelope): 122,452,870 (1,276 per frame).
// - E, 2026-10-19 (c7f13bb, all real modules: D1 oscillators and noise, D2 svf/adsr/lfo, FXW reverb and compressor):
//   590,803,521 (6,154 per frame), L1 hits 759,464,564, estimated cycles 777,955,924.
#[library_benchmark]
#[bench::g1_16v(args = (16,), setup = g1, config = common::budget(G1_BUDGET_IR))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = g1, benchmarks = [render]);
main!(library_benchmark_groups = g1);
