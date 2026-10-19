//! Undo and redo (lane S2s): round trips over every op kind, gestures, the patch-model property, failure, and the
//! requests an undo makes for the host.

use enth_patch::canon::identical;
use enth_patch::model::{ModuleInstance, Route, Section};
use enth_patch::{MacroIndex, ModSource, ModuleKind, Op, Patch, Position, RouteAddr, SlotIndex};
use proptest::prelude::*;

use super::kit::{addr, edit, edit_in, id, live, patch};
use crate::bridge::{GestureStep, HostParam, HostWrite};
use crate::{Session, SessionError, ShellRequest};

fn macro1() -> MacroIndex {
    MacroIndex::from_one_based(1).unwrap()
}

fn slot(n: usize) -> SlotIndex {
    SlotIndex::from_one_based(n).unwrap()
}

fn lfo_route() -> RouteAddr {
    RouteAddr {
        src: ModSource::Module(id("lfo1")),
        dst: addr("filter1.cutoff"),
    }
}

/// One transaction per op kind, each valid after the ones before it, on [`patch`].
fn script() -> Vec<Vec<Op>> {
    let at = |section, index| Position { section, index };
    vec![
        vec![Op::SetParam {
            addr: addr("filter1.cutoff"),
            value: 2000.0,
        }],
        vec![Op::SetParam {
            addr: addr("voice.glide"),
            value: 120.0,
        }],
        vec![Op::SetMacro {
            index: macro1(),
            value: 0.5,
        }],
        vec![Op::SetMacroName {
            index: macro1(),
            name: Some("Brightness".to_owned()),
        }],
        vec![Op::SetLabel {
            id: id("filter1"),
            label: Some("Tone".to_owned()),
        }],
        vec![Op::SetSlot {
            slot: slot(4),
            addr: Some(addr("voice.glide")),
        }],
        vec![Op::SetDepth {
            route: lfo_route(),
            depth: 40.0,
        }],
        vec![Op::InsertRoute {
            index: 1,
            route: Route {
                src: ModSource::Velocity,
                dst: addr("osc1.level"),
                depth: 30.0,
            },
        }],
        vec![Op::RemoveRoute {
            route: RouteAddr {
                src: ModSource::Velocity,
                dst: addr("osc1.level"),
            },
        }],
        vec![Op::SetEnabled {
            id: id("filter1"),
            enabled: false,
        }],
        vec![Op::AddModule {
            at: at(Section::VoiceFx, 1),
            module: ModuleInstance::new(id("filter2"), ModuleKind::Svf),
        }],
        vec![Op::Move {
            id: id("filter2"),
            to: at(Section::VoiceFx, 0),
        }],
        vec![Op::RemoveModule { id: id("filter2") }],
        vec![
            Op::AddModule {
                at: at(Section::Modulators, 2),
                module: ModuleInstance::new(id("env2"), ModuleKind::Adsr),
            },
            Op::SetAmpEnv { id: id("env2") },
        ],
        vec![Op::SetSlot {
            slot: slot(4),
            addr: None,
        }],
        vec![Op::SetMeta {
            meta: enth_patch::PatchMeta {
                name: "S2s".to_owned(),
                ..enth_patch::PatchMeta::default()
            },
        }],
    ]
}

/// The script on `session`, then undo to the start and redo to the end, comparing with the snapshots.
fn round_trip(session: &mut Session) {
    let start = session.patch().clone();
    let mut snapshots = vec![start];
    for ops in script() {
        edit(session, ops);
        snapshots.push(session.patch().clone());
    }
    let steps = snapshots.len() - 1;
    for k in (0..steps).rev() {
        assert_eq!(session.undo(), Ok(true));
        assert!(identical(session.patch(), &snapshots[k]), "undo to {k}");
    }
    assert_eq!(session.undo(), Ok(false), "nothing left to undo");
    assert!(!session.history().is_dirty());
    for (k, expected) in snapshots.iter().enumerate().skip(1) {
        assert_eq!(session.redo(), Ok(true));
        assert!(identical(session.patch(), expected), "redo to {k}");
    }
    assert_eq!(session.redo(), Ok(false), "nothing left to redo");
}

#[test]
fn undo_and_redo_round_trip_every_op_kind_detached() {
    round_trip(&mut Session::new(patch()));
}

#[test]
fn undo_and_redo_round_trip_every_op_kind_while_active() {
    let (mut session, _engine) = live(patch());
    round_trip(&mut session);
    // Every step left a plan pending or events sent; nothing blocked, nothing failed.
    assert_eq!(session.status().last_error, None);
}

