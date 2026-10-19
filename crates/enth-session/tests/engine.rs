//! The session against a real engine, through the public API only (lane S2s): undo and redo reach the audio thread,
//! a full command ring neither blocks nor drops, epochs hold end to end, and the fold compares against the value the
//! plan holds.
//!
//! **These tests need lane SK**: `Session::activate`, `pump` and `edit` are SK's, and on the S2s base they are
//! stubs. Every test here is `#[ignore = "needs lane SK"]`; the in-crate suite (`src/tests`) covers the same
//! ground with a hand-built port. They were run against a scratch implementation of the three stubs while S2s was
//! written, and they pass there. **When SK lands: delete [`lane_sk_has_not_landed_yet`], remove the `ignore`
//! attributes, and run them.** A failure then is a bug in SK's glue or a wrong assumption in these tests; read
//! `docs/session.md` (section "Undo, redo and the engine") before changing either.

mod common;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use enth_engine::{Engine, Event, EventKind, PrepareCfg, compile};
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModuleKind, Op, ParamAddr, Patch, Position};
use enth_session::{Activation, HostValues, Session, SessionConfig, SessionError};

use common::{addr, custom_patch, id};

const SR: f32 = 48_000.0;
const CUTOFF: &str = "filter1.cutoff";
const RESONANCE: &str = "filter1.resonance";
const LEVEL: &str = "osc1.level";
/// Slot 2 of `custom_patch`; no edit in the concurrency test touches it, only the host does.
const MASTER: &str = "master.level";

const ACTIVATION: Activation = Activation {
    sample_rate: SR,
    offline: false,
};

fn set(a: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr(a),
        value,
    }
}

fn add_filter2() -> Op {
    Op::AddModule {
        at: Position {
            section: Section::VoiceFx,
            index: 1,
        },
        module: ModuleInstance::new(id("filter2"), ModuleKind::Svf),
    }
}

fn active(patch: Patch) -> (Session, Engine) {
    active_with(patch, SessionConfig::default())
}

fn active_with(patch: Patch, config: SessionConfig) -> (Session, Engine) {
    let mut session = Session::with_config(patch, config);
    let engine = session.activate(ACTIVATION).expect("activate");
    (session, engine)
}

/// Run `blocks` blocks of 64 frames, the events in the first one.
fn run(engine: &mut Engine, blocks: usize, events: &[Event]) {
    let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
    for k in 0..blocks {
        engine.process(&mut l, &mut r, if k == 0 { events } else { &[] });
    }
}

/// The engine's base value of `a`, looked up through the plan the patch compiles to (the same patch gives the same
/// ids; the session's own map is private).
fn base_of(engine: &Engine, patch: &Patch, a: &str) -> f32 {
    let (_, map) = compile(patch, &PrepareCfg::new(SR)).unwrap();
    let id = map.get(&addr(a)).expect("the parameter is in the plan");
    engine.base(id).expect("the engine knows the id")
}

fn has_filter2(session: &Session) -> bool {
    let want = id("filter2");
    session.patch().modules().any(|(_, m)| m.id == want)
}

fn value(session: &Session, a: &str) -> f32 {
    session.patch().value(&addr(a)).unwrap()
}

/// The engine plays exactly what the patch says, for every address.
fn assert_in_step(session: &Session, engine: &Engine, addrs: &[&str]) {
    for a in addrs {
        assert_eq!(
            base_of(engine, session.patch(), a).to_bits(),
            value(session, a).to_bits(),
            "{a}: the engine and the patch disagree"
        );
    }
}

/// When lane SK lands this test fails: that is the signal to run the rest of this file (see the module docs).
#[test]
fn lane_sk_has_not_landed_yet() {
    let mut session = Session::default();
    let Err(err) = session.activate(ACTIVATION) else {
        panic!(
            "lane SK landed: remove the #[ignore] attributes in tests/engine.rs, run them, and delete this test"
        )
    };
    assert!(matches!(
        err,
        SessionError::Unsupported {
            what: "activate",
            lane: "SK"
        }
    ));
}

#[test]
#[ignore = "needs lane SK"]
fn undo_and_redo_reach_the_running_engine() {
    let (mut session, mut engine) = active(custom_patch());
    let start = value(&session, CUTOFF);
    run(&mut engine, 4, &[]);

    session.edit(vec![set(CUTOFF, 2000.0)], None).unwrap();
    run(&mut engine, 4, &[]);
    assert_eq!(base_of(&engine, session.patch(), CUTOFF), 2000.0);

    assert_eq!(session.undo(), Ok(true));
    run(&mut engine, 4, &[]);
    assert_eq!(base_of(&engine, session.patch(), CUTOFF), start);
    assert_in_step(&session, &engine, &[CUTOFF]);

    assert_eq!(session.redo(), Ok(true));
    run(&mut engine, 4, &[]);
    assert_eq!(base_of(&engine, session.patch(), CUTOFF), 2000.0);
    assert_eq!(engine.stats().stale_events, 0);
    assert_eq!(engine.stats().dropped_events, 0);
}

