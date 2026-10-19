//! The host-state-load policy (lane S2s): what a load does to the history, and that the engine always gets the
//! loaded patch.

use enth_patch::Op;
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModuleKind, Patch, Position};

use super::kit::{addr, base, edit, edit_in, id, live, patch, port, run, settle};
use crate::state::{HistoryFate, RECONCILE_STEPS};
use crate::{Session, ShellRequest};

fn glide(value: f32) -> Op {
    Op::SetParam {
        addr: addr("voice.glide"),
        value,
    }
}

/// A session after `n` separate edits of the glide, with the text each state would be saved as, oldest first
/// (index 0 is the state before the first edit).
fn line(session: &mut Session, n: usize) -> Vec<String> {
    let mut states = vec![session.patch().to_json()];
    for k in 1..=n {
        edit(session, vec![glide(k as f32)]);
        states.push(session.patch().to_json());
    }
    states
}

#[test]
fn a_load_of_the_state_the_session_holds_keeps_the_history() {
    let mut session = Session::new(patch());
    let states = line(&mut session, 3);
    assert!(session.history().is_dirty());
    let report = session.load_patch(states.last().unwrap());
    assert_eq!(report.errors, []);
    assert_eq!(report.history, Some(HistoryFate::Kept));
    assert!(session.history().can_undo(), "the history survives");
    assert!(!session.history().is_dirty(), "the load counts as saved");
    assert_eq!(session.undo(), Ok(true));
    assert!(session.history().is_dirty());
}

#[test]
fn a_hosts_undo_walks_the_history_instead_of_destroying_it() {
    let mut session = Session::new(patch());
    let states = line(&mut session, 3);
    // REAPER's Undo loads the state saved at an earlier undo point: two of our steps back.
    let report = session.load_patch(&states[1]);
    assert_eq!(report.history, Some(HistoryFate::Undone(2)));
    assert_eq!(session.patch().to_json(), states[1]);
    assert!(!session.history().is_dirty());
    assert!(session.history().can_undo() && session.history().can_redo());
    // And its Redo is two steps ahead.
    let report = session.load_patch(&states[3]);
    assert_eq!(report.history, Some(HistoryFate::Redone(2)));
    assert_eq!(session.patch().to_json(), states[3]);
    assert!(!session.history().can_redo());
    // The editor's own undo and redo are on the same line.
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(session.patch().to_json(), states[2]);
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(session.patch().to_json(), states[0]);
    assert_eq!(session.undo(), Ok(false));
}

#[test]
fn the_nearest_matching_state_wins() {
    // Two states with the same text (edit, then edit back): a load of it goes to the nearer one, back first.
    let mut session = Session::new(patch());
    let start = session.patch().to_json();
    edit(&mut session, vec![glide(5.0)]);
    edit(&mut session, vec![glide(0.0)]);
    edit(&mut session, vec![glide(7.0)]);
    let report = session.load_patch(&start);
    // glide 0 is the default, so the state after two edits equals the start: one step back from glide 7.
    assert_eq!(report.history, Some(HistoryFate::Undone(1)));
}

#[test]
fn a_load_of_a_state_the_history_does_not_know_starts_a_new_line() {
    let mut session = Session::new(patch());
    line(&mut session, 3);
    let other = Patch::init().to_json();
    let report = session.load_patch(&other);
    assert_eq!(report.history, Some(HistoryFate::Cleared));
    assert!(!session.history().can_undo() && !session.history().can_redo());
    assert!(!session.history().is_dirty());
    assert_eq!(session.patch().to_json(), other);
}

