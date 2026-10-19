//! Route depth value events (P1b lane E+): a depth change without a recompile, ramped over 10 ms, that lands on
//! exactly what a recompiled plan holds. Every `process` call runs under `assert_no_alloc`.
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, bits, id, reset_saw_patch};
use enth_engine::{
    DEPTH_SMOOTHING_MS, Engine, Event, EventKind, Link, LinkCapacity, NoteKey, Plan, PlanMap,
    PrepareCfg, SEG, SendError, compile,
};
use enth_patch::catalogue::LfoParam;
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route, RouteAddr};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn plan(patch: &Patch) -> (Plan, PlanMap) {
    compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles")
}

fn route(src: &str, dst: &str, depth: f32) -> Route {
    Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    }
}

fn raddr(src: &str, dst: &str) -> RouteAddr {
    RouteAddr {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
    }
}

/// A reset-phase saw with a voice LFO on its level and on the pan (destinations without state, so a render that took
/// another path to the same values gives the same bits once the values agree), and a macro on the fine tune.
fn lfo_patch(level_depth: f32) -> Patch {
    let mut p = reset_saw_patch();
    let mut lfo = ModuleInstance::new(id("lfo1"), ModuleKind::Lfo);
    lfo.values[LfoParam::Rate.index()] = 3.0;
    p.modulators.push(lfo);
    p.routes.push(route("macro1", "osc1.fine", 10.0));
    p.routes.push(route("lfo1", "osc1.level", level_depth));
    p.routes.push(route("lfo1", "voice.pan", 40.0));
    p
}

/// Run `segments` one-segment calls under `assert_no_alloc`; `before(k, link)` may send commands first.
fn run(
    engine: &mut Engine,
    link: &mut Link,
    segments: usize,
    mut before: impl FnMut(usize, &mut Link),
) -> (Vec<f32>, Vec<f32>) {
    let mut left = vec![0.0; segments * SEG];
    let mut right = vec![0.0; segments * SEG];
    for k in 0..segments {
        before(k, link);
        let events = if k == 0 {
            vec![Event {
                offset: 0,
                kind: EventKind::NoteOn {
                    key: NoteKey::new(57, 0),
                    velocity: 0.9,
                },
            }]
        } else {
            Vec::new()
        };
        let (l, r) = (
            &mut left[k * SEG..(k + 1) * SEG],
            &mut right[k * SEG..(k + 1) * SEG],
        );
        assert_no_alloc(|| engine.process(l, r, &events));
        link.collect();
    }
    (left, right)
}

/// Segments a depth ramp lasts at the test rate.
fn ramp_segments() -> usize {
    (f64::from(DEPTH_SMOOTHING_MS) * f64::from(SR) / 1000.0 / SEG as f64).round() as usize
}

#[test]
fn a_depth_event_equals_a_recompiled_render_once_its_ramp_settles() {
    let (p0, map) = plan(&lfo_patch(-30.0));
    let level = map.depth(&raddr("lfo1", "osc1.level")).expect("a term");
    assert_eq!(p0.route_depth(level), Some(-30.0f32 / 100.0 * 100.0));
    let at = 100;
    // The depth event.
    let (mut e, mut link) = Engine::with_link(p0, SEED, LinkCapacity::default());
    let (l_event, r_event) = run(&mut e, &mut link, 300, |k, link| {
        if k == at {
            link.send_depth(0, level, -65.0).unwrap();
        }
    });
    assert_eq!(e.plan().route_depth(level), Some(-65.0f32 / 100.0 * 100.0));
    assert_eq!(e.stats().swaps, 0, "no recompile");
    // The recompile, swapped in at the same boundary (an instant swap: nothing added or removed).
    let (mut e2, mut link2) =
        Engine::with_link(plan(&lfo_patch(-30.0)).0, SEED, LinkCapacity::default());
    let mut edited = Some(plan(&lfo_patch(-65.0)).0);
    let (l_swap, r_swap) = run(&mut e2, &mut link2, 300, |k, link| {
        if k == at {
            link.send_plan(edited.take().unwrap()).unwrap();
        }
    });
    assert_eq!(e2.stats().swaps, 1);
    // Without the edit at all.
    let (mut e3, mut link3) =
        Engine::with_link(plan(&lfo_patch(-30.0)).0, SEED, LinkCapacity::default());
    let (l_none, _) = run(&mut e3, &mut link3, 300, |_, _| {});

    let start = at * SEG;
    let settled = (at + ramp_segments() + 1) * SEG;
    assert_eq!(
        bits(&l_event[..start]),
        bits(&l_swap[..start]),
        "the same before"
    );
    // The ramp is smoothed: it differs from the jump and from no edit while it runs.
    assert_ne!(
        bits(&l_event[start..settled]),
        bits(&l_swap[start..settled])
    );
    assert_ne!(
        bits(&l_event[start..settled]),
        bits(&l_none[start..settled])
    );
    // Then the very same bits as the recompiled plan.
    assert_eq!(bits(&l_event[settled..]), bits(&l_swap[settled..]));
    assert_eq!(bits(&r_event[settled..]), bits(&r_swap[settled..]));
    assert_ne!(bits(&l_event[settled..]), bits(&l_none[settled..]));
}

