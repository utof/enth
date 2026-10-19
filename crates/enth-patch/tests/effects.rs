//! How an op reaches the audio (`ops::effect`), the op names, `Op::apply` and the text of a failed transaction
//! (S1b). `effect` decides in `enth-session` whether an edit is a smoothed value event or a plan recompile, so
//! each op kind is pinned, and for `SetParam` every parameter of the catalogue is checked against its
//! `structural` flag.

mod common;

use common::*;
use enth_patch::addr::{MacroIndex, ModuleId, ParamAddr, Position, RouteAddr, SlotIndex, Target};
use enth_patch::catalogue::{ModuleKind, VOICE_PARAMS};
use enth_patch::model::{ModuleInstance, Patch, PatchMeta, Route, Section};
use enth_patch::ops::{Effect, Op, TxError, apply, apply_all, effect};

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).unwrap()
}

/// `base_patch` plus a filter and an LFO with one route between them.
fn routed() -> Patch {
    let mut p = base_patch();
    for (name, kind) in [("filter1", ModuleKind::Svf), ("lfo1", ModuleKind::Lfo)] {
        let section = kind.section();
        let at = Position {
            section,
            index: p.section(section).len(),
        };
        let module = ModuleInstance::new(id(name), kind);
        apply(&mut p, Op::AddModule { at, module }).unwrap();
    }
    let r: RouteAddr = "lfo1 -> filter1.cutoff".parse().unwrap();
    apply(
        &mut p,
        Op::InsertRoute {
            index: 0,
            route: Route {
                src: r.src,
                dst: r.dst,
                depth: 25.0,
            },
        },
    )
    .unwrap();
    p
}

#[test]
fn set_param_effect_follows_the_structural_flag_of_every_parameter() {
    let p = routed();
    let (mut structural, mut smooth) = (0, 0);
    let mut check = |addr: ParamAddr, flag: bool| {
        let op = Op::SetParam {
            addr: addr.clone(),
            value: 0.0,
        };
        let got = effect(&p, &op);
        if flag {
            structural += 1;
            assert_eq!(got, Effect::Structural, "{addr}");
        } else {
            smooth += 1;
            assert_eq!(got, Effect::Value(addr.clone()), "{addr}");
        }
    };
    for (_, m) in p.modules() {
        for spec in m.kind.params() {
            check(
                ParamAddr::module(m.id.clone(), spec.key).unwrap(),
                spec.flags.structural,
            );
        }
    }
    for spec in VOICE_PARAMS {
        check(
            ParamAddr::new(Target::Voice, spec.key).unwrap(),
            spec.flags.structural,
        );
    }
    // The check is only meaningful if both kinds exist in the catalogue.
    assert!(structural > 0 && smooth > 0, "{structural} {smooth}");
}

#[test]
fn the_named_structural_parameters_recompile() {
    let p = routed();
    for addr in ["lfo1.scope", "voice.polyphony"] {
        let op = Op::SetParam {
            addr: addr.parse().unwrap(),
            value: 1.0,
        };
        assert_eq!(effect(&p, &op), Effect::Structural, "{addr}");
    }
    for addr in ["filter1.cutoff", "osc1.semi", "voice.pan", "master.level"] {
        let a: ParamAddr = addr.parse().unwrap();
        let op = Op::SetParam {
            addr: a.clone(),
            value: 0.0,
        };
        assert_eq!(effect(&p, &op), Effect::Value(a), "{addr}");
    }
}

#[test]
fn a_set_param_that_would_not_apply_is_reported_structural() {
    let p = routed();
    for addr in ["nosuch.cutoff", "lfo1.cutoff"] {
        let op = Op::SetParam {
            addr: addr.parse().unwrap(),
            value: 0.0,
        };
        assert_eq!(effect(&p, &op), Effect::Structural, "{addr}");
    }
}

#[test]
fn every_other_op_has_its_effect() {
    let p = routed();
    let route: RouteAddr = "lfo1 -> filter1.cutoff".parse().unwrap();
    let macro2 = MacroIndex::new(2).unwrap();
    let at = Position {
        section: Section::Fx,
        index: 0,
    };
    let reverb = ModuleInstance::new(id("reverb1"), ModuleKind::Reverb);

    let depth = Op::SetDepth {
        route: route.clone(),
        depth: 5.0,
    };
    assert_eq!(effect(&p, &depth), Effect::Depth(route.clone()));
    let knob = Op::SetMacro {
        index: macro2,
        value: 0.5,
    };
    assert_eq!(effect(&p, &knob), Effect::Macro(macro2));

    let quiet = [
        Op::SetSlot {
            slot: SlotIndex::new(0).unwrap(),
            addr: None,
        },
        Op::SetLabel {
            id: id("filter1"),
            label: Some("tone".into()),
        },
        Op::SetMeta {
            meta: PatchMeta::default(),
        },
        Op::SetMacroName {
            index: macro2,
            name: Some("Brightness".into()),
        },
    ];
    for op in &quiet {
        assert_eq!(effect(&p, op), Effect::NoAudio, "{}", op.name());
    }

    let structural = [
        Op::AddModule { at, module: reverb },
        Op::RemoveModule { id: id("filter1") },
        Op::Move {
            id: id("osc1"),
            to: Position {
                section: Section::Generators,
                index: 0,
            },
        },
        Op::InsertRoute {
            index: 0,
            route: Route {
                src: route.src.clone(),
                dst: route.dst.clone(),
                depth: 1.0,
            },
        },
        Op::RemoveRoute {
            route: route.clone(),
        },
        Op::SetEnabled {
            id: id("filter1"),
            enabled: false,
        },
        Op::SetAmpEnv { id: id("env1") },
    ];
    for op in &structural {
        assert_eq!(effect(&p, op), Effect::Structural, "{}", op.name());
    }
}

