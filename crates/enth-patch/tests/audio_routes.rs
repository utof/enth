//! Audio routes between generators (S2a lane RTE): what the catalogue promises, what `validate` and the ops accept and
//! refuse (the codes themselves are pinned in `rejections.rs`), that ops with audio routes apply and invert exactly,
//! that a loop can never be built, and that the canonical form and the hashes treat a generator-sourced route like
//! any other. Fixed proptest seed from `.cargo/config.toml`; default case counts.

mod common;

use common::*;
use enth_patch::addr::{ModSource, ModuleId, ParamAddr, Position, RouteAddr};
use enth_patch::canon::{canonical_effort, canonical_hash, canonicalise, exact_hash};
use enth_patch::catalogue::{CATALOGUE, ModuleKind};
use enth_patch::file::load;
use enth_patch::model::{ModuleInstance, Patch, Route, Section};
use enth_patch::ops::{Op, apply, apply_all};
use enth_patch::validate::{
    RouteKind, audio_into_sync_master, check_route_ends, closes_audio_cycle, validate, violations,
};
use proptest::prelude::*;

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).unwrap()
}

fn add(p: &mut Patch, name: &str, kind: ModuleKind) {
    let index = p.section(kind.section()).len();
    let at = Position {
        section: kind.section(),
        index,
    };
    apply(
        p,
        &Op::AddModule {
            at,
            module: ModuleInstance::new(id(name), kind),
        },
    )
    .unwrap();
}

fn route(src: &str, dst: &str, depth: f32) -> Route {
    Route {
        src: ModSource::Module(id(src)),
        dst: dst.parse().unwrap(),
        depth,
    }
}

fn insert(
    p: &mut Patch,
    src: &str,
    dst: &str,
    depth: f32,
) -> Result<Vec<Op>, enth_patch::PatchError> {
    let index = p.routes.len();
    apply(
        p,
        &Op::InsertRoute {
            index,
            route: route(src, dst, depth),
        },
    )
}

/// `osc1` (the base patch's), `noise1`, `wt1`, `wt2` and `wt3`... four generators at most: osc1, wt1, wt2, wt3.
fn four() -> Patch {
    let mut p = base_patch();
    for name in ["wt1", "wt2", "wt3"] {
        add(&mut p, name, ModuleKind::Wt);
    }
    validate(&p).unwrap();
    p
}

#[test]
fn audio_parameters_are_inputs_of_generators_and_never_modulatable() {
    // The engine runs audio routes into generators only (their pre-level buffers are read before the voice
    // effects), and a control route can never reach an audio input: a new `audio` parameter that breaks either
    // belongs to a lane that also builds the call site.
    let mut seen = 0;
    for spec in &CATALOGUE {
        for p in spec.params {
            if p.flags.audio {
                seen += 1;
                assert_eq!(
                    spec.section,
                    Section::Generators,
                    "{}.{} is an audio input outside the generators",
                    spec.key,
                    p.key
                );
                assert!(
                    !p.flags.modulatable,
                    "{}.{} is an audio input and modulatable",
                    spec.key, p.key
                );
            }
        }
    }
    assert!(seen >= 2, "wt.pm and wt.am at least");
}

#[test]
fn the_ends_of_a_route_are_classified() {
    let p = four();
    let kind = |src: &str, dst: &str| {
        let e = check_route_ends(&p, &route(src, dst, 10.0));
        (
            e.kind,
            e.src_error.map(|e| e.code()),
            e.dst_error.map(|e| e.code()),
        )
    };
    assert_eq!(kind("osc1", "wt1.pm"), (Some(RouteKind::Audio), None, None));
    assert_eq!(kind("wt1", "wt2.am"), (Some(RouteKind::Audio), None, None));
    assert_eq!(
        kind("env1", "osc1.semi"),
        (Some(RouteKind::Control), None, None)
    );
    assert_eq!(
        kind("osc1", "wt1.level"),
        (None, None, Some("not_audio_dest"))
    );
    assert_eq!(
        kind("osc1", "osc1.semi"),
        (None, None, Some("not_audio_dest"))
    );
    assert_eq!(
        kind("env1", "wt1.pm"),
        (None, Some("not_audio_source"), None)
    );
    assert_eq!(
        kind("env1", "wt1.cutoff"),
        (None, None, Some("unknown_param"))
    );
    assert_eq!(kind("nope", "wt1.pm"), (None, Some("unknown_module"), None));
    let velocity = Route {
        src: ModSource::Velocity,
        dst: "wt1.am".parse().unwrap(),
        depth: 1.0,
    };
    assert_eq!(
        check_route_ends(&p, &velocity).src_error.map(|e| e.code()),
        Some("not_audio_source")
    );
}

