//! Host automation safety (P1b lane E+): slot events resolved through each plan's slot table, epoch-tagged value
//! events, and what a plan swap inside a host block does to both. Every `process` call runs under `assert_no_alloc`.
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, bits, id};
use enth_engine::{
    Engine, Event, EventKind, Link, LinkCapacity, NoteKey, ParamId, Plan, PlanMap, PrepareCfg,
    compile,
};
use enth_patch::catalogue::{OscParam, SvfParam, Wave};
use enth_patch::{ModuleInstance, ModuleKind, ParamAddr, Patch};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn plan(patch: &Patch) -> (Plan, PlanMap) {
    compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles")
}

fn addr(s: &str) -> ParamAddr {
    s.parse().unwrap()
}

fn on(key: u8, id: i32) -> EventKind {
    EventKind::NoteOn {
        key: NoteKey::new(key, id),
        velocity: 0.8,
    }
}

/// Two oscillators, the second silent; slot 1 (index 0) drives `osc1.level`. `swapped` lists them the other way
/// round: the same modules (an instant swap) with every parameter id moved, so an id from one plan names another
/// parameter in the other.
fn two_oscs(swapped: bool) -> Patch {
    let mut p = Patch::init();
    let mut osc2 = ModuleInstance::new(id("osc2"), ModuleKind::Osc);
    osc2.values[OscParam::Wave.index()] = Wave::Pulse.index() as f32;
    osc2.values[OscParam::Level.index()] = 0.0;
    if swapped {
        p.generators.insert(0, osc2);
    } else {
        p.generators.push(osc2);
    }
    p.slots[0] = Some(addr("osc1.level"));
    p
}

/// Run `blocks` host blocks of `block` frames under `assert_no_alloc`. Before block `k`, `before(k, link)` may send
/// commands (outside the guard) and returns that block's events (offset, kind).
fn run(
    engine: &mut Engine,
    link: &mut Link,
    blocks: usize,
    block: usize,
    mut before: impl FnMut(usize, &mut Link) -> Vec<(u32, EventKind)>,
) -> Vec<f32> {
    let mut left = vec![0.0; blocks * block];
    let mut right = vec![0.0; blocks * block];
    for k in 0..blocks {
        let events: Vec<Event> = before(k, link)
            .into_iter()
            .map(|(offset, kind)| Event { offset, kind })
            .collect();
        let (l, r) = (
            &mut left[k * block..(k + 1) * block],
            &mut right[k * block..(k + 1) * block],
        );
        assert_no_alloc(|| engine.process(l, r, &events));
        link.collect();
    }
    left
}

#[test]
fn a_plan_swap_inside_a_block_never_routes_a_value_to_the_wrong_parameter() {
    let (_, map_a) = plan(&two_oscs(false));
    let (_, map_b) = plan(&two_oscs(true));
    let level_a = map_a.get(&addr("osc1.level")).unwrap();
    let level_b = map_b.get(&addr("osc1.level")).unwrap();
    assert_ne!(level_a, level_b, "the edit moves the id");
    assert_eq!(
        map_b.addr(level_a).map(ToString::to_string).as_deref(),
        Some("osc2.level"),
        "plan A's id for osc1.level is osc2.level in plan B"
    );
    let spec = OscParam::Level.spec();
    let n = 0.3f32;
    let v = spec.from_norm(n);
    // Before block 20 the session sends plan B; that block's events were computed against plan A, the one running
    // when the block began. The swap happens at the first segment boundary inside the block, before any of them.
    let render = |block: usize, event: Option<EventKind>| {
        let (mut e, mut link) =
            Engine::with_link(plan(&two_oscs(false)).0, SEED, LinkCapacity::default());
        let mut b = Some(plan(&two_oscs(true)).0);
        let out = run(&mut e, &mut link, 60, block, |k, link| match k {
            0 => vec![(0, on(57, 0)), (0, on(64, 1))],
            20 => {
                assert_eq!(link.send_plan(b.take().unwrap()).unwrap(), 1);
                event.map(|kind| vec![(5, kind)]).unwrap_or_default()
            }
            _ => Vec::new(),
        });
        (out, e)
    };
    for block in [32, 48, 64, 100, 333] {
        let (none, _) = render(block, None);
        let (slot, e) = render(block, Some(EventKind::Slot { slot: 0, value: n }));
        assert_eq!(
            e.base(level_b),
            Some(v),
            "block {block}: the slot found osc1.level in plan B"
        );
        assert_eq!(
            e.base(level_a),
            Some(0.0),
            "block {block}: osc2.level untouched"
        );
        // The reference: the right id of plan B, tagged with B's epoch.
        let (reference, _) = render(
            block,
            Some(EventKind::EpochParam {
                epoch: 1,
                id: level_b,
                value: v,
            }),
        );
        assert_eq!(bits(&slot), bits(&reference), "block {block}");
        assert_ne!(bits(&slot), bits(&none), "block {block}: the value applied");
        // An epoch-tagged event computed against plan A is dropped, not misrouted.
        let (stale, e) = render(
            block,
            Some(EventKind::EpochParam {
                epoch: 0,
                id: level_a,
                value: v,
            }),
        );
        assert_eq!(e.stats().stale_events, 1);
        assert_eq!(bits(&stale), bits(&none), "block {block}: dropped");
        // The hazard the slot table removes: an untagged id from plan A lands on osc2.level of plan B.
        let (hazard, e) = render(
            block,
            Some(EventKind::Param {
                id: level_a,
                value: v,
            }),
        );
        assert_eq!(
            e.base(level_a),
            Some(v),
            "block {block}: misrouted to osc2.level"
        );
        assert_ne!(bits(&hazard), bits(&reference), "block {block}");
    }
}

