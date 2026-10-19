//! `process()` never allocates or frees: every host call runs inside `assert_no_alloc`, including the rare paths
//! where allocation sneaks in (AGENTS.md): a full event queue, full polyphony, every slot busy, junk events, odd
//! block shapes. Debug builds only (`assert_no_alloc` drops its checks in release). Under nextest an abort fails
//! exactly one test. A negative control shows the guard bites.
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, engine, events, every_kind_patch, note, saw_patch};
use enth_engine::segment::EVENT_CAPACITY;
use enth_engine::{Engine, Event, EventKind, NoteKey, ParamId};
use enth_patch::Patch;
use enth_patch::catalogue::VoiceParam;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// Run `events` (absolute frames) through `engine` in `block`-frame calls, each inside the guard.
fn guarded(engine: &mut Engine, ev: &[(u64, EventKind)], frames: usize, block: usize) -> Vec<f32> {
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let mut chunk: Vec<Event> = Vec::with_capacity(ev.len() + 1);
    let (mut start, mut next) = (0usize, 0usize);
    while start < frames {
        let end = (start + block).min(frames);
        chunk.clear();
        while let Some(&(f, kind)) = ev.get(next) {
            if f >= end as u64 {
                break;
            }
            chunk.push(Event {
                offset: (f - start as u64) as u32,
                kind,
            });
            next += 1;
        }
        let (l, r) = (&mut left[start..end], &mut right[start..end]);
        assert_no_alloc(|| engine.process(l, r, &chunk));
        start = end;
    }
    left
}

#[test]
fn process_does_not_allocate_on_the_happy_path() {
    for patch in [saw_patch(), every_kind_patch(400.0)] {
        for block in [1, 7, 64, 512, 4096] {
            let notes = [
                note(60, 0, 9000),
                note(64, 100, 9000),
                note(67, 5000, 12_000),
            ];
            let mut e = engine(&patch, SEED);
            let out = guarded(&mut e, &events(&notes), 24_000, block);
            assert!(out.iter().any(|x| *x != 0.0), "not silent");
        }
    }
}

#[test]
fn sixteen_voices_and_dropped_notes_do_not_allocate() {
    let mut patch = every_kind_patch(2000.0);
    patch.voice.values[VoiceParam::Polyphony.index()] = 16.0;
    // 40 overlapping notes: 16 play, the rest are dropped (S2b steals instead).
    let notes: Vec<_> = (0..40)
        .map(|i| note(36 + i as u8, i * 10, 20_000))
        .collect();
    let mut e = engine(&patch, SEED);
    guarded(&mut e, &events(&notes), 24_000, 256);
    let s = e.stats();
    assert_eq!((s.notes, s.dropped_notes), (40, 24));
}

#[test]
fn a_full_event_queue_drops_without_allocating() {
    let mut e = engine(&saw_patch(), SEED);
    // More events than the queue holds, all in one host block and due at the same boundary.
    let ev: Vec<_> = (0..EVENT_CAPACITY + 500)
        .map(|i| {
            let key = NoteKey::new(60, i as i32);
            (
                0u64,
                if i % 2 == 0 {
                    EventKind::NoteOn { key, velocity: 0.5 }
                } else {
                    EventKind::NoteOff { key, velocity: 0.0 }
                },
            )
        })
        .collect();
    guarded(&mut e, &ev, 4096, 4096);
    assert_eq!(e.stats().dropped_events, 500);
}

#[test]
fn junk_events_and_odd_blocks_do_not_allocate() {
    let mut e = engine(&every_kind_patch(400.0), SEED);
    let junk = [
        Event {
            offset: 0,
            kind: EventKind::Param {
                id: ParamId::Global(60_000),
                value: 1.0,
            },
        },
        Event {
            offset: 3,
            kind: EventKind::Param {
                id: ParamId::Voice(0),
                value: f32::NAN,
            },
        },
        Event {
            offset: 9_999,
            kind: EventKind::NoteOn {
                key: NoteKey::new(127, 1),
                velocity: f32::NAN,
            },
        },
        Event {
            offset: 1,
            kind: EventKind::NoteOff {
                key: NoteKey {
                    port: -1,
                    channel: -1,
                    key: -1,
                    note_id: -1,
                },
                velocity: 0.0,
            },
        },
    ];
    let (mut l, mut r) = (vec![0.0f32; 300], vec![0.0f32; 200]);
    assert_no_alloc(|| e.process(&mut l, &mut r, &junk));
    assert_no_alloc(|| e.process(&mut [], &mut [], &junk));
    let (mut l, mut r) = (vec![0.0f32; 1], vec![0.0f32; 1]);
    for _ in 0..1000 {
        assert_no_alloc(|| e.process(&mut l, &mut r, &[]));
    }
}

#[test]
fn building_an_engine_may_allocate_but_running_it_must_not() {
    // The negative space of the tests above: compile and Engine::new allocate, outside the guard.
    let patch = Patch::init();
    let mut e = engine(&patch, SEED);
    let (mut l, mut r) = (vec![0.0f32; 64], vec![0.0f32; 64]);
    assert_no_alloc(|| e.process(&mut l, &mut r, &[]));
}

/// Negative control: an allocation inside the guard aborts the process, so run it in a child test process.
#[test]
#[cfg(not(target_family = "wasm"))]
#[allow(
    clippy::disallowed_methods,
    reason = "the negative control re-runs this test binary as a child process, flagged by an environment variable"
)]
fn an_allocation_inside_the_guard_is_detected() {
    if std::env::var_os("ENTH_ENGINE_NOALLOC_CHILD").is_some() {
        assert_no_alloc(|| {
            let v: Vec<f32> = Vec::with_capacity(16);
            std::hint::black_box(v);
        });
        return;
    }
    let exe = std::env::current_exe().expect("test executable path");
    let out = std::process::Command::new(exe)
        .args([
            "an_allocation_inside_the_guard_is_detected",
            "--exact",
            "--nocapture",
        ])
        .env("ENTH_ENGINE_NOALLOC_CHILD", "1")
        .output()
        .expect("spawn child test");
    assert!(
        !out.status.success(),
        "an allocation inside assert_no_alloc was not detected"
    );
}
