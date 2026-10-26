//! Instruction count of the G1 reference patch with the stereo voice path forced on (lane SVP, S2a).
//!
//! The same patch, chord, 2 s render and 512-frame calls as `g1`, compiled with `StereoMode::Forced`: the difference to
//! `g1` is what a stereo plan costs on a patch whose content is mono: a second `voice_fx` chain on the right bus, the
//! bus copy, and the two-channel balance. `g1` itself is the mono path and must not move (docs/dsp/engine.md,
//! "Stereo voice path").
//! Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate (`cargo xtask bench --bench stereo`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/g1.rs"]
mod g1_patch;

use std::hint::black_box;

use enth_engine::{Engine, Event, EventKind, NoteKey, PrepareCfg, StereoMode, compile_with};
use g1_patch::g1_patch;
use gungraun::{
    Callgrind, EventKind as Ir, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group,
    main,
};

const SR: f32 = 48_000.0;
/// Frames per host call, as `g1`.
const BLOCK: usize = 512;
/// Frames per render: 2 s at 48 kHz, as `g1`.
const FRAMES: usize = 96_000;

/// G1's cap: 12,000 instructions per stereo frame; the stereo path must stay inside it too.
const G1_PER_FRAME: u64 = 12_000;

/// The hard limit: the first measurement (see the comment below) plus 10 %, rounded up to a round number.
const STEREO_BUDGET_IR: u64 = 715_000_000;

const _: () = assert!(STEREO_BUDGET_IR <= FRAMES as u64 * G1_PER_FRAME);

struct Prepared {
    engine: Engine,
    left: Vec<f32>,
    right: Vec<f32>,
    events: Vec<Event>,
}

/// The G1 patch with 16 held notes, compiled with the stereo flag forced on. Allocates; a gungraun `setup`.
fn g1_stereo(voices: usize) -> Prepared {
    let (plan, _) = compile_with(&g1_patch(), &PrepareCfg::new(SR), StereoMode::Forced)
        .expect("the G1 patch compiles");
    assert!(plan.is_stereo());
    let events = (0..voices)
        .map(|i| Event {
            offset: 0,
            kind: EventKind::NoteOn {
                key: NoteKey::new(36 + (i as u8 * 5) % 60, i as i32),
                velocity: 0.8,
            },
        })
        .collect();
    Prepared {
        engine: Engine::new(plan, 0x0E17_5EED),
        left: vec![0.0; FRAMES],
        right: vec![0.0; FRAMES],
        events,
    }
}

fn render_stereo(mut p: Prepared) -> f32 {
    let mut first = true;
    for (l, r) in p.left.chunks_mut(BLOCK).zip(p.right.chunks_mut(BLOCK)) {
        let events: &[Event] = if first { &p.events } else { &[] };
        first = false;
        p.engine.process(black_box(l), black_box(r), events);
    }
    black_box(p.left[p.left.len() / 2] + p.right[p.right.len() / 3])
}

fn budget(ir: u64) -> LibraryBenchmarkConfig {
    LibraryBenchmarkConfig::default()
        .tool(Callgrind::default().hard_limits([(Ir::Ir, ir)]))
        .clone()
}

// Budget: the measurement plus 10 %. G1 forced stereo, 16 voices:
// - SVP, 2026-10-26 (x86_64 Linux, Intel i7-9750H, valgrind/callgrind via gungraun 0.20.0, the pinned toolchain):
//   647,369,152 instructions (6,743 per frame), L1 hits 834,771,124, estimated cycles 855,327,724. The mono `g1`
//   bench on the same tree: 602,861,693 (6,280 per frame); on main 2809273: 602,615,188. So forcing the stereo path
//   on G1 costs +44.5 M (+7.4 %, +464 per frame): the second `svf` per voice, the bus copy and the two-channel
//   balance; the reverb and the compressor run once. The mono path moved +246,505 instructions (+0.04 %, 2.6 per
//   frame: one flag test per active voice and segment).
#[library_benchmark]
#[bench::g1_stereo_16v(args = (16,), setup = g1_stereo, config = budget(STEREO_BUDGET_IR))]
fn render(p: Prepared) -> f32 {
    render_stereo(p)
}

library_benchmark_group!(name = stereo, benchmarks = [render]);
main!(library_benchmark_groups = stereo);