#[test]
fn every_op_has_its_own_name() {
    let route: RouteAddr = "lfo1 -> filter1.cutoff".parse().unwrap();
    let at = Position {
        section: Section::Fx,
        index: 0,
    };
    let ops = [
        (
            "SetParam",
            Op::SetParam {
                addr: "voice.pan".parse().unwrap(),
                value: 0.0,
            },
        ),
        (
            "AddModule",
            Op::AddModule {
                at,
                module: ModuleInstance::new(id("reverb1"), ModuleKind::Reverb),
            },
        ),
        ("RemoveModule", Op::RemoveModule { id: id("a") }),
        (
            "Move",
            Op::Move {
                id: id("a"),
                to: at,
            },
        ),
        (
            "InsertRoute",
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: route.src.clone(),
                    dst: route.dst.clone(),
                    depth: 0.0,
                },
            },
        ),
        (
            "RemoveRoute",
            Op::RemoveRoute {
                route: route.clone(),
            },
        ),
        ("SetDepth", Op::SetDepth { route, depth: 0.0 }),
        (
            "SetEnabled",
            Op::SetEnabled {
                id: id("a"),
                enabled: true,
            },
        ),
        ("SetAmpEnv", Op::SetAmpEnv { id: id("a") }),
        (
            "SetMacro",
            Op::SetMacro {
                index: MacroIndex::new(0).unwrap(),
                value: 0.0,
            },
        ),
        (
            "SetSlot",
            Op::SetSlot {
                slot: SlotIndex::new(0).unwrap(),
                addr: None,
            },
        ),
        (
            "SetLabel",
            Op::SetLabel {
                id: id("a"),
                label: None,
            },
        ),
        (
            "SetMeta",
            Op::SetMeta {
                meta: PatchMeta::default(),
            },
        ),
        (
            "SetMacroName",
            Op::SetMacroName {
                index: MacroIndex::new(0).unwrap(),
                name: None,
            },
        ),
    ];
    for (name, op) in &ops {
        assert_eq!(op.name(), *name);
    }
}

#[test]
fn the_apply_method_is_the_apply_function() {
    let op = Op::SetParam {
        addr: "filter1.cutoff".parse().unwrap(),
        value: 1234.0,
    };
    let (mut by_fn, mut by_method) = (routed(), routed());
    let inverse = apply(&mut by_fn, &op).unwrap();
    assert_eq!(op.apply(&mut by_method).unwrap(), inverse);
    assert_eq!(inverse.len(), 1);
    assert!(identical(&by_fn, &by_method));

    let bad = Op::SetParam {
        addr: "nosuch.cutoff".parse().unwrap(),
        value: 0.0,
    };
    let before = by_method.clone();
    let want = apply(&mut by_fn, &bad).unwrap_err();
    assert_eq!(bad.apply(&mut by_method).unwrap_err(), want);
    assert!(identical(&by_method, &before));
}

#[test]
fn a_failed_transaction_names_the_op_and_says_why() {
    let mut p = routed();
    let before = p.clone();
    let good = Op::SetParam {
        addr: "filter1.cutoff".parse().unwrap(),
        value: 500.0,
    };
    let bad = Op::SetParam {
        addr: "nosuch.cutoff".parse().unwrap(),
        value: 0.0,
    };

    let tx = apply_all(&mut p, &[good.clone(), bad.clone()]).unwrap_err();
    assert_eq!(tx.index, 1);
    assert!(identical(&p, &before));
    let text = tx.to_string();
    assert_eq!(text, format!("op 1: {}", tx.error));
    assert!(text.contains("nosuch"), "{text}");

    // A lone op reports index 0, with the same text shape.
    let tx = apply_all(&mut p, &[bad]).unwrap_err();
    assert_eq!(tx.index, 0);
    assert!(tx.to_string().starts_with("op 0: "), "{tx}");

    // It is an error type in its own right, with no further cause.
    let err: &dyn std::error::Error = &tx;
    assert!(err.source().is_none());
    let copy: TxError = tx.clone();
    assert_eq!(tx, copy);
}
