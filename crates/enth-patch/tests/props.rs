//! The S1b property suite for ops and the file layer: apply then inverse is the identity, transactions are atomic,
//! the JSON round trip is bit-exact, history agrees with a snapshot model, the wire form round-trips. Neighbours:
//! `canon.rs` (canonical forms and hashes), `fuzz.rs` (damaged input), `negative.rs` (the properties can fail),
//! `rejections.rs` (every refusal and its code), `migrate.rs` (fixtures). Fixed seed in the gate
//! (`PROPTEST_RNG_SEED`), default cases; "exact" equality compares f32 bits (`canon::identical`).

mod common;

use common::*;
use enth_patch::addr::ModuleId;
use enth_patch::canon::exact_hash;
use enth_patch::catalogue::ModuleKind;
use enth_patch::file::{self, load, load_value};
use enth_patch::history::{GestureId, History};
use enth_patch::model::Patch;
use enth_patch::ops::{Op, apply, apply_all};
use enth_patch::validate::validate;
use enth_patch::wire;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

fn valid(p: &Patch) -> Result<(), TestCaseError> {
    let r = validate(p);
    prop_assert!(r.is_ok(), "{}", r.unwrap_err());
    Ok(())
}

/// Apply every op of `inv` in order (each must apply).
fn apply_each(p: &mut Patch, inv: &[Op]) -> Result<(), TestCaseError> {
    for o in inv {
        if let Err(e) = apply(p, o) {
            return Err(TestCaseError::fail(format!("inverse op {o:?} failed: {e}")));
        }
    }
    Ok(())
}

