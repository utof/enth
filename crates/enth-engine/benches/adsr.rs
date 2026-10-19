//! Instruction budgets for the ADSR envelope module (P1a lane D2), via gungraun (callgrind).
//!
//! Every patch holds an `adsr` (the amplitude envelope), so the `engine` bench already includes one per voice; this
//! bench adds a second, unrouted envelope (`env2`) per voice, which is the cost of one more envelope. Setup is
//! outside the measured function; each case renders 1 s at 48 kHz (1,500 segments) of a held chord. Subtract the
//! `engine` bench with the same voices to read the second envelope's own cost (`docs/dsp/envelope.md` records both).
//! Budgets are the measured count plus 10 %.
//!
//! - `held_*`: the default envelope, all voices in sustain (a branch and a copy per voice and segment).
//! - `release_16v`: every key released after 0.1 s with a 400 ms release, then the tail: the exponential segment
//!   (a multiply and an add per voice and segment) and the segment starts (three `libm` calls).
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::ModuleKind;
use gungraun::{library_benchmark, library_benchmark_group, main};

fn held(voices: usize) -> common::Prepared {
    common::prepare(&common::patch_with(ModuleKind::Adsr), voices, 1.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-19, 1 s at 48 kHz), plus 10 %, rounded up to
// 1,000: 4,301,515 and 47,071,463 instructions. The patch is the `engine` bench's (3,873,015 and 43,370,997, one
// ADSR already in it) plus a second ADSR, so the second one costs 428,500 and 3,700,466: 4.8 instructions per sample
// per voice at 16 voices (docs/dsp/envelope.md).
#[library_benchmark]
#[bench::held_1v(args = (1,), setup = held, config = common::budget(4_732_000))]
#[bench::held_16v(args = (16,), setup = held, config = common::budget(51_779_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = adsr, benchmarks = [render]);
main!(library_benchmark_groups = adsr);
