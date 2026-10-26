//! Instruction budgets for the `mseg` module (S2a lane MSEG), via gungraun (callgrind).
//!
//! The MSEG is an unrouted extra module here, so the cost measured is its own tick (the position step, the shape
//! lookup and the curve, once per voice and 32-frame segment), not the modulation matrix. Setup is outside the
//! measured function; each case renders 1 s at 48 kHz (1,500 segments) of a held chord. Subtract the `engine` bench
//! with the same voices to read the MSEG's own cost (`docs/dsp/mseg.md` records both). Budgets are the measured
//! count plus 10 %.
//!
//! - `voice_loop_*`: the default shape (a three-point triangle, one bar), looping, per voice.
//! - `voice_curves_16v`: sixteen points, each with a bend, so the lookup walks the longest list and every segment has a
//!   division.
//! - `voice_sustain_16v`: a sustain loop held for the whole render (the wrap's integer remainder every lap).
//! - `global_loop_16v`: the one global MSEG, with 16 voices playing.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::catalogue::{MSEG_POINT_BASE, MsegMode, MsegParam, MsegScope};
use enth_patch::{ModuleKind, Patch, Section};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn with_mseg(set: &[(MsegParam, f32)], points: &[(f32, f32, f32)]) -> Patch {
    let mut p = common::patch_with(ModuleKind::Mseg);
    let m = &mut p.section_mut(Section::Modulators)[1];
    for &(param, v) in set {
        m.values[param.index()] = v;
    }
    if !points.is_empty() {
        m.values[MsegParam::Points.index()] = points.len() as f32;
        for (i, &(t, l, c)) in points.iter().enumerate() {
            let base = MSEG_POINT_BASE + 3 * i;
            m.values[base] = t;
            m.values[base + 1] = l;
            m.values[base + 2] = c;
        }
    }
    p
}

fn voice_loop(voices: usize) -> common::Prepared {
    common::prepare(&with_mseg(&[], &[]), voices, 1.0)
}

fn voice_curves(voices: usize) -> common::Prepared {
    let points: Vec<(f32, f32, f32)> = (0..16)
        .map(|i| {
            let curve = [60.0, -45.0, 20.0, -80.0][i % 4];
            (
                i as f32 * 6.25,
                [100.0, 20.0, 70.0, 5.0][(i / 2) % 4],
                curve,
            )
        })
        .collect();
    // Beats are on; the length is the default bar.
    common::prepare(&with_mseg(&[], &points), voices, 1.0)
}

fn voice_sustain(voices: usize) -> common::Prepared {
    let points = [
        (0.0, 0.0, 0.0),
        (10.0, 100.0, 0.0),
        (30.0, 60.0, 0.0),
        (50.0, 80.0, 0.0),
        (100.0, 0.0, 0.0),
    ];
    // 100 ms, so the loop between points 3 and 4 wraps about every 20 ms: 50 laps in the render.
    let set = [
        (MsegParam::Mode, MsegMode::SustainLoop.index() as f32),
        (MsegParam::Sync, 0.0),
        (MsegParam::Length, 100.0),
        (MsegParam::LoopStart, 3.0),
        (MsegParam::LoopEnd, 4.0),
    ];
    common::prepare(&with_mseg(&set, &points), voices, 1.0)
}

fn global_loop(voices: usize) -> common::Prepared {
    let set = [(MsegParam::Scope, MsegScope::Global.index() as f32)];
    common::prepare(&with_mseg(&set, &[]), voices, 1.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-26, 1 s at 48 kHz), plus 10 %, rounded up to
// 1,000. Measured: 4,415,521, 41,951,591, 43,207,231, 42,443,093 and 37,932,307 instructions; over the `engine`
// bench's 3,621,357 (1 voice) and 37,256,482 (16 voices) that is 16.5 instructions per sample for one voice (the
// per-segment fixed part dominates), then 6.1 per sample per voice for the default shape, 7.8 for sixteen bent
// points and 6.8 for a held sustain loop, and 14.1 per frame (not per voice) for the global MSEG (docs/dsp/mseg.md).
#[library_benchmark]
#[bench::voice_loop_1v(args = (1,), setup = voice_loop, config = common::budget(4_858_000))]
#[bench::voice_loop_16v(args = (16,), setup = voice_loop, config = common::budget(46_147_000))]
#[bench::voice_curves_16v(args = (16,), setup = voice_curves, config = common::budget(47_528_000))]
#[bench::voice_sustain_16v(args = (16,), setup = voice_sustain, config = common::budget(46_688_000))]
#[bench::global_loop_16v(args = (16,), setup = global_loop, config = common::budget(41_726_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = mseg, benchmarks = [render]);
main!(library_benchmark_groups = mseg);
