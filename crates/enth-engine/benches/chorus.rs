//! Instruction budgets for the `chorus` module (S2a lane MOD), via gungraun (callgrind).
//!
//! Setup (building the patch, preparing the engine, queueing the chord) is outside the measured function. Each case
//! renders 1 s at 48 kHz (1,500 segments) of the held saw chord through one `chorus`. The engine skeleton's own cost
//! (segment loop, FIFO, voice sum, pan, the default `adsr` and `osc`) is in the number too: subtract the `engine`
//! bench with the same voices to read the module's own cost (`docs/dsp/chorus.md` records both). Budgets are the
//! measured count plus 10 %. The effect is global, so its cost does not depend on the voice count; the 16-voice case
//! only checks that.
//!
//! - `default_*`: the catalogue defaults (chorus mode, 0.5 Hz, depth 50 %, 7 ms, width 100 %, mix 100 %), static
//!   parameters (the per-segment work is the sweep step and the control ramps);
//! - `ensemble_1v`: three taps per channel on two sweeps, the most expensive mode;
//! - `flanger_1v`: the flanger at 2 ms with feedback 70 %, the zero-delay-feedback solve on every tap;
//! - `mix0_1v`: `mix` 0, the bypass: the input passes bit for bit and the module costs nothing beyond the call.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties and a memory
//! copy, so the subnormal tests (`tests/chorus.rs`) and the move-and-copy unit test stay mandatory. Needs valgrind and
//! a `gungraun-runner` of the same version as the gungraun crate (`cargo xtask bench --bench chorus`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::catalogue::{ChorusMode, ChorusParam};
use enth_patch::{ModuleKind, Patch};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn patch(set: &[(ChorusParam, f32)]) -> Patch {
    let kind = ModuleKind::Chorus;
    let mut p = common::patch_with(kind);
    for &(param, v) in set {
        p.section_mut(kind.section())[0].values[param.index()] = v;
    }
    p
}

fn default(voices: usize) -> common::Prepared {
    common::prepare(&patch(&[]), voices, 1.0)
}

fn ensemble(voices: usize) -> common::Prepared {
    let p = patch(&[(ChorusParam::Mode, ChorusMode::Ensemble.index() as f32)]);
    common::prepare(&p, voices, 1.0)
}

fn flanger(voices: usize) -> common::Prepared {
    let p = patch(&[
        (ChorusParam::Mode, ChorusMode::Flanger.index() as f32),
        (ChorusParam::Delay, 2.0),
        (ChorusParam::Feedback, 70.0),
    ]);
    common::prepare(&p, voices, 1.0)
}

fn mixzero(voices: usize) -> common::Prepared {
    let p = patch(&[(ChorusParam::Mix, 0.0)]);
    common::prepare(&p, voices, 1.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-26), plus 10 %, rounded up to 1,000. 1 s at
// 48 kHz per case. Measured instructions (budget in the attribute) and the module's own cost, with the `engine`
// bench's 3,616,649 (1 voice) and 37,296,489 (16 voices) subtracted, per frame (the effect is global, so one frame is
// one stereo sample pair whatever the voice count):
//   default_1v   18,379,246   307.6   (the 16-voice case has the same 14,762,597 of its own)
//   default_16v  52,059,086   307.6
//   ensemble_1v  49,678,246   959.6
//   flanger_1v   17,398,246   287.1
//   mix0_1v      17,386,246   286.9   (the core keeps running at mix 0 so its line stays warm; only the blend is off)
// Every patch holds an `osc` and an `adsr`, so a lane that changes either (or the engine skeleton) re-measures these
// budgets with the `engine` ones; a change to `enth-dsp::chorus` re-measures all five in the same commit.
#[library_benchmark]
#[bench::default_1v(args = (1,), setup = default, config = common::budget(20_218_000))]
#[bench::default_16v(args = (16,), setup = default, config = common::budget(57_265_000))]
#[bench::ensemble_1v(args = (1,), setup = ensemble, config = common::budget(54_647_000))]
#[bench::flanger_1v(args = (1,), setup = flanger, config = common::budget(19_139_000))]
#[bench::mix0_1v(args = (1,), setup = mixzero, config = common::budget(19_125_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = chorus, benchmarks = [render]);
main!(library_benchmark_groups = chorus);
