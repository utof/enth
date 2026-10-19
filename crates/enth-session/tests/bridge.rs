//! The host-parameter bridge: permanent ids, the patch-wins fold, host writes and names (lane S1s).

mod common;

use enth_params::{Kind, ParamSpec};
use enth_patch::canon::identical;
use enth_patch::catalogue::{MACRO_VALUE, MASTER_PARAMS, VOICE_PARAMS};
use enth_patch::model::SLOTS;
use enth_patch::ops::canonical_value;
use enth_patch::{CATALOGUE, Op, Patch};
use enth_session::bridge::{
    Fold, HOST_PARAM_IDS, HOST_PARAMS, fold_against, fold_value, free_slot, host_layout_changed,
    host_norm, host_param_name, host_param_names, host_updates, host_values, slot_target,
    slots_for, slots_written,
};
use enth_session::{HostParam, HostValues, Session};

#[test]
fn host_param_ids_are_permanent() {
    // docs/naming.md: changing any of these breaks saved automation.
    assert_eq!(HOST_PARAMS, 72);
    assert_eq!(HOST_PARAM_IDS[0], "macro_1");
    assert_eq!(HOST_PARAM_IDS[7], "macro_8");
    assert_eq!(HOST_PARAM_IDS[8], "slot_1");
    assert_eq!(HOST_PARAM_IDS[71], "slot_64");
    for (i, id) in HOST_PARAM_IDS.iter().enumerate() {
        let expected = if i < 8 {
            format!("macro_{}", i + 1)
        } else {
            format!("slot_{}", i - 7)
        };
        assert_eq!(*id, expected);
    }
    let all: Vec<_> = HostParam::all().collect();
    assert_eq!(all.len(), 72);
    for (i, p) in all.iter().enumerate() {
        assert_eq!(p.index(), i);
        assert_eq!(HostParam::from_index(i), Some(*p));
        assert_eq!(HostParam::from_id(p.id()), Some(*p));
    }
    assert_eq!(HostParam::from_index(72), None);
    assert_eq!(HostParam::from_id("slot_65"), None);
    assert_eq!(HostParam::from_id("Slot_1"), None);
    assert_eq!(HostParam::Slot(common::slot(1)).default_norm(), 0.5);
    assert_eq!(HostParam::Macro(common::macro_index(1)).default_norm(), 0.0);
}

/// Every parameter a slot or macro can drive.
fn automatable_specs() -> Vec<&'static ParamSpec> {
    let mut specs: Vec<&'static ParamSpec> = vec![&MACRO_VALUE];
    specs.extend(VOICE_PARAMS.iter());
    specs.extend(MASTER_PARAMS.iter());
    for m in CATALOGUE {
        specs.extend(m.params.iter());
    }
    specs.retain(|s| s.flags.automatable && !s.flags.structural);
    specs
}

/// Plain values across a spec's range: every step of a discrete spec, else `n` evenly spaced values.
fn plains(spec: &ParamSpec, n: usize) -> Vec<f32> {
    if let Some(steps) = spec.step_count() {
        return (0..=steps).map(|i| spec.min() + i as f32).collect();
    }
    let (lo, hi) = (spec.min(), spec.max());
    (0..=n)
        .filter_map(|i| canonical_value(spec, lo + (hi - lo) * (i as f32 / n as f32)).ok())
        .collect()
}

#[test]
fn a_host_value_equal_to_to_norm_keeps_the_exact_patch_value() {
    let specs = automatable_specs();
    assert!(specs.len() > 20, "the catalogue shrank? {}", specs.len());
    for spec in specs {
        for p in plains(spec, 4000) {
            assert_eq!(
                fold_value(spec, host_norm(spec, p), p),
                Fold::Keep,
                "{}: {p}",
                spec.key
            );
        }
    }
}

#[test]
fn folding_is_idempotent_and_writes_canonical_values() {
    for spec in automatable_specs() {
        let start = spec.default;
        for i in 0..=1000 {
            let n = i as f32 / 1000.0;
            match fold_value(spec, n, start) {
                Fold::Keep => {}
                Fold::Write(v) => {
                    assert_eq!(canonical_value(spec, v), Ok(v), "{}: {v}", spec.key);
                    assert_ne!(v.to_bits(), start.to_bits());
                    // Folding the same host value again, or after a save and reload, never moves the patch.
                    assert_eq!(fold_value(spec, n, v), Fold::Keep, "{}: n = {n}", spec.key);
                }
            }
        }
    }
}

#[test]
fn a_fold_writes_from_norm_when_the_host_truly_differs() {
    let spec = Patch::init().spec(&common::addr("voice.glide")).unwrap();
    let n = 0.5;
    assert_eq!(fold_value(spec, n, 0.0), Fold::Write(spec.from_norm(n)));
}