#[test]
fn nothing_to_undo_or_redo_delivers_nothing() {
    let (mut session, _engine) = live(patch());
    assert_eq!(session.undo(), Ok(false));
    assert_eq!(session.redo(), Ok(false));
    assert_eq!(session.drain_requests().count(), 0);
    assert!(!session.status().compile_pending);
}

#[test]
fn a_new_edit_clears_redo() {
    let mut session = Session::new(patch());
    edit(
        &mut session,
        vec![Op::SetParam {
            addr: addr("voice.glide"),
            value: 10.0,
        }],
    );
    assert_eq!(session.undo(), Ok(true));
    assert!(session.history().can_redo());
    edit(
        &mut session,
        vec![Op::SetParam {
            addr: addr("voice.glide"),
            value: 20.0,
        }],
    );
    assert!(!session.history().can_redo());
    assert_eq!(session.redo(), Ok(false));
}

fn drag(session: &mut Session, a: &str, values: &[f32]) {
    let g = session.begin_gesture();
    for &value in values {
        edit_in(
            session,
            vec![Op::SetParam {
                addr: addr(a),
                value,
            }],
            Some(g),
        );
    }
}

#[test]
fn a_drag_is_one_undo_step_and_one_redo_step() {
    let (mut session, _engine) = live(patch());
    let before = session.patch().value(&addr("filter1.cutoff")).unwrap();
    drag(
        &mut session,
        "filter1.cutoff",
        &[500.0, 800.0, 1500.0, 3000.0],
    );
    session.end_gesture();
    assert_eq!(session.history().next_undo().unwrap().forward.len(), 1);
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(
        session.patch().value(&addr("filter1.cutoff")),
        Some(before),
        "the whole drag in one step"
    );
    assert!(!session.history().can_undo());
    assert_eq!(session.redo(), Ok(true));
    assert_eq!(session.patch().value(&addr("filter1.cutoff")), Some(3000.0));
}

#[test]
fn a_drag_that_ends_where_it_started_leaves_no_step() {
    let mut session = Session::new(patch());
    let before = session.patch().value(&addr("filter1.cutoff")).unwrap();
    drag(&mut session, "filter1.cutoff", &[500.0, 900.0, before]);
    session.end_gesture();
    assert!(!session.history().can_undo());
    assert_eq!(session.undo(), Ok(false));
}

#[test]
fn depth_and_macro_drags_merge_into_one_op_pair() {
    let mut session = Session::new(patch());
    let g = session.begin_gesture();
    for depth in [30.0, 35.0, 50.0] {
        edit_in(
            &mut session,
            vec![Op::SetDepth {
                route: lfo_route(),
                depth,
            }],
            Some(g),
        );
    }
    session.end_gesture();
    let g = session.begin_gesture();
    for value in [0.1, 0.2, 0.9] {
        edit_in(
            &mut session,
            vec![Op::SetMacro {
                index: macro1(),
                value,
            }],
            Some(g),
        );
    }
    session.end_gesture();
    let stack = session.history.undo_stack();
    assert_eq!(stack.len(), 2);
    assert!(
        stack
            .iter()
            .all(|e| e.forward.len() == 1 && e.inverse.len() == 1)
    );
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(session.patch().macros[0].value, 0.0);
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(session.patch().routes[0].depth, 25.0);
}

#[test]
fn undo_ends_a_drag_in_flight_and_undoes_it_whole() {
    let (mut session, _engine) = live(patch());
    let start = session.patch().clone();
    let g = session.begin_gesture();
    for (a, value) in [
        ("filter1.cutoff", 900.0),
        ("voice.glide", 60.0),
        ("filter1.cutoff", 1100.0),
    ] {
        edit_in(
            &mut session,
            vec![Op::SetParam {
                addr: addr(a),
                value,
            }],
            Some(g),
        );
    }
    assert!(session.status().gesture_open);
    assert_eq!(session.undo(), Ok(true));
    assert!(!session.status().gesture_open, "the drag was closed");
    assert!(
        identical(session.patch(), &start),
        "the drag was undone whole"
    );
    assert_eq!(session.redo(), Ok(true));
    assert_eq!(session.patch().value(&addr("voice.glide")), Some(60.0));
    assert_eq!(session.patch().value(&addr("filter1.cutoff")), Some(1100.0));
}

