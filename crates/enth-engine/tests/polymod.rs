//! Per-voice macro offsets for CLAP polyphonic modulation (P1b lane E+): a macro plus a per-voice offset, addressed
//! by note id, reaching voice-rate destinations only. Every `process` call runs under `assert_no_alloc`.
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, bits, id, reset_saw_patch};
use enth_engine::{Engine, Event, EventKind, NoteKey, ParamId, PlanMap, PrepareCfg, SEG, compile};
use enth_patch::catalogue::{CompParam, OscParam};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// macro1 (at 0.2) moves `osc1.level` (voice rate) by −50 % and `comp1.mix` (global) by −50 %.
fn patch() -> Patch {
    let mut p = reset_saw_patch();
    p.fx.push(ModuleInstance::new(id("comp1"), ModuleKind::Comp));
    p.macros[0].value = 0.2;
    for dst in ["osc1.level", "comp1.mix"] {
        p.routes.push(Route {
            src: "macro1".parse().unwrap(),
            dst: dst.parse().unwrap(),
            depth: -50.0,
        });
    }
    p
}

fn on(key: u8, note_id: i32) -> EventKind {
    EventKind::NoteOn {
        key: NoteKey::new(key, note_id),
        velocity: 0.8,
    }
}

fn poly(note_id: i32, offset: f32) -> EventKind {
    EventKind::MacroVoice {
        key: NoteKey::note_id(note_id),
        index: 0,
        offset,
    }
}

/// Render `segments` one-segment calls; `events(k)` gives segment `k`'s events.
fn render(
    segments: usize,
    mut events: impl FnMut(usize) -> Vec<EventKind>,
) -> (Vec<f32>, Engine, PlanMap) {
    let (plan, map) = compile(&patch(), &PrepareCfg::new(SR)).unwrap();
    let mut e = Engine::new(plan, SEED);
    let mut left = vec![0.0; segments * SEG];
    let mut right = vec![0.0; segments * SEG];
    for k in 0..segments {
        let ev: Vec<Event> = events(k)
            .into_iter()
            .map(|kind| Event { offset: 0, kind })
            .collect();
        let (l, r) = (
            &mut left[k * SEG..(k + 1) * SEG],
            &mut right[k * SEG..(k + 1) * SEG],
        );
        assert_no_alloc(|| e.process(l, r, &ev));
    }
    (left, e, map)
}

fn voice_of(e: &Engine, note_id: i32) -> usize {
    *e.active_voices()
        .iter()
        .find(|&&v| e.voices().key(v).note_id == note_id)
        .expect("a voice plays the note")
}

#[test]
fn an_offset_moves_only_the_matching_voice_and_only_voice_rate_destinations() {
    let (_, e, map) = render(40, |k| match k {
        0 => vec![on(60, 10), on(67, 20)],
        5 => vec![poly(20, 0.5)],
        _ => Vec::new(),
    });
    let level = map.get(&"osc1.level".parse().unwrap()).unwrap();
    let mix = map.get(&"comp1.mix".parse().unwrap()).unwrap();
    assert!(matches!(level, ParamId::Voice(_)) && matches!(mix, ParamId::Global(_)));
    let (v10, v20) = (voice_of(&e, 10), voice_of(&e, 20));
    assert_eq!(e.macro_offset(0, v20), Some(0.5));
    assert_eq!(e.macro_offset(0, v10), Some(0.0));
    // n = to_norm(100) + (−0.5) · source, with source = clamp(macro + offset, 0, 1).
    let spec = OscParam::Level.spec();
    let n0 = spec.to_norm(100.0);
    let at = |m: f32| spec.from_norm((n0 + -0.5f32 * m.clamp(0.0, 1.0)).clamp(0.0, 1.0));
    assert_eq!(e.value(level, v20), Some(at(0.2 + 0.5)));
    assert_eq!(e.value(level, v10), Some(at(0.2)));
    // The global destination hears the macro alone.
    let mix_spec = CompParam::Mix.spec();
    let mix_n0 = mix_spec.to_norm(100.0);
    assert_eq!(
        e.value(mix, 0),
        Some(mix_spec.from_norm((mix_n0 + -0.5f32 * 0.2).clamp(0.0, 1.0)))
    );
}