/// A filter whose cutoff (Log 20 Hz–20 kHz: `from_norm(to_norm(x))` is not `x` for most values) is on slot 1.
fn cutoff_patch(cutoff: f32) -> Patch {
    let mut p = Patch::init();
    let mut f = ModuleInstance::new(id("filter1"), ModuleKind::Svf);
    f.values[SvfParam::Cutoff.index()] = cutoff;
    p.voice_fx.push(f);
    p.slots[0] = Some(addr("filter1.cutoff"));
    p
}

/// Twelve segments (longer than a crossfade), with `events` in the first.
fn base_after(engine: &mut Engine, link: &mut Link, events: Vec<(u32, EventKind)>) {
    let mut events = Some(events);
    run(engine, link, 12, 32, |_, _| {
        events.take().unwrap_or_default()
    });
}

/// A cutoff the normalised round trip moves (`from_norm(to_norm(x)) != x`; most values, report 01 F3).
fn moved_by_round_trip(from: f32) -> f32 {
    let spec = SvfParam::Cutoff.spec();
    (0..10_000)
        .map(|i| from + i as f32 * 0.37)
        .find(|&x| spec.from_norm(spec.to_norm(x)).to_bits() != x.to_bits())
        .expect("most values move")
}

#[test]
fn the_hosts_echo_of_the_patch_value_sets_it_exactly_and_other_values_go_through_from_norm() {
    let spec = SvfParam::Cutoff.spec();
    let p = moved_by_round_trip(1234.0);
    assert_ne!(
        spec.from_norm(spec.to_norm(p)).to_bits(),
        p.to_bits(),
        "a value the round trip moves"
    );
    let (pl, map) = plan(&cutoff_patch(p));
    let cutoff = map.get(&addr("filter1.cutoff")).unwrap();
    assert_eq!(pl.slot_target(0), Some(cutoff));
    assert_eq!(pl.slot_target(1), None);
    let (mut e, mut link) = Engine::with_link(pl, SEED, LinkCapacity::default());
    let echo = spec.to_norm(p);
    base_after(
        &mut e,
        &mut link,
        vec![(
            0,
            EventKind::Slot {
                slot: 0,
                value: echo,
            },
        )],
    );
    assert_eq!(e.base(cutoff).map(f32::to_bits), Some(p.to_bits()), "exact");
    base_after(
        &mut e,
        &mut link,
        vec![(
            0,
            EventKind::Slot {
                slot: 0,
                value: 0.25,
            },
        )],
    );
    assert_eq!(e.base(cutoff), Some(spec.from_norm(0.25)));
    assert_eq!(e.host_slot(0), Some(0.25));
    // Out of range and NaN: clamped, ignored; an unknown slot changes nothing.
    base_after(
        &mut e,
        &mut link,
        vec![
            (
                0,
                EventKind::Slot {
                    slot: 0,
                    value: 7.0,
                },
            ),
            (
                0,
                EventKind::Slot {
                    slot: 0,
                    value: f32::NAN,
                },
            ),
            (
                0,
                EventKind::Slot {
                    slot: 200,
                    value: 0.5,
                },
            ),
        ],
    );
    assert_eq!(e.base(cutoff), Some(spec.from_norm(1.0)));
    assert_eq!(e.host_slot(0), Some(1.0));
    // An editor edit (a value event from the command ring) becomes the slot's patch value: the host's echo of it
    // (`to_norm` of the same value) lands exactly.
    let edit = moved_by_round_trip(777.0);
    link.send_param(0, cutoff, edit).unwrap();
    base_after(
        &mut e,
        &mut link,
        vec![(
            0,
            EventKind::Slot {
                slot: 0,
                value: spec.to_norm(edit),
            },
        )],
    );
    assert_eq!(e.base(cutoff).map(f32::to_bits), Some(edit.to_bits()));
}

