//! The engine side of the session (lane SK): activation, the pump, edit delivery against a real engine, audition
//! notes, bounded queues, telemetry maps, and the SK duties from the F1b review (prune the telemetry maps, a stale
//! gesture id is no gesture, an edit clears the kept patch text, a `SetSlot`'s host write only once its plan went
//! out). Every test drives a real `Engine` built by `Session::activate` on this thread, rendering between steps as
//! the audio thread would.

mod common;

use enth_engine::{Engine, Event, EventKind, LinkCapacity, MAX_PLANS_IN_FLIGHT, NoteKey};
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModSource, ModuleKind, Op, ParamAddr, Patch, Position, RouteAddr};
use enth_session::{
    AUDITION_PORT, Activation, GestureStep, HostParam, HostWrite, Session, SessionConfig,
    SessionError, ShellRequest,
};

const SR: f32 = 48_000.0;

fn live() -> Activation {
    Activation {
        sample_rate: SR,
        offline: false,
    }
}

/// Render `frames` frames with no host events (the audio thread's side between session entries).
fn run(engine: &mut Engine, frames: usize) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    engine.process(&mut l, &mut r, &[]);
}

fn active(patch: Patch) -> (Session, Engine) {
    let mut session = Session::new(patch);
    let engine = session.activate(live()).expect("a valid patch activates");
    (session, engine)
}

fn add_filter() -> Op {
    Op::AddModule {
        at: Position {
            section: Section::VoiceFx,
            index: 0,
        },
        module: ModuleInstance::new(common::id("filter9"), ModuleKind::Svf),
    }
}

fn host_writes(session: &mut Session) -> Vec<HostWrite> {
    session
        .drain_requests()
        .filter_map(|r| match r {
            ShellRequest::Host(w) => Some(w),
            _ => None,
        })
        .collect()
}

#[test]
fn activate_builds_an_engine_with_link_tap_and_map() {
    let (session, engine) = active(common::custom_patch());
    let status = session.status();
    assert_eq!(status.active, Some(live()));
    assert_eq!(status.epoch, 0);
    assert_eq!(status.plans_in_flight, 0);
    assert!(!status.compile_pending);
    assert_eq!(engine.epoch(), 0);
    assert_eq!(engine.render_seed(), enth_session::RENDER_SEED);
    assert!(
        engine.tap().is_some(),
        "the tap is attached before the engine leaves"
    );
    assert!(session.telemetry_source().is_some());
    let map = session.telemetry_map(0).expect("the first plan's map");
    assert!(map.param_index(&common::addr("filter1.cutoff")).is_some());
    // The tap starts disabled: the editor enables it.
    assert!(!engine.tap().unwrap().is_enabled());
}

#[test]
fn the_persisted_bypass_is_set_on_the_fresh_engine() {
    let mut session = Session::new(common::custom_patch());
    session.set_monitor_bypass(true);
    let engine = session.activate(live()).unwrap();
    assert!(engine.monitor_bypass());
    assert!(
        !engine.is_bypass_fading(),
        "a fresh engine switches at once"
    );
}

#[test]
fn an_unsupported_sample_rate_is_a_typed_error_and_leaves_the_session_detached() {
    let mut session = Session::default();
    let Err(err) = session.activate(Activation {
        sample_rate: 1.0,
        offline: false,
    }) else {
        panic!("1 Hz is not a supported sample rate")
    };
    assert!(matches!(err, SessionError::Compile(_)), "{err:?}");
    assert!(!session.is_active());
}

#[test]
fn a_value_edit_reaches_the_engine_exactly() {
    let (mut session, mut engine) = active(common::custom_patch());
    let cutoff = common::addr("filter1.cutoff");
    let (plan, map) =
        enth_engine::compile(session.patch(), &enth_engine::PrepareCfg::new(SR)).unwrap();
    drop(plan);
    let id = map.get(&cutoff).unwrap();
    let applied = session
        .edit(
            vec![Op::SetParam {
                addr: cutoff.clone(),
                value: 777.25,
            }],
            None,
        )
        .unwrap();
    assert_eq!(applied.events, 1);
    assert!(!applied.recompile);
    run(&mut engine, 64);
    assert_eq!(engine.base(id), Some(777.25));
    assert_eq!(session.patch().value(&cutoff), Some(777.25));
    assert!(session.history().can_undo());
}

