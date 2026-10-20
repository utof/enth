//! The editor's preset load (lane PBC of R1/S1): one undo step, delivered like a host state load with no host writes,
//! marking the project dirty, against an active session built by hand ([`super::kit`]) with a real engine. The file
//! format, index and search are in `tests/presets.rs`.

use enth_patch::{Op, Patch, PatchMeta};

use super::kit::{
    addr, base, block_plans, edit, edit_in, epoch, id, live, patch, port, run, settle,
};
use crate::presets::PresetKey;
use crate::state::HistoryFate;
use crate::{Session, ShellRequest};

fn set(a: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr(a),
        value,
    }
}

fn named(name: &str, tags: &[&str]) -> Op {
    Op::SetMeta {
        meta: PatchMeta {
            name: name.to_owned(),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            ..PatchMeta::default()
        },
    }
}

/// The kit's patch with other values and a name: the same modules, routes and slots, so the host sees the same
/// parameter names.
fn bright() -> Patch {
    let mut p = patch();
    enth_patch::apply_all(
        &mut p,
        &[
            set("filter1.cutoff", 3000.0),
            set("voice.glide", 40.0),
            set("osc1.semi", 7.0),
            named("Bright", &["lead"]),
        ],
    )
    .unwrap();
    p
}

/// A preset with another module set and no slots: the host's parameter names change.
fn plain() -> Patch {
    let mut p = Patch::init();
    enth_patch::apply(&mut p, named("Plain", &["pad"])).unwrap();
    p
}

/// What the requests a step left hold, in order, without the data: host writes, rename, dirty.
fn kinds(session: &mut Session) -> Vec<&'static str> {
    session
        .drain_requests()
        .map(|r| match r {
            ShellRequest::Host(_) => "host",
            ShellRequest::RenameParams => "rename",
            ShellRequest::MonitorBypass(_) => "monitor",
            ShellRequest::PatchEdited => "dirty",
        })
        .collect()
}

fn same(a: &Patch, b: &Patch) -> bool {
    enth_patch::canon::identical(a, b)
}

#[test]
fn a_preset_load_is_one_undo_step_whatever_it_changes() {
    for (name, preset) in [("bright", bright()), ("plain", plain())] {
        let (mut session, _engine) = live(patch());
        let start = session.patch().clone();
        let loaded = session.load_preset(&preset, None).unwrap();
        assert!(loaded.changed && loaded.undoable, "{name}");
        assert!(loaded.ops > 1, "{name}: a load is many ops");
        let mut want = preset.clone();
        want.engine.clone_from(&start.engine);
        assert!(
            same(session.patch(), &want),
            "{name}: the patch is the preset"
        );
        assert_eq!(
            session.history().next_undo().unwrap().load.as_deref(),
            Some(preset.meta.name.as_str())
        );
        // One undo restores everything; one redo restores the preset; nothing else is on the stack.
        assert_eq!(session.undo(), Ok(true), "{name}");
        assert!(same(session.patch(), &start), "{name}: one undo");
        assert!(!session.history().can_undo(), "{name}: a single step");
        assert_eq!(session.redo(), Ok(true), "{name}");
        assert!(same(session.patch(), &want), "{name}: one redo");
        assert!(!session.history().can_redo(), "{name}");
        assert_eq!(session.undo(), Ok(true), "{name}");
        assert_eq!(session.undo(), Ok(false), "{name}");
    }
}

#[test]
fn a_load_its_undo_and_its_redo_make_no_host_writes_and_mark_the_project_dirty() {
    let (mut session, _engine) = live(patch());
    session.drain_requests().for_each(drop);
    // Same host layout: the load asks for nothing but the dirty mark.
    let loaded = session.load_preset(&bright(), None).unwrap();
    assert_eq!(loaded.applied.host_writes, 0);
    assert!(loaded.applied.recompile && !loaded.applied.renamed);
    assert_eq!(kinds(&mut session), ["dirty"], "load");
    session.undo().unwrap();
    assert_eq!(kinds(&mut session), ["dirty"], "undo");
    session.redo().unwrap();
    assert_eq!(kinds(&mut session), ["dirty"], "redo");
    // A load that changes the host's names asks for the rename, then the dirty mark, still no write.
    session.undo().unwrap();
    session.drain_requests().for_each(drop);
    let loaded = session.load_preset(&plain(), None).unwrap();
    assert!(loaded.applied.renamed);
    assert_eq!(kinds(&mut session), ["rename", "dirty"], "load, new names");
    session.undo().unwrap();
    assert_eq!(kinds(&mut session), ["rename", "dirty"], "undo, names back");
    session.redo().unwrap();
    assert_eq!(kinds(&mut session), ["rename", "dirty"], "redo");
}