#[test]
fn chains_and_diamonds_are_valid_and_loops_are_not() {
    let mut p = four();
    // A chain osc1 -> wt1 -> wt2 -> wt3 ...
    insert(&mut p, "osc1", "wt1.pm", 40.0).unwrap();
    insert(&mut p, "wt1", "wt2.pm", 40.0).unwrap();
    insert(&mut p, "wt2", "wt3.am", 40.0).unwrap();
    // ... a diamond: osc1 reaches wt3 by two ways, and wt1 feeds wt3 as well.
    insert(&mut p, "osc1", "wt3.pm", 10.0).unwrap();
    insert(&mut p, "wt1", "wt3.pm", 10.0).unwrap();
    validate(&p).unwrap();
    // Every way back is a loop, and the refusal names the route that closes it.
    for (src, dst) in [
        ("wt3", "osc1.semi"), // not an audio input at all
        ("wt3", "wt1.pm"),
        ("wt3", "wt2.am"),
        ("wt2", "wt1.am"),
        ("wt1", "wt1.pm"),
        ("wt3", "wt3.pm"),
    ] {
        let before = p.clone();
        let e = insert(&mut p, src, dst, 5.0).unwrap_err();
        let want = if dst == "osc1.semi" {
            "not_audio_dest"
        } else {
            "audio_cycle"
        };
        assert_eq!(e.code(), want, "{src} -> {dst}: {e}");
        assert!(identical(&p, &before));
    }
    // The same loops written into a file are found, on the route that closes them.
    let mut q = p.clone();
    q.routes.push(route("wt3", "wt1.pm", 5.0));
    q.routes.push(route("wt2", "wt3.pm", 5.0)); // a repeat of an edge that exists is no loop
    let v = violations(&q);
    let codes: Vec<(&str, &str)> = v
        .iter()
        .map(|v| (v.path.as_str(), v.error.code()))
        .collect();
    assert_eq!(codes, [("routes[5]", "audio_cycle")], "{v:?}");
    assert!(closes_audio_cycle(&p, &route("wt3", "wt1.pm", 5.0)));
    assert!(!closes_audio_cycle(&p, &route("osc1", "wt2.pm", 5.0)));
    assert!(!closes_audio_cycle(
        &p,
        &Route {
            src: ModSource::Velocity,
            dst: "wt1.pm".parse().unwrap(),
            depth: 5.0
        }
    ));
}

#[test]
fn the_hard_sync_rule_never_fires_on_todays_catalogue() {
    // The master is the first enabled osc; with no osc input an audio route can never reach it, so every wiring the
    // other rules accept passes the sync rule too (F1m S4; `validate.rs` tests the rule itself with a kind that has
    // an audio input). The patch is `osc1` first, then three `wt`; also with osc1 moved last and with it disabled.
    for variant in 0..3 {
        let mut p = four();
        match variant {
            1 => {
                apply(
                    &mut p,
                    &Op::Move {
                        id: id("osc1"),
                        to: Position {
                            section: Section::Generators,
                            index: 3,
                        },
                    },
                )
                .unwrap();
            }
            2 => {
                apply(
                    &mut p,
                    &Op::SetEnabled {
                        id: id("osc1"),
                        enabled: false,
                    },
                )
                .unwrap();
            }
            _ => {}
        }
        for src in ["osc1", "wt1", "wt2", "wt3"] {
            for dst in ["wt1.pm", "wt2.am", "wt3.pm", "osc1.level"] {
                let r = route(src, dst, 20.0);
                assert!(!audio_into_sync_master(&p, &r), "{src} -> {dst}");
                if let Err(e) = insert(&mut p, src, dst, 20.0) {
                    assert_ne!(e.code(), "audio_into_sync_master", "{src} -> {dst}");
                }
            }
        }
        assert!(
            violations(&p)
                .iter()
                .all(|v| v.error.code() != "audio_into_sync_master")
        );
    }
}

#[test]
fn a_disabled_module_still_counts_for_loops() {
    // A loop through a bypassed generator would appear the moment it is enabled, so it is refused now.
    let mut p = four();
    insert(&mut p, "wt1", "wt2.pm", 20.0).unwrap();
    insert(&mut p, "wt2", "wt3.pm", 20.0).unwrap();
    apply(
        &mut p,
        &Op::SetEnabled {
            id: id("wt2"),
            enabled: false,
        },
    )
    .unwrap();
    assert_eq!(
        insert(&mut p, "wt3", "wt1.am", 20.0).unwrap_err().code(),
        "audio_cycle"
    );
    // ... and bypassing or enabling never builds one.
    apply(
        &mut p,
        &Op::SetEnabled {
            id: id("wt2"),
            enabled: true,
        },
    )
    .unwrap();
    validate(&p).unwrap();
}

