//! Canonical form and the two hashes (S1b): invariance under what cannot be heard, sensitivity to what can, and the
//! hard-sync exception. Fixed proptest seed from `.cargo/config.toml`; default case counts.

mod common;

use common::*;
use enth_patch::addr::{ModSource, ModuleId, ParamAddr, Position};
use enth_patch::canon::{LEAF_BUDGET, canonical_effort, canonical_hash, canonicalise, exact_hash};
use enth_patch::catalogue::{LfoParam, ModuleKind, OscParam};
use enth_patch::model::{ModuleInstance, Patch, PatchMeta, Route, Section};
use enth_patch::ops::{Op, apply, canonical_value};
use enth_patch::validate::validate;
use proptest::prelude::*;

/// Everything about a patch that no listener (and no render) can tell apart: stamp, meta, labels, macro names,
/// slots, a disabled module in front of the generators with routes through it, and a route of depth 0.
fn with_noise(p: &Patch, bits: u32) -> Patch {
    let mut q = p.clone();
    q.engine = format!("9.{}.0", bits % 7);
    q.meta = PatchMeta {
        name: text(bits),
        author: text(bits >> 3),
        description: text(bits >> 6),
        tags: vec![text(bits >> 9)],
    };
    for (i, section) in Section::ALL.into_iter().enumerate() {
        for (j, m) in q.section_mut(section).iter_mut().enumerate() {
            m.label = opt_text(bits.rotate_left((i * 5 + j) as u32));
        }
    }
    for (i, m) in q.macros.iter_mut().enumerate() {
        m.name = opt_text(bits.rotate_left(i as u32 * 3 + 1));
    }
    let targets = automatable_targets(&q);
    for (i, slot) in [0usize, 7, 63].into_iter().enumerate() {
        let addr = targets[(bits as usize >> (i * 4)) % targets.len()].clone();
        let _ = apply(
            &mut q,
            &Op::SetSlot {
                slot: enth_patch::addr::SlotIndex::new(slot).unwrap(),
                addr: Some(addr),
            },
        );
    }
    // A disabled module (of a kind picked from the bits) at the front of its section, wired in both directions.
    let kind = ModuleKind::ALL[bits as usize % ModuleKind::ALL.len()];
    let mut m = ModuleInstance::new(q.free_id(kind), kind);
    m.enabled = false;
    let id = m.id.clone();
    let at = Position {
        section: kind.section(),
        index: 0,
    };
    if apply(&mut q, &Op::AddModule { at, module: m }).is_ok() {
        let into = mod_targets(&q)
            .into_iter()
            .find(|a| a.module_id().is_some_and(|x| *x != id));
        if kind.section() == Section::Modulators
            && let Some(dst) = into
        {
            let route = Route {
                src: ModSource::Module(id.clone()),
                dst,
                depth: 50.0,
            };
            let _ = apply(&mut q, &Op::InsertRoute { index: 0, route });
        }
        if let Some(dst) = mod_targets(&q)
            .into_iter()
            .find(|a| a.module_id() == Some(&id))
        {
            let route = Route {
                src: ModSource::Velocity,
                dst,
                depth: 25.0,
            };
            let _ = apply(&mut q, &Op::InsertRoute { index: 0, route });
        }
    }
    // A depth-0 route on a live pair that has none yet.
    let mut dsts = mod_targets(&q);
    dsts.reverse();
    'found: for dst in dsts {
        for src in [ModSource::ModWheel, ModSource::Key] {
            let index = q.routes.len();
            let route = Route {
                src,
                dst: dst.clone(),
                depth: 0.0,
            };
            if apply(&mut q, &Op::InsertRoute { index, route }).is_ok() {
                break 'found;
            }
        }
    }
    q
}

