//! Instruction budgets for the `delay` module (S2a lane DLY), via gungraun (callgrind).
//!
//! Setup (building the patch, preparing the engine, queueing the chord) is outside the measured function. Each case
//! renders 1 s at 48 kHz (1,500 segments) of the held saw chord through one `delay` (a global effect: it runs once
//! per segment whatever the voice count). The engine skeleton's own cost (segment loop, FIFO, voice sum, pan, the
//! default `adsr` and `osc`) is in the number too: subtract the `engine` bench with the same voices to read the
//! module's own cost (`docs/dsp/delay.md` records both). Budgets are the measured count plus 10 %.
//!
//! - `default_*`: the catalogue defaults (synced dotted eighth at the default 140 BPM, feedback 35 %, filters
//!   150 Hz / 6 kHz, mix 100 %), static parameters.
//! - `ms_16v`: `sync` off, 300 ms.
//! - `pingpong_16v`: ping-pong on.
//! - `fb100_16v`: feedback 100 % with the filters wide open, so the soft clip is engaged on the loud chord.
//! - `mix0_16v`: `mix` 0: the loop still runs (its rings keep filling), the output is left alone.
//! - `retime_16v`: the tempo alternates between 120 and 140 BPM every 960 frames, so a 20 ms time-change crossfade
//!   (two taps and the fade weight) runs for the whole second: the dearest state of the module.
//!
//! Instruction counts are exact and independent of runner load; they under-report subnormal penalties, so the
//! subnormal tests stay mandatory. Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate.
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use std::hint::black_box;

use enth_engine::{Engine, Event, EventKind, NoteKey, PrepareCfg, compile};
use enth_patch::catalogue::DelayParam;
use enth_patch::{ModuleKind, Patch, Section};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn patch(set: &[(DelayParam, f32)]) -> Patch {
    let mut p = common::patch_with(ModuleKind::Delay);
    for &(param, v) in set {
        p.section_mut(Section::Fx)[0].values[param.index()] = v;
    }
    p
}

fn default(voices: usize) -> common::Prepared {
    common::prepare(&patch(&[]), voices, 1.0)
}

fn ms(voices: usize) -> common::Prepared {
    let p = patch(&[(DelayParam::Sync, 0.0), (DelayParam::Time, 300.0)]);
    common::prepare(&p, voices, 1.0)
}

fn pingpong(voices: usize) -> common::Prepared {
    common::prepare(&patch(&[(DelayParam::PingPong, 1.0)]), voices, 1.0)
}

fn fb100(voices: usize) -> common::Prepared {
    let p = patch(&[
        (DelayParam::Feedback, 100.0),
        (DelayParam::LowCut, 20.0),
        (DelayParam::HighCut, 20_000.0),
    ]);
    common::prepare(&p, voices, 1.0)
}

fn mixzero(voices: usize) -> common::Prepared {
    common::prepare(&patch(&[(DelayParam::Mix, 0.0)]), voices, 1.0)
}

/// A chord of `voices` notes and a tempo that changes every 960 frames: the same engine as `common::prepare`, with
/// the extra events (`common::Prepared` keeps its events private).
struct Retime {
    engine: Engine,
    left: Vec<f32>,
    right: Vec<f32>,
    events: Vec<(usize, EventKind)>,
}

fn retime(voices: usize) -> Retime {
    let (plan, _) =
        compile(&patch(&[]), &PrepareCfg::new(common::SR)).expect("bench patch compiles");
    let frames = common::SR as usize;
    let mut events: Vec<(usize, EventKind)> = (0..voices)
        .map(|i| {
            (
                0,
                EventKind::NoteOn {
                    key: NoteKey::new(36 + (i as u8 * 5) % 60, i as i32),
                    velocity: 0.8,
                },
            )
        })
        .collect();
    for (n, at) in (0..frames).step_by(960).enumerate() {
        let bpm = if n % 2 == 0 { 120.0 } else { 140.0 };
        events.push((at, EventKind::Tempo { bpm }));
    }
    Retime {
        engine: Engine::new(plan, 0x0E17_5EED),
        left: vec![0.0; frames],
        right: vec![0.0; frames],
        events,
    }
}

fn render_retime(mut p: Retime) -> f32 {
    let mut next = 0;
    let mut start = 0;
    let mut chunk: Vec<Event> = Vec::with_capacity(4);
    while start < p.left.len() {
        let end = (start + common::BLOCK).min(p.left.len());
        chunk.clear();
        while let Some(&(f, kind)) = p.events.get(next) {
            if f >= end {
                break;
            }
            chunk.push(Event {
                offset: (f - start) as u32,
                kind,
            });
            next += 1;
        }
        p.engine.process(
            black_box(&mut p.left[start..end]),
            black_box(&mut p.right[start..end]),
            &chunk,
        );
        start = end;
    }
    black_box(p.left[p.left.len() / 2] + p.right[p.right.len() / 3])
}

// Budgets: measured on x86_64 Linux with the pinned toolchain (2026-10-27), plus 10 %, rounded up to 1,000. 1 s at
// 48 kHz per case. Measured instructions (budget in the attribute) and the module's own cost, with the `engine` bench's
// 3,612,149 (1 voice) and 37,291,989 (16 voices) subtracted, per frame (a global module costs the same whatever the
// voice count; the difference between the 1- and 16-voice rows is the engine's own, not the delay's):
//   default_1v   10,834,090   150
//   default_16v  44,827,038   157
//   ms_16v       44,725,376   155
//   pingpong_16v 45,326,225   167
//   fb100_16v    45,073,429   162
//   mix0_16v     44,299,038   146
//   retime_16v   46,453,827   191   (a 20 ms time-change crossfade running for the whole second)
// Every patch holds an `osc` and an `adsr`, so a lane that changes either (or the engine skeleton) re-measures these
// budgets with the `engine` ones; a change to `enth-dsp::delay` re-measures all seven in the same commit.
#[library_benchmark]
#[bench::default_1v(args = (1,), setup = default, config = common::budget(11_918_000))]
#[bench::default_16v(args = (16,), setup = default, config = common::budget(49_310_000))]
#[bench::ms_16v(args = (16,), setup = ms, config = common::budget(49_198_000))]
#[bench::pingpong_16v(args = (16,), setup = pingpong, config = common::budget(49_859_000))]
#[bench::fb100_16v(args = (16,), setup = fb100, config = common::budget(49_581_000))]
#[bench::mix0_16v(args = (16,), setup = mixzero, config = common::budget(48_729_000))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

#[library_benchmark]
#[bench::retime_16v(args = (16,), setup = retime, config = common::budget(51_100_000))]
fn render_retime_case(p: Retime) -> f32 {
    render_retime(p)
}

library_benchmark_group!(name = delay, benchmarks = [render, render_retime_case]);
main!(library_benchmark_groups = delay);