#[test]
fn an_audio_route_is_the_same_depth_as_any_other() {
    let mut p = four();
    assert_eq!(
        insert(&mut p, "osc1", "wt1.pm", 100.5).unwrap_err().code(),
        "out_of_range"
    );
    insert(&mut p, "osc1", "wt1.pm", -100.0).unwrap();
    insert(&mut p, "osc1", "wt1.am", 100.0).unwrap();
    let r: RouteAddr = "osc1 -> wt1.pm".parse().unwrap();
    apply(
        &mut p,
        &Op::SetDepth {
            route: r,
            depth: 12.5,
        },
    )
    .unwrap();
    validate(&p).unwrap();
    // A depth of 0 is a route that does nothing; it is still a route (and still refuses a loop).
    insert(&mut p, "wt1", "wt2.pm", 0.0).unwrap();
    assert_eq!(
        insert(&mut p, "wt2", "wt1.pm", 0.0).unwrap_err().code(),
        "audio_cycle"
    );
}

#[test]
fn removing_a_generator_takes_its_audio_routes_and_the_inverse_puts_them_back() {
    let mut p = four();
    insert(&mut p, "osc1", "wt1.pm", 40.0).unwrap();
    insert(&mut p, "wt1", "wt2.pm", 30.0).unwrap();
    insert(&mut p, "wt2", "wt3.am", 20.0).unwrap();
    insert(&mut p, "osc1", "wt3.pm", 10.0).unwrap();
    let before = p.clone();
    let inv = apply(&mut p, &Op::RemoveModule { id: id("wt2") }).unwrap();
    assert_eq!(p.routes.len(), 2);
    validate(&p).unwrap();
    for op in &inv {
        apply(&mut p, op).unwrap();
    }
    assert!(identical(&p, &before));
}

#[test]
fn files_with_audio_routes_round_trip_bit_for_bit() {
    let mut p = four();
    insert(&mut p, "osc1", "wt1.pm", 33.25).unwrap();
    insert(&mut p, "wt1", "wt2.am", -7.5).unwrap();
    let json = p.to_json();
    assert!(
        json.contains("\"src\":\"osc1\"") && json.contains("wt1.pm"),
        "{json}"
    );
    let loaded = load(&json).unwrap();
    assert_eq!(loaded.notes.len(), 0);
    assert!(identical(&loaded.patch, &p));
    assert_eq!(loaded.patch.to_json(), json);
    assert_eq!(exact_hash(&loaded.patch), exact_hash(&p));
}

// ---------------------------------------------------------------------------------------------------------------
// Apply and inverse over op sequences that are mostly audio routes
// ---------------------------------------------------------------------------------------------------------------