#[test]
#[ignore = "needs lane SK"]
fn a_structural_undo_and_redo_swap_plans_in_order_with_nothing_stale() {
    let (mut session, mut engine) = active(custom_patch());
    run(&mut engine, 4, &[]);
    assert_eq!(engine.epoch(), 0);

    // Structure, then a value on the module it added, in one block's worth of calls: the value must reach the
    // plan that has the module.
    session.edit(vec![add_filter2()], None).unwrap();
    session
        .edit(vec![set("filter2.cutoff", 4321.0)], None)
        .unwrap();
    run(&mut engine, 4, &[]);
    assert_eq!(engine.epoch(), 1);
    assert_eq!(base_of(&engine, session.patch(), "filter2.cutoff"), 4321.0);

    // Undo removes the value edit, then the module: two steps, the second one structural.
    assert_eq!(session.undo(), Ok(true));
    run(&mut engine, 4, &[]);
    assert_eq!(session.undo(), Ok(true));
    run(&mut engine, 4, &[]);
    assert!(!has_filter2(&session));
    assert_eq!(engine.epoch(), 2);
    assert_in_step(&session, &engine, &[CUTOFF, RESONANCE, LEVEL]);

    assert_eq!(session.redo(), Ok(true));
    assert_eq!(session.redo(), Ok(true));
    run(&mut engine, 4, &[]);
    assert_eq!(engine.epoch(), 3);
    assert_eq!(base_of(&engine, session.patch(), "filter2.cutoff"), 4321.0);
    assert_eq!(engine.stats().stale_events, 0, "no event named a dead plan");
    assert_eq!(engine.stats().dropped_events, 0);
}

#[test]
#[ignore = "needs lane SK"]
fn a_drag_is_one_step_and_the_engine_follows_every_event_of_it() {
    let (mut session, mut engine) = active(custom_patch());
    let start = value(&session, CUTOFF);
    let g = session.begin_gesture();
    for k in 0..200 {
        let hz = 500.0 + k as f32 * 7.0;
        session.edit(vec![set(CUTOFF, hz)], Some(g)).unwrap();
        if k % 16 == 0 {
            run(&mut engine, 1, &[]);
        }
    }
    session.end_gesture();
    run(&mut engine, 4, &[]);
    assert_eq!(
        base_of(&engine, session.patch(), CUTOFF),
        500.0 + 199.0 * 7.0
    );

    assert_eq!(session.undo(), Ok(true));
    run(&mut engine, 4, &[]);
    assert_eq!(value(&session, CUTOFF), start, "the whole drag is one step");
    assert_in_step(&session, &engine, &[CUTOFF]);
    assert!(!session.history().can_undo() || value(&session, CUTOFF) == start);
}

/// Nothing runs the engine while the editor works: the command ring (shrunk here) fills. The session neither
/// blocks nor drops: values wait in a bounded queue, the overflow becomes a recompile, and when the audio thread
/// comes back the engine ends where the patch is.
#[test]
#[ignore = "needs lane SK"]
fn a_full_ring_never_blocks_and_never_drops() {
    for value_queue in [256, 2] {
        let mut config = SessionConfig::default();
        config.link.commands = 4;
        config.value_queue = value_queue;
        let (mut session, mut engine) = active_with(custom_patch(), config);
        run(&mut engine, 2, &[]);

        let g = session.begin_gesture();
        for k in 0..300 {
            let f = k as f32;
            let ops = vec![
                set(CUTOFF, 400.0 + f * 11.0),
                set(RESONANCE, 5.0 + f * 0.25),
                set(LEVEL, 10.0 + f * 0.2),
            ];
            session
                .edit(ops, Some(g))
                .expect("an edit never fails for want of room");
            if k % 50 == 49 {
                // The audio thread comes back for a moment.
                run(&mut engine, 1, &[]);
                session.pump();
            }
        }
        session.end_gesture();
        // Let it drain: every pump moves what fits, every block frees the ring.
        for _ in 0..64 {
            session.pump();
            run(&mut engine, 1, &[]);
        }
        assert_in_step(&session, &engine, &[CUTOFF, RESONANCE, LEVEL]);
        assert_eq!(
            engine.stats().dropped_events,
            0,
            "value_queue = {value_queue}"
        );

        // And undo through a full ring is as good as an edit.
        for _ in 0..4 {
            session.pump();
            run(&mut engine, 1, &[]);
        }
        assert_eq!(session.undo(), Ok(true));
        for _ in 0..64 {
            session.pump();
            run(&mut engine, 1, &[]);
        }
        assert_in_step(&session, &engine, &[CUTOFF, RESONANCE, LEVEL]);
    }
}

