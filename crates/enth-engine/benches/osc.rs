//! Instruction budget for the oscillator module (P1a lane D1), via gungraun (callgrind): the engine skeleton of
//! `benches/engine.rs` (one reference saw `osc1` and the envelope) plus a second `osc2` in each of its modes, so a
//! case minus the matching `engine` case is the cost of one `osc` of that kind. 1 s at 48 kHz per case.
//!
//! The cases: the three polyBLEP/BLAMP waves (saw, triangle, pulse), the sine, the wavetable (formant) between two
//! frames (position 40 %) and on a frame (position 0 %: one Hermite read instead of two), and hard sync (a synced saw behind the silent-bus master `osc1`). Every case also pays for `osc1` (a saw).
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate
//! (`cargo xtask bench --bench osc`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::catalogue::{OscParam, Wave, Wavetable};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind};
use gungraun::{library_benchmark, library_benchmark_group, main};

/// Which `osc2` the case plays.
#[derive(Clone, Copy)]
enum Mode {
    Saw,
    Triangle,
    Pulse,
    Sine,
    Table,
    TableOnFrame,
    SyncedSaw,
}

fn setup(mode: Mode, voices: usize) -> common::Prepared {
    let mut p = common::base_patch();
    let mut osc = ModuleInstance::new(ModuleId::new("osc2").expect("valid id"), ModuleKind::Osc);
    let v = &mut osc.values;
    let wave = match mode {
        Mode::Saw | Mode::SyncedSaw => Wave::Saw,
        Mode::Triangle => Wave::Triangle,
        Mode::Pulse => Wave::Pulse,
        Mode::Sine => Wave::Sine,
        Mode::Table | Mode::TableOnFrame => Wave::Table,
    };
    v[OscParam::Wave.index()] = wave.index() as f32;
    v[OscParam::Table.index()] = Wavetable::Formant.index() as f32;
    v[OscParam::Position.index()] = if matches!(mode, Mode::TableOnFrame) {
        0.0
    } else {
        40.0
    };
    v[OscParam::Pw.index()] = 30.0;
    v[OscParam::Sync.index()] = f32::from(matches!(mode, Mode::SyncedSaw));
    v[OscParam::Semi.index()] = if matches!(mode, Mode::SyncedSaw) {
        15.0
    } else {
        0.0
    };
    p.generators.push(osc);
    common::prepare(&p, voices, 1.0)
}

fn saw_1v() -> common::Prepared {
    setup(Mode::Saw, 1)
}
fn saw() -> common::Prepared {
    setup(Mode::Saw, 16)
}
fn triangle() -> common::Prepared {
    setup(Mode::Triangle, 16)
}
fn pulse() -> common::Prepared {
    setup(Mode::Pulse, 16)
}
fn sine() -> common::Prepared {
    setup(Mode::Sine, 16)
}
fn table() -> common::Prepared {
    setup(Mode::Table, 16)
}
fn table_on_frame() -> common::Prepared {
    setup(Mode::TableOnFrame, 16)
}
fn synced_saw() -> common::Prepared {
    setup(Mode::SyncedSaw, 16)
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (see docs/dsp/osc.md), plus 10 %.
#[library_benchmark]
#[bench::saw_1v(setup = saw_1v, config = common::budget(6_830_000))]
#[bench::saw_16v(setup = saw, config = common::budget(80_640_000))]
#[bench::triangle_16v(setup = triangle, config = common::budget(94_490_000))]
#[bench::pulse_16v(setup = pulse, config = common::budget(89_370_000))]
#[bench::sine_16v(setup = sine, config = common::budget(94_990_000))]
#[bench::table_16v(setup = table, config = common::budget(139_170_000))]
#[bench::table_on_frame_16v(setup = table_on_frame, config = common::budget(123_080_000))]
#[bench::synced_saw_16v(setup = synced_saw, config = common::budget(83_430_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = osc, benchmarks = [render]);
main!(library_benchmark_groups = osc);