/// An op chosen from raw numbers, with a taste for audio routes: inserts between generators (valid, a loop, to a
/// parameter that is not an input, from a control source), removals, depth changes, and the structural ops that
/// move, bypass, add or remove the generators around them.
fn audio_op(p: &Patch, (c, a, b, bits): Raw, fresh: &mut u32) -> Op {
    let gens: Vec<ModuleId> = p.generators.iter().map(|m| m.id.clone()).collect();
    let pick = |ids: &[ModuleId], i: u8| ids[usize::from(i) % ids.len()].clone();
    let any_gen = |i: u8| {
        if gens.is_empty() {
            id("nope")
        } else {
            pick(&gens, i)
        }
    };
    let depth = match bits & 7 {
        0 => 0.0,
        1 => 100.0,
        2 => -100.0,
        3 => 100.5,
        _ => any_f32_in(bits >> 3, -100.0, 100.0),
    };
    match c % 12 {
        // A generator into one of the audio inputs (or, now and then, into another parameter).
        0..=4 => {
            let inputs = params_where(p, |s| s.flags.audio);
            let mut dst = if inputs.is_empty() || bits & 0x700 == 0 {
                ParamAddr::module(any_gen(b), "level")
                    .unwrap_or_else(|_| "osc1.semi".parse().unwrap())
            } else {
                inputs[usize::from(b) % inputs.len()].clone()
            };
            let mut src = ModSource::Module(any_gen(a));
            if bits & 0x1800 == 0 {
                // a control source into an audio input
                src = ModSource::Velocity;
            } else if bits & 0x6000 == 0
                && let Some(r) = p.routes.get(usize::from(a) % p.routes.len().max(1))
            {
                // a route that is already there
                (src, dst) = (r.src.clone(), r.dst.clone());
            }
            Op::InsertRoute {
                index: usize::from(b) % (p.routes.len() + 1),
                route: Route { src, dst, depth },
            }
        }
        5 => match p.routes.get(usize::from(a) % p.routes.len().max(1)) {
            Some(r) => Op::RemoveRoute {
                route: RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                },
            },
            None => Op::SetEnabled {
                id: any_gen(a),
                enabled: true,
            },
        },
        6 => match p.routes.get(usize::from(a) % p.routes.len().max(1)) {
            Some(r) => Op::SetDepth {
                route: RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                },
                depth: any_f32_in(bits, -100.0, 100.0),
            },
            None => Op::SetEnabled {
                id: any_gen(a),
                enabled: true,
            },
        },
        7 | 8 => {
            *fresh += 1;
            let kinds = [
                ModuleKind::Wt,
                ModuleKind::Wt,
                ModuleKind::Osc,
                ModuleKind::Noise,
            ];
            let kind = kinds[usize::from(a) % kinds.len()];
            let mut m = ModuleInstance::new(id(&format!("g{fresh}")), kind);
            m.enabled = bits & 0x100 == 0;
            Op::AddModule {
                at: Position {
                    section: Section::Generators,
                    index: usize::from(b) % (p.generators.len() + 2),
                },
                module: m,
            }
        }
        9 => Op::RemoveModule { id: any_gen(a) },
        10 => Op::Move {
            id: any_gen(a),
            to: Position {
                section: Section::Generators,
                index: usize::from(b) % (p.generators.len() + 1),
            },
        },
        _ => Op::SetEnabled {
            id: any_gen(a),
            enabled: b % 3 != 0,
        },
    }
}

fn audio_build(raws: &[Raw]) -> Patch {
    let mut p = four();
    let mut fresh = 0;
    for r in raws {
        let op = audio_op(&p, *r, &mut fresh);
        let _ = apply(&mut p, &op);
    }
    p
}