proptest! {
    /// Built patches are valid, and an op either applies (leaving a valid patch, and its inverse restores the
    /// original bit for bit) or fails with a typed error (leaving the patch untouched).
    #[test]
    fn apply_then_inverse_is_identity(setup in raws(0..60), r in raw()) {
        let mut p = build(&setup);
        valid(&p)?;
        let before = p.clone();
        let mut fresh = 1000;
        let op = make_op(&p, r, &mut fresh);
        match apply(&mut p, &op) {
            Ok(inv) => {
                valid(&p)?;
                apply_each(&mut p, &inv)?;
                prop_assert!(identical(&p, &before), "{op:?}\ninverse {inv:?}");
            }
            Err(e) => {
                assert_typed(&e);
                prop_assert!(identical(&p, &before), "a failed {op:?} changed the patch ({e})");
            }
        }
    }

    /// Transactions are atomic and invertible, and redoing reproduces the result.
    #[test]
    fn transactions_are_atomic_and_invertible(setup in raws(0..40), rs in raws(1..14)) {
        let mut p = build(&setup);
        let before = p.clone();
        // Ops are generated against the evolving patch so later ops can use modules earlier ones add.
        let mut scratch = p.clone();
        let mut fresh = 2000;
        let mut ops = Vec::new();
        for r in &rs {
            let op = make_op(&scratch, *r, &mut fresh);
            let _ = apply(&mut scratch, &op);
            ops.push(op);
        }
        match apply_all(&mut p, &ops) {
            Ok(inv) => {
                valid(&p)?;
                let after = p.clone();
                apply_all(&mut p, &inv).map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert!(identical(&p, &before));
                apply_all(&mut p, &ops).map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert!(identical(&p, &after));
            }
            Err(tx) => {
                prop_assert!(tx.index < ops.len());
                assert_typed(&tx.error);
                prop_assert!(identical(&p, &before), "failed at {} but the patch changed", tx.index);
            }
        }
    }

    /// Writing and reading a patch gives it back bit for bit, and writing that gives the same bytes.
    #[test]
    fn json_round_trip_is_bit_exact(setup in raws(0..60)) {
        let p = build(&setup);
        let json = p.to_json();
        let loaded = load(&json).map_err(|e| TestCaseError::fail(format!("{e}\n{json}")))?;
        prop_assert!(loaded.notes.is_empty());
        prop_assert!(identical(&loaded.patch, &p), "{json}");
        prop_assert_eq!(loaded.patch.to_json(), json.clone());
        let pretty = p.to_json_pretty();
        let again = load(&pretty).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(identical(&again.patch, &p));
        prop_assert_eq!(again.patch.to_json_pretty(), pretty);
        prop_assert_eq!(exact_hash(&loaded.patch), exact_hash(&p));
        // the typed path is the only writer: it agrees with serde on the PatchFile
        prop_assert_eq!(serde_json::to_string(&p.to_file()).unwrap(), json);
    }

    /// Files are written with the shortest text that reads back to the same bits: no number has more than the 9
    /// significant digits an `f32` can need (widening to `f64` first would write `0.10000000149011612` for `0.1`).
    #[test]
    fn files_hold_the_shortest_number_text(setup in raws(0..60)) {
        let p = build(&setup);
        prop_assert!(max_digits(&p.to_json()) <= 9);
        prop_assert!(max_digits(&p.to_json_pretty()) <= 9);
    }

    /// Every f32 a parameter can hold survives a patch file: values drawn uniformly over the representable
    /// numbers of each range (subnormals and neighbours of the bounds included).
    #[test]
    fn every_parameter_value_round_trips(bits in prop::collection::vec(any::<u32>(), 8)) {
        let mut p = base_patch();
        let mut fresh = 0;
        // one module of every kind, then a random in-range value in every parameter
        for (i, kind) in ModuleKind::ALL.iter().enumerate() {
            let at = enth_patch::addr::Position { section: kind.section(), index: p.section(kind.section()).len() };
            if p.section(kind.section()).len() < kind.section().max_len() {
                fresh += 1;
                let mut m = enth_patch::model::ModuleInstance::new(ModuleId::new(&format!("m{fresh}")).unwrap(), *kind);
                for (j, spec) in kind.params().iter().enumerate() {
                    m.values[j] = value_for(spec, (bits[(i + j) % bits.len()] & !0x1f) | (8 + (j as u32 % 16)));
                }
                apply(&mut p, Op::AddModule { at, module: m }).unwrap();
            }
        }
        valid(&p)?;
        let loaded = load(&p.to_json()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(identical(&loaded.patch, &p));
        // and through serde_json::Value on the migration path
        let v: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        let via = load_value(v).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(identical(&via.patch, &p));
    }

    /// Reading the current schema is the identity for the migration framework.
    #[test]
    fn migrate_is_the_identity_on_the_current_schema(setup in raws(0..30)) {
        let p = build(&setup);
        let v: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        let m = enth_patch::migrate::migrate(v.clone()).unwrap();
        prop_assert_eq!(m.value, v);
        prop_assert!(m.notes.is_empty());
        prop_assert_eq!(m.from, file::SCHEMA);
    }

    /// The wire form of an op list (inverses included) resolves back to the same effect.
    #[test]
    fn wire_form_round_trips(setup in raws(0..50), rs in raws(1..8)) {
        let mut p = build(&setup);
        let mut fresh = 3000;
        for r in &rs {
            let op = make_op(&p, *r, &mut fresh);
            let before = p.clone();
            let Ok(inv) = apply(&mut p, &op) else { continue };
            // forward: resolving the wire form of `op` against the old patch does the same thing
            let json = wire::to_json(&before, std::slice::from_ref(&op));
            let resolved = wire::resolve(&before, &json).map_err(|e| TestCaseError::fail(format!("{e}\n{json}")))?;
            let mut replay = before.clone();
            apply_all(&mut replay, &resolved).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert!(identical(&replay, &p), "{json}");
            // inverse: its wire form resolves to exactly the inverse ops
            let inv_json = wire::to_json(&p, &inv);
            let inv_back = wire::resolve(&p, &inv_json).map_err(|e| TestCaseError::fail(format!("{e}\n{inv_json}")))?;
            prop_assert_eq!(&inv_back, &inv, "{}", inv_json);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// History against a snapshot model
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Step {
    Edit(Raw),
    GestureEdit(Raw),
    EndGesture,
    Undo,
    Redo,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => raw().prop_map(Step::Edit),
        6 => raw().prop_map(Step::GestureEdit),
        2 => Just(Step::EndGesture),
        2 => Just(Step::Undo),
        2 => Just(Step::Redo),
    ]
}

/// Drives a real [`History`] and a model that keeps a snapshot per undo step; they must agree after every step.
fn history_matches_model(setup: &[Raw], steps: &[Step]) -> Result<(), TestCaseError> {
    let mut p = build(setup);
    let mut h = History::new();
    // model: (gesture tag, state after) per entry; the initial state below them
    let mut done: Vec<(Option<GestureId>, Patch)> = Vec::new();
    let mut undone: Vec<(Option<GestureId>, Patch)> = Vec::new();
    let initial = p.clone();
    let mut last_merge: Option<GestureId> = None;
    let mut open: Option<(GestureId, Patch)> = None;
    let mut fresh = 5000;
    for s in steps {
        match s {
            Step::Edit(r) | Step::GestureEdit(r) => {
                let gesture = if matches!(s, Step::GestureEdit(_)) {
                    if open.is_none() {
                        let g = h.begin_gesture(&p);
                        open = Some((g, p.clone()));
                    }
                    open.as_ref().map(|(g, _)| *g)
                } else {
                    if let Some((g, start)) = open.take() {
                        h.end_gesture(&p);
                        end_in_model(&mut done, &p, g, &start, &mut last_merge);
                    }
                    None
                };
                let op = make_op(&p, *r, &mut fresh);
                if h.edit(&mut p, vec![op], gesture).is_ok() {
                    undone.clear();
                    if gesture.is_some() && last_merge == gesture {
                        if let Some(last) = done.last_mut() {
                            last.1 = p.clone();
                        }
                    } else {
                        done.push((gesture, p.clone()));
                    }
                    last_merge = gesture;
                }
            }
            Step::EndGesture => {
                if let Some((g, start)) = open.take() {
                    h.end_gesture(&p);
                    end_in_model(&mut done, &p, g, &start, &mut last_merge);
                }
            }
            Step::Undo => {
                open = None;
                last_merge = None;
                let did = h
                    .undo(&mut p)
                    .map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert_eq!(did, !done.is_empty());
                if let Some(e) = done.pop() {
                    undone.push(e);
                }
            }
            Step::Redo => {
                open = None;
                last_merge = None;
                let did = h
                    .redo(&mut p)
                    .map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert_eq!(did, !undone.is_empty());
                if let Some(e) = undone.pop() {
                    done.push(e);
                }
            }
        }
        let expected = done.last().map_or(&initial, |(_, s)| s);
        prop_assert!(identical(&p, expected), "after {s:?}");
        prop_assert_eq!(h.undo_stack().len(), done.len());
        prop_assert_eq!(h.redo_stack().len(), undone.len());
    }
    // Everything undoes back to the start, then redoes forward again.
    while h
        .undo(&mut p)
        .map_err(|e| TestCaseError::fail(e.to_string()))?
    {}
    prop_assert!(identical(&p, &initial));
    while h
        .redo(&mut p)
        .map_err(|e| TestCaseError::fail(e.to_string()))?
    {}
    // (the redo stack's bottom entry is the farthest state)
    let end = undone.first().or(done.last()).map_or(&initial, |(_, s)| s);
    prop_assert!(identical(&p, end));
    Ok(())
}

/// A depth drag (`SetDepth` on one route) and a macro drag (`SetMacro` on one macro) merge like a knob drag: one
/// entry holding one forward op and one inverse, and one undo returns to where the drag started (F1b review).
#[test]
fn depth_and_macro_drags_merge_per_target() {
    use enth_patch::{MacroIndex, ModSource, Route, RouteAddr};
    let mut p = Patch::init();
    let route = RouteAddr {
        src: ModSource::Velocity,
        dst: "osc1.fine".parse().unwrap(),
    };
    let m1 = MacroIndex::from_one_based(1).unwrap();
    let insert = Op::InsertRoute {
        index: 0,
        route: Route {
            src: route.src.clone(),
            dst: route.dst.clone(),
            depth: 10.0,
        },
    };
    apply(&mut p, &insert).unwrap();
    let start = p.clone();
    let mut h = History::new();
    let g = h.begin_gesture(&p);
    for i in 1..=40u8 {
        let depth = 10.0 + f32::from(i);
        h.edit(
            &mut p,
            vec![Op::SetDepth {
                route: route.clone(),
                depth,
            }],
            Some(g),
        )
        .unwrap();
    }
    h.end_gesture(&p);
    let g = h.begin_gesture(&p);
    for i in 1..=40u8 {
        let value = f32::from(i) / 40.0;
        h.edit(&mut p, vec![Op::SetMacro { index: m1, value }], Some(g))
            .unwrap();
    }
    h.end_gesture(&p);
    let sizes: Vec<(usize, usize)> = h
        .undo_stack()
        .iter()
        .map(|e| (e.forward.len(), e.inverse.len()))
        .collect();
    assert_eq!(sizes, [(1, 1), (1, 1)]);
    assert!(h.undo(&mut p).unwrap());
    assert!(h.undo(&mut p).unwrap());
    assert!(identical(&p, &start));
}

/// The model's end of a gesture: annul its entry if the gesture ended where it started.
fn end_in_model(
    done: &mut Vec<(Option<GestureId>, Patch)>,
    p: &Patch,
    gesture: GestureId,
    start: &Patch,
    last_merge: &mut Option<GestureId>,
) {
    *last_merge = None;
    if done.last().is_some_and(|(g, _)| *g == Some(gesture)) && identical(p, start) {
        done.pop();
    }
}

proptest! {
    #[test]
    fn history_matches_snapshot_model(setup in raws(0..20), steps in prop::collection::vec(step(), 1..60)) {
        history_matches_model(&setup, &steps)?;
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Does the generator reach what the properties are about?
// ---------------------------------------------------------------------------------------------------------------

/// A census of the generator over a fixed corpus: every op kind both applies and fails often enough, and built
/// patches have routes, slots, macros, labels and several modules. If this fails after a generator change, the
/// properties above have gone blind.
#[test]
fn the_generator_exercises_every_op_and_every_patch_feature() {
    use std::collections::BTreeMap;
    let mut runner = TestRunner::new(Config::default());
    let mut ok: BTreeMap<&str, u32> = BTreeMap::new();
    let mut err: BTreeMap<&str, u32> = BTreeMap::new();
    let mut codes: BTreeMap<&str, u32> = BTreeMap::new();
    let (mut routes, mut slots, mut named, mut labelled, mut multi, mut remove_with_routes) =
        (0, 0, 0, 0, 0, 0);
    let cases = 3000;
    for _ in 0..cases {
        let (setup, r) = (raws(0..60), raw())
            .new_tree(&mut runner)
            .unwrap()
            .current();
        let p = build(&setup);
        routes += u32::from(!p.routes.is_empty());
        slots += u32::from(p.slots.iter().any(Option::is_some));
        named += u32::from(p.macros.iter().any(|m| m.name.is_some()));
        labelled += u32::from(p.modules().any(|(_, m)| m.label.is_some()));
        multi += u32::from(p.modules().count() >= 5);
        let mut q = p.clone();
        let mut fresh = 1000;
        let op = make_op(&p, r, &mut fresh);
        if let Op::RemoveModule { id } = &op {
            remove_with_routes += u32::from(
                p.routes.iter().any(|r| r.dst.module_id() == Some(id))
                    || p.slots.iter().flatten().any(|a| a.module_id() == Some(id)),
            );
        }
        match apply(&mut q, &op) {
            Ok(_) => *ok.entry(op.name()).or_default() += 1,
            Err(e) => {
                *err.entry(op.name()).or_default() += 1;
                *codes.entry(e.code()).or_default() += 1;
            }
        }
    }
    eprintln!("applied: {ok:?}\nfailed: {err:?}\ncodes: {codes:?}");
    eprintln!(
        "patches with routes {routes}, slots {slots}, macro names {named}, labels {labelled}, >=5 modules {multi}; removals touching routes/slots {remove_with_routes} (of {cases})"
    );
    for name in [
        "SetParam",
        "AddModule",
        "RemoveModule",
        "Move",
        "InsertRoute",
        "RemoveRoute",
        "SetDepth",
        "SetEnabled",
        "SetAmpEnv",
        "SetMacro",
        "SetSlot",
        "SetLabel",
        "SetMeta",
        "SetMacroName",
    ] {
        assert!(
            ok.get(name).copied().unwrap_or(0) >= 5,
            "{name} rarely applies: {ok:?}"
        );
    }
    for name in [
        "SetParam",
        "AddModule",
        "RemoveModule",
        "InsertRoute",
        "SetEnabled",
        "SetAmpEnv",
        "SetSlot",
    ] {
        assert!(
            err.get(name).copied().unwrap_or(0) >= 3,
            "{name} rarely fails: {err:?}"
        );
    }
    assert!(
        routes > cases / 4
            && slots > cases / 8
            && named > cases / 8
            && labelled > cases / 8
            && multi > cases / 10,
        "thin patches"
    );
    assert!(
        remove_with_routes >= 5,
        "RemoveModule rarely has routes or slots to restore: {remove_with_routes}"
    );
    for code in [
        "out_of_range",
        "non_finite",
        "unknown_module",
        "duplicate_route",
        "route_rate_mismatch",
        "in_use_as_amp_env",
        "wrong_section",
        "too_many",
        "not_modulatable",
        "not_automatable",
        "not_integral",
        "bad_index",
    ] {
        assert!(codes.contains_key(code), "{code} never produced: {codes:?}");
    }
}