/// The audio thread runs flat out while the main thread edits, undoes, redoes, folds, pumps and structurally
/// edits. The audio thread never takes a lock the session holds (it only owns the engine), so it keeps rendering:
/// the block counter moves while the main thread works, and at the end both sides agree.
#[test]
#[ignore = "needs lane SK"]
#[allow(
    clippy::disallowed_methods,
    reason = "a test thread stands in for the host's audio thread"
)]
fn the_session_never_stops_the_audio_thread() {
    let (mut session, mut engine) = active(custom_patch());
    let stop = AtomicBool::new(false);
    let blocks = AtomicU64::new(0);
    // The shell's job: what the host wrote to slot 2, as the audio thread sees it.
    let slot_2 = AtomicU32::new(0);
    let mut host = session.host_values();
    session.fold_host(&host);

    let engine = std::thread::scope(|s| {
        let audio = s.spawn(|| {
            let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
            let note = Event {
                offset: 0,
                kind: EventKind::NoteOn {
                    key: enth_engine::NoteKey::new(60, 1),
                    velocity: 0.8,
                },
            };
            engine.process(&mut l, &mut r, &[note]);
            while !stop.load(Ordering::Relaxed) {
                let bits = slot_2.load(Ordering::Relaxed);
                if bits == 0 {
                    engine.process(&mut l, &mut r, &[]);
                } else {
                    let event = Event {
                        offset: 0,
                        kind: EventKind::Slot {
                            slot: 1,
                            value: f32::from_bits(bits),
                        },
                    };
                    engine.process(&mut l, &mut r, &[event]);
                }
                blocks.fetch_add(1, Ordering::Relaxed);
            }
            engine
        });

        let mut last = 0;
        let mut moved = 0u32;
        for k in 0..400u32 {
            let f = k as f32;
            match k % 8 {
                0 => {
                    session.edit(vec![set(CUTOFF, 300.0 + f)], None).unwrap();
                }
                1 => {
                    let _ = session.undo();
                }
                2 => {
                    let _ = session.redo();
                }
                3 => {
                    let g = session.begin_gesture();
                    for j in 0..20 {
                        session
                            .edit(vec![set(RESONANCE, j as f32)], Some(g))
                            .unwrap();
                    }
                    session.end_gesture();
                }
                4 => {
                    let ops = if has_filter2(&session) {
                        vec![Op::RemoveModule { id: id("filter2") }]
                    } else {
                        vec![add_filter2()]
                    };
                    session.edit(ops, None).unwrap();
                }
                5 => {
                    host.slots[1] = (f / 400.0).clamp(0.01, 0.99);
                    slot_2.store(host.slots[1].to_bits(), Ordering::Relaxed);
                    session.fold_host(&host);
                }
                _ => {
                    session.pump();
                }
            }
            // The audio thread made progress since the last look (it is never waiting on us). Yield so a
            // loaded machine gives it a turn; bounded, so a regression fails instead of hanging.
            let mut spins = 0;
            while blocks.load(Ordering::Relaxed) == last && spins < 200_000 {
                std::thread::yield_now();
                spins += 1;
            }
            let now = blocks.load(Ordering::Relaxed);
            assert!(now > last, "the audio thread stopped at step {k}");
            moved += 1;
            last = now;
        }
        assert_eq!(moved, 400);
        stop.store(true, Ordering::Relaxed);
        audio.join().expect("the audio thread did not panic")
    });
    let mut engine = engine;
    // The audio thread is back on this one; let the last events and plan land.
    let last = Event {
        offset: 0,
        kind: EventKind::Slot {
            slot: 1,
            value: host.slots[1],
        },
    };
    let once = [last];
    for k in 0..64 {
        session.pump();
        run(&mut engine, 1, if k == 0 { &once } else { &[] });
    }
    assert_in_step(&session, &engine, &[CUTOFF, RESONANCE, LEVEL, MASTER]);
    assert_eq!(engine.stats().dropped_events, 0);
}