#[test]
fn undo_closes_the_host_gestures_a_drag_opened_before_it_writes() {
    let (mut session, _engine) = live(patch());
    edit(
        &mut session,
        vec![Op::SetParam {
            addr: addr("filter1.cutoff"),
            value: 700.0,
        }],
    );
    session.drain_requests().for_each(drop);
    // Lane SK's edit opens a host gesture at a drag's first write to a slotted address; stand in for it.
    let slot1 = HostParam::Slot(slot(1));
    session.begin_gesture();
    session.gestures.host_open.push(slot1);
    session.requests.push(ShellRequest::Host(HostWrite {
        param: slot1,
        step: GestureStep::Begin,
    }));
    assert_eq!(session.undo(), Ok(true));
    let requests: Vec<_> = session.drain_requests().collect();
    let host: Vec<_> = requests
        .iter()
        .filter_map(|r| match r {
            ShellRequest::Host(w) => Some((w.param, w.step)),
            _ => None,
        })
        .collect();
    let norm = crate::bridge::host_values(session.patch()).get(slot1);
    assert_eq!(
        host,
        vec![
            (slot1, GestureStep::Begin),
            (slot1, GestureStep::End),
            (slot1, GestureStep::Begin),
            (slot1, GestureStep::Set(norm)),
            (slot1, GestureStep::End),
        ],
        "the drag's gesture ends, then the undo writes a complete one"
    );
    assert_eq!(requests.last(), Some(&ShellRequest::PatchEdited));
}

#[test]
fn undo_tells_the_host_what_it_moved() {
    let (mut session, _engine) = live(patch());
    edit(
        &mut session,
        vec![
            Op::SetParam {
                addr: addr("filter1.cutoff"),
                value: 700.0,
            },
            Op::SetParam {
                addr: addr("voice.glide"), // not slotted: no host write
                value: 90.0,
            },
            Op::SetMacro {
                index: macro1(),
                value: 0.75,
            },
        ],
    );
    session.drain_requests().for_each(drop);
    assert_eq!(session.undo(), Ok(true));
    let writes: Vec<_> = session
        .drain_requests()
        .filter_map(|r| match r {
            ShellRequest::Host(w) => Some(w),
            _ => None,
        })
        .collect();
    let params: Vec<_> = writes.iter().map(|w| w.param).collect();
    assert_eq!(
        params,
        vec![
            HostParam::Macro(macro1()),
            HostParam::Macro(macro1()),
            HostParam::Macro(macro1()),
            HostParam::Slot(slot(1)),
            HostParam::Slot(slot(1)),
            HostParam::Slot(slot(1)),
        ],
        "a begin, set and end for the macro and for the slot (the inverse ops run last to first), none for glide"
    );
    let want = crate::bridge::host_values(session.patch());
    assert_eq!(writes[1].step, GestureStep::Set(0.0));
    assert_eq!(
        writes[4].step,
        GestureStep::Set(want.get(HostParam::Slot(slot(1))))
    );
}

#[test]
fn undo_asks_for_a_rescan_only_when_the_host_would_see_other_names() {
    let mut session = Session::new(patch());
    // A label on a module no slot drives, a slot assignment, a macro name.
    edit(
        &mut session,
        vec![Op::SetLabel {
            id: id("osc1"),
            label: Some("Saw".to_owned()),
        }],
    );
    edit(
        &mut session,
        vec![Op::SetLabel {
            id: id("filter1"),
            label: Some("Tone".to_owned()),
        }],
    );
    edit(
        &mut session,
        vec![Op::SetMacroName {
            index: macro1(),
            name: Some("Bright".to_owned()),
        }],
    );
    session.drain_requests().for_each(drop);
    let renames = |session: &mut Session| {
        session
            .drain_requests()
            .any(|r| r == ShellRequest::RenameParams)
    };
    assert_eq!(session.undo(), Ok(true)); // the macro name
    assert!(renames(&mut session));
    assert_eq!(session.undo(), Ok(true)); // "Tone": slots 1 and 3 drive its parameters
    assert!(renames(&mut session));
    assert_eq!(session.undo(), Ok(true)); // "Saw": no slot drives osc1
    assert!(!renames(&mut session), "no host name changed");
}

#[test]
fn a_step_that_no_longer_applies_is_refused_and_changes_nothing() {
    let mut session = Session::new(patch());
    edit(
        &mut session,
        vec![Op::AddModule {
            at: Position {
                section: Section::VoiceFx,
                index: 1,
            },
            module: ModuleInstance::new(id("filter2"), ModuleKind::Svf),
        }],
    );
    // The patch changes behind the history's back (a bug elsewhere, or a hand-edited session): the inverse
    // (remove filter2) has nothing to remove.
    enth_patch::apply(&mut session.patch, Op::RemoveModule { id: id("filter2") }).unwrap();
    session.drain_requests().for_each(drop);
    let before = session.patch().clone();
    let Err(err) = session.undo() else {
        panic!("the inverse cannot apply")
    };
    assert!(matches!(err, SessionError::Edit(_)));
    assert_eq!(session.status().last_error, Some(err));
    assert!(identical(session.patch(), &before));
    assert!(session.history().can_undo(), "the step is still there");
    assert_eq!(session.drain_requests().count(), 0);
}