#[test]
fn a_structural_edit_sends_one_plan_and_later_values_use_its_epoch() {
    let (mut session, mut engine) = active(common::custom_patch());
    let applied = session.edit(vec![add_filter()], None).unwrap();
    assert!(applied.recompile);
    assert_eq!(session.status().epoch, 1);
    assert!(!session.status().compile_pending);
    run(&mut engine, 64);
    assert_eq!(engine.epoch(), 1);

    let cutoff: ParamAddr = "filter9.cutoff".parse().unwrap();
    session
        .edit(
            vec![Op::SetParam {
                addr: cutoff.clone(),
                value: 3210.0,
            }],
            None,
        )
        .unwrap();
    run(&mut engine, 64);
    let (_, map) =
        enth_engine::compile(session.patch(), &enth_engine::PrepareCfg::new(SR)).unwrap();
    assert_eq!(engine.base(map.get(&cutoff).unwrap()), Some(3210.0));
    assert_eq!(engine.stats().stale_events, 0);
}

#[test]
fn edits_coalesce_while_two_plans_are_in_flight() {
    let (mut session, mut engine) = active(common::custom_patch());
    // No audio runs, so nothing comes back: the third recompile waits.
    for _ in 0..MAX_PLANS_IN_FLIGHT {
        session.edit(vec![add_filter()], None).unwrap();
        session
            .edit(
                vec![Op::RemoveModule {
                    id: common::id("filter9"),
                }],
                None,
            )
            .unwrap();
    }
    let status = session.status();
    assert_eq!(status.plans_in_flight, MAX_PLANS_IN_FLIGHT);
    assert!(status.compile_pending);
    assert_eq!(status.epoch, MAX_PLANS_IN_FLIGHT as u64);
    // More edits change nothing for the link: still one pending compile.
    session.edit(vec![add_filter()], None).unwrap();
    assert_eq!(session.status().epoch, MAX_PLANS_IN_FLIGHT as u64);

    // The audio thread swaps both plans in and hands them back; the next pump sends exactly one plan, compiled from
    // the patch as it is now (with filter9).
    run(&mut engine, 64 * 20);
    let report = session.pump();
    assert_eq!(report.collected, MAX_PLANS_IN_FLIGHT);
    assert_eq!(report.sent_plan, Some(MAX_PLANS_IN_FLIGHT as u64 + 1));
    assert!(!session.status().compile_pending);
    run(&mut engine, 64 * 20);
    assert_eq!(engine.epoch(), MAX_PLANS_IN_FLIGHT as u64 + 1);
    let (_, map) =
        enth_engine::compile(session.patch(), &enth_engine::PrepareCfg::new(SR)).unwrap();
    let cutoff: ParamAddr = "filter9.cutoff".parse().unwrap();
    assert!(map.get(&cutoff).is_some());
    assert!(engine.base(map.get(&cutoff).unwrap()).is_some());
}

#[test]
fn telemetry_maps_are_pruned_to_the_plans_the_tap_may_publish() {
    let (mut session, mut engine) = active(common::custom_patch());
    for i in 0..10 {
        let op = if i % 2 == 0 {
            add_filter()
        } else {
            Op::RemoveModule {
                id: common::id("filter9"),
            }
        };
        session.edit(vec![op], None).unwrap();
        run(&mut engine, 64 * 20);
    }
    let epoch = session.status().epoch;
    assert_eq!(epoch, 10);
    assert!(session.telemetry_map(epoch).is_some());
    let kept = (0..=epoch)
        .filter(|e| session.telemetry_map(*e).is_some())
        .count();
    assert_eq!(kept, MAX_PLANS_IN_FLIGHT + 1);
    assert!(session.telemetry_map(0).is_none());
}

#[test]
fn a_stale_gesture_id_counts_as_no_gesture() {
    let (mut session, _engine) = active(common::custom_patch());
    let stale = session.begin_gesture();
    let saved = session.save(None);
    session.load(&saved);
    let _ = session.drain_requests().count();
    // The load cleared the history and closed the gesture: the old id names nothing.
    let applied = session
        .edit(
            vec![Op::SetParam {
                addr: common::addr("filter1.cutoff"),
                value: 500.0,
            }],
            Some(stale),
        )
        .unwrap();
    assert_eq!(applied.host_writes, 1);
    let slot1 = HostParam::Slot(common::slot(1));
    assert_eq!(
        host_writes(&mut session),
        vec![
            HostWrite {
                param: slot1,
                step: GestureStep::Begin
            },
            HostWrite {
                param: slot1,
                step: GestureStep::Set(
                    common::custom_patch()
                        .spec(&common::addr("filter1.cutoff"))
                        .unwrap()
                        .to_norm(500.0)
                )
            },
            HostWrite {
                param: slot1,
                step: GestureStep::End
            },
        ]
    );
    // Its own undo step, not merged into a gesture.
    assert_eq!(session.history().next_undo().unwrap().gesture, None);
    assert!(!session.status().gesture_open);
}