#[test]
fn non_finite_host_values_are_ignored_and_out_of_range_ones_clamp() {
    let spec = &MACRO_VALUE;
    for n in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(fold_value(spec, n, 0.3), Fold::Keep);
    }
    assert_eq!(fold_value(spec, 7.0, 0.3), Fold::Write(1.0));
    assert_eq!(fold_value(spec, -7.0, 0.3), Fold::Write(0.0));
    assert_eq!(fold_value(spec, -0.0, 0.0), Fold::Keep);
}

#[test]
fn discrete_specs_fold_to_whole_values() {
    for spec in automatable_specs() {
        if !matches!(spec.kind, Kind::Int { .. } | Kind::Enum(_) | Kind::Toggle) {
            continue;
        }
        if let Fold::Write(v) = fold_value(spec, 0.37, spec.max()) {
            assert_eq!(v, v.round(), "{}", spec.key);
        }
    }
}

#[test]
fn the_first_fold_only_records_the_patch_wins() {
    let patch = common::custom_patch();
    let mut session = Session::new(patch.clone());
    // Host values that disagree with the patch everywhere (a host that restored nothing: all defaults).
    let report = session.fold_host(&HostValues::default());
    assert!(report.baseline);
    assert_eq!(report.written, 0);
    assert!(identical(session.patch(), &patch));
    // Unchanged since then: still nothing, though they still disagree.
    let report = session.fold_host(&HostValues::default());
    assert!(!report.baseline);
    assert_eq!(report.written, 0);
    assert!(identical(session.patch(), &patch));
}

#[test]
fn a_changed_host_value_is_folded_without_history() {
    let mut session = Session::new(common::custom_patch());
    let mut host = session.host_values();
    session.fold_host(&host);

    host.set(HostParam::Slot(common::slot(1)), 0.25);
    host.set(HostParam::Macro(common::macro_index(1)), 0.75);
    let report = session.fold_host(&host);
    assert_eq!(report.written, 2);
    let cutoff = common::addr("filter1.cutoff");
    let spec = session.patch().spec(&cutoff).unwrap();
    assert_eq!(session.patch().value(&cutoff), Some(spec.from_norm(0.25)));
    assert_eq!(session.patch().macros[0].value, 0.75);
    assert!(
        !session.history().can_undo(),
        "host folds are not undo steps"
    );
    assert_eq!(
        session.drain_requests().count(),
        0,
        "host folds write nothing back to the host"
    );
}

#[test]
fn a_changed_host_value_equal_to_to_norm_writes_nothing() {
    let patch = common::custom_patch();
    let mut session = Session::new(patch.clone());
    let mut host = HostValues::default();
    session.fold_host(&host);
    host = session.host_values(); // every slot moves to exactly to_norm(patch value)
    assert_eq!(session.fold_host(&host).written, 0);
    assert!(identical(session.patch(), &patch));
}

#[test]
fn a_load_resets_the_baseline() {
    let mut session = Session::new(common::custom_patch());
    let mut host = session.host_values();
    session.fold_host(&host);
    let r = session.load_patch(&common::custom_patch().to_json());
    assert_eq!(r.errors, [], "{r:?}");
    host.set(HostParam::Slot(common::slot(1)), 0.9);
    let report = session.fold_host(&host);
    assert!(report.baseline, "the first fold after a load only records");
    assert!(identical(session.patch(), &common::custom_patch()));
}

#[test]
fn host_values_follow_the_patch() {
    let patch = common::custom_patch();
    let v = host_values(&patch);
    assert_eq!(v.macros[0], 0.25);
    assert_eq!(v.macros[1], 0.0);
    let cutoff = common::addr("filter1.cutoff");
    let spec = patch.spec(&cutoff).unwrap();
    assert_eq!(v.slots[0], spec.to_norm(1234.5));
    assert_eq!(v.slots[2], 0.5, "an empty slot shows its default");
    assert_eq!(v, Session::new(patch).host_values());
}

