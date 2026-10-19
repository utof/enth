//! Instruction budgets for the SVF filter module (P1a lane D2), via gungraun (callgrind).
//!
//! Setup (building the patch, preparing the engine, queueing the chord) is outside the measured function. Each case
//! renders 1 s at 48 kHz (1,500 segments). The engine skeleton's own cost (segment loop, FIFO, voice sum, pan, the
//! default `adsr` and `osc`) is in the number too: subtract the `engine` bench with the same voices to read the
//! filter's own cost (`docs/dsp/svf.md` records both). Budgets are the measured count plus 10 %.
//!
//! - `lp_*`: the default low-pass, static parameters (the coefficients are cached, no `tanf`).
//! - `drive_*`: the same with 12 dB of drive (the per-sample saturator, one `sqrt` and one division).
//! - `moving_*`: the cutoff changes at every segment (a new `tanf` and a coefficient ramp every 32 frames per
//!   voice): the cost of an envelope or LFO routed to the cutoff, which S2b will make real.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use std::hint::black_box;

use enth_engine::{Engine, Event, EventKind, NoteKey, PrepareCfg, SEG, compile};
use enth_patch::catalogue::SvfParam;
use enth_patch::{ModuleKind, ParamAddr, Patch, Section};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn lp(voices: usize) -> common::Prepared {
    common::prepare(&common::patch_with(ModuleKind::Svf), voices, 1.0)
}

fn drive_patch() -> Patch {
    let mut p = common::patch_with(ModuleKind::Svf);
    p.section_mut(Section::VoiceFx)[0].values[SvfParam::Drive.index()] = 12.0;
    p
}

fn drive(voices: usize) -> common::Prepared {
    common::prepare(&drive_patch(), voices, 1.0)
}

/// An engine whose filter cutoff is set to a new value at every segment boundary.
struct Moving {
    engine: Engine,
    left: Vec<f32>,
    right: Vec<f32>,
    /// Block 0: the chord and the first cutoff; then one cutoff per segment.
    first: Vec<Event>,
    cutoffs: Vec<Event>,
}

fn moving(voices: usize) -> Moving {
    let patch = common::patch_with(ModuleKind::Svf);
    let (plan, map) = compile(&patch, &PrepareCfg::new(common::SR)).expect("bench patch compiles");
    let id = map
        .get(&"filter2.cutoff".parse::<ParamAddr>().expect("address"))
        .expect("the filter is in the plan");
    let frames = common::SR as usize;
    let segments = frames / SEG;
    let cutoffs = (0..segments)
        .map(|i| Event {
            offset: (i * SEG) as u32,
            kind: EventKind::Param {
                id,
                // 200 Hz ... 8 kHz, a different value every segment (a fixed pseudo-random walk).
                value: 200.0 + ((i * 2749) % 7800) as f32,
            },
        })
        .collect();
    let first = (0..voices)
        .map(|i| Event {
            offset: 0,
            kind: EventKind::NoteOn {
                key: NoteKey::new(36 + (i as u8 * 5) % 60, i as i32),
                velocity: 0.8,
            },
        })
        .collect();
    Moving {
        engine: Engine::new(plan, 0x0E17_5EED),
        left: vec![0.0; frames],
        right: vec![0.0; frames],
        first,
        cutoffs,
    }
}

fn render_moving(mut m: Moving) -> f32 {
    for (i, (l, r)) in m
        .left
        .chunks_mut(SEG)
        .zip(m.right.chunks_mut(SEG))
        .enumerate()
    {
        if i == 0 {
            let mut ev = m.first.clone();
            ev.push(m.cutoffs[0]);
            m.engine.process(black_box(l), black_box(r), &ev);
        } else {
            m.engine.process(
                black_box(l),
                black_box(r),
                std::slice::from_ref(&m.cutoffs[i]),
            );
        }
    }
    black_box(m.left[m.left.len() / 2] + m.right[m.right.len() / 3])
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-19, 1 s at 48 kHz), plus 10 %, rounded up to
// 1,000. Measured: 5,517,718 (lp_1v), 65,201,335 (lp_16v), 71,442,823 (drive_16v, 12 dB) and 71,381,472 (moving_16v,
// a cutoff event every segment); over the `engine` bench's 3,873,015 and 43,370,997 that is 28.4 instructions per
// sample per voice for a static filter, 36.6 with the saturator and 36.5 with the cutoff moving (docs/dsp/svf.md).
#[library_benchmark]
#[bench::lp_1v(args = (1,), setup = lp, config = common::budget(6_070_000))]
#[bench::lp_16v(args = (16,), setup = lp, config = common::budget(71_722_000))]
#[bench::drive_16v(args = (16,), setup = drive, config = common::budget(78_588_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

#[library_benchmark]
#[bench::moving_16v(args = (16,), setup = moving, config = common::budget(78_520_000))]
fn render_cutoff_moving(m: Moving) -> f32 {
    render_moving(m)
}

library_benchmark_group!(name = svf, benchmarks = [render, render_cutoff_moving]);
main!(library_benchmark_groups = svf);