#[test]
fn a_gesture_opens_one_host_gesture_per_parameter() {
    let (mut session, _engine) = active(common::custom_patch());
    let g = session.begin_gesture();
    for v in [400.0, 600.0, 800.0] {
        session
            .edit(
                vec![Op::SetParam {
                    addr: common::addr("filter1.cutoff"),
                    value: v,
                }],
                Some(g),
            )
            .unwrap();
    }
    session.end_gesture();
    let steps: Vec<GestureStep> = host_writes(&mut session)
        .into_iter()
        .map(|w| w.step)
        .collect();
    assert_eq!(steps.len(), 5, "{steps:?}");
    assert_eq!(steps[0], GestureStep::Begin);
    assert!(matches!(
        steps[1..4],
        [
            GestureStep::Set(_),
            GestureStep::Set(_),
            GestureStep::Set(_)
        ]
    ));
    assert_eq!(steps[4], GestureStep::End);
    assert!(session.history().can_undo());
    let step = session.history().next_undo().unwrap();
    assert_eq!(step.forward.len(), 1, "one merged undo step: {step:?}");
    assert_eq!(step.gesture, Some(g));
}

#[test]
fn an_edit_clears_the_kept_text_of_a_patch_that_did_not_load() {
    let (mut session, _engine) = active(common::custom_patch());
    let report = session.load_patch("{\"schema\": 999}");
    assert!(report.fell_back);
    assert!(session.status().patch_unloaded);
    assert_eq!(session.save_patch(None), "{\"schema\": 999}");
    let osc = session.patch().generators[0].id.clone();
    session
        .edit(
            vec![Op::SetParam {
                addr: ParamAddr::module(osc, "level").unwrap(),
                value: 40.0,
            }],
            None,
        )
        .unwrap();
    assert!(!session.status().patch_unloaded);
    let saved = session.save_patch(None);
    assert_ne!(saved, "{\"schema\": 999}");
    assert!(enth_patch::file::load(&saved).is_ok());
}

#[test]
fn a_set_slot_writes_the_host_only_once_its_plan_went_out() {
    let (mut session, mut engine) = active(common::custom_patch());
    // Fill the link: two plans in flight, nothing processed.
    session.edit(vec![add_filter()], None).unwrap();
    session
        .edit(
            vec![Op::RemoveModule {
                id: common::id("filter9"),
            }],
            None,
        )
        .unwrap();
    let _ = session.drain_requests().count();
    let slot5 = common::slot(5);
    let applied = session
        .edit(
            vec![Op::SetSlot {
                slot: slot5,
                addr: Some(common::addr("filter1.resonance")),
            }],
            None,
        )
        .unwrap();
    assert!(applied.recompile);
    assert!(session.status().compile_pending);
    assert_eq!(applied.host_writes, 0);
    assert_eq!(host_writes(&mut session), Vec::new());
    assert!(session.drain_requests().all(|r| r
        != ShellRequest::Host(HostWrite {
            param: HostParam::Slot(slot5),
            step: GestureStep::Begin
        })));

    // With room on the link the next reassignment's plan goes out in the same entry, and its write follows.
    run(&mut engine, 64 * 20);
    session.pump();
    run(&mut engine, 64 * 20);
    session.pump();
    let applied = session
        .edit(
            vec![Op::SetSlot {
                slot: slot5,
                addr: Some(common::addr("filter1.cutoff")),
            }],
            None,
        )
        .unwrap();
    assert!(!session.status().compile_pending);
    assert_eq!(applied.host_writes, 1);
    let writes = host_writes(&mut session);
    assert_eq!(writes.len(), 3);
    assert!(writes.iter().all(|w| w.param == HostParam::Slot(slot5)));
}

#[test]
fn a_depth_edit_is_an_event_and_a_change_from_zero_is_a_plan() {
    let (mut session, mut engine) = active(common::custom_patch());
    let route = RouteAddr {
        src: ModSource::Module(common::id("lfo1")),
        dst: common::addr("filter1.cutoff"),
    };
    let applied = session
        .edit(
            vec![Op::SetDepth {
                route: route.clone(),
                depth: 60.0,
            }],
            None,
        )
        .unwrap();
    assert_eq!(applied.events, 1);
    assert!(!applied.recompile);
    let applied = session
        .edit(
            vec![Op::SetDepth {
                route: route.clone(),
                depth: 0.0,
            }],
            None,
        )
        .unwrap();
    assert!(applied.recompile);
    run(&mut engine, 64 * 20);
    let applied = session
        .edit(vec![Op::SetDepth { route, depth: 10.0 }], None)
        .unwrap();
    assert!(applied.recompile);
    assert_eq!(engine.stats().stale_events, 0);
}

