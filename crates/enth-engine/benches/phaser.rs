//! Instruction budgets for the `phaser` module (S2a lane MOD), via gungraun (callgrind).
//!
//! Setup (building the patch, preparing the engine, queueing the chord) is outside the measured function. Each case
//! renders 1 s at 48 kHz (1,500 segments) of the held saw chord through one `phaser`. The engine skeleton's own cost
//! (segment loop, FIFO, voice sum, pan, the default `adsr` and `osc`) is in the number too: subtract the `engine`
//! bench with the same voices to read the module's own cost (`docs/dsp/phaser.md` records both). Budgets are the
//! measured count plus 10 %. The effect is global, so its cost does not depend on the voice count; the 16-voice case
//! only checks that.
//!
//! - `default_*`: the catalogue defaults (6 stages, 0.3 Hz, depth 50 %, centre 1 kHz, feedback 30 %, width 50 %,
//!   mix 100 %); the per-segment work is the sweep step, two `exp2` and two `tan` for the corner of each channel;
//! - `stages2_1v` and `stages12_1v`: the shortest and the longest chain, the cost per stage;
//! - `mix0_1v`: `mix` 0, the bypass: the input passes bit for bit and the module costs nothing beyond the call.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests (`tests/phaser.rs`) stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as
//! the gungraun crate (`cargo xtask bench --bench phaser`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::catalogue::PhaserParam;
use enth_patch::{ModuleKind, Patch};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn patch(set: &[(PhaserParam, f32)]) -> Patch {
    let kind = ModuleKind::Phaser;
    let mut p = common::patch_with(kind);
    for &(param, v) in set {
        p.section_mut(kind.section())[0].values[param.index()] = v;
    }
    p
}

fn default(voices: usize) -> common::Prepared {
    common::prepare(&patch(&[]), voices, 1.0)
}

fn stages2(voices: usize) -> common::Prepared {
    common::prepare(&patch(&[(PhaserParam::Stages, 2.0)]), voices, 1.0)
}

fn stages12(voices: usize) -> common::Prepared {
    common::prepare(&patch(&[(PhaserParam::Stages, 12.0)]), voices, 1.0)
}

fn mixzero(voices: usize) -> common::Prepared {
    common::prepare(&patch(&[(PhaserParam::Mix, 0.0)]), voices, 1.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-26), plus 10 %, rounded up to 1,000. 1 s at
// 48 kHz per case. Measured instructions (budget in the attribute) and the module's own cost, with the `engine`
// bench's 3,616,649 (1 voice) and 37,296,489 (16 voices) subtracted, per frame (the effect is global, so one frame is
// one stereo sample pair whatever the voice count):
//   default_1v   20,794,530   357.9   (6 stages)
//   default_16v  54,474,370   357.9
//   stages2_1v   12,643,530   188.1
//   stages12_1v  32,727,030   606.5   (so 42 per stage per frame, 21 per stage per channel sample, and 104 for the rest)
//   mix0_1v      20,026,529   341.9   (the chain keeps running at mix 0 so its state stays warm; only the blend is off)
// Every patch holds an `osc` and an `adsr`, so a lane that changes either (or the engine skeleton) re-measures these
// budgets with the `engine` ones; a change to `enth-dsp::phaser` re-measures all five in the same commit.
#[library_benchmark]
#[bench::default_1v(args = (1,), setup = default, config = common::budget(22_874_000))]
#[bench::default_16v(args = (16,), setup = default, config = common::budget(59_922_000))]
#[bench::stages2_1v(args = (1,), setup = stages2, config = common::budget(13_908_000))]
#[bench::stages12_1v(args = (1,), setup = stages12, config = common::budget(36_000_000))]
#[bench::mix0_1v(args = (1,), setup = mixzero, config = common::budget(22_030_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = phaser, benchmarks = [render]);
main!(library_benchmark_groups = phaser);