proptest! {
    /// An op either applies (leaving a valid patch with no audio loop, and its inverse restores the original bit for
    /// bit) or fails with a typed error and changes nothing.
    #[test]
    fn audio_ops_apply_then_invert(setup in raws(0..80), r in raw()) {
        let mut p = audio_build(&setup);
        prop_assert!(validate(&p).is_ok(), "{:?}", validate(&p));
        let before = p.clone();
        let mut fresh = 5000;
        let op = audio_op(&p, r, &mut fresh);
        match apply(&mut p, &op) {
            Ok(inv) => {
                prop_assert!(validate(&p).is_ok(), "{op:?} -> {:?}", validate(&p));
                for o in &inv {
                    apply(&mut p, o).map_err(|e| TestCaseError::fail(format!("inverse {o:?}: {e}")))?;
                }
                prop_assert!(identical(&p, &before), "{op:?}\ninverse {inv:?}");
            }
            Err(e) => {
                assert_typed(&e);
                prop_assert!(identical(&p, &before), "a failed {op:?} changed the patch ({e})");
            }
        }
    }

    /// Transactions of audio ops are atomic and invertible, and redoing reproduces the result.
    #[test]
    fn audio_transactions_are_atomic_and_invertible(setup in raws(0..40), rs in raws(1..14)) {
        let mut p = audio_build(&setup);
        let before = p.clone();
        let mut scratch = p.clone();
        let mut fresh = 6000;
        let mut ops = Vec::new();
        for r in &rs {
            let op = audio_op(&scratch, *r, &mut fresh);
            let _ = apply(&mut scratch, &op);
            ops.push(op);
        }
        match apply_all(&mut p, &ops) {
            Ok(inv) => {
                prop_assert!(validate(&p).is_ok());
                let after = p.clone();
                apply_all(&mut p, &inv).map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert!(identical(&p, &before));
                apply_all(&mut p, &ops).map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert!(identical(&p, &after));
            }
            Err(tx) => {
                assert_typed(&tx.error);
                prop_assert!(identical(&p, &before), "failed at {} but the patch changed", tx.index);
            }
        }
    }

    /// The census: the generator really does build audio routes, and meets every refusal.
    #[test]
    fn the_audio_generator_reaches_its_cases(setups in prop::collection::vec(raws(60..61), 8)) {
        let mut applied = 0;
        let mut codes = std::collections::BTreeSet::new();
        for setup in &setups {
            let mut p = four();
            let mut fresh = 0;
            for r in setup {
                let op = audio_op(&p, *r, &mut fresh);
                match apply(&mut p, &op) {
                    Ok(_) => {
                        if matches!(op, Op::InsertRoute { .. }) {
                            applied += 1;
                        }
                    }
                    Err(e) => {
                        codes.insert(e.code());
                    }
                }
            }
        }
        prop_assert!(applied >= 8, "only {applied} audio routes were ever inserted");
        for code in ["audio_cycle", "not_audio_source", "not_audio_dest", "duplicate_route"] {
            prop_assert!(codes.contains(code), "{code} never produced: {codes:?}");
        }
    }

    /// The canonical form of a patch with audio routes: valid, idempotent, and the same however the patch is
    /// written (ids renamed, generators and routes shuffled).
    #[test]
    fn canonical_form_of_audio_routes_survives_the_symmetries(setup in raws(0..80), seed in any::<u64>()) {
        let p = audio_build(&setup);
        let c = canonicalise(&p);
        prop_assert!(validate(&c).is_ok(), "{:?}", validate(&c));
        prop_assert!(identical(&canonicalise(&c), &c));
        let q = scramble(&p, seed);
        prop_assert!(validate(&q).is_ok(), "{:?}", validate(&q));
        prop_assert_eq!(canonical_hash(&p), canonical_hash(&q));
        prop_assert!(identical(&canonicalise(&p), &canonicalise(&q)));
        // a route the canonical form keeps is one the patch had, between modules it kept
        prop_assert!(c.routes.len() <= p.routes.len());
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The canonical form and the hashes
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_hashes_hear_where_an_audio_route_goes() {
    let build = |routes: &[(&str, &str, f32)]| {
        let mut p = four();
        for (src, dst, depth) in routes {
            insert(&mut p, src, dst, *depth).unwrap();
        }
        p
    };
    let base = build(&[("osc1", "wt1.pm", 50.0)]);
    let same = build(&[("osc1", "wt1.pm", 50.0)]);
    assert_eq!(canonical_hash(&base), canonical_hash(&same));
    // Which input, which carrier, which source and which depth are all sound.
    let others = vec![
        build(&[("osc1", "wt1.am", 50.0)]),
        build(&[("osc1", "wt1.pm", 51.0)]),
        build(&[("osc1", "wt1.pm", -50.0)]),
        build(&[("wt2", "wt1.pm", 50.0)]),
        build(&[("osc1", "wt1.pm", 50.0), ("osc1", "wt1.am", 50.0)]),
        build(&[]),
    ];
    for other in &others {
        assert_ne!(canonical_hash(&base), canonical_hash(other));
        assert_ne!(exact_hash(&base), exact_hash(other));
    }
    // Two identical carriers are twins; which of them the route reaches is only a name.
    let a = build(&[("osc1", "wt2.pm", 50.0)]);
    let b = build(&[("osc1", "wt3.pm", 50.0)]);
    assert_eq!(canonical_hash(&a), canonical_hash(&b));
    assert_ne!(exact_hash(&a), exact_hash(&b));
    assert_eq!(canonical_effort(&a), 1);
}

#[test]
fn a_silent_generator_that_feeds_a_route_is_not_dropped() {
    // `level` 0 hides a generator from the mix, not from the carrier it feeds (the matcher's "silent is absent"
    // gains the clause "unless an audio route leaves it"; the canonical form keeps the module and the route).
    let mut p = four();
    insert(&mut p, "osc1", "wt1.pm", 50.0).unwrap();
    apply(
        &mut p,
        &Op::SetParam {
            addr: "osc1.level".parse().unwrap(),
            value: 0.0,
        },
    )
    .unwrap();
    let c = canonicalise(&p);
    assert_eq!(c.generators.len(), 4);
    assert_eq!(c.routes.len(), 1);
    assert_eq!(c.routes[0].depth, 50.0);
}

#[test]
fn bypassing_either_end_takes_an_audio_route_out_of_the_canonical_form() {
    for end in ["osc1", "wt1"] {
        let mut p = four();
        insert(&mut p, "osc1", "wt1.pm", 50.0).unwrap();
        apply(
            &mut p,
            &Op::SetEnabled {
                id: id(end),
                enabled: false,
            },
        )
        .unwrap();
        let c = canonicalise(&p);
        assert!(c.routes.is_empty(), "{end}");
        validate(&c).unwrap();
    }
    // A depth-0 audio route is the same patch as no route.
    let mut p = four();
    insert(&mut p, "osc1", "wt1.pm", 0.0).unwrap();
    assert_eq!(canonical_hash(&p), canonical_hash(&four()));
}
