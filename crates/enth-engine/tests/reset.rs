//! `Engine::reset` and `Engine::position` (CLAP `reset`, `NOTE_END` offsets) and audition notes sent through the
//! command ring (P1b lane E+). Every `process` and `reset` call runs under `assert_no_alloc`.
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, bits, every_kind_patch, id};
use enth_engine::{
    Engine, Event, EventKind, Link, LinkCapacity, NoteKey, Plan, PlanMap, PrepareCfg, SEG,
    SendError, compile,
};
use enth_patch::catalogue::{MasterParam, SvfParam, VoiceParam};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn plan(patch: &Patch) -> (Plan, PlanMap) {
    compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles")
}

fn on(key: u8, note_id: i32) -> EventKind {
    EventKind::NoteOn {
        key: NoteKey::new(key, note_id),
        velocity: 0.8,
    }
}

fn off(key: u8, note_id: i32) -> EventKind {
    EventKind::NoteOff {
        key: NoteKey::new(key, note_id),
        velocity: 0.0,
    }
}

/// Every kind, routes from both LFO scopes and a macro, a glide.
fn patch() -> Patch {
    let mut p = every_kind_patch(300.0);
    p.voice.values[VoiceParam::Glide.index()] = 40.0;
    for (src, dst, depth) in [
        ("lfo1", "filter1.cutoff", 25.0),
        ("lfo2", "comp1.mix", -30.0),
        ("macro1", "reverb1.wet", 20.0),
    ] {
        p.routes.push(Route {
            src: src.parse().unwrap(),
            dst: dst.parse().unwrap(),
            depth,
        });
    }
    p
}

/// Run `segments` one-segment calls under `assert_no_alloc`; `events(k)` gives segment `k`'s events.
fn run(
    engine: &mut Engine,
    segments: usize,
    mut events: impl FnMut(usize) -> Vec<EventKind>,
) -> (Vec<f32>, Vec<f32>) {
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
        assert_no_alloc(|| engine.process(l, r, &ev));
    }
    (left, right)
}

/// The second performance: a chord, a glide, a release with the reverb ringing on.
fn second(k: usize) -> Vec<EventKind> {
    match k {
        0 => vec![on(48, 0), on(55, 1)],
        30 => vec![on(62, 2)],
        150 => vec![off(48, 0), off(55, 1), off(62, 2)],
        _ => Vec::new(),
    }
}

#[test]
fn after_reset_the_output_equals_a_fresh_engine_with_the_same_bases() {
    let (pl, map) = plan(&patch());
    let id = |a: &str| map.get(&a.parse().unwrap()).unwrap();
    let (cutoff, level) = (id("filter1.cutoff"), id("master.level"));
    let (mut e, _link) = Engine::with_link(pl, SEED, LinkCapacity::default());
    // A first performance that leaves everything moving: voices, glides, an LFO's phase, a reverb tail, the
    // compressor's envelope, a smoother in flight (the cutoff at its last segment), base values off the patch.
    run(&mut e, 200, |k| match k {
        0 => vec![on(60, 0), on(64, 1), on(67, 2)],
        10 => vec![EventKind::Param {
            id: cutoff,
            value: 500.0,
        }],
        20 => vec![EventKind::Macro {
            index: 0,
            value: 0.7,
        }],
        30 => vec![EventKind::Param {
            id: level,
            value: -6.0,
        }],
        100 => vec![off(60, 0), off(64, 1)],
        199 => vec![EventKind::Param {
            id: cutoff,
            value: 800.0,
        }],
        _ => Vec::new(),
    });
    assert!(e.position() > 0 && !e.active_voices().is_empty());
    assert_no_alloc(|| e.reset());
    assert_eq!(e.position(), 0);
    assert_eq!(e.active_voices().len(), 0);
    assert_eq!(e.stats().notes, 0, "the note index restarts");
    assert_eq!(e.stats().segments, 0);
    assert_eq!(e.base(cutoff), Some(800.0), "base values are kept");
    let (l, r) = run(&mut e, 600, second);

    // A fresh engine whose patch holds those bases.
    let mut fresh = patch();
    fresh.voice_fx[0].values[SvfParam::Cutoff.index()] = 800.0;
    fresh.macros[0].value = 0.7;
    fresh.master[MasterParam::Level.index()] = -6.0;
    let mut f = Engine::new(plan(&fresh).0, SEED);
    let (fl, fr) = run(&mut f, 600, second);
    assert!(fl.iter().any(|x| *x != 0.0));
    assert_eq!(bits(&l), bits(&fl));
    assert_eq!(bits(&r), bits(&fr));
    assert_eq!(e.position(), 600 * SEG as u64);
}