#[test]
fn the_first_undo_hands_an_unreadable_patchs_place_to_the_user() {
    let mut session = Session::new(patch());
    edit(
        &mut session,
        vec![Op::SetParam {
            addr: addr("voice.glide"),
            value: 10.0,
        }],
    );
    session.unloaded_patch = Some("from a newer enth".to_owned());
    session.last_error = Some(SessionError::Busy);
    assert_eq!(session.undo(), Ok(true));
    assert!(!session.status().patch_unloaded);
    assert_eq!(session.status().last_error, None);
    assert_ne!(session.save_patch(None), "from a newer enth");
}

/// The model the property test compares with: the snapshots a user would see, and where they are.
#[derive(Clone, Debug)]
enum Action {
    Edit(usize),
    DragStart,
    DragStep(f32),
    DragEnd,
    Undo,
    Redo,
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        4 => (0..script().len()).prop_map(Action::Edit),
        1 => Just(Action::DragStart),
        3 => (200.0f32..8000.0).prop_map(Action::DragStep),
        1 => Just(Action::DragEnd),
        3 => Just(Action::Undo),
        2 => Just(Action::Redo),
    ]
}

/// Undo and redo against the patch model: whatever the sequence of edits, drags, undos and redos, the session's
/// patch is the model's snapshot at the model's position.
fn check_against_the_model(actions: &[Action], active: bool) {
    let scripted = script();
    let (mut session, _engine) = if active {
        let (s, e) = live(patch());
        (s, Some(e))
    } else {
        (Session::new(patch()), None)
    };
    let mut line: Vec<Patch> = vec![session.patch().clone()];
    let mut at = 0usize;
    // A drag in flight: the snapshot before it, and the gesture's id.
    let mut drag: Option<Patch> = None;
    let close = |line: &mut Vec<Patch>, at: &mut usize, drag: &mut Option<Patch>, now: &Patch| {
        if let Some(start) = drag.take()
            && !identical(&start, now)
        {
            line.truncate(*at + 1);
            line.push(now.clone());
            *at += 1;
        }
    };
    for action in actions {
        match action {
            Action::Edit(i) => {
                // An op the patch refuses (an earlier undo removed what it needs) is skipped on both sides.
                let ops = scripted[*i].clone();
                let mut probe = session.patch().clone();
                if enth_patch::apply_all(&mut probe, &ops).is_err() {
                    continue;
                }
                let gesture = session.gestures.open;
                if drag.is_some() {
                    edit_in(&mut session, ops, gesture);
                    line.truncate(at + 1); // recording clears redo, even if the drag is annulled later
                } else {
                    edit(&mut session, ops);
                    line.truncate(at + 1);
                    line.push(session.patch().clone());
                    at += 1;
                }
            }
            Action::DragStart => {
                close(&mut line, &mut at, &mut drag, &session.patch().clone());
                session.begin_gesture();
                drag = Some(session.patch().clone());
            }
            Action::DragStep(value) => {
                if drag.is_some() {
                    let gesture = session.gestures.open;
                    edit_in(
                        &mut session,
                        vec![Op::SetParam {
                            addr: addr("voice.glide"),
                            value: value.round(),
                        }],
                        gesture,
                    );
                    line.truncate(at + 1);
                }
            }
            Action::DragEnd => {
                session.end_gesture();
                close(&mut line, &mut at, &mut drag, &session.patch().clone());
            }
            Action::Undo | Action::Redo => {
                // Undo and redo end a drag in flight first.
                session.end_gesture();
                close(&mut line, &mut at, &mut drag, &session.patch().clone());
                let undo = matches!(action, Action::Undo);
                let moved = if undo { session.undo() } else { session.redo() };
                let can = if undo { at > 0 } else { at + 1 < line.len() };
                assert_eq!(moved, Ok(can));
                if can {
                    at = if undo { at - 1 } else { at + 1 };
                }
            }
        }
        if drag.is_none() {
            assert!(identical(session.patch(), &line[at]), "after {action:?}");
        }
        if drag.is_none() {
            assert_eq!(session.history().can_undo(), at > 0);
        }
        session.drain_requests().for_each(drop);
    }
    session.end_gesture();
    let now = session.patch().clone();
    close(&mut line, &mut at, &mut drag, &now);
    assert!(identical(session.patch(), &line[at]));
}

proptest! {
    #[test]
    fn undo_and_redo_follow_the_patch_model(actions in prop::collection::vec(action(), 0..60)) {
        check_against_the_model(&actions, false);
    }

    #[test]
    fn undo_and_redo_follow_the_patch_model_while_active(actions in prop::collection::vec(action(), 0..40)) {
        check_against_the_model(&actions, true);
    }
}
