//! Instruction budgets for the `dist` module (S2a lane DST), via gungraun (callgrind).
//!
//! Setup (building the patch, preparing the engine, queueing the chord) is outside the measured function. Each case
//! renders 1 s at 48 kHz (1,500 segments) of the held saw chord through one `dist`. The engine skeleton's own cost
//! (segment loop, FIFO, voice sum, pan, the default `adsr` and `osc`) is in the number too: subtract the `engine`
//! bench with the same voices to read the module's own cost (`docs/dsp/dist.md` records both). Budgets are the
//! measured count plus 10 %.
//!
//! - `default_*`: the catalogue defaults (soft clip, 6 dB, oversampling x2, tone open, no split, mix 100 %), static
//!   parameters (the controls are cached, no `exp2` or `tan` after the first segment).
//! - `x4_*`: the same at 4x oversampling, the most expensive setting of the clips and folds.
//! - `off_*`: oversampling off, ADAA-1 alone.
//! - `mix0_*`: `mix` 0, the bypass: the voice is untouched and the module costs nothing beyond the call.
//! - `full_*`: every stage in play: x4, a sine fold at 24 dB with bias, `keep_lows` 200 Hz, tone 6 kHz, mix 70 %.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::catalogue::{DistMode, DistParam, Oversample};
use enth_patch::{ModuleKind, Patch, Section};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn patch(set: &[(DistParam, f32)]) -> Patch {
    let mut p = common::patch_with(ModuleKind::Dist);
    for &(param, v) in set {
        p.section_mut(Section::VoiceFx)[0].values[param.index()] = v;
    }
    p
}

fn default(voices: usize) -> common::Prepared {
    common::prepare(&patch(&[]), voices, 1.0)
}

fn x4(voices: usize) -> common::Prepared {
    let p = patch(&[(DistParam::Oversample, Oversample::X4.index() as f32)]);
    common::prepare(&p, voices, 1.0)
}

fn off(voices: usize) -> common::Prepared {
    let p = patch(&[(DistParam::Oversample, Oversample::Off.index() as f32)]);
    common::prepare(&p, voices, 1.0)
}

fn mixzero(voices: usize) -> common::Prepared {
    let p = patch(&[(DistParam::Mix, 0.0)]);
    common::prepare(&p, voices, 1.0)
}

fn full(voices: usize) -> common::Prepared {
    let p = patch(&[
        (DistParam::Oversample, Oversample::X4.index() as f32),
        (DistParam::Mode, DistMode::SineFold.index() as f32),
        (DistParam::Drive, 24.0),
        (DistParam::Bias, 20.0),
        (DistParam::KeepLows, 200.0),
        (DistParam::Tone, 6_000.0),
        (DistParam::Mix, 70.0),
    ]);
    common::prepare(&p, voices, 1.0)
}

#[library_benchmark]
#[bench::default_1v(args = (1,), setup = default, config = common::budget(u64::MAX))]
#[bench::default_16v(args = (16,), setup = default, config = common::budget(u64::MAX))]
#[bench::x4_16v(args = (16,), setup = x4, config = common::budget(u64::MAX))]
#[bench::off_16v(args = (16,), setup = off, config = common::budget(u64::MAX))]
#[bench::mix0_16v(args = (16,), setup = mixzero, config = common::budget(u64::MAX))]
#[bench::full_16v(args = (16,), setup = full, config = common::budget(u64::MAX))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = dist, benchmarks = [render]);
main!(library_benchmark_groups = dist);