#[test]
fn editor_ops_imply_host_writes() {
    let patch = common::custom_patch();
    let mut out = Vec::new();
    let cutoff = common::addr("filter1.cutoff");
    let mut after = patch.clone();
    let op = Op::SetParam {
        addr: cutoff.clone(),
        value: 800.0,
    };
    enth_patch::apply(&mut after, &op).unwrap();
    host_updates(&after, &op, &mut out);
    let spec = after.spec(&cutoff).unwrap();
    assert_eq!(
        out,
        vec![(HostParam::Slot(common::slot(1)), spec.to_norm(800.0))]
    );

    out.clear();
    let op = Op::SetParam {
        addr: common::addr("filter1.resonance"),
        value: 50.0,
    };
    host_updates(&patch, &op, &mut out);
    assert_eq!(out, [], "an address without a slot writes nothing");

    let op = Op::SetMacro {
        index: common::macro_index(1),
        value: 0.25,
    };
    host_updates(&patch, &op, &mut out);
    assert_eq!(out, vec![(HostParam::Macro(common::macro_index(1)), 0.25)]);

    out.clear();
    let mut after = patch.clone();
    let op = Op::SetSlot {
        slot: common::slot(5),
        addr: Some(common::addr("filter1.resonance")),
    };
    enth_patch::apply(&mut after, &op).unwrap();
    host_updates(&after, &op, &mut out);
    assert_eq!(out.len(), 1, "a reassignment sets at most one host value");
    assert_eq!(out[0].0, HostParam::Slot(common::slot(5)));

    out.clear();
    let op = Op::SetSlot {
        slot: common::slot(1),
        addr: None,
    };
    host_updates(&patch, &op, &mut out);
    assert_eq!(out, [], "a cleared slot is left alone");
}

#[test]
fn names() {
    let patch = common::custom_patch();
    let name = |p| host_param_name(&patch, p);
    assert_eq!(name(HostParam::Macro(common::macro_index(1))), "Brightness");
    assert_eq!(name(HostParam::Macro(common::macro_index(2))), "Macro 2");
    assert_eq!(name(HostParam::Slot(common::slot(1))), "Tone · Cutoff");
    assert_eq!(name(HostParam::Slot(common::slot(2))), "Master level");
    assert_eq!(name(HostParam::Slot(common::slot(64))), "Glide");
    assert_eq!(name(HostParam::Slot(common::slot(3))), "Slot 3");

    let mut unlabelled = patch.clone();
    enth_patch::apply(
        &mut unlabelled,
        Op::SetLabel {
            id: common::id("filter1"),
            label: None,
        },
    )
    .unwrap();
    assert_eq!(
        host_param_name(&unlabelled, HostParam::Slot(common::slot(1))),
        "Filter 1 · Cutoff"
    );
}

#[test]
fn slot_lookups() {
    let patch = common::custom_patch();
    let cutoff = common::addr("filter1.cutoff");
    assert_eq!(
        slots_for(&patch, &cutoff).collect::<Vec<_>>(),
        vec![common::slot(1)]
    );
    assert_eq!(free_slot(&patch), Some(common::slot(3)));
    let t = slot_target(&patch, common::slot(1)).unwrap();
    assert_eq!(t.addr, cutoff);
    assert_eq!(t.spec.key, "cutoff");
    assert_eq!(slot_target(&patch, common::slot(3)), None);

    let mut full = Patch::init();
    for i in 0..SLOTS {
        full.slots[i] = Some(common::addr("master.level"));
    }
    assert_eq!(free_slot(&full), None);
}

/// The slots a recompiling transaction's value ops write: what its plan must forget of the host's remembered values.
#[test]
fn slots_written_by_a_transaction() {
    let mut patch = common::custom_patch();
    let cutoff = common::addr("filter1.cutoff");
    // A second slot on the cutoff (the last one): both are written.
    patch.slots[SLOTS - 1] = Some(cutoff.clone());
    let set = |addr: &str, value: f32| Op::SetParam {
        addr: common::addr(addr),
        value,
    };
    let slotted: Vec<usize> = slots_for(&patch, &cutoff)
        .map(enth_patch::SlotIndex::get)
        .collect();
    let expected = slotted.iter().fold(0u64, |m, &i| m | 1 << i);
    assert_eq!(slotted.len(), 2);
    assert_eq!(
        slots_written(&patch, &[set("filter1.cutoff", 900.0)]),
        expected
    );
    // An unslotted value and a non-value op write none.
    assert_eq!(slots_written(&patch, &[set("osc1.level", 50.0)]), 0);
    assert_eq!(
        slots_written(
            &patch,
            &[Op::SetMacro {
                index: common::macro_index(1),
                value: 0.5
            }]
        ),
        0
    );
    assert_eq!(slots_written(&patch, &[]), 0);
}

/// Fold against the plan's compiled value (lane S2s, F1b review issue 4): the engine's rule, on the engine's value.
#[test]
fn fold_against_the_plans_value_follows_the_engines_slot_rule() {
    for spec in automatable_specs() {
        for plain in plains(spec, 12) {
            for other in plains(spec, 5) {
                // The engine holds `other` (the compiled value) while the patch holds `plain` (a fold moved it).
                let n = spec.to_norm(other);
                match fold_against(spec, n, plain, other) {
                    // The host returned to the plan's value: the target is exactly that value.
                    Fold::Write(v) => assert_eq!(v.to_bits(), other.to_bits(), "{}", spec.key),
                    Fold::Keep => assert_eq!(plain.to_bits(), other.to_bits(), "{}", spec.key),
                }
                // With the plan's value equal to the patch's, it is the plain rule.
                for h in [0.0f32, 0.3, 0.5, 1.0, n] {
                    assert_eq!(
                        fold_against(spec, h, plain, plain),
                        fold_value(spec, h, plain),
                        "{} at {h}",
                        spec.key
                    );
                }
            }
        }
    }
}