#[test]
fn a_load_with_slots_to_move_still_writes_nothing_to_the_host() {
    // Every one of the three slots' values changes (cutoff, master level, resonance), which an edit would write.
    let mut preset = patch();
    enth_patch::apply_all(
        &mut preset,
        &[
            set("filter1.cutoff", 700.0),
            set("master.level", -9.0),
            set("filter1.resonance", 55.0),
        ],
    )
    .unwrap();
    let (mut session, _engine) = live(patch());
    session.drain_requests().for_each(drop);
    session.load_preset(&preset, None).unwrap();
    let requests: Vec<_> = session.drain_requests().collect();
    assert!(
        !requests.iter().any(|r| matches!(r, ShellRequest::Host(_))),
        "{requests:?}"
    );
    // The same three edits one at a time are three host writes each: the contrast is the point.
    let (mut session, _engine) = live(patch());
    session.drain_requests().for_each(drop);
    let applied = edit(
        &mut session,
        vec![
            set("filter1.cutoff", 700.0),
            set("master.level", -9.0),
            set("filter1.resonance", 55.0),
        ],
    );
    assert_eq!(applied.host_writes, 3);
}

#[test]
fn each_step_sends_one_plan_that_forgets_every_remembered_slot_value() {
    let (mut session, mut engine) = live(patch());
    assert_eq!(settle(&mut session), 0);
    session.load_preset(&bright(), None).unwrap();
    assert_eq!(epoch(&session), 1, "the load is one plan");
    run(&mut session, &mut engine, 4);
    session.undo().unwrap();
    assert_eq!(epoch(&session), 2, "its undo is one plan");
    run(&mut session, &mut engine, 4);
    session.redo().unwrap();
    assert_eq!(epoch(&session), 3, "its redo is one plan");
    // With two plans in flight the compile waits, marked to forget every slot (u64::MAX is "all of them").
    let (mut session, _engine) = live(patch());
    block_plans(&mut session);
    session.load_preset(&bright(), None).unwrap();
    assert!(port(&session).compile_pending);
    assert_eq!(port(&session).forget_slots, u64::MAX);
}

#[test]
fn the_engine_plays_the_loaded_values_and_the_restored_ones() {
    let (mut session, mut engine) = live(patch());
    let cutoff = |s: &Session, e: &enth_engine::Engine| base(s, e, "filter1.cutoff");
    let start = cutoff(&session, &engine);
    session.load_preset(&bright(), None).unwrap();
    settle(&mut session);
    run(&mut session, &mut engine, 4);
    assert_eq!(cutoff(&session, &engine), 3000.0);
    assert_eq!(base(&session, &engine, "osc1.semi"), 7.0);
    session.undo().unwrap();
    settle(&mut session);
    run(&mut session, &mut engine, 4);
    assert_eq!(cutoff(&session, &engine), start);
    session.redo().unwrap();
    settle(&mut session);
    run(&mut session, &mut engine, 4);
    assert_eq!(cutoff(&session, &engine), 3000.0);
}

#[test]
fn a_stale_host_value_never_overwrites_a_loaded_patch() {
    let (mut session, _engine) = live(patch());
    // The host still reports the values of the patch from before the load.
    let host = session.host_values();
    assert!(session.fold_host(&host).baseline);
    session.load_preset(&bright(), None).unwrap();
    assert!(
        session.host.memos.is_empty(),
        "nothing is remembered across a load"
    );
    let report = session.fold_host(&host);
    assert!(report.baseline, "the first fold after a load only records");
    assert_eq!(session.patch().value(&addr("filter1.cutoff")), Some(3000.0));
    // And the same after the undo of it.
    session.undo().unwrap();
    let after = session.host_values();
    let stale = {
        let mut h = after;
        h.slots[0] = 0.01;
        h
    };
    assert!(session.fold_host(&stale).baseline);
    assert_ne!(session.patch().value(&addr("filter1.cutoff")), Some(3000.0));
}

