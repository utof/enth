//! Shared helpers for the gungraun benches (lane S2a's scaffold): build a patch with one module of a kind, prepare
//! an engine with a held chord outside the measured function, and render it in host blocks inside it.
//!
//! A module lane's bench (`benches/<kind>.rs`) does
//!
//! ```ignore
//! #[path = "common/mod.rs"]
//! mod common;
//! fn setup(voices: usize) -> common::Prepared { common::prepare(&common::patch_with(ModuleKind::Svf), voices, 1.0) }
//! #[library_benchmark]
//! #[bench::svf_16v(args = (16,), setup = setup, config = common::budget(<measured + 10 %>))]
//! fn render(p: common::Prepared) -> f32 { common::render(p) }
//! ```
//!
//! and subtracts the matching `engine` bench (same voices, no extra module) to read its own cost.
#![allow(dead_code, reason = "each bench uses a different subset")]

use std::hint::black_box;

use enth_engine::{
    Engine, Event, EventKind, NoteKey, Plan, PrepareCfg, StereoMode, compile, compile_with,
};
use enth_patch::catalogue::{AdsrParam, VoiceParam};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch};
use gungraun::{Callgrind, EventKind as Ir, LibraryBenchmarkConfig};

/// The sample rate of every bench.
pub const SR: f32 = 48_000.0;

/// Frames per host call.
pub const BLOCK: usize = 512;

/// Absolute instruction budget: the measured value plus 10 % (docs/benchmarks.md). Absolute limits need no stored
/// baseline, so they work in CI.
pub fn budget(ir: u64) -> LibraryBenchmarkConfig {
    LibraryBenchmarkConfig::default()
        .tool(Callgrind::default().hard_limits([(Ir::Ir, ir)]))
        .clone()
}

/// `Patch::init()` (one saw, the default envelope) with polyphony 16 and a sustain of 100 %, so every voice holds
/// a constant level for the whole render.
pub fn base_patch() -> Patch {
    let mut p = Patch::init();
    p.voice.values[VoiceParam::Polyphony.index()] = 16.0;
    p.modulators[0].values[AdsrParam::Sustain.index()] = 100.0;
    p
}

/// [`base_patch`] plus one module of `kind` with default values, in its section (an `osc` becomes `osc2`).
pub fn patch_with(kind: ModuleKind) -> Patch {
    let mut p = base_patch();
    let id = ModuleId::new(&format!("{}2", kind.spec().id_prefix)).expect("valid id");
    p.section_mut(kind.section())
        .push(ModuleInstance::new(id, kind));
    p
}

/// An engine ready to render, with its output buffers and events.
pub struct Prepared {
    engine: Engine,
    left: Vec<f32>,
    right: Vec<f32>,
    events: Vec<Event>,
}

/// Compile `patch` and queue a held chord of `voices` notes (C2 upwards in fifths and fourths) at frame 0 for a
/// render of `seconds`. Allocates; call it from a gungraun `setup`.
pub fn prepare(patch: &Patch, voices: usize, seconds: f32) -> Prepared {
    let (plan, _) = compile(patch, &PrepareCfg::new(SR)).expect("bench patch compiles");
    prepared(plan, voices, seconds)
}

/// [`prepare`] with the stereo voice path forced on: every per-voice module runs for the left bus and again, as its
/// right-channel twin, for the right one (lane PRF's paired kernels). The patch is the same, so the difference to
/// [`prepare`] is what a stereo plan costs on it.
pub fn prepare_stereo(patch: &Patch, voices: usize, seconds: f32) -> Prepared {
    let (plan, _) = compile_with(patch, &PrepareCfg::new(SR), StereoMode::Forced)
        .expect("bench patch compiles");
    assert!(plan.is_stereo());
    prepared(plan, voices, seconds)
}

fn prepared(plan: Plan, voices: usize, seconds: f32) -> Prepared {
    let events = (0..voices)
        .map(|i| Event {
            offset: 0,
            kind: EventKind::NoteOn {
                key: NoteKey::new(36 + (i as u8 * 5) % 60, i as i32),
                velocity: 0.8,
            },
        })
        .collect();
    let frames = (seconds * SR) as usize;
    Prepared {
        engine: Engine::new(plan, 0x0E17_5EED),
        left: vec![0.0; frames],
        right: vec![0.0; frames],
        events,
    }
}

/// Render the prepared engine in [`BLOCK`]-frame host calls (the measured part).
pub fn render(mut p: Prepared) -> f32 {
    let mut first = true;
    for (l, r) in p.left.chunks_mut(BLOCK).zip(p.right.chunks_mut(BLOCK)) {
        let events: &[Event] = if first { &p.events } else { &[] };
        first = false;
        p.engine.process(black_box(l), black_box(r), events);
    }
    black_box(p.left[p.left.len() / 2] + p.right[p.right.len() / 3])
}