#[test]
fn reset_during_a_crossfade_hands_the_old_plan_back_and_never_frees_it() {
    let base = patch();
    let mut edited = base.clone();
    edited
        .modulators
        .push(ModuleInstance::new(id("lfo3"), ModuleKind::Lfo));
    // A return ring of one: the first swap fills it, so the reset has to keep the second replaced plan pending.
    let capacity = LinkCapacity {
        commands: 16,
        returns: 1,
    };
    let (mut e, mut link): (Engine, Link) = Engine::with_link(plan(&base).0, SEED, capacity);
    link.send_plan(plan(&base).0).unwrap();
    run(
        &mut e,
        2,
        |k| if k == 0 { vec![on(60, 0)] } else { Vec::new() },
    );
    assert_eq!(e.stats().swaps, 1);
    link.send_plan(plan(&edited).0).unwrap();
    run(&mut e, 2, |_| Vec::new());
    assert!(e.is_crossfading());
    assert_eq!(e.plans_held(), 2);
    assert_no_alloc(|| e.reset());
    assert!(!e.is_crossfading());
    assert_eq!(
        e.plans_held(),
        2,
        "pending: the ring is full, the plan is kept"
    );
    assert_eq!(e.epoch(), 2, "the new plan runs");
    assert_eq!(link.collect(), 1);
    run(&mut e, 1, |_| Vec::new());
    assert_eq!(e.plans_held(), 1);
    assert_eq!(link.collect(), 1, "handed back once there was room");
    assert_eq!(link.in_flight(), 0);
}

#[test]
fn position_counts_host_frames_whatever_the_block() {
    let (mut e, _) = Engine::with_link(plan(&patch()).0, SEED, LinkCapacity::default());
    let mut expected = 0u64;
    for block in [1usize, 31, 32, 33, 100, 4096] {
        let (mut l, mut r) = (vec![0.0; block], vec![0.0; block]);
        assert_eq!(e.position(), expected);
        assert_no_alloc(|| e.process(&mut l, &mut r, &[]));
        expected += block as u64;
    }
    assert_eq!(e.position(), expected);
}

/// Editor and Recreate notes: their own note port, so their `NOTE_END`s can be filtered out.
const AUDITION_PORT: i16 = 0x7F00;

fn audition(key: u8, note_id: i32, down: bool) -> EventKind {
    let key = NoteKey {
        port: AUDITION_PORT,
        ..NoteKey::new(key, note_id)
    };
    if down {
        EventKind::NoteOn { key, velocity: 0.8 }
    } else {
        EventKind::NoteOff { key, velocity: 0.0 }
    }
}

#[test]
fn audition_notes_play_without_host_events_like_host_events_at_that_boundary() {
    let render = |via_ring: bool| {
        let (mut e, mut link) = Engine::with_link(plan(&patch()).0, SEED, LinkCapacity::default());
        let mut left = vec![0.0; 900 * SEG];
        let mut ends = Vec::new();
        for k in 0..900 {
            let mut host = Vec::new();
            for (at, kind) in [(3, audition(60, 1, true)), (40, audition(60, 1, false))] {
                if k == at {
                    if via_ring {
                        link.send_event(kind).unwrap();
                    } else {
                        host.push(Event { offset: 0, kind });
                    }
                }
            }
            let mut right = [0.0; SEG];
            let l = &mut left[k * SEG..(k + 1) * SEG];
            assert_no_alloc(|| e.process(l, &mut right, &host));
            ends.extend(e.note_ends().iter().map(|n| n.key.port));
        }
        (left, ends)
    };
    let (ring, ends) = render(true);
    let (host, _) = render(false);
    assert!(ring[3 * SEG..].iter().any(|x| *x != 0.0), "it sounds");
    assert!(ring[..3 * SEG].iter().all(|x| *x == 0.0));
    assert_eq!(bits(&ring), bits(&host));
    assert_eq!(
        ends,
        [AUDITION_PORT],
        "its NOTE_END names the audition port"
    );
}

#[test]
fn a_full_command_ring_of_audition_events_drains_without_allocating() {
    let capacity = LinkCapacity {
        commands: 64,
        returns: 4,
    };
    let (mut e, mut link) = Engine::with_link(plan(&patch()).0, SEED, capacity);
    let mut sent = 0;
    loop {
        match link.send_event(audition(40 + (sent % 40) as u8, sent, sent % 2 == 0)) {
            Ok(()) => sent += 1,
            Err(SendError::Full(())) => break,
            Err(SendError::InFlight(())) => unreachable!(),
        }
    }
    assert_eq!(sent, 64);
    run(&mut e, 4, |_| Vec::new());
    assert_eq!(link.free_slots(), 64, "drained in one segment");
    assert_eq!(e.stats().notes, 32);
    assert_eq!(e.stats().dropped_events, 0);
}

#[test]
fn an_audition_event_that_finds_the_event_queue_full_is_dropped_and_counted() {
    let (mut e, mut link) = Engine::with_link(plan(&patch()).0, SEED, LinkCapacity::default());
    // Fill the queue with events due at frame 64, past both segments this 64-frame call renders.
    let full: Vec<Event> = (0..enth_engine::segment::EVENT_CAPACITY)
        .map(|i| Event {
            offset: 63,
            kind: EventKind::Macro {
                index: (i % 8) as u8,
                value: 0.5,
            },
        })
        .collect();
    let (mut l, mut r) = ([0.0; 64], [0.0; 64]);
    assert_no_alloc(|| e.process(&mut l, &mut r, &full));
    assert_eq!(e.stats().dropped_events, 0);
    link.send_event(audition(60, 1, true)).unwrap();
    assert_no_alloc(|| e.process(&mut l, &mut r, &[]));
    assert_eq!(
        e.stats().dropped_events,
        1,
        "dropped, counted, nothing allocated"
    );
    assert_eq!(e.stats().notes, 0);
    link.send_event(audition(60, 1, true)).unwrap();
    assert_no_alloc(|| e.process(&mut l, &mut r, &[]));
    assert_eq!(e.stats().notes, 1, "room again once the queue drained");
}
