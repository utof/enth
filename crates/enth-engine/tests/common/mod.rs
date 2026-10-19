//! Helpers shared by the engine's integration tests: patches built in code and a frame-based driver.
#![allow(dead_code, reason = "each test binary uses a different subset")]

use enth_engine::{Engine, Event, EventKind, NoteKey, PrepareCfg, compile};
use enth_patch::catalogue::{AdsrParam, LfoParam, LfoScope, OscParam, PhaseMode, Wave};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch};

pub const SR: f32 = 48_000.0;
pub const SEED: u64 = 0x0E17_5EED;

pub fn id(s: &str) -> ModuleId {
    ModuleId::new(s).unwrap()
}

/// `Patch::init()`: one saw (free phase) and the default envelope.
pub fn saw_patch() -> Patch {
    Patch::init()
}

/// The saw with a reset phase (start phase 0), so every note starts identically.
pub fn reset_saw_patch() -> Patch {
    let mut p = Patch::init();
    p.generators[0].values[OscParam::PhaseMode.index()] = PhaseMode::Reset.index() as f32;
    p
}

/// One module of every kind (and both LFO scopes), with a long release: the patch the tail tests run, so every
/// module that replaces its stub is covered automatically.
pub fn every_kind_patch(release_ms: f32) -> Patch {
    let mut p = Patch::init();
    p.modulators[0].values[AdsrParam::Release.index()] = release_ms;
    let mut osc2 = ModuleInstance::new(id("osc2"), ModuleKind::Osc);
    osc2.values[OscParam::Wave.index()] = Wave::Pulse.index() as f32;
    osc2.values[OscParam::Fine.index()] = 7.0;
    p.generators.push(osc2);
    p.generators
        .push(ModuleInstance::new(id("noise1"), ModuleKind::Noise));
    p.voice_fx
        .push(ModuleInstance::new(id("filter1"), ModuleKind::Svf));
    p.modulators
        .push(ModuleInstance::new(id("lfo1"), ModuleKind::Lfo));
    let mut lfo2 = ModuleInstance::new(id("lfo2"), ModuleKind::Lfo);
    lfo2.values[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
    p.modulators.push(lfo2);
    p.fx.push(ModuleInstance::new(id("comp1"), ModuleKind::Comp));
    p.fx.push(ModuleInstance::new(id("reverb1"), ModuleKind::Reverb));
    p
}

/// A note in frames.
#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub key: u8,
    pub velocity: f32,
    pub on: u64,
    pub off: u64,
}

pub fn note(key: u8, on: u64, off: u64) -> Note {
    Note {
        key,
        velocity: 0.8,
        on,
        off,
    }
}

/// Absolute-frame events for `notes`, sorted stably by frame (each note's on before its off).
pub fn events(notes: &[Note]) -> Vec<(u64, EventKind)> {
    let mut ev = Vec::new();
    for (i, n) in notes.iter().enumerate() {
        let key = NoteKey::new(n.key, i as i32);
        ev.push((
            n.on,
            EventKind::NoteOn {
                key,
                velocity: n.velocity,
            },
        ));
        ev.push((n.off, EventKind::NoteOff { key, velocity: 0.0 }));
    }
    ev.sort_by_key(|(f, _)| *f);
    ev
}

pub fn engine(patch: &Patch, seed: u64) -> Engine {
    let (plan, _) = compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles");
    Engine::new(plan, seed)
}

/// Drive `engine` for `frames` frames in host blocks of `block`, delivering `events` (absolute frames). `per_block`
/// runs after each block (for scans).
pub fn drive(
    engine: &mut Engine,
    events: &[(u64, EventKind)],
    frames: usize,
    block: usize,
    mut per_block: impl FnMut(&Engine, &[f32], &[f32]),
) -> (Vec<f32>, Vec<f32>) {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let mut next = 0;
    let mut start = 0;
    let mut chunk = Vec::new();
    while start < frames {
        let end = (start + block).min(frames);
        chunk.clear();
        while let Some(&(f, kind)) = events.get(next) {
            if f >= end as u64 {
                break;
            }
            chunk.push(Event {
                offset: (f - start as u64) as u32,
                kind,
            });
            next += 1;
        }
        engine.process(&mut left[start..end], &mut right[start..end], &chunk);
        per_block(engine, &left[start..end], &right[start..end]);
        start = end;
    }
    (left, right)
}

/// Render `notes` through `patch`: (left, right, the engine afterwards).
pub fn render(
    patch: &Patch,
    notes: &[Note],
    frames: usize,
    block: usize,
    seed: u64,
) -> (Vec<f32>, Vec<f32>, Engine) {
    let mut e = engine(patch, seed);
    let (l, r) = drive(&mut e, &events(notes), frames, block, |_, _, _| {});
    (l, r, e)
}

pub fn bits(x: &[f32]) -> Vec<u32> {
    x.iter().map(|v| v.to_bits()).collect()
}