#[test]
fn a_patch_that_does_not_load_clears_the_history_and_keeps_its_text() {
    let mut session = Session::new(patch());
    line(&mut session, 2);
    let report = session.load_patch(r#"{"schema": 99}"#);
    assert!(report.fell_back);
    assert_eq!(report.history, Some(HistoryFate::Cleared));
    assert!(!session.history().can_undo());
    assert!(session.status().patch_unloaded);
}

#[test]
fn only_states_near_enough_are_looked_for() {
    let mut session = Session::new(patch());
    let states = line(&mut session, RECONCILE_STEPS + 6);
    let end = states.len() - 1;
    // Exactly the window back: found. One more: a new line.
    let mut near = Session::new(patch());
    line(&mut near, RECONCILE_STEPS + 6);
    assert_eq!(
        near.load_patch(&states[end - RECONCILE_STEPS]).history,
        Some(HistoryFate::Undone(RECONCILE_STEPS))
    );
    assert_eq!(
        session
            .load_patch(&states[end - RECONCILE_STEPS - 1])
            .history,
        Some(HistoryFate::Cleared)
    );
}

#[test]
fn a_load_closes_a_drag_in_flight_without_writing_to_the_host() {
    let mut session = Session::new(patch());
    let g = session.begin_gesture();
    edit_in(&mut session, vec![glide(12.0)], Some(g));
    // A host gesture the drag opened (lane SK's edit does it at a slotted address's first write).
    let slot1 = crate::bridge::HostParam::Slot(enth_patch::SlotIndex::from_one_based(1).unwrap());
    session.gestures.host_open.push(slot1);
    let now = session.patch().to_json();
    let report = session.load_patch(&now);
    assert_eq!(report.history, Some(HistoryFate::Kept));
    assert!(!session.status().gesture_open, "the drag is closed");
    assert!(
        !session
            .drain_requests()
            .any(|r| matches!(r, ShellRequest::Host(_))),
        "a host-thread load writes nothing to the host"
    );
    // The editor's next end_gesture balances the host gesture; the drag's step is there.
    session.end_gesture();
    assert!(matches!(
        session.drain_requests().next(),
        Some(ShellRequest::Host(w)) if w.param == slot1 && w.step == crate::bridge::GestureStep::End
    ));
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(session.patch().value(&addr("voice.glide")), Some(0.0));
}

#[test]
fn a_load_ends_the_open_gesture_and_a_kept_history_keeps_counting_ids() {
    let mut session = Session::new(patch());
    let g = session.begin_gesture();
    edit_in(&mut session, vec![glide(3.0)], Some(g));
    let now = session.patch().to_json();
    session.load_patch(&now);
    assert!(
        !session.status().gesture_open,
        "a stale id is no open gesture"
    );
    // The history was kept, so its ids go on: a drag that outlived the load cannot merge into the next one.
    let next = session.begin_gesture();
    assert_ne!(next, g);
    session.end_gesture();
    // A clearing load restarts them (lane SK's `edit` compares with the open gesture, not with old ids).
    session.load_patch(&Patch::init().to_json());
    assert!(!session.status().gesture_open);
}

#[test]
fn whatever_the_history_does_the_engine_gets_the_loaded_patch() {
    for target in [0usize, 1, 3] {
        // 0: two steps back, 1: one back, 3: where it is.
        let (mut session, mut engine) = live(patch());
        let states = line(&mut session, 3);
        run(&mut session, &mut engine, 10);
        session.drain_requests().for_each(drop);
        let report = session.load_patch(&states[target]);
        assert!(report.history.is_some());
        // The load marks every remembered host slot value to be forgotten, and the plan that carries the mark goes
        // out at once (lane SK's pump) or at the next pump (before it), clearing it.
        assert!(
            port(&session).forget_slots == u64::MAX || !session.status().compile_pending,
            "the loaded patch wins over host values: target {target}"
        );
        assert!(
            session
                .drain_requests()
                .any(|r| r == ShellRequest::RenameParams)
        );
        assert_eq!(settle(&mut session), 1);
        run(&mut session, &mut engine, 20);
        let glide_now = session.patch().value(&addr("voice.glide")).unwrap();
        assert_eq!(glide_now, target as f32);
        assert_eq!(
            base(&session, &engine, "voice.glide"),
            glide_now,
            "target {target}"
        );
    }
}

#[test]
fn a_structural_state_is_found_in_the_history_too() {
    let mut session = Session::new(patch());
    let start = session.patch().to_json();
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
    edit(&mut session, vec![glide(9.0)]);
    let report = session.load_patch(&start);
    assert_eq!(report.history, Some(HistoryFate::Undone(2)));
    assert!(session.patch().find(&id("filter2")).is_none());
    assert_eq!(session.redo(), Ok(true));
    assert!(session.patch().find(&id("filter2")).is_some());
}
