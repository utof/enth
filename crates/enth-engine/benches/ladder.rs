//! Instruction budgets for the `ladder` module (S2a lane FLT), via gungraun (callgrind).
//!
//! Setup (building the patch, preparing the engine, queueing the chord) is outside the measured function. Each case
//! renders 1 s at 48 kHz (1,500 segments). The engine skeleton's own cost (segment loop, FIFO, voice sum, pan, the
//! default `adsr` and `osc`) is in the number too: subtract the `engine` bench with the same voices to read the
//! ladder's own cost (`docs/dsp/ladder.md` records both). Budgets are the measured count plus 10 %.
//!
//! - `lp24_*`: the default (a 24 dB low-pass at 2 kHz, resonance 10 %), static parameters: the coefficients (one `tan`)
//!   are cached, so the cost is the per-sample loop alone.
//! - `drive_16v`: the same with 12 dB of drive, the input saturator in the loop.
//! - `moving_16v`: the cutoff changes at every segment (a new `tan` and a ramp of the coefficients every 32 frames per
//!   voice): the cost of an envelope or LFO routed to the cutoff.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use std::hint::black_box;

use enth_engine::{Engine, Event, EventKind, NoteKey, PrepareCfg, SEG, compile};
use enth_patch::catalogue::LadderParam;
use enth_patch::{ModuleKind, ParamAddr, Patch};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn statics(voices: usize) -> common::Prepared {
    common::prepare(&common::patch_with(ModuleKind::Ladder), voices, 1.0)
}

fn drive_patch() -> Patch {
    let mut p = common::patch_with(ModuleKind::Ladder);
    p.voice_fx[0].values[LadderParam::Drive.index()] = 12.0;
    p
}

fn drive(voices: usize) -> common::Prepared {
    common::prepare(&drive_patch(), voices, 1.0)
}

/// An engine whose ladder cutoff is set to a new value at every segment boundary.
struct Moving {
    engine: Engine,
    left: Vec<f32>,
    right: Vec<f32>,
    /// Block 0: the chord and the first cutoff; then one per segment.
    first: Vec<Event>,
    cutoffs: Vec<Event>,
}

fn moving(voices: usize) -> Moving {
    let patch = common::patch_with(ModuleKind::Ladder);
    let (plan, map) = compile(&patch, &PrepareCfg::new(common::SR)).expect("bench patch compiles");
    let addr = format!("{}2.cutoff", ModuleKind::Ladder.spec().id_prefix);
    let id = map
        .get(&addr.parse::<ParamAddr>().expect("address"))
        .expect("the ladder is in the plan");
    let frames = common::SR as usize;
    let segments = frames / SEG;
    let cutoffs = (0..segments)
        .map(|i| Event {
            offset: (i * SEG) as u32,
            kind: EventKind::Param {
                id,
                // 400 Hz ... 4 kHz, a different value every segment (a fixed pseudo-random walk).
                value: 400.0 + ((i * 2749) % 3600) as f32,
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

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-26, 1 s at 48 kHz), plus 10 %, rounded up to
// 1,000. Measured: 6,427,120 (lp24_1v), 78,720,770 (lp24_16v), 86,666,546 (drive_16v, 12 dB) and 101,111,733
// (moving_16v, a cutoff event every segment); over the `engine` bench's 3,615,181 and 37,272,521 (same build) that is
// 54.0 instructions per sample per voice for a static filter, 64.3 with the saturator and 83.1 with the cutoff moving:
// the extra 29 are the ramp of the coefficients and, every segment, a new `tan` and the reciprocal that solves the
// loop (docs/dsp/ladder.md).
#[library_benchmark]
#[bench::lp24_1v(args = (1,), setup = statics, config = common::budget(7_070_000))]
#[bench::lp24_16v(args = (16,), setup = statics, config = common::budget(86_593_000))]
#[bench::drive_16v(args = (16,), setup = drive, config = common::budget(95_334_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

#[library_benchmark]
#[bench::moving_16v(args = (16,), setup = moving, config = common::budget(111_223_000))]
fn render_cutoff_moving(m: Moving) -> f32 {
    render_moving(m)
}

library_benchmark_group!(name = ladder, benchmarks = [render, render_cutoff_moving]);
main!(library_benchmark_groups = ladder);
