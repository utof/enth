//! How a transaction reaches the engine, the host and the shell (lane S2s): the delivery per op kind, epochs end to
//! end, one plan per structural step, and the bounded queue behind a full ring.

use enth_engine::LinkCapacity;
use enth_patch::model::{ModuleInstance, Route, Section};
use enth_patch::{MacroIndex, ModSource, ModuleKind, Op, Position, RouteAddr, SlotIndex};

use super::kit::{
    addr, base, block_plans, edit, id, live, live_with, patch, port, run, send_plan, settle,
};
use crate::apply::send_or_queue;
use crate::link::Queued;
use crate::{Applied, Session, SessionConfig, ShellRequest};

fn slot(n: usize) -> SlotIndex {
    SlotIndex::from_one_based(n).unwrap()
}

fn macro1() -> MacroIndex {
    MacroIndex::from_one_based(1).unwrap()
}

fn set(a: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr(a),
        value,
    }
}

fn lfo_route() -> RouteAddr {
    RouteAddr {
        src: ModSource::Module(id("lfo1")),
        dst: addr("filter1.cutoff"),
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

/// What a delivery did, as a comparable tuple: (events, recompile, host parameters set, rename requested).
type Summary = (u32, bool, u32, bool);

/// A row of the delivery table: a name, the transaction, and what it must do.
type Row = (&'static str, Vec<Op>, Summary);

/// Reduce an [`Applied`] and the requests it left to a [`Summary`].
fn summary(applied: Applied, session: &mut Session) -> Summary {
    let renamed = session
        .drain_requests()
        .any(|r| r == ShellRequest::RenameParams);
    (
        applied.events,
        applied.recompile,
        applied.host_writes,
        renamed,
    )
}

/// The delivery table, one row per op kind (lane S2s acceptance: "a table-driven test per Op"), against a live
/// session, with the plan blocked so a `SetSlot`'s host write is withheld as documented.
#[test]
fn every_op_kind_reaches_the_engine_the_host_and_the_shell_as_documented() {
    #[rustfmt::skip]
    let table: Vec<Row> = vec![
        // (name, ops, (events, recompile, host params set, rename))
        ("SetParam, slotted", vec![set("filter1.cutoff", 3000.0)], (1, false, 1, false)),
        ("SetParam, unslotted", vec![set("voice.glide", 50.0)], (1, false, 0, false)),
        ("SetParam, built-in slotted", vec![set("master.level", -3.0)], (1, false, 1, false)),
        ("SetDepth, non-zero to non-zero", vec![Op::SetDepth { route: lfo_route(), depth: 60.0 }], (1, false, 0, false)),
        ("SetDepth, to zero", vec![Op::SetDepth { route: lfo_route(), depth: 0.0 }], (0, true, 0, false)),
        ("SetMacro", vec![Op::SetMacro { index: macro1(), value: 0.4 }], (1, false, 1, false)),
        ("AddModule (no slot drives it: no new name)", vec![add_filter2()], (0, true, 0, false)),
        ("RemoveModule", vec![Op::RemoveModule { id: id("lfo1") }], (0, true, 0, false)),
        ("RemoveModule with slots", vec![Op::RemoveModule { id: id("filter1") }], (0, true, 0, true)),
        ("Move", vec![add_filter2(), Op::Move { id: id("filter2"), to: Position { section: Section::VoiceFx, index: 0 } }], (0, true, 0, false)),
        ("InsertRoute", vec![Op::InsertRoute { index: 1, route: Route { src: ModSource::Velocity, dst: addr("osc1.level"), depth: 30.0 } }], (0, true, 0, false)),
        ("RemoveRoute", vec![Op::RemoveRoute { route: lfo_route() }], (0, true, 0, false)),
        ("SetEnabled", vec![Op::SetEnabled { id: id("filter1"), enabled: false }], (0, true, 0, false)),
        ("SetAmpEnv", vec![Op::AddModule { at: Position { section: Section::Modulators, index: 2 }, module: ModuleInstance::new(id("env2"), ModuleKind::Adsr) }, Op::SetAmpEnv { id: id("env2") }], (0, true, 0, false)),
        ("SetSlot, assign (no host write while the plan waits)", vec![Op::SetSlot { slot: slot(4), addr: Some(addr("voice.glide")) }], (0, true, 0, true)),
        ("SetSlot, clear", vec![Op::SetSlot { slot: slot(1), addr: None }], (0, true, 0, true)),
        ("SetLabel, a module a slot drives", vec![Op::SetLabel { id: id("filter1"), label: Some("Tone".to_owned()) }], (0, false, 0, true)),
        ("SetLabel, a module no slot drives", vec![Op::SetLabel { id: id("osc1"), label: Some("Saw".to_owned()) }], (0, false, 0, false)),
        ("SetMacroName", vec![Op::SetMacroName { index: macro1(), name: Some("Bright".to_owned()) }], (0, false, 0, true)),
        ("SetMeta", vec![Op::SetMeta { meta: enth_patch::PatchMeta { name: "x".to_owned(), ..enth_patch::PatchMeta::default() } }], (0, false, 0, false)),
        ("a recompile skips the value events of its transaction, not the host write", vec![set("filter1.cutoff", 900.0), add_filter2()], (0, true, 1, false)),
    ];
    for (name, ops, expected) in table {
        let (mut session, _engine) = live(patch());
        block_plans(&mut session);
        let applied = edit(&mut session, ops);
        assert_eq!(summary(applied, &mut session), expected, "{name}");
        assert_eq!(
            session.status().compile_pending,
            expected.1,
            "{name}: a plan is pending exactly when one is needed"
        );
        assert_eq!(session.status().last_error, None, "{name}");
    }
}

#[test]
fn a_recompile_forgets_the_host_values_of_the_slots_it_writes_and_only_those() {
    let (mut session, _engine) = live(patch());
    block_plans(&mut session);
    // Slot 1 drives filter1.cutoff, slot 3 filter1.resonance, slot 2 master.level.
    edit(
        &mut session,
        vec![
            set("filter1.cutoff", 900.0),
            set("master.level", -4.0),
            add_filter2(),
        ],
    );
    assert_eq!(port(&session).forget_slots, 0b011);
    assert!(port(&session).compile_pending);
    assert!(port(&session).queue.is_empty(), "nothing waits");
}

#[test]
fn an_edit_and_its_undo_reach_the_engine_through_the_latest_plan_and_epoch() {
    let (mut session, mut engine) = live(patch());
    let old_id = port(&session).map.get(&addr("osc1.level")).unwrap();
    // A structural edit that moves every id: a generator ahead of osc1.
    edit(
        &mut session,
        vec![Op::AddModule {
            at: Position {
                section: Section::Generators,
                index: 0,
            },
            module: ModuleInstance::new(id("osc2"), ModuleKind::Osc),
        }],
    );
    assert_eq!(settle(&mut session), 1);
    run(&mut session, &mut engine, 20);
    assert_eq!(engine.epoch(), 1, "the engine swapped");
    assert_ne!(
        port(&session).map.get(&addr("osc1.level")),
        Some(old_id),
        "the structural edit moved osc1's ids, so a stale map would hit the wrong parameter"
    );

    let level = session.patch().value(&addr("osc1.level")).unwrap();
    edit(&mut session, vec![set("osc1.level", level - 6.0)]);
    run(&mut session, &mut engine, 20);
    assert_eq!(base(&session, &engine, "osc1.level"), level - 6.0);
    assert_eq!(engine.stats().stale_events, 0, "sent with the new epoch");
    assert_eq!(session.undo(), Ok(true));
    run(&mut session, &mut engine, 20);
    assert_eq!(base(&session, &engine, "osc1.level"), level);
    assert_eq!(
        base(&session, &engine, "osc2.level"),
        level,
        "osc2 is untouched"
    );
    assert_eq!(engine.stats().stale_events, 0);
    assert_eq!(session.redo(), Ok(true));
    run(&mut session, &mut engine, 20);
    assert_eq!(base(&session, &engine, "osc1.level"), level - 6.0);
}

#[test]
fn undoing_a_structural_step_is_exactly_one_plan() {
    let (mut session, mut engine) = live(patch());
    edit(
        &mut session,
        vec![
            Op::AddModule {
                at: Position {
                    section: Section::Modulators,
                    index: 2,
                },
                module: ModuleInstance::new(id("env2"), ModuleKind::Adsr),
            },
            Op::SetAmpEnv { id: id("env2") },
            set("voice.glide", 40.0),
        ],
    );
    assert_eq!(settle(&mut session), 1);
    run(&mut session, &mut engine, 20);

    assert_eq!(session.undo(), Ok(true));
    assert_eq!(settle(&mut session), 2, "one plan, not one per op");
    assert!(!session.status().compile_pending);
    assert_eq!(settle(&mut session), 2, "and nothing more is pending");
    run(&mut session, &mut engine, 20);
    assert_eq!(engine.epoch(), 2);

    assert_eq!(session.redo(), Ok(true));
    assert_eq!(settle(&mut session), 3);
    assert_eq!(settle(&mut session), 3);
}

#[test]
fn several_ops_on_one_parameter_send_its_final_value_once() {
    let (mut session, mut engine) = live(patch());
    let applied = edit(
        &mut session,
        vec![set("filter1.cutoff", 1000.0), set("filter1.cutoff", 2500.0)],
    );
    assert_eq!(applied.events, 1);
    assert_eq!(applied.host_writes, 1);
    run(&mut session, &mut engine, 10);
    assert_eq!(base(&session, &engine, "filter1.cutoff"), 2500.0);
}

#[test]
fn a_depth_the_plan_has_no_term_for_becomes_a_plan() {
    let (mut session, mut engine) = live(patch());
    // lfo1 off: the next plan has no term for its route.
    edit(
        &mut session,
        vec![Op::SetEnabled {
            id: id("lfo1"),
            enabled: false,
        }],
    );
    assert_eq!(settle(&mut session), 1);
    run(&mut session, &mut engine, 10);
    assert!(
        port(&session).map.depth(&lfo_route()).is_none(),
        "no depth id for a disabled source"
    );
    // `ops::effect` still calls this a depth change, so the session must not drop it: it recompiles.
    let before = session.patch().clone();
    let op = Op::SetDepth {
        route: lfo_route(),
        depth: 60.0,
    };
    enth_patch::apply(&mut session.patch, &op).unwrap();
    let applied = session.deliver(&before, &[op], false);
    assert!(applied.recompile);
    assert_eq!(applied.events, 0);
    assert_eq!(settle(&mut session), 2, "a plan carries the depth");
}

/// Never blocks, never drops (invariant I5), at this layer: a full command ring queues the last value per parameter
/// behind earlier ones, a queue that outgrows its bound becomes a plan (the patch carries every value), audition
/// note-offs in the queue are never dropped, and nothing waits for the audio thread.
#[test]
fn a_full_ring_queues_values_and_an_overflow_becomes_a_plan() {
    let config = SessionConfig {
        link: LinkCapacity {
            commands: 2,
            returns: 4,
        },
        value_queue: 2,
        ..SessionConfig::default()
    };
    let (mut session, mut engine) = live_with(patch(), config);
    let one = |session: &mut Session, a: &str, v: f32| edit(session, vec![set(a, v)]);

    // The ring takes two commands; the engine is not running.
    assert_eq!(one(&mut session, "filter1.cutoff", 1000.0).events, 1);
    assert_eq!(one(&mut session, "filter1.resonance", 20.0).events, 1);
    assert_eq!(port(&session).link.free_slots(), 0);
    assert!(port(&session).queue.is_empty(), "nothing waits");
    // Held back, not lost, not blocking: the last value per parameter wins.
    one(&mut session, "voice.glide", 10.0);
    one(&mut session, "voice.glide", 30.0);
    one(&mut session, "master.level", -2.0);
    assert_eq!(session.status().queued_events, 2);
    assert!(
        matches!(port(&session).queue[0], Queued::Param { value, .. } if value == 30.0),
        "{:?}",
        port(&session).queue
    );
    assert!(!session.status().compile_pending);
    // A note-off waits in the same queue and must survive an overflow.
    let key = enth_engine::NoteKey::new(60, 1);
    session
        .port
        .as_mut()
        .unwrap()
        .queue
        .push(Queued::NoteOff { key });
    // One distinct parameter too many: the queue of values is dropped, a plan is pending, the note-off stays.
    let applied = one(&mut session, "osc1.level", 55.0);
    assert!(applied.recompile);
    assert!(session.status().compile_pending);
    assert_eq!(port(&session).queue, vec![Queued::NoteOff { key }]);
    assert_eq!(port(&session).forget_slots, u64::MAX);

    // Once the engine runs, the ring has room again and the plan (compiled from the patch) carries every value.
    run(&mut session, &mut engine, 5);
    assert_eq!(port(&session).link.free_slots(), 2);
    assert_eq!(send_plan(&mut session), Some(1));
    run(&mut session, &mut engine, 20);
    for (a, v) in [
        ("filter1.cutoff", 1000.0),
        ("filter1.resonance", 20.0),
        ("voice.glide", 30.0),
        ("master.level", -2.0),
        ("osc1.level", 55.0),
    ] {
        assert_eq!(base(&session, &engine, a), v, "{a}");
    }
    assert_eq!(
        port(&session).queue,
        vec![Queued::NoteOff { key }],
        "still waiting for the pump"
    );
}

#[test]
fn events_wait_behind_earlier_ones_so_order_is_kept() {
    let config = SessionConfig {
        link: LinkCapacity {
            commands: 1,
            returns: 4,
        },
        ..SessionConfig::default()
    };
    let (mut session, mut engine) = live_with(patch(), config);
    // Straight at the rule (`send_or_queue`): an entry pumps first, which would empty the queue before the next
    // command could be seen waiting behind it.
    let cap = session.config.value_queue;
    let param = |session: &Session, a: &str, value: f32| Queued::Param {
        epoch: 0,
        id: port(session).map.get(&addr(a)).unwrap(),
        value,
    };
    let command = param(&session, "voice.glide", 10.0);
    assert!(send_or_queue(session.port.as_mut().unwrap(), cap, command));
    let command = param(&session, "voice.glide", 20.0);
    assert!(send_or_queue(session.port.as_mut().unwrap(), cap, command)); // the ring is full: queued
    assert_eq!(session.status().queued_events, 1);
    run(&mut session, &mut engine, 5);
    // The ring has room, but an event is already waiting, so a new one goes behind it rather than past it.
    let command = param(&session, "master.level", -1.0);
    assert!(send_or_queue(session.port.as_mut().unwrap(), cap, command));
    assert_eq!(session.status().queued_events, 2, "behind the waiting one");
    assert!(matches!(port(&session).queue[0], Queued::Param { value, .. } if value == 20.0));
}

#[test]
fn a_value_for_a_parameter_the_plan_lacks_waits_for_the_next_plan() {
    let (mut session, mut engine) = live(patch());
    edit(
        &mut session,
        vec![Op::SetEnabled {
            id: id("lfo1"),
            enabled: false,
        }],
    );
    assert_eq!(settle(&mut session), 1);
    run(&mut session, &mut engine, 10);
    assert_eq!(port(&session).map.get(&addr("lfo1.rate")), None);
    let applied = edit(&mut session, vec![set("lfo1.rate", 5.0)]);
    assert_eq!((applied.events, applied.recompile), (0, false));
    assert!(!session.status().compile_pending);
    // The edit is in the patch; enabling the module is structural, and that plan carries it.
    edit(
        &mut session,
        vec![Op::SetEnabled {
            id: id("lfo1"),
            enabled: true,
        }],
    );
    assert_eq!(settle(&mut session), 2);
    run(&mut session, &mut engine, 10);
    assert_eq!(base(&session, &engine, "lfo1.rate"), 5.0);
}
