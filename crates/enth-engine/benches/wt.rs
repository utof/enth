//! Instruction budgets for the `wt` module (S2a lane WT), via gungraun (callgrind).
//!
//! Setup (building the patch, preparing the engine, queueing the chord) is outside the measured function. Each case
//! renders 1 s at 48 kHz (1,500 segments) with the held chord of `common::prepare`. The engine skeleton's own cost
//! (segment loop, FIFO, voice sum, pan, the default `adsr` and `osc`) is in the number too: subtract the `engine`
//! bench with the same voices to read the module's own cost (`docs/dsp/wt.md` records both).
//!
//! - `plain_*`: the default `wt` (the sine table, no modulation, feedback or warp): the table read alone.
//! - `pm_*`: the same with phase modulation from the patch's `osc1` through an audio route (the reader's `pm` path:
//!   the routed block, the per-sample offset, the pm slope tracking), the case the brief budgets.
//! - `full_*`: the worst case: the formant table moving between frames, a warp, PM from `osc1`, AM from `osc1` and
//!   50 % feedback, every optional path on at once.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::catalogue::{Warp, WtParam, WtTable};
use enth_patch::{ModuleKind, Patch, Route};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn route(src: &str, dst: &str) -> Route {
    Route {
        src: src.parse().expect("route source"),
        dst: dst.parse().expect("route destination"),
        depth: 100.0,
    }
}

fn plain(voices: usize) -> common::Prepared {
    common::prepare(&common::patch_with(ModuleKind::Wt), voices, 1.0)
}

fn pm_patch() -> Patch {
    let mut p = common::patch_with(ModuleKind::Wt);
    p.routes.push(route("osc1", "wt2.pm"));
    p
}

fn pm(voices: usize) -> common::Prepared {
    common::prepare(&pm_patch(), voices, 1.0)
}

fn full_patch() -> Patch {
    let mut p = pm_patch();
    p.routes.push(route("osc1", "wt2.am"));
    let v = &mut p.generators[1].values;
    v[WtParam::Table.index()] = WtTable::Formant.index() as f32;
    v[WtParam::Position.index()] = 40.0;
    v[WtParam::Warp.index()] = Warp::Bend.index() as f32;
    v[WtParam::WarpAmount.index()] = 60.0;
    v[WtParam::Feedback.index()] = 50.0;
    p
}

fn full(voices: usize) -> common::Prepared {
    common::prepare(&full_patch(), voices, 1.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-26, 1 s at 48 kHz), plus 10 %, rounded up to
// 1,000. Measured: 8,552,995 (plain_1v), 112,263,005 (plain_16v), 121,614,216 (pm_16v) and 170,628,380 (full_16v);
// over the `engine` bench's 3,610,682 and 37,290,522 that is 103 and 97.6 instructions per sample and voice for a
// plain wt, +12.2 for PM from another generator and +76.0 for everything on at once (docs/dsp/wt.md). The table read
// avoids `floor` (a library call on baseline x86-64) when the read phase is within a cycle of [0, 1): 135,134,973 ->
// 112,263,005 for plain_16v.
#[library_benchmark]
#[bench::plain_1v(args = (1,), setup = plain, config = common::budget(9_409_000))]
#[bench::plain_16v(args = (16,), setup = plain, config = common::budget(123_490_000))]
#[bench::pm_16v(args = (16,), setup = pm, config = common::budget(133_776_000))]
#[bench::full_16v(args = (16,), setup = full, config = common::budget(187_692_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = wt, benchmarks = [render]);
main!(library_benchmark_groups = wt);