#[test]
fn host_slot_values_survive_a_recompile_and_are_forgotten_when_the_slot_moves() {
    let spec = SvfParam::Cutoff.spec();
    let base = cutoff_patch(2000.0);
    let (pl, map) = plan(&base);
    let cutoff = map.get(&addr("filter1.cutoff")).unwrap();
    let (mut e, mut link) = Engine::with_link(pl, SEED, LinkCapacity::default());
    base_after(
        &mut e,
        &mut link,
        vec![(
            0,
            EventKind::Slot {
                slot: 0,
                value: 0.3,
            },
        )],
    );
    assert_eq!(e.base(cutoff), Some(spec.from_norm(0.3)));
    // A recompile of an edit elsewhere (a structural edit that adds an oscillator): the new plan starts from the
    // patch value, and the engine re-applies the host's value at the swap.
    let mut edited = base.clone();
    edited
        .generators
        .push(ModuleInstance::new(id("osc2"), ModuleKind::Osc));
    let (p1, map1) = plan(&edited);
    let cutoff1 = map1.get(&addr("filter1.cutoff")).unwrap();
    assert_ne!(cutoff1, cutoff);
    link.send_plan(p1).unwrap();
    base_after(&mut e, &mut link, Vec::new());
    assert_eq!(e.epoch(), 1);
    assert_eq!(e.base(cutoff1), Some(spec.from_norm(0.3)), "re-applied");
    // The host's value equal to `to_norm` of the new plan's patch value keeps the exact patch value.
    let exact = moved_by_round_trip(1234.0);
    let mut moved = edited.clone();
    moved.voice_fx[0].values[SvfParam::Cutoff.index()] = exact;
    let (p2, map2) = plan(&moved);
    let cutoff2 = map2.get(&addr("filter1.cutoff")).unwrap();
    link.send_plan(p2).unwrap();
    base_after(
        &mut e,
        &mut link,
        vec![(
            0,
            EventKind::Slot {
                slot: 0,
                value: spec.to_norm(exact),
            },
        )],
    );
    assert_eq!(e.base(cutoff2).map(f32::to_bits), Some(exact.to_bits()));
    // The slot reassigned to another parameter: the remembered value is forgotten, and the new target keeps its
    // patch value until the host moves it.
    let mut reassigned = moved.clone();
    reassigned.slots[0] = Some(addr("filter1.resonance"));
    let (p3, map3) = plan(&reassigned);
    let resonance = map3.get(&addr("filter1.resonance")).unwrap();
    link.send_plan(p3).unwrap();
    base_after(&mut e, &mut link, Vec::new());
    assert_eq!(e.host_slot(0), None);
    assert_eq!(e.base(resonance), Some(SvfParam::Resonance.spec().default));
    assert_eq!(
        e.base(map3.get(&addr("filter1.cutoff")).unwrap()),
        Some(exact)
    );
}

#[test]
fn a_slot_whose_target_is_disabled_remembers_the_value_for_when_it_comes_back() {
    let spec = SvfParam::Cutoff.spec();
    let mut off = cutoff_patch(2000.0);
    off.voice_fx[0].enabled = false;
    let (pl, _) = plan(&off);
    assert_eq!(pl.slot_target(0), None, "the target is not in the plan");
    let (mut e, mut link) = Engine::with_link(pl, SEED, LinkCapacity::default());
    base_after(
        &mut e,
        &mut link,
        vec![(
            0,
            EventKind::Slot {
                slot: 0,
                value: 0.6,
            },
        )],
    );
    assert_eq!(e.host_slot(0), Some(0.6));
    let (p1, map1) = plan(&cutoff_patch(2000.0));
    link.send_plan(p1).unwrap();
    base_after(&mut e, &mut link, Vec::new());
    assert_eq!(
        e.base(map1.get(&addr("filter1.cutoff")).unwrap()),
        Some(spec.from_norm(0.6))
    );
}

#[test]
fn slots_resolve_through_both_plans_during_a_crossfade() {
    // The swap adds a module (a crossfade): a slot event during the fade reaches both plans' tables.
    let base = cutoff_patch(2000.0);
    let mut edited = base.clone();
    edited
        .modulators
        .push(ModuleInstance::new(id("lfo3"), ModuleKind::Lfo));
    let (p1, map1) = plan(&edited);
    let cutoff1 = map1.get(&addr("filter1.cutoff")).unwrap();
    let (mut e, mut link) = Engine::with_link(plan(&base).0, SEED, LinkCapacity::default());
    let mut p1 = Some(p1);
    run(&mut e, &mut link, 20, 32, |k, link| match k {
        0 => vec![(0, on(60, 0))],
        5 => {
            link.send_plan(p1.take().unwrap()).unwrap();
            Vec::new()
        }
        6 => vec![(
            0,
            EventKind::Slot {
                slot: 0,
                value: 0.4,
            },
        )],
        _ => Vec::new(),
    });
    assert_eq!(e.stats().crossfades, 1);
    assert_eq!(
        e.base(cutoff1),
        Some(SvfParam::Cutoff.spec().from_norm(0.4))
    );
    // ParamId values are per plan, so the running plan's id is the one to read.
    assert!(matches!(cutoff1, ParamId::Voice(_)));
}