#[test]
fn zero_or_unmatched_offsets_change_nothing() {
    let notes = |k: usize| match k {
        0 => vec![on(60, 10), on(67, 20)],
        _ => Vec::new(),
    };
    let (plain, _, _) = render(100, notes);
    for extra in [poly(20, 0.0), poly(99, 0.7), poly(10, f32::NAN)] {
        let (l, _, _) = render(100, |k| {
            let mut ev = notes(k);
            if k == 5 {
                ev.push(extra);
            }
            ev
        });
        assert_eq!(bits(&l), bits(&plain), "{extra:?}");
    }
    // An offset on one voice leaves the other voice's values bit-identical to the plain render.
    let (_, with, map) = render(40, |k| match k {
        0 => vec![on(60, 10), on(67, 20)],
        5 => vec![poly(20, -0.3)],
        _ => Vec::new(),
    });
    let (_, without, _) = render(40, notes);
    let level = map.get(&"osc1.level".parse().unwrap()).unwrap();
    let v10 = voice_of(&with, 10);
    assert_eq!(
        with.value(level, v10).map(f32::to_bits),
        without.value(level, v10).map(f32::to_bits)
    );
}

#[test]
fn offsets_clamp_reset_on_a_new_note_and_take_wildcards() {
    let (_, e, _) = render(8, |k| match k {
        0 => vec![on(60, 10), on(67, 20)],
        5 => vec![poly(20, 5.0)],
        _ => Vec::new(),
    });
    assert_eq!(
        e.macro_offset(0, voice_of(&e, 20)),
        Some(1.0),
        "clamped to ±1"
    );
    let (_, e, _) = render(60, |k| match k {
        0 => vec![on(60, 10), on(67, 20)],
        5 => vec![poly(20, 5.0)],
        10 => vec![EventKind::MacroVoice {
            key: NoteKey::note_id(-1),
            index: 0,
            offset: -0.25,
        }],
        // An index outside the eight macros is ignored.
        12 => vec![EventKind::MacroVoice {
            key: NoteKey::note_id(-1),
            index: 8,
            offset: 0.9,
        }],
        _ => Vec::new(),
    });
    let (v10, v20) = (voice_of(&e, 10), voice_of(&e, 20));
    assert_eq!(
        e.macro_offset(0, v10),
        Some(-0.25),
        "a wildcard reaches every voice"
    );
    assert_eq!(e.macro_offset(0, v20), Some(-0.25));
    // A note that lands on a voice starts without an offset (mono retrigger here).
    let mut mono = patch();
    mono.voice.values[enth_patch::catalogue::VoiceParam::Mode.index()] =
        enth_patch::catalogue::VoiceMode::Mono.index() as f32;
    let (plan, _) = compile(&mono, &PrepareCfg::new(SR)).unwrap();
    let mut m = Engine::new(plan, SEED);
    let step = |m: &mut Engine, ev: &[EventKind]| {
        let ev: Vec<Event> = ev.iter().map(|&kind| Event { offset: 0, kind }).collect();
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        assert_no_alloc(|| m.process(&mut l, &mut r, &ev));
    };
    step(&mut m, &[on(60, 1)]);
    step(&mut m, &[poly(1, 0.4)]);
    let v = m.active_voices()[0];
    assert_eq!(m.macro_offset(0, v), Some(0.4));
    step(&mut m, &[on(64, 2)]);
    assert_eq!(m.active_voices(), &[v], "the mono voice retriggered");
    assert_eq!(m.macro_offset(0, v), Some(0.0), "the new note starts at 0");
}

#[test]
fn offsets_are_forgotten_when_the_voice_ends() {
    let (_, e, _) = render(3000, |k| match k {
        0 => vec![on(60, 10)],
        5 => vec![poly(10, 0.6)],
        20 => vec![EventKind::NoteOff {
            key: NoteKey::new(60, 10),
            velocity: 0.0,
        }],
        _ => Vec::new(),
    });
    assert_eq!(e.active_voices().len(), 0);
    assert_eq!(e.macro_offset(0, 0), Some(0.0));
}
