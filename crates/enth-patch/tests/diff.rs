//! `ops_between` (lane PBC of R1/S1; moved down from `enth-cli`'s `diff_ops`): the list it returns, applied to `a`, gives
//! `b`. Random pairs of patches built from the shared op generator (`common`), so both have the same base ids and
//! their own modules, routes, slots, macros, labels and metadata; "equal" is `canon::identical` (f32 bits, everything
//! included) once the engine stamp, which no op changes, is copied over.

mod common;

use common::*;
use enth_patch::catalogue::{LfoParam, LfoScope, ModuleKind};
use enth_patch::history::History;
use enth_patch::model::{ModuleInstance, Patch, Route, Section};
use enth_patch::ops::{Op, apply_all};
use enth_patch::validate::validate;
use enth_patch::{ModuleId, Position, ops_between};
use proptest::prelude::*;

proptest! {
    /// The property the preset load relies on: for any two valid patches the list exists, applies atomically, every
    /// intermediate result is valid, and the end is `b` bit for bit (up to the engine stamp). Its inverse (what
    /// `apply_all` returns) takes `b` back to `a`, which is what undoing a preset load runs.
    #[test]
    fn applying_ops_between_a_and_b_gives_b(sa in raws(0..60), sb in raws(0..60)) {
        let a = build(&sa);
        let mut b = build(&sb);
        "9.9.9".clone_into(&mut b.engine);
        let ops = ops_between(&a, &b);
        prop_assert!(ops.is_some(), "no list for\n{a:#?}\n->\n{b:#?}");
        let ops = ops.unwrap();
        let mut cur = a.clone();
        let inverse = apply_all(&mut cur, &ops).map_err(|e| TestCaseError::fail(format!("{e}")))?;
        let r = validate(&cur);
        prop_assert!(r.is_ok(), "{}", r.unwrap_err());
        prop_assert_eq!(&cur.engine, &a.engine, "no op changes the engine stamp");
        let mut want = b.clone();
        want.engine.clone_from(&a.engine);
        prop_assert!(identical(&cur, &want), "ops {ops:?}");
        apply_all(&mut cur, &inverse).map_err(|e| TestCaseError::fail(format!("{e}")))?;
        prop_assert!(identical(&cur, &a), "the inverse did not restore a");
    }

    /// A patch needs no ops to become itself, and the list from `a` to `b` is empty exactly when they are the same
    /// up to the stamp.
    #[test]
    fn no_ops_between_a_patch_and_itself(sa in raws(0..60)) {
        let a = build(&sa);
        prop_assert_eq!(ops_between(&a, &a), Some(Vec::new()));
    }

    /// The same list serves a history: recording it as a load step gives one undo step whose undo and redo walk
    /// `a` and `b` (the editor preset load, without the session).
    #[test]
    fn a_load_step_undoes_and_redoes_in_one_step(sa in raws(0..40), sb in raws(0..40)) {
        let a = build(&sa);
        let b = build(&sb);
        let ops = ops_between(&a, &b).expect("a list exists");
        let mut cur = a.clone();
        let mut history = History::new();
        history.edit_load(&mut cur, ops.clone(), "Loaded".to_owned()).unwrap();
        prop_assert!(identical(&cur, &b));
        prop_assert_eq!(history.undo_stack().len(), usize::from(!ops.is_empty()));
        if ops.is_empty() {
            return Ok(());
        }
        prop_assert_eq!(history.undo_stack()[0].load.as_deref(), Some("Loaded"));
        prop_assert!(history.undo(&mut cur).unwrap());
        prop_assert!(identical(&cur, &a), "one undo restores a");
        prop_assert!(history.redo(&mut cur).unwrap());
        prop_assert!(identical(&cur, &b), "one redo restores b");
    }
}

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).unwrap()
}

/// A hand-built pair that moves everything at once: another amplitude envelope, kinds swapped under one id, a
/// different section order, routes in another order. The generator reaches these too; this one reads as a regression.
#[test]
fn a_patch_that_changes_every_kind_of_thing_still_gets_a_list() {
    let mut a = base_patch();
    let mut b = base_patch();
    // Build both through ops so every id and index is valid.
    let add = |p: &mut Patch, kind: ModuleKind, name: &str| {
        let m = ModuleInstance::new(id(name), kind);
        let at = Position {
            section: kind.section(),
            index: p.section(kind.section()).len(),
        };
        apply_all(p, &[Op::AddModule { at, module: m }]).unwrap();
    };
    add(&mut a, ModuleKind::Osc, "osc2");
    add(&mut a, ModuleKind::Adsr, "env2");
    add(&mut a, ModuleKind::Lfo, "lfo1");
    apply_all(
        &mut a,
        &[
            Op::SetAmpEnv { id: id("env2") },
            Op::RemoveModule { id: id("env1") },
        ],
    )
    .unwrap();
    add(&mut b, ModuleKind::Lfo, "lfo1");
    add(&mut b, ModuleKind::Osc, "osc3");
    b.meta.name = "Other".to_owned();
    b.meta.tags = vec!["pad".to_owned()];
    let ops = ops_between(&a, &b).expect("a list exists");
    let mut cur = a.clone();
    apply_all(&mut cur, &ops).unwrap();
    assert!(identical(&cur, &b), "{ops:?}");
    assert!(
        ops.iter().any(|op| matches!(op, Op::SetMeta { .. })),
        "the name and tags travel as one SetMeta"
    );
}