/// F1b review issue 4, end to end: after the host moved the slot away and back to `to_norm` of the value the plan
/// holds, the patch ends on the exact value the engine plays.
#[test]
#[ignore = "needs lane SK"]
fn a_host_returning_to_the_compiled_value_leaves_the_patch_on_what_the_engine_plays() {
    let probe = custom_patch();
    let spec = probe.spec(&addr(CUTOFF)).unwrap();
    let p0 = (300..9000)
        .map(|hz| hz as f32 + 0.5)
        .find(|&v| spec.from_norm(spec.to_norm(v)) != v && spec.to_norm(v) < 0.9)
        .expect("log scales have such values");

    let mut patch = custom_patch();
    enth_patch::apply(&mut patch, set(CUTOFF, p0)).unwrap();
    let (mut session, mut engine) = active(patch);
    let mut host = session.host_values();
    assert!(session.fold_host(&host).baseline);
    let n0 = spec.to_norm(p0);

    let slot_1 = |n: f32| Event {
        offset: 0,
        kind: EventKind::Slot { slot: 0, value: n },
    };
    host.slots[0] = n0 + 0.2;
    session.fold_host(&host);
    run(&mut engine, 3, &[slot_1(n0 + 0.2)]);
    assert_ne!(value(&session, CUTOFF).to_bits(), p0.to_bits());
    assert_in_step(&session, &engine, &[CUTOFF]);

    host.slots[0] = n0;
    session.fold_host(&host);
    run(&mut engine, 3, &[slot_1(n0)]);
    assert_eq!(
        base_of(&engine, session.patch(), CUTOFF).to_bits(),
        p0.to_bits()
    );
    assert_eq!(value(&session, CUTOFF).to_bits(), p0.to_bits());
}

/// A second `activate` (the host re-activates at a new sample rate, with or without a `deactivate` and a fold in
/// between) starts a new engine whose epochs begin at 0 again. What the fold remembered about the old engine's value
/// must not survive: the new plan is compiled from the patch, so it holds the value the fold wrote. (`activate` has
/// to drop `host.memos`; the fold's memo is keyed by epoch, and epoch 0 of the new engine equals epoch 0 of the old.)
#[test]
#[ignore = "needs lane SK"]
fn a_reactivation_forgets_what_the_fold_remembered_about_the_old_engine() {
    let probe = custom_patch();
    let spec = probe.spec(&addr(CUTOFF)).unwrap();
    let p0 = (300..9000)
        .map(|hz| hz as f32 + 0.5)
        .find(|&v| spec.from_norm(spec.to_norm(v)) != v && spec.to_norm(v) < 0.9)
        .expect("log scales have such values");
    let mut patch = custom_patch();
    enth_patch::apply(&mut patch, set(CUTOFF, p0)).unwrap();
    let (mut session, mut engine) = active(patch);
    let mut host = session.host_values();
    assert!(session.fold_host(&host).baseline);
    let n0 = spec.to_norm(p0);

    let slot_1 = |n: f32| Event {
        offset: 0,
        kind: EventKind::Slot { slot: 0, value: n },
    };
    host.slots[0] = n0 + 0.2;
    session.fold_host(&host);
    run(&mut engine, 3, &[slot_1(n0 + 0.2)]);

    // The host re-activates; the new engine starts from the patch (the fold's value) and gets the host's values
    // again as slot events, as the shell does after activation.
    session.deactivate();
    drop(engine);
    let mut engine = session.activate(ACTIVATION).expect("activate");
    run(&mut engine, 3, &[slot_1(n0 + 0.2)]);
    assert_in_step(&session, &engine, &[CUTOFF]);

    host.slots[0] = n0;
    session.fold_host(&host);
    run(&mut engine, 3, &[slot_1(n0)]);
    assert_eq!(
        base_of(&engine, session.patch(), CUTOFF).to_bits(),
        value(&session, CUTOFF).to_bits(),
        "the patch ends on what the new engine plays"
    );
}

/// Rescan requests follow the assignments through undo and redo, with a real engine behind the session.
#[test]
#[ignore = "needs lane SK"]
fn undoing_a_slot_assignment_asks_the_shell_to_rename_and_the_engine_follows() {
    let (mut session, mut engine) = active(custom_patch());
    let _ = session.drain_requests().count();
    let free = enth_session::bridge::free_slot(session.patch()).unwrap();
    session
        .edit(
            vec![Op::SetSlot {
                slot: free,
                addr: Some(addr(LEVEL)),
            }],
            None,
        )
        .unwrap();
    run(&mut engine, 4, &[]);
    assert!(
        session
            .drain_requests()
            .any(|r| r == enth_session::ShellRequest::RenameParams)
    );

    assert_eq!(session.undo(), Ok(true));
    run(&mut engine, 4, &[]);
    assert!(
        session
            .drain_requests()
            .any(|r| r == enth_session::ShellRequest::RenameParams),
        "the name goes back"
    );
    assert!(session.patch().slots[free.get()].is_none());
    assert_eq!(engine.stats().stale_events, 0);
    let _: (ParamAddr, HostValues) = (addr(LEVEL), session.host_values());
}