proptest! {
    // Some properties skip cases that do not apply (an op refused by the rate rule, a patch without enough
    // oscillators); with the nightly's larger case counts the default limit of rejects would trip.
    #![proptest_config(ProptestConfig { max_global_rejects: 1 << 20, ..ProptestConfig::default() })]

    /// `canonicalise` is idempotent, produces a valid patch, and `canonical_hash` is the hash of its form.
    #[test]
    fn canonical_form_is_idempotent_and_valid(setup in raws(0..60)) {
        let p = build(&setup);
        let c = canonicalise(&p);
        prop_assert!(validate(&c).is_ok(), "{:?}", validate(&c));
        prop_assert!(identical(&canonicalise(&c), &c));
        prop_assert_eq!(canonical_hash(&p), canonical_hash(&c));
        prop_assert_eq!(canonical_hash(&p), canonical_hash(&p.clone()));
        // The form holds no disabled module, no depth-0 route and nothing that is only a note to the author.
        prop_assert!(c.modules().all(|(_, m)| m.enabled && m.label.is_none()));
        prop_assert!(c.routes.iter().all(|r| r.depth != 0.0));
        prop_assert!(c.slots.iter().all(Option::is_none) && c.macros.iter().all(|m| m.name.is_none()));
        prop_assert!(c.engine.is_empty() && c.meta == PatchMeta::default());
    }

    /// Same patch, written differently (ids renamed, commutative sections and the routes shuffled): same form.
    #[test]
    fn canonical_hash_survives_relabelling_and_reordering(setup in raws(0..60), seed in any::<u64>()) {
        let p = build(&setup);
        let q = scramble(&p, seed);
        prop_assert!(validate(&q).is_ok(), "the scramble broke the patch: {:?}", validate(&q));
        prop_assert_eq!(canonical_hash(&p), canonical_hash(&q));
        prop_assert!(identical(&canonicalise(&p), &canonicalise(&q)));
    }

    /// The same with hard sync on and several oscillators in play: the sync master stays first, the others move.
    #[test]
    fn canonical_hash_survives_symmetries_with_hard_sync(
        setup in raws(0..40), bits in any::<u32>(), extra in 1usize..5, seed in any::<u64>(),
    ) {
        let p = with_synced_oscs(&build(&setup), bits, extra);
        prop_assume!(sync_active(&p));
        let q = scramble(&p, seed);
        prop_assert!(validate(&q).is_ok());
        prop_assert_eq!(canonical_hash(&p), canonical_hash(&q));
    }

    /// Meta, labels, macro names, slots, the engine stamp, disabled modules (with their routes) and depth-0 routes
    /// change the exact hash only where they are content, and never the canonical one.
    #[test]
    fn canonical_hash_ignores_what_cannot_be_heard(setup in raws(0..60), bits in any::<u32>(), seed in any::<u64>()) {
        let p = build(&setup);
        let q = with_noise(&p, bits);
        prop_assert!(validate(&q).is_ok(), "{:?}", validate(&q));
        prop_assert_eq!(canonical_hash(&p), canonical_hash(&q));
        // ... also together with the symmetries
        prop_assert_eq!(canonical_hash(&p), canonical_hash(&scramble(&q, seed)));
    }

    /// Every change that can be heard changes the canonical hash: a parameter of an enabled module or of
    /// `voice`/`master`, a macro value, the depth of a live route, the amplitude envelope. A parameter of a
    /// disabled module is not heard.
    #[test]
    fn canonical_hash_hears_a_changed_parameter(setup in raws(1..60), pick in any::<u16>(), bits in any::<u32>()) {
        let p = build(&setup);
        let mut places: Vec<(ParamAddr, bool)> = Vec::new();
        for (_, m) in p.modules() {
            for spec in m.kind.params() {
                places.push((ParamAddr::module(m.id.clone(), spec.key).unwrap(), m.enabled));
            }
        }
        for s in &enth_patch::catalogue::VOICE_PARAMS {
            places.push((ParamAddr::new(enth_patch::addr::Target::Voice, s.key).unwrap(), true));
        }
        for s in &enth_patch::catalogue::MASTER_PARAMS {
            places.push((ParamAddr::new(enth_patch::addr::Target::Master, s.key).unwrap(), true));
        }
        let (addr, live) = places[usize::from(pick) % places.len()].clone();
        let spec = p.spec(&addr).unwrap();
        let old = p.value(&addr).unwrap();
        let candidates = [
            value_for(spec, bits),
            old.next_up(),
            old.next_down(),
            spec.min(),
            spec.max(),
            spec.default,
            old + 1.0,
            old - 1.0,
        ];
        let new = candidates
            .into_iter()
            .find_map(|v| canonical_value(spec, v).ok().filter(|c| c.to_bits() != old.to_bits()));
        prop_assume!(new.is_some());
        let mut q = p.clone();
        // (Changing an LFO's scope can be refused by the rate rule; that is not what this test is about.)
        prop_assume!(apply(&mut q, &Op::SetParam { addr: addr.clone(), value: new.unwrap() }).is_ok());
        if live {
            prop_assert_ne!(canonical_hash(&p), canonical_hash(&q), "{} {} -> {}", addr, old, new.unwrap());
        } else {
            prop_assert_eq!(canonical_hash(&p), canonical_hash(&q), "{} is on a disabled module", addr);
        }
        prop_assert_ne!(exact_hash(&p), exact_hash(&q), "the exact hash sees disabled modules too");
    }

    #[test]
    fn canonical_hash_hears_macros_depths_and_the_amp_envelope(setup in raws(1..60), pick in any::<u8>(), bits in any::<u32>()) {
        let p = build(&setup);
        // A macro value.
        let i = usize::from(pick) % 8;
        let mut q = p.clone();
        q.macros[i].value = if p.macros[i].value == 1.0 { 0.5 } else { unit(bits).clamp(f32::MIN_POSITIVE, 1.0) };
        prop_assume!(q.macros[i].value.to_bits() != p.macros[i].value.to_bits());
        prop_assert_ne!(canonical_hash(&p), canonical_hash(&q));
        // The depth of a route: heard if the route is live (both ends enabled), and 0 is a route that is not there.
        if !p.routes.is_empty() {
            let r = &p.routes[usize::from(pick) % p.routes.len()];
            let enabled = |id: &ModuleId| p.find(id).is_some_and(|(_, m)| m.enabled);
            let live = r.dst.module_id().is_none_or(enabled)
                && match &r.src { ModSource::Module(id) => enabled(id), _ => true };
            let depth = if r.depth == 33.0 { -33.0 } else { 33.0 };
            let mut q = p.clone();
            let addr = enth_patch::addr::RouteAddr { src: r.src.clone(), dst: r.dst.clone() };
            apply(&mut q, &Op::SetDepth { route: addr, depth }).unwrap();
            if live {
                prop_assert_ne!(canonical_hash(&p), canonical_hash(&q));
            } else {
                prop_assert_eq!(canonical_hash(&p), canonical_hash(&q));
            }
        }
        // Naming another enabled envelope the amplitude envelope.
        let others: Vec<ModuleId> = p
            .modulators
            .iter()
            .filter(|m| m.enabled && m.kind == ModuleKind::Adsr && m.id != p.voice.amp_env)
            .map(|m| m.id.clone())
            .collect();
        if let Some(id) = others.get(usize::from(pick) % others.len().max(1)) {
            let mut q = p.clone();
            apply(&mut q, &Op::SetAmpEnv { id: id.clone() }).unwrap();
            // Only a real change when the two envelopes are not the same module up to symmetry; if they are the
            // same, swapping which of them is "the" amplitude envelope still is a different patch graph unless
            // they are interchangeable, which the hash may not see. Equal hashes need identical envelopes.
            let a = p.find(&p.voice.amp_env).unwrap().1;
            let b = p.find(id).unwrap().1;
            let same_values = a.values.iter().zip(&b.values).all(|(x, y)| x.to_bits() == y.to_bits());
            if !same_values {
                prop_assert_ne!(canonical_hash(&p), canonical_hash(&q));
            }
        }
    }

    /// With hard sync on, the first enabled oscillator is the master: swapping it with a different oscillator is
    /// a different patch, while the others commute among themselves.
    #[test]
    fn the_sync_master_is_part_of_the_hash(setup in raws(0..40), bits in any::<u32>(), extra in 3usize..5, pick in any::<u8>()) {
        let mut p = build(&setup);
        let old: Vec<ModuleId> = p.generators.iter().map(|m| m.id.clone()).collect();
        for id in old {
            apply(&mut p, &Op::RemoveModule { id }).unwrap();
        }
        let mut p = with_synced_oscs(&p, bits, extra);
        if extra == 3 && bits & 1 == 1 {
            // A disabled oscillator in front: "first" means the first enabled one.
            let mut m = ModuleInstance::new(p.free_id(ModuleKind::Osc), ModuleKind::Osc);
            m.enabled = false;
            m.values[OscParam::Sync.index()] = 1.0;
            apply(&mut p, &Op::AddModule { at: Position { section: Section::Generators, index: 0 }, module: m }).unwrap();
        }
        prop_assume!(sync_active(&p));
        let enabled: Vec<usize> = p
            .generators
            .iter()
            .enumerate()
            .filter(|(_, m)| m.enabled && m.kind == ModuleKind::Osc)
            .map(|(i, _)| i)
            .collect();
        prop_assume!(enabled.len() >= 3);
        let bitwise_differ = |a: usize, b: usize| {
            p.generators[a].values.iter().zip(&p.generators[b].values).any(|(x, y)| x.to_bits() != y.to_bits())
        };
        // Master against another oscillator: different master, different patch.
        let other = enabled[1 + usize::from(pick) % (enabled.len() - 1)];
        prop_assume!(bitwise_differ(enabled[0], other));
        let mut swapped = p.clone();
        swapped.generators.swap(enabled[0], other);
        prop_assert_ne!(canonical_hash(&p), canonical_hash(&swapped), "sync master swapped with generators[{}]", other);
        // Two non-masters swapped: the same patch.
        let mut commuted = p.clone();
        commuted.generators.swap(enabled[1], enabled[2]);
        prop_assert_eq!(canonical_hash(&p), canonical_hash(&commuted));
    }

    /// Without hard sync every oscillator is interchangeable: the master swap is not a difference.
    #[test]
    fn without_sync_oscillators_commute(setup in raws(0..40), bits in any::<u32>(), extra in 2usize..5, pick in any::<u8>()) {
        let mut p = with_synced_oscs(&build(&setup), bits, extra);
        let syncs: Vec<ParamAddr> = p
            .generators
            .iter()
            .filter(|m| m.kind == ModuleKind::Osc)
            .map(|m| ParamAddr::module(m.id.clone(), OscParam::Sync.key()).unwrap())
            .collect();
        for addr in syncs {
            apply(&mut p, &Op::SetParam { addr, value: 0.0 }).unwrap();
        }
        prop_assert!(!sync_active(&p));
        let enabled: Vec<usize> = p
            .generators
            .iter()
            .enumerate()
            .filter(|(_, m)| m.enabled && m.kind == ModuleKind::Osc)
            .map(|(i, _)| i)
            .collect();
        prop_assume!(enabled.len() >= 2);
        let other = enabled[1 + usize::from(pick) % (enabled.len() - 1)];
        let mut swapped = p.clone();
        swapped.generators.swap(enabled[0], other);
        prop_assert_eq!(canonical_hash(&p), canonical_hash(&swapped));
    }

    /// The exact hash is the "same file content" hash: it sees ids and order (which seeds and summation follow) and
    /// ignores only what no render uses: the stamp, meta, labels, macro names and slots.
    #[test]
    fn exact_hash_sees_ids_and_order_but_not_notes(setup in raws(1..60), bits in any::<u32>(), seed in any::<u64>()) {
        let p = build(&setup);
        prop_assert_ne!(exact_hash(&p), canonical_hash(&p));
        // Notes to the author do not matter.
        let mut notes = p.clone();
        notes.engine = "9.9.9".to_owned();
        notes.meta.name = text(bits);
        for (i, m) in notes.macros.iter_mut().enumerate() { m.name = opt_text(bits.rotate_left(i as u32)); }
        for section in Section::ALL { for m in notes.section_mut(section) { m.label = opt_text(bits); } }
        prop_assert_eq!(exact_hash(&p), exact_hash(&notes));
        // Another id for any module is another patch.
        let mut rng = Rng(seed);
        let renamed = relabel(&p, &mut rng);
        prop_assert_ne!(exact_hash(&p), exact_hash(&renamed));
        // So is another order, even of commutative modules.
        let eligible: Vec<Section> = Section::ALL.into_iter().filter(|&s| p.section(s).len() >= 2).collect();
        if !eligible.is_empty() {
            let section = eligible[rng.below(eligible.len())];
            let n = p.section(section).len();
            let i = rng.below(n);
            let j = (i + 1 + rng.below(n - 1)) % n;
            let mut q = p.clone();
            q.section_mut(section).swap(i, j);
            prop_assert_ne!(exact_hash(&p), exact_hash(&q));
            // The serial chains keep their order in the canonical form too (different modules swapped: a different
            // patch); the summed and the evaluated sections do not care about the exact hash's order.
            let (x, y) = (&p.section(section)[i], &p.section(section)[j]);
            let differ = x.kind != y.kind || x.values.iter().zip(&y.values).any(|(a, b)| a.to_bits() != b.to_bits());
            if differ && matches!(section, Section::VoiceFx | Section::Fx) && x.enabled && y.enabled {
                prop_assert_ne!(canonical_hash(&p), canonical_hash(&q));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Hand-built cases
// ---------------------------------------------------------------------------------------------------------------

/// A hexagon of six identical LFOs against two triangles of them: every module has one route in and one out, so
/// colour refinement alone cannot tell them apart. They are different patches; the canonical form must say so
/// (this is why the ordering search exists), and a rotated, relabelled hexagon must still equal the hexagon.
#[test]
fn a_hexagon_of_lfos_is_not_two_triangles() {
    let hexagon = lfo_graph(6, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 0)]);
    let triangles = lfo_graph(6, &[(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3)]);
    assert_ne!(canonical_hash(&hexagon), canonical_hash(&triangles));
    let rotated = lfo_graph(6, &[(3, 4), (4, 5), (5, 0), (0, 1), (1, 2), (2, 3)]);
    assert_eq!(canonical_hash(&hexagon), canonical_hash(&rotated));
    let mut rng = Rng(7);
    for _ in 0..20 {
        assert_eq!(
            canonical_hash(&hexagon),
            canonical_hash(&scramble(&hexagon, rng.next()))
        );
        assert_eq!(
            canonical_hash(&triangles),
            canonical_hash(&scramble(&triangles, rng.next()))
        );
    }
}

/// Identical modules with identical routes are interchangeable; the search must not blow up on a full patch of
/// them (twin pruning): seven LFOs next to the envelope, four oscillators, four filters.
#[test]
fn a_pile_of_twins_is_cheap_and_symmetric() {
    let mut p = base_patch();
    let add = |p: &mut Patch, id: String, kind: ModuleKind, at: usize| {
        let section = kind.section();
        let m = ModuleInstance::new(ModuleId::new(&id).unwrap(), kind);
        apply(
            p,
            &Op::AddModule {
                at: Position { section, index: at },
                module: m,
            },
        )
        .unwrap();
    };
    for i in 0..7 {
        let at = p.modulators.len();
        add(&mut p, format!("lfo{}", i + 1), ModuleKind::Lfo, at);
    }
    for i in 0..3 {
        let at = p.generators.len();
        add(&mut p, format!("osc{}", i + 2), ModuleKind::Osc, at);
    }
    for i in 0..4 {
        let at = p.voice_fx.len();
        add(&mut p, format!("filter{}", i + 1), ModuleKind::Svf, at);
    }
    validate(&p).unwrap();
    // Interchangeable modules are tried once, not in every order: one complete ordering is enough (no timing here:
    // without the pruning this would be 7! * 3! * 4! orderings and hit the budget).
    assert_eq!(canonical_effort(&p), 1);
    assert_eq!(canonical_hash(&p), canonical_hash(&scramble(&p, 99)));
}

/// A ring of seven identical LFOs: no two are twins, all are automorphic. Any rotation or naming is the same patch,
/// and a ring is not a line of the same length.
#[test]
fn a_ring_of_seven_is_found_in_every_rotation() {
    let ring = |shift: usize| {
        let edges: Vec<(usize, usize)> = (0..7)
            .map(|i| ((i + shift) % 7, (i + shift + 1) % 7))
            .collect();
        lfo_graph(7, &edges)
    };
    let line = lfo_graph(7, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6)]);
    let h = canonical_hash(&ring(0));
    for shift in 1..7 {
        assert_eq!(h, canonical_hash(&ring(shift)));
    }
    assert_ne!(h, canonical_hash(&line));
    assert!(
        canonical_effort(&ring(0)) <= 7,
        "one individualisation of the first class of 7"
    );
    assert!(canonical_effort(&ring(0)) < LEAF_BUDGET);
    let mut rng = Rng(11);
    for _ in 0..10 {
        assert_eq!(h, canonical_hash(&scramble(&ring(0), rng.next())));
    }
}