#[test]
fn audition_notes_are_released_by_a_load() {
    let (mut session, _engine) = live(patch());
    session.port.as_mut().unwrap().auditioned = true;
    session.load_preset(&bright(), None).unwrap();
    assert!(!port(&session).auditioned, "the load released them");
}

#[test]
fn loading_the_patch_you_have_changes_nothing_and_says_nothing() {
    let (mut session, _engine) = live(patch());
    session.drain_requests().for_each(drop);
    let key = PresetKey::factory("bass/Same");
    let loaded = session.load_preset(&patch(), Some(&key)).unwrap();
    assert!(!loaded.changed && loaded.undoable && loaded.ops == 0);
    assert!(!session.history().can_undo(), "no entry");
    assert_eq!(kinds(&mut session), Vec::<&str>::new(), "no request");
    assert_eq!(epoch(&session), 0, "no plan");
    // The bookmark moves all the same: the header shows what the user picked.
    assert_eq!(
        session.instance().preset.as_ref().unwrap().last_key(),
        Some(key)
    );
    // Not even a different engine stamp is a change (no op sets it).
    let mut stamped = patch();
    "9.9.9".clone_into(&mut stamped.engine);
    assert!(!session.load_preset(&stamped, None).unwrap().changed);
}

#[test]
fn consecutive_loads_are_separate_steps_and_an_edit_after_one_does_not_join_it() {
    let (mut session, _engine) = live(patch());
    let start = session.patch().clone();
    session.load_preset(&bright(), None).unwrap();
    let after_first = session.patch().clone();
    session.load_preset(&plain(), None).unwrap();
    edit(&mut session, vec![set("master.level", -6.0)]);
    let undo = |s: &mut Session| {
        assert_eq!(s.undo(), Ok(true));
        s.patch().clone()
    };
    let p = undo(&mut session);
    assert_eq!(p.value(&addr("master.level")), Some(0.0), "the edit alone");
    assert_eq!(p.meta.name, "Plain");
    assert!(
        same(&undo(&mut session), &after_first),
        "the second load alone"
    );
    assert!(same(&undo(&mut session), &start), "the first load alone");
    assert_eq!(session.undo(), Ok(false));
    // The edit between loads is its own step too, labelled as an edit and not as a load.
    session.load_preset(&bright(), None).unwrap();
    edit(&mut session, vec![set("master.level", -6.0)]);
    assert_eq!(session.history().next_undo().unwrap().load, None);
}

#[test]
fn a_load_closes_a_drag_first_and_clears_the_redo_line() {
    let (mut session, _engine) = live(patch());
    let g = session.begin_gesture();
    edit_in(&mut session, vec![set("filter1.cutoff", 900.0)], Some(g));
    edit_in(&mut session, vec![set("filter1.cutoff", 1100.0)], Some(g));
    session.load_preset(&bright(), None).unwrap();
    assert_eq!(session.gestures.open, None, "the drag ended");
    assert_eq!(session.gestures.host_open.len(), 0);
    // The drag is one step under the load.
    assert_eq!(
        session.history().next_undo().unwrap().load.as_deref(),
        Some("Bright")
    );
    session.undo().unwrap();
    assert_eq!(session.patch().value(&addr("filter1.cutoff")), Some(1100.0));
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(
        session.patch().value(&addr("filter1.cutoff")),
        patch().value(&addr("filter1.cutoff"))
    );
    // A load after an undo clears what could have been redone, like an edit.
    assert!(session.history().can_redo());
    session.load_preset(&plain(), None).unwrap();
    assert!(!session.history().can_redo());
}

#[test]
fn the_engine_stamp_is_the_sessions_own() {
    let mut p = patch();
    "0.1.0".clone_into(&mut p.engine);
    let (mut session, _engine) = live(p);
    let mut preset = bright();
    "9.9.9".clone_into(&mut preset.engine);
    session.load_preset(&preset, None).unwrap();
    assert_eq!(session.patch().engine, "0.1.0");
    session.undo().unwrap();
    session.redo().unwrap();
    assert_eq!(session.patch().engine, "0.1.0");
}

