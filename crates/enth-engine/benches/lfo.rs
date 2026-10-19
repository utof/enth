//! Instruction budgets for the LFO module (P1a lane D2), via gungraun (callgrind).
//!
//! Routes arrive with lane S2b, so the LFO here is an unrouted extra module: the cost measured is its own tick (the
//! phase advance, the shape and, for the voice scope, one core per active voice), not the modulation matrix.
//! Setup is outside the measured function; each case renders 1 s at 48 kHz (1,500 segments) of a held chord.
//! Subtract the `engine` bench with the same voices to read the LFO's own cost (`docs/dsp/lfo.md` records both).
//! Budgets are the measured count plus 10 %.
//!
//! - `voice_sine_*`: a per-voice sine (one `sinf` per voice and segment).
//! - `voice_random_16v`: a per-voice smooth random (no `libm`; a wrap draws from the seeded stream).
//! - `global_sine`: the one global LFO (one `sinf` per segment).
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::catalogue::{LfoParam, LfoScope, LfoShape};
use enth_patch::{ModuleKind, Patch, Section};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn with_lfo(set: &[(LfoParam, f32)]) -> Patch {
    let mut p = common::patch_with(ModuleKind::Lfo);
    let lfo = &mut p.section_mut(Section::Modulators)[1];
    for &(param, v) in set {
        lfo.values[param.index()] = v;
    }
    p
}

fn voice_sine(voices: usize) -> common::Prepared {
    common::prepare(&with_lfo(&[]), voices, 1.0)
}

fn voice_random(voices: usize) -> common::Prepared {
    let p = with_lfo(&[
        (LfoParam::Shape, LfoShape::SmoothRandom.index() as f32),
        (LfoParam::Rate, 20.0),
    ]);
    common::prepare(&p, voices, 1.0)
}

fn global_sine(voices: usize) -> common::Prepared {
    let p = with_lfo(&[(LfoParam::Scope, LfoScope::Global.index() as f32)]);
    common::prepare(&p, voices, 1.0)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-19, 1 s at 48 kHz), plus 10 %, rounded up to
// 1,000. Measured: 4,603,182, 48,928,795, 47,774,635 and 43,795,237 instructions; over the `engine` bench's
// 3,873,015 and 43,370,997 that is 7.2 instructions per sample per voice for a sine, 5.7 for a smooth random, and
// 8.8 per frame (not per voice) for a global sine (docs/dsp/lfo.md).
#[library_benchmark]
#[bench::voice_sine_1v(args = (1,), setup = voice_sine, config = common::budget(5_064_000))]
#[bench::voice_sine_16v(args = (16,), setup = voice_sine, config = common::budget(53_822_000))]
#[bench::voice_random_16v(args = (16,), setup = voice_random, config = common::budget(52_553_000))]
#[bench::global_sine_16v(args = (16,), setup = global_sine, config = common::budget(48_175_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = lfo, benchmarks = [render]);
main!(library_benchmark_groups = lfo);