#[test]
fn fold_against_still_ignores_non_finite_host_values() {
    let spec = &MACRO_VALUE;
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(fold_against(spec, bad, 0.25, 0.5), Fold::Keep);
    }
}

#[test]
fn all_seventy_two_names_come_from_the_patch() {
    let patch = common::custom_patch();
    let names = host_param_names(&patch);
    assert_eq!(names.len(), HOST_PARAMS);
    for p in HostParam::all() {
        assert_eq!(names[p.index()], host_param_name(&patch, p));
        assert_ne!(names[p.index()], "");
    }
    // Empty slots are "Slot N", macros "Macro N" until named.
    assert_eq!(names[8 + 5], "Slot 6");
    assert_eq!(names[7], "Macro 8");
    // A full patch: every slot assigned still gives a non-empty name each.
    let mut full = common::custom_patch();
    for i in 0..SLOTS {
        full.slots[i] = Some(common::addr(if i % 2 == 0 {
            "filter1.cutoff"
        } else {
            "voice.glide"
        }));
    }
    assert!(host_param_names(&full).iter().all(|n| !n.is_empty()));
}

#[test]
fn the_host_rescans_only_when_a_name_or_a_kind_of_target_changes() {
    let base = common::custom_patch();
    let with = |ops: Vec<Op>| {
        let mut p = base.clone();
        enth_patch::apply_all(&mut p, &ops).unwrap();
        p
    };
    let label = |id: &str, text: Option<&str>| Op::SetLabel {
        id: common::id(id),
        label: text.map(str::to_owned),
    };
    // Nothing changed.
    assert!(!host_layout_changed(&base, &base));
    // A slot's module is renamed: its name changes.
    assert!(host_layout_changed(
        &base,
        &with(vec![label("filter1", Some("Wah"))])
    ));
    // A module no slot drives is renamed: the host sees nothing.
    assert!(!host_layout_changed(
        &base,
        &with(vec![label("lfo1", Some("Wobble"))])
    ));
    // A macro is named, a slot assigned, a slot cleared.
    assert!(host_layout_changed(
        &base,
        &with(vec![Op::SetMacroName {
            index: common::macro_index(2),
            name: Some("Air".to_owned())
        }])
    ));
    assert!(host_layout_changed(
        &base,
        &with(vec![Op::SetSlot {
            slot: common::slot(3),
            addr: Some(common::addr("filter1.resonance"))
        }])
    ));
    assert!(host_layout_changed(
        &base,
        &with(vec![Op::SetSlot {
            slot: common::slot(1),
            addr: None
        }])
    ));
    // Values, routes and metadata are not host-visible layout.
    assert!(!host_layout_changed(
        &base,
        &with(vec![Op::SetParam {
            addr: common::addr("filter1.cutoff"),
            value: 900.0
        }])
    ));
    // A slot moves to a parameter of another kind that has the same display name: the value text differs.
    let mut a = common::init_patch();
    enth_patch::apply_all(
        &mut a,
        &[
            Op::AddModule {
                at: enth_patch::Position {
                    section: enth_patch::Section::Generators,
                    index: 1,
                },
                module: enth_patch::ModuleInstance::new(
                    common::id("noise1"),
                    enth_patch::ModuleKind::Noise,
                ),
            },
            label("osc1", Some("Src")),
            label("noise1", Some("Src")),
            Op::SetSlot {
                slot: common::slot(1),
                addr: Some(common::addr("osc1.level")),
            },
        ],
    )
    .unwrap();
    let b = {
        let mut b = a.clone();
        enth_patch::apply(
            &mut b,
            Op::SetSlot {
                slot: common::slot(1),
                addr: Some(common::addr("noise1.level")),
            },
        )
        .unwrap();
        b
    };
    assert_eq!(
        host_param_name(&a, HostParam::Slot(common::slot(1))),
        "Src · Level"
    );
    assert_eq!(
        host_param_name(&b, HostParam::Slot(common::slot(1))),
        "Src · Level"
    );
    // Same name, other kind of parameter: the rescan follows the spec's identity (conservative when two kinds
    // happen to share a spec).
    let (osc, noise) = (
        a.spec(&common::addr("osc1.level")).unwrap(),
        b.spec(&common::addr("noise1.level")).unwrap(),
    );
    assert_eq!(host_layout_changed(&a, &b), !std::ptr::eq(osc, noise));
    // Undo-like: the same layout twice is unchanged.
    assert!(!host_layout_changed(&b, &b.clone()));
}