/// A patch with two LFOs of the given scope and a route from the first to the second one's rate.
fn two_lfos(scope: LfoScope) -> Patch {
    let mut p = base_patch();
    for name in ["lfo1", "lfo2"] {
        let mut m = ModuleInstance::new(id(name), ModuleKind::Lfo);
        m.values[LfoParam::Scope.index()] = scope.index() as f32;
        let at = Position {
            section: Section::Modulators,
            index: p.modulators.len(),
        };
        apply_all(&mut p, &[Op::AddModule { at, module: m }]).unwrap();
    }
    let route = Route {
        src: "lfo1".parse().unwrap(),
        dst: "lfo2.rate".parse().unwrap(),
        depth: 40.0,
    };
    apply_all(&mut p, &[Op::InsertRoute { index: 0, route }]).unwrap();
    p
}

/// Changing both LFOs' scope under a route they share: set one at a time, the route would pass through a per-voice
/// source driving a global target (`route_rate_mismatch`), so the first plan is refused and the second one, which
/// takes the route out and puts it back, gets the list.
#[test]
fn a_kept_route_that_a_scope_change_would_break_is_taken_out_and_put_back() {
    let (global, voice) = (two_lfos(LfoScope::Global), two_lfos(LfoScope::Voice));
    let ops = ops_between(&global, &voice).expect("the second plan finds a list");
    let mut cur = global.clone();
    apply_all(&mut cur, &ops).unwrap();
    assert!(identical(&cur, &voice));
    assert!(ops.iter().any(|op| matches!(op, Op::RemoveRoute { .. })));
    assert!(ops.iter().any(|op| matches!(op, Op::InsertRoute { .. })));
}

/// The usual case keeps the route where it is and only moves its depth, so `enth diff` reads like an edit.
#[test]
fn a_route_both_patches_have_only_changes_depth() {
    let a = two_lfos(LfoScope::Voice);
    let mut b = a.clone();
    apply_all(
        &mut b,
        &[Op::SetDepth {
            route: "lfo1 -> lfo2.rate".parse().unwrap(),
            depth: -25.0,
        }],
    )
    .unwrap();
    let ops = ops_between(&a, &b).unwrap();
    assert!(matches!(ops.as_slice(), [Op::SetDepth { .. }]), "{ops:?}");
}

/// `modulators` holds eight modules in both patches and the amplitude envelope is another one in each: the new one
/// has no place while the old one holds its own, so a module with a route and a slot steps out and comes back.
#[test]
fn a_full_modulator_section_still_swaps_the_amplitude_envelope() {
    let lfos = |p: &mut Patch| {
        for n in 1..=7 {
            let at = Position {
                section: Section::Modulators,
                index: p.modulators.len(),
            };
            let module = ModuleInstance::new(id(&format!("lfo{n}")), ModuleKind::Lfo);
            apply_all(p, &[Op::AddModule { at, module }]).unwrap();
        }
        let route = Route {
            src: "lfo7".parse().unwrap(),
            dst: "lfo6.rate".parse().unwrap(),
            depth: 30.0,
        };
        let slot = Op::SetSlot {
            slot: enth_patch::SlotIndex::new(3).unwrap(),
            addr: Some("lfo7.rate".parse().unwrap()),
        };
        apply_all(p, &[Op::InsertRoute { index: 0, route }, slot]).unwrap();
    };
    let mut a = base_patch();
    lfos(&mut a);
    assert_eq!(a.modulators.len(), 8);
    let mut b = base_patch();
    let env2 = ModuleInstance::new(id("env2"), ModuleKind::Adsr);
    let at = Position {
        section: Section::Modulators,
        index: 1,
    };
    apply_all(
        &mut b,
        &[
            Op::AddModule { at, module: env2 },
            Op::SetAmpEnv { id: id("env2") },
            Op::RemoveModule { id: id("env1") },
        ],
    )
    .unwrap();
    lfos(&mut b);
    assert_eq!(b.modulators.len(), 8);
    for (from, to) in [(&a, &b), (&b, &a)] {
        let ops =
            ops_between(from, to).expect("a list exists although no module can be added first");
        let mut cur = from.clone();
        apply_all(&mut cur, &ops).unwrap();
        assert!(identical(&cur, to));
    }
}