#[test]
fn depth_ids_exist_only_for_terms_in_the_plan() {
    let mut p = lfo_patch(-30.0);
    p.routes.push(route("velocity", "osc1.semi", 0.0)); // depth 0: left out of the plan
    let mut lfo2 = ModuleInstance::new(id("lfo2"), ModuleKind::Lfo);
    lfo2.enabled = false;
    p.modulators.push(lfo2);
    p.routes.push(route("lfo2", "osc1.fine", 20.0)); // from a disabled module
    let (pl, map) = plan(&p);
    for (src, dst) in [
        ("macro1", "osc1.fine"),
        ("lfo1", "osc1.level"),
        ("lfo1", "voice.pan"),
    ] {
        let d = map
            .depth(&raddr(src, dst))
            .unwrap_or_else(|| panic!("{src} -> {dst}"));
        assert!(pl.route_depth(d).is_some());
    }
    assert_eq!(
        map.depth(&raddr("velocity", "osc1.semi")),
        None,
        "depth 0: recompile"
    );
    assert_eq!(
        map.depth(&raddr("lfo2", "osc1.fine")),
        None,
        "disabled: recompile"
    );
    assert_eq!(
        map.depth(&raddr("key", "osc1.level")),
        None,
        "no such route"
    );
}

#[test]
fn stale_nan_and_out_of_range_depth_events() {
    let (p0, map) = plan(&lfo_patch(-30.0));
    let level = map.depth(&raddr("lfo1", "osc1.level")).unwrap();
    let pan = map.depth(&raddr("lfo1", "voice.pan")).unwrap();
    let (mut e, mut link) = Engine::with_link(p0, SEED, LinkCapacity::default());
    run(&mut e, &mut link, 40, |k, link| {
        if k == 2 {
            link.send_depth(7, level, -10.0).unwrap(); // another plan's epoch
            link.send_depth(0, pan, f32::NAN).unwrap();
            link.send_depth(0, level, 250.0).unwrap(); // clamped to 100 %
        }
    });
    assert_eq!(e.stats().stale_events, 1);
    assert_eq!(e.plan().route_depth(pan), Some(40.0));
    assert_eq!(e.plan().route_depth(level), Some(100.0));
}

#[test]
fn a_full_command_ring_of_depth_events_drains_without_allocating() {
    let (p0, map) = plan(&lfo_patch(-30.0));
    let level = map.depth(&raddr("lfo1", "osc1.level")).unwrap();
    let capacity = LinkCapacity {
        commands: 64,
        returns: 4,
    };
    let (mut e, mut link) = Engine::with_link(p0, SEED, capacity);
    run(&mut e, &mut link, 30, |k, link| {
        if k == 3 {
            let mut sent = 0;
            loop {
                match link.send_depth(0, level, -(sent as f32) % 100.0) {
                    Ok(()) => sent += 1,
                    Err(SendError::Full(())) => break,
                    Err(SendError::InFlight(())) => unreachable!(),
                }
            }
            assert_eq!(sent, 64);
        }
    });
    assert_eq!(link.free_slots(), 64, "drained in one segment");
    assert_eq!(e.plan().route_depth(level), Some(-63.0));
}

#[test]
fn a_depth_change_mid_ramp_restarts_the_ramp_from_where_it_is() {
    let (p0, map) = plan(&lfo_patch(-30.0));
    let level = map.depth(&raddr("lfo1", "osc1.level")).unwrap();
    let (mut e, mut link) = Engine::with_link(p0, SEED, LinkCapacity::default());
    let mut seen = Vec::new();
    for k in 0..60 {
        if k == 5 {
            link.send_depth(0, level, 50.0).unwrap();
        }
        if k == 10 {
            link.send_depth(0, level, 0.0).unwrap();
        }
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        assert_no_alloc(|| e.process(&mut l, &mut r, &[]));
        seen.push(e.plan().route_depth(level).unwrap());
    }
    let n = ramp_segments();
    assert!(seen[5] > -30.0 && seen[5] < 50.0, "ramping: {}", seen[5]);
    assert!(seen[9] < 50.0, "not there yet");
    assert_eq!(
        seen[10 + n - 1],
        0.0,
        "the restarted ramp lands after a full ramp"
    );
    assert!(seen[10 + n - 2] != 0.0);
    assert_eq!(seen[59], 0.0);
}