#[test]
fn a_macro_edit_is_an_event_and_a_host_write() {
    let (mut session, _engine) = active(common::custom_patch());
    let index = common::macro_index(3);
    let applied = session
        .edit(vec![Op::SetMacro { index, value: 0.75 }], None)
        .unwrap();
    assert_eq!(applied.events, 1);
    let writes = host_writes(&mut session);
    assert_eq!(
        writes[1],
        HostWrite {
            param: HostParam::Macro(index),
            step: GestureStep::Set(0.75)
        }
    );
}

#[test]
fn a_rejected_edit_changes_and_delivers_nothing() {
    let (mut session, _engine) = active(common::custom_patch());
    let before = session.patch().clone();
    let err = session
        .edit(
            vec![Op::SetParam {
                addr: common::addr("filter1.cutoff"),
                value: f32::NAN,
            }],
            None,
        )
        .unwrap_err();
    assert!(matches!(err, SessionError::Edit(_)));
    assert!(enth_patch::canon::identical(session.patch(), &before));
    assert_eq!(session.drain_requests().count(), 0);
    assert!(!session.history().can_undo());
}

#[test]
fn edits_while_detached_change_the_patch_and_activate_compiles_it() {
    let mut session = Session::new(common::custom_patch());
    session.edit(vec![add_filter()], None).unwrap();
    let engine = session.activate(live()).unwrap();
    let map = session.telemetry_map(0).unwrap();
    assert!(engine.plan().tap_params().len().ge(&map.params.len()));
    let (_, plan_map) =
        enth_engine::compile(session.patch(), &enth_engine::PrepareCfg::new(SR)).unwrap();
    assert!(plan_map.get(&"filter9.cutoff".parse().unwrap()).is_some());
}

#[test]
fn a_full_ring_queues_values_and_an_overflow_becomes_a_plan() {
    let config = SessionConfig {
        link: LinkCapacity {
            commands: 4,
            returns: 4,
        },
        value_queue: 8,
        ..SessionConfig::default()
    };
    let mut session = Session::with_config(common::custom_patch(), config);
    let mut engine = session.activate(live()).unwrap();
    let cutoff = common::addr("filter1.cutoff");
    let resonance = common::addr("filter1.resonance");
    // Four fill the ring, the rest queue (the last per parameter kept), then the queue overflows.
    for i in 0..40 {
        let addr = if i % 2 == 0 { &cutoff } else { &resonance };
        let value = if i % 2 == 0 {
            200.0 + i as f32
        } else {
            i as f32
        };
        session
            .edit(
                vec![Op::SetParam {
                    addr: addr.clone(),
                    value,
                }],
                None,
            )
            .unwrap();
    }
    let status = session.status();
    assert!(status.queued_events <= 2, "{status:?}");
    // Drain: the engine empties the ring every segment, the session refills it.
    for _ in 0..10 {
        run(&mut engine, 64);
        session.pump();
    }
    run(&mut engine, 64 * 20);
    session.pump();
    run(&mut engine, 64 * 20);
    let (_, map) =
        enth_engine::compile(session.patch(), &enth_engine::PrepareCfg::new(SR)).unwrap();
    assert_eq!(
        engine.base(map.get(&cutoff).unwrap()),
        session.patch().value(&cutoff)
    );
    assert_eq!(
        engine.base(map.get(&resonance).unwrap()),
        session.patch().value(&resonance)
    );
    assert_eq!(session.status().queued_events, 0);
}

#[test]
fn audition_notes_play_and_release_and_are_refused_offline() {
    let (mut session, mut engine) = active(Patch::init());
    let note = session.audition_on(60, 0.8).unwrap();
    assert_eq!(note.port, AUDITION_PORT);
    assert_eq!(note.key, 60);
    assert!(note.note_id >= 0);
    let second = session.audition_on(60, 0.8).unwrap();
    assert_ne!(note.note_id, second.note_id, "a fresh id per note");
    run(&mut engine, 256);
    assert_eq!(engine.active_voices().len(), 2);
    session.audition_off(note).unwrap();
    session.audition_all_off().unwrap();
    let mut ended = Vec::new();
    for _ in 0..200 {
        run(&mut engine, 256);
        ended.extend(engine.note_ends().iter().map(|e| e.key));
        if engine.active_voices().is_empty() {
            break;
        }
    }
    assert_eq!(engine.active_voices().len(), 0);
    assert_eq!(ended.len(), 2);
    assert!(ended.iter().all(|k| k.port == AUDITION_PORT));

    let mut offline = Session::default();
    let _engine = offline
        .activate(Activation {
            sample_rate: SR,
            offline: true,
        })
        .unwrap();
    assert_eq!(offline.audition_on(60, 1.0), Err(SessionError::Offline));
}