#[test]
fn a_preset_that_is_not_a_valid_patch_is_refused_and_changes_nothing() {
    let (mut session, _engine) = live(patch());
    let start = session.patch().clone();
    let mut broken = bright();
    broken.routes.push(enth_patch::model::Route {
        src: enth_patch::ModSource::Module(id("ghost")),
        dst: addr("osc1.level"),
        depth: 10.0,
    });
    let err = session
        .load_preset(&broken, Some(&PresetKey::user("broken")))
        .unwrap_err();
    assert_eq!(err.code(), "bad_patch", "{err}");
    assert!(same(session.patch(), &start));
    assert!(!session.history().can_undo());
    assert_eq!(session.instance().preset, None, "the bookmark did not move");
    assert_eq!(epoch(&session), 0);
}

#[test]
fn a_load_counts_as_saved_and_an_edit_or_an_undo_makes_it_dirty() {
    let (mut session, _engine) = live(patch());
    edit(&mut session, vec![set("voice.glide", 10.0)]);
    assert!(session.history().is_dirty());
    session.load_preset(&bright(), None).unwrap();
    assert!(!session.history().is_dirty(), "the patch is the preset");
    edit(&mut session, vec![set("voice.glide", 20.0)]);
    assert!(session.history().is_dirty());
    session.undo().unwrap();
    assert!(!session.history().is_dirty(), "back on the preset");
    session.undo().unwrap();
    assert!(session.history().is_dirty(), "before the load");
    session.redo().unwrap();
    assert!(!session.history().is_dirty());
}

#[test]
fn the_bookmark_follows_the_key_and_a_load_without_one_clears_it() {
    let (mut session, _engine) = live(patch());
    assert_eq!(session.instance().preset, None);
    let key = PresetKey::factory("pads/Bright");
    session.load_preset(&bright(), Some(&key)).unwrap();
    assert_eq!(
        session.instance().preset.as_ref().unwrap().last_key(),
        Some(key.clone())
    );
    // Undo does not move it (the header keeps the name until another is picked).
    session.undo().unwrap();
    assert_eq!(
        session.instance().preset.as_ref().unwrap().last_key(),
        Some(key)
    );
    session.load_preset(&plain(), None).unwrap();
    assert_eq!(session.instance().preset.as_ref().unwrap().last_key(), None);
}

#[test]
fn a_session_without_an_engine_loads_too() {
    let mut session = Session::new(patch());
    session.drain_requests().for_each(drop);
    let loaded = session.load_preset(&plain(), None).unwrap();
    assert!(loaded.changed);
    assert!(!loaded.applied.recompile, "no engine, no plan");
    assert!(kinds(&mut session).contains(&"dirty"));
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(session.patch().meta.name, patch().meta.name);
}

#[test]
fn a_hosts_state_load_after_a_preset_load_finds_its_place_in_the_history() {
    // REAPER's Undo of a project step loads the state saved at the earlier undo point: the history reconciles with a
    // load step like with any other.
    let (mut session, _engine) = live(patch());
    let before = session.save_patch(None);
    session.load_preset(&bright(), None).unwrap();
    let after = session.save_patch(None);
    let report = session.load_patch(&before);
    assert_eq!(report.errors, []);
    assert_eq!(report.history, Some(HistoryFate::Undone(1)));
    assert!(session.history().can_redo());
    let report = session.load_patch(&after);
    assert_eq!(report.history, Some(HistoryFate::Redone(1)));
    assert_eq!(session.save_patch(None), after);
    // The editor's undo goes on from there.
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(session.save_patch(None), before);
}

#[test]
fn the_loaded_state_round_trips_through_the_hosts_save() {
    let (mut session, _engine) = live(patch());
    session
        .load_preset(&bright(), Some(&PresetKey::factory("bass/Bright")))
        .unwrap();
    let mut other = Session::new(patch());
    assert_eq!(other.load_patch(&session.save_patch(None)).errors, []);
    assert_eq!(other.load_instance(&session.save_instance()).errors, []);
    assert_eq!(other.patch().to_json(), session.patch().to_json());
    assert_eq!(other.instance().preset, session.instance().preset);
}
