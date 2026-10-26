//! Instruction budgets for the `formant` module (S2a lane FLT), via gungraun (callgrind).
//!
//! Setup (building the patch, preparing the engine, queueing the chord) is outside the measured function. Each case
//! renders 1 s at 48 kHz (1,500 segments). The engine skeleton's own cost (segment loop, FIFO, voice sum, pan, the
//! default `adsr` and `osc`) is in the number too: subtract the `engine` bench with the same voices to read the
//! formant's own cost (`docs/dsp/formant.md` records both). Budgets are the measured count plus 10 %.
//!
//! - `static_*`: the default vowel ("a", 850/1610 Hz, resonance 50 %), static parameters: the tuning (two `exp2`
//!   and three `tan`) is cached, so the cost is the three band-pass sections.
//! - `moving_*`: `f2` changes at every segment (a new tuning and a ramp of twelve coefficients and three output
//!   weights every 32 frames per voice): the cost of an envelope or LFO routed to a formant, the "yoi" sweep.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use std::hint::black_box;

use enth_engine::{Engine, Event, EventKind, NoteKey, PrepareCfg, SEG, compile};
use enth_patch::{ModuleKind, ParamAddr};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn statics(voices: usize) -> common::Prepared {
    common::prepare(&common::patch_with(ModuleKind::Formant), voices, 1.0)
}

/// An engine whose second formant is set to a new value at every segment boundary.
struct Moving {
    engine: Engine,
    left: Vec<f32>,
    right: Vec<f32>,
    /// Block 0: the chord and the first `f2`; then one per segment.
    first: Vec<Event>,
    centres: Vec<Event>,
}

fn moving(voices: usize) -> Moving {
    let patch = common::patch_with(ModuleKind::Formant);
    let (plan, map) = compile(&patch, &PrepareCfg::new(common::SR)).expect("bench patch compiles");
    let addr = format!("{}2.f2", ModuleKind::Formant.spec().id_prefix);
    let id = map
        .get(&addr.parse::<ParamAddr>().expect("address"))
        .expect("the formant is in the plan");
    let frames = common::SR as usize;
    let segments = frames / SEG;
    let centres = (0..segments)
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
        centres,
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
            ev.push(m.centres[0]);
            m.engine.process(black_box(l), black_box(r), &ev);
        } else {
            m.engine.process(
                black_box(l),
                black_box(r),
                std::slice::from_ref(&m.centres[i]),
            );
        }
    }
    black_box(m.left[m.left.len() / 2] + m.right[m.right.len() / 3])
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-26, 1 s at 48 kHz), plus 10 %, rounded up to
// 1,000. Measured: 6,068,771 (static_1v), 74,135,421 (static_16v) and 110,086,193 (moving_16v, an `f2` event every
// segment); over the `engine` bench's 3,621,181 and 37,278,521 (same build) that is 48.0 instructions per sample per
// voice for a static vowel and 94.8 with `f2` moving: the extra 47 are the ramp of fifteen coefficients and, every
// segment, a new tuning (three `tan`, two `exp2`) (docs/dsp/formant.md).
#[library_benchmark]
#[bench::static_1v(args = (1,), setup = statics, config = common::budget(6_676_000))]
#[bench::static_16v(args = (16,), setup = statics, config = common::budget(81_549_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

#[library_benchmark]
#[bench::moving_16v(args = (16,), setup = moving, config = common::budget(121_095_000))]
fn render_f2_moving(m: Moving) -> f32 {
    render_moving(m)
}

library_benchmark_group!(name = formant, benchmarks = [render, render_f2_moving]);
main!(library_benchmark_groups = formant);