#[test]
fn a_full_audition_queue_refuses_note_ons_and_never_drops_note_offs() {
    let config = SessionConfig {
        link: LinkCapacity {
            commands: 2,
            returns: 4,
        },
        audition_queue: 2,
        ..SessionConfig::default()
    };
    let mut session = Session::with_config(Patch::init(), config);
    let mut engine = session.activate(live()).unwrap();
    let a = session.audition_on(60, 1.0).unwrap();
    let b = session.audition_on(62, 1.0).unwrap();
    let c = session.audition_on(64, 1.0).unwrap();
    let d = session.audition_on(65, 1.0).unwrap();
    assert_eq!(session.audition_on(67, 1.0), Err(SessionError::Busy));
    for note in [a, b, c, d] {
        session.audition_off(note).unwrap();
    }
    assert_eq!(session.status().queued_events, 6);
    for _ in 0..20 {
        run(&mut engine, 64);
        session.pump();
    }
    assert_eq!(session.status().queued_events, 0);
    for _ in 0..200 {
        run(&mut engine, 256);
    }
    assert_eq!(engine.active_voices().len(), 0, "every note-off arrived");
}

#[test]
fn a_load_while_active_sends_its_plan_and_forgets_host_slots() {
    let (mut session, mut engine) = active(common::custom_patch());
    // The host automates slot 1 (filter1.cutoff): the engine remembers the value.
    let slot = [Event {
        offset: 0,
        kind: EventKind::Slot {
            slot: 0,
            value: 0.9,
        },
    }];
    let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
    engine.process(&mut l, &mut r, &slot);
    assert_eq!(engine.host_slot(0), Some(0.9));
    let saved = session.save(None);
    session.load(&saved);
    assert_eq!(
        session.status().epoch,
        1,
        "the load's plan went out at once"
    );
    run(&mut engine, 64 * 4);
    assert_eq!(engine.host_slot(0), None, "the loaded patch wins");
    let (_, map) =
        enth_engine::compile(session.patch(), &enth_engine::PrepareCfg::new(SR)).unwrap();
    let cutoff = common::addr("filter1.cutoff");
    assert_eq!(
        engine.base(map.get(&cutoff).unwrap()),
        session.patch().value(&cutoff)
    );
}

#[test]
fn a_session_engine_renders_like_enth_render() {
    let patch = common::custom_patch();
    let perf = enth_render::Performance {
        notes: vec![
            enth_render::Note {
                key: 57,
                velocity: 100.0 / 127.0,
                on_s: 0.0103,
                off_s: 0.3,
            },
            enth_render::Note {
                key: 64,
                velocity: 1.0,
                on_s: 0.05,
                off_s: 0.41,
            },
        ],
        duration_s: 0.6,
        ..Default::default()
    };
    let spec = enth_render::RenderSpec::default();
    let reference = enth_render::render(&patch, &perf, &spec).unwrap();

    let mut session = Session::new(patch);
    let mut engine = session
        .activate(Activation {
            sample_rate: spec.sample_rate as f32,
            offline: true,
        })
        .unwrap();
    let frames = reference.left.len();
    let frame = |t: f64| (t * f64::from(spec.sample_rate)).round() as u64;
    let mut events: Vec<(u64, EventKind)> = Vec::new();
    for (i, n) in perf.notes.iter().enumerate() {
        let key = NoteKey::new(n.key, i as i32);
        events.push((
            frame(n.on_s),
            EventKind::NoteOn {
                key,
                velocity: n.velocity,
            },
        ));
        events.push((frame(n.off_s), EventKind::NoteOff { key, velocity: 0.0 }));
    }
    events.sort_by_key(|e| e.0);
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    let mut at = 0usize;
    let blocks = [1usize, 37, 64, 100, 333, 4096];
    let mut b = 0;
    while at < frames {
        let n = blocks[b % blocks.len()].min(frames - at);
        b += 1;
        let block: Vec<Event> = events
            .iter()
            .filter(|(f, _)| (*f as usize) >= at && (*f as usize) < at + n)
            .map(|(f, kind)| Event {
                offset: (*f as usize - at) as u32,
                kind: *kind,
            })
            .collect();
        engine.process(&mut left[at..at + n], &mut right[at..at + n], &block);
        at += n;
    }
    assert!(left.iter().any(|s| *s != 0.0));
    assert_eq!(left, reference.left);
    assert_eq!(right, reference.right);
}