/// Two LFOs with the same parameters but different jobs (one drives the filter, the other an oscillator) are told
/// apart by their routes, in whichever order they are written.
#[test]
fn modules_with_equal_parameters_are_told_apart_by_their_routes() {
    // lfo1 -> filter / lfo2 -> osc and the mirror image are the same patch up to naming ...
    assert_eq!(
        canonical_hash(&two_lfos(true)),
        canonical_hash(&two_lfos(false))
    );
    // ... but a patch where the same LFO drives both is not.
    let mut both = two_lfos(true);
    let route = Route {
        src: ModSource::Module(ModuleId::new("lfo1").unwrap()),
        dst: "osc1.semi".parse().unwrap(),
        depth: 30.0,
    };
    apply(
        &mut both,
        &Op::RemoveRoute {
            route: "lfo2 -> osc1.semi".parse().unwrap(),
        },
    )
    .unwrap();
    apply(&mut both, &Op::InsertRoute { index: 1, route }).unwrap();
    assert_ne!(canonical_hash(&two_lfos(true)), canonical_hash(&both));
}

/// The canonical form of a small patch, spelled out, so the rules are visible in one place.
#[test]
fn the_canonical_form_of_a_small_patch() {
    let mut p = base_patch();
    p.meta.name = "x".to_owned();
    // generators: osc1 (kept in place, it is the first enabled one with sync on the second), a disabled osc, osc3.
    let add = |p: &mut Patch, id: &str, kind: ModuleKind, enabled: bool, section: Section| {
        let mut m = ModuleInstance::new(ModuleId::new(id).unwrap(), kind);
        m.enabled = enabled;
        let at = Position {
            section,
            index: p.section(section).len(),
        };
        apply(p, &Op::AddModule { at, module: m }).unwrap();
    };
    add(&mut p, "ghost", ModuleKind::Osc, false, Section::Generators);
    add(&mut p, "zeta", ModuleKind::Osc, true, Section::Generators);
    apply(
        &mut p,
        &Op::SetParam {
            addr: "zeta.sync".parse().unwrap(),
            value: 1.0,
        },
    )
    .unwrap();
    apply(
        &mut p,
        &Op::SetParam {
            addr: "zeta.semi".parse().unwrap(),
            value: 7.0,
        },
    )
    .unwrap();
    add(&mut p, "wob", ModuleKind::Lfo, true, Section::Modulators);
    let route = Route {
        src: ModSource::Velocity,
        dst: "zeta.semi".parse().unwrap(),
        depth: 0.0,
    };
    apply(&mut p, &Op::InsertRoute { index: 0, route }).unwrap();
    let route = Route {
        src: ModSource::Module(ModuleId::new("wob").unwrap()),
        dst: "ghost.semi".parse().unwrap(),
        depth: 10.0,
    };
    apply(&mut p, &Op::InsertRoute { index: 1, route }).unwrap();
    let route = Route {
        src: ModSource::Module(ModuleId::new("wob").unwrap()),
        dst: "zeta.fine".parse().unwrap(),
        depth: 10.0,
    };
    apply(&mut p, &Op::InsertRoute { index: 2, route }).unwrap();

    let c = canonicalise(&p);
    let ids = |s: &[ModuleInstance]| {
        s.iter()
            .map(|m| m.id.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    // The ghost and its route are gone, the depth-0 route is gone, the meta is gone; ids are `<prefix><n>` in order,
    // the sync master (osc1) first.
    assert_eq!(ids(&c.generators), ["osc1", "osc2"]);
    assert_eq!(ids(&c.modulators), ["env1", "lfo1"]);
    assert_eq!(c.routes.len(), 1);
    assert_eq!(
        format!("{} -> {}", c.routes[0].src, c.routes[0].dst),
        "lfo1 -> osc2.fine"
    );
    assert_eq!(c.meta, PatchMeta::default());
    assert_eq!(c.voice.amp_env.as_str(), "env1");
}
