//! Audio routes between generators (lane RTE): what the engine does with a patch that has them, end to end.
//!
//! The carriers here are `wt` modules held silent (`level` 0, which lane WT's module honours exactly), so a route
//! into `wt.pm` or `wt.am` cannot change a sample of the output. That is what makes these tests exact: the render
//! with the routes must equal, bit for bit, the render of the same patch without them, whatever order the routes put
//! the generators in. The observable delivery (the pre-level samples, scaled by depth, reaching `audio_in`) is
//! tested with a probe in `routing.rs`'s own tests, and what a sounding `wt` does with its inputs (PM, AM, ring
//! modulation) in `tests/wt.rs`.
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, bits, id, note, render};
use enth_engine::{
    DEPTH_SMOOTHING_MS, Engine, Event, EventKind, Link, LinkCapacity, NoteKey, Plan, PlanMap,
    PrepareCfg, SEG, StereoMode, compile, compile_with,
};
use enth_patch::catalogue::{LfoParam, OscParam, Wave, WtParam};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route, RouteAddr};
use proptest::prelude::*;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

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

fn plan(patch: &Patch) -> (Plan, PlanMap) {
    compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles")
}

/// Generators named by `names` (`wt…`, `noise…`, otherwise `osc…`), in that patch order, with a reset phase so every
/// note starts alike and a different pitch offset each so the sum is not symmetrical; and the default envelope.
fn gens(names: &[&str]) -> Patch {
    let mut p = Patch::init();
    p.generators.clear();
    for (i, name) in names.iter().enumerate() {
        let kind = match name {
            n if n.starts_with("wt") => ModuleKind::Wt,
            n if n.starts_with("noise") => ModuleKind::Noise,
            _ => ModuleKind::Osc,
        };
        let mut m = ModuleInstance::new(id(name), kind);
        if kind == ModuleKind::Osc {
            m.values[OscParam::Fine.index()] = 5.0 * i as f32;
            m.values[OscParam::Level.index()] = 60.0 + 10.0 * i as f32;
        }
        if kind == ModuleKind::Wt {
            // A silent carrier: the routes still arrive, and cannot change a bit of the sum.
            m.values[WtParam::Level.index()] = 0.0;
        }
        p.generators.push(m);
    }
    p
}

fn with_routes(mut p: Patch, routes: &[Route]) -> Patch {
    p.routes.extend_from_slice(routes);
    p
}

fn chord() -> Vec<common::Note> {
    vec![
        note(48, 0, 9_000),
        note(55, 400, 9_000),
        note(64, 800, 7_000),
    ]
}

/// The two renders equal, in both channels, for a chord with a release tail.
fn assert_routes_change_nothing(with: &Patch, without: &Patch) {
    let frames = 14_000;
    let (l0, r0, _) = render(without, &chord(), frames, 512, SEED);
    let (l1, r1, _) = render(with, &chord(), frames, 480, SEED); // another host block size, too
    assert!(l0.iter().any(|x| x.abs() > 0.05), "the patch sounds");
    assert_eq!(bits(&l0), bits(&l1));
    assert_eq!(bits(&r0), bits(&r1));
}

#[test]
fn routes_into_a_silent_carrier_leave_every_bit_alone() {
    // Patch order differs from the route order: the sources come after the carrier.
    let base = gens(&["wt1", "osc1", "osc2"]);
    let routed = with_routes(
        base.clone(),
        &[
            route("osc2", "wt1.pm", 40.0),
            route("osc1", "wt1.am", -25.0),
        ],
    );
    assert_routes_change_nothing(&routed, &base);
    // Noise and oscillator sources, the carrier between them.
    let base = gens(&["osc1", "wt1", "noise1"]);
    let routed = with_routes(
        base.clone(),
        &[
            route("noise1", "wt1.pm", 100.0),
            route("osc1", "wt1.pm", 5.0),
        ],
    );
    assert_routes_change_nothing(&routed, &base);
    // A chain against patch order: osc1 -> wt2 -> wt1.
    let base = gens(&["wt1", "wt2", "osc1"]);
    let routed = with_routes(
        base.clone(),
        &[route("wt2", "wt1.pm", 20.0), route("osc1", "wt2.am", 30.0)],
    );
    assert_routes_change_nothing(&routed, &base);
}

#[test]
fn hard_sync_keeps_its_rule_under_reordering() {
    // The sync master is the first oscillator in patch order (osc1, second); osc2 follows it with `sync` on, and the
    // routes pull the carrier, which comes first in the patch, to the end of the run.
    let mut base = gens(&["wt1", "osc1", "osc2"]);
    base.generators[2].values[OscParam::Sync.index()] = 1.0;
    base.generators[2].values[OscParam::Semi.index()] = 7.0; // well off the master's pitch
    let routed = with_routes(
        base.clone(),
        &[route("osc2", "wt1.pm", 40.0), route("osc1", "wt1.am", 10.0)],
    );
    assert_routes_change_nothing(&routed, &base);
    // Sanity: the sync matters in this patch (it is not a no-op that hides a broken order).
    let mut free = base.clone();
    free.generators[2].values[OscParam::Sync.index()] = 0.0;
    let (a, _, _) = render(&base, &chord(), 4_000, 512, SEED);
    let (b, _, _) = render(&free, &chord(), 4_000, 512, SEED);
    assert_ne!(bits(&a), bits(&b));
}

proptest! {
    // The nightly runs 10,000 cases; a wiring with no route into a `wt` is rejected, so the limit on rejects is raised
    // and at least one generator is always a `wt` (the nightly hit the default 1,024 rejects at 525 successes).
    #![proptest_config(ProptestConfig { cases: 24, max_global_rejects: 1 << 16, ..ProptestConfig::default() })]

    /// Any acyclic wiring of up to four generators into silent carriers leaves the render alone; any wiring with a
    /// loop is refused with its own code.
    #[test]
    fn random_acyclic_wirings_change_no_bit(
        kinds in proptest::collection::vec(0u8..3, 2..=4),
        rank in proptest::collection::vec(0u32..1000, 4),
        wiring in proptest::collection::vec((0usize..4, 0usize..4, 0u8..2, 1u32..100), 1..6),
    ) {
        let has_wt = kinds.contains(&0);
        let names: Vec<String> = kinds
            .iter()
            .enumerate()
            .map(|(i, &k)| if !has_wt && i + 1 == kinds.len() { 0 } else { k })
            .enumerate()
            .map(|(i, k)| match k {
                0 => format!("wt{}", i + 1),
                1 => format!("noise{}", i + 1),
                _ => format!("osc{}", i + 1),
            })
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let base = gens(&refs);
        // A route from the lower rank to the higher one cannot loop (ties by index).
        let key = |i: usize| (rank[i], i);
        let mut routes: Vec<Route> = Vec::new();
        for &(a, b, param, depth) in &wiring {
            let (a, b) = (a % names.len(), b % names.len());
            let (s, d) = if key(a) < key(b) { (a, b) } else { (b, a) };
            if s == d || !names[d].starts_with("wt") {
                continue;
            }
            let dst = format!("{}.{}", names[d], if param == 0 { "pm" } else { "am" });
            if routes.iter().any(|r| r.src.to_string() == names[s] && r.dst.to_string() == dst) {
                continue;
            }
            routes.push(route(&names[s], &dst, depth as f32));
        }
        prop_assume!(!routes.is_empty());
        let routed = with_routes(base.clone(), &routes);
        let frames = 6_000;
        let notes = [note(52, 0, 3_000), note(59, 200, 4_000)];
        let (l0, r0, _) = render(&base, &notes, frames, 512, SEED);
        let (l1, r1, _) = render(&routed, &notes, frames, 512, SEED);
        prop_assert_eq!(bits(&l0), bits(&l1));
        prop_assert_eq!(bits(&r0), bits(&r1));
    }
}

#[test]
fn loops_and_wrong_ends_do_not_compile() {
    let code = |routes: &[Route], names: &[&str]| {
        let p = with_routes(gens(names), routes);
        compile(&p, &PrepareCfg::new(SR))
            .expect_err("refused")
            .code()
    };
    assert_eq!(
        code(
            &[route("wt1", "wt2.pm", 10.0), route("wt2", "wt1.am", 10.0)],
            &["wt1", "wt2"]
        ),
        "audio_cycle"
    );
    assert_eq!(
        code(&[route("wt1", "wt1.pm", 10.0)], &["wt1"]),
        "audio_cycle"
    );
    assert_eq!(
        code(&[route("osc1", "osc2.level", 10.0)], &["osc1", "osc2"]),
        "bad_route"
    );
    assert_eq!(
        code(&[route("osc1", "wt1.level", 10.0)], &["osc1", "wt1"]),
        "bad_route"
    );
}

/// Run `segments` one-segment calls under `assert_no_alloc` with a chord held from the first; `before(k, link)` may
/// send commands first (outside the guard).
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
        let events: Vec<Event> = if k == 0 {
            [48, 55, 64]
                .iter()
                .enumerate()
                .map(|(i, &key)| Event {
                    offset: 0,
                    kind: EventKind::NoteOn {
                        key: NoteKey::new(key, i as i32),
                        velocity: 0.8,
                    },
                })
                .collect()
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

#[test]
fn a_swap_that_adds_an_audio_route_allocates_nothing() {
    let base = gens(&["wt1", "osc1", "osc2"]);
    let routed = with_routes(
        base.clone(),
        &[
            route("osc2", "wt1.pm", 40.0),
            route("osc1", "wt1.am", -25.0),
        ],
    );
    let (mut e, mut link) = Engine::with_link(plan(&base).0, SEED, LinkCapacity::default());
    let mut next = Some(plan(&routed).0);
    let (l, r) = run(&mut e, &mut link, 300, |k, link| {
        if k == 100 {
            link.send_plan(next.take().unwrap()).unwrap();
        }
    });
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(e.stats().crossfades, 0, "nothing was added or removed");
    // Nothing added or removed: no crossfade, and the (silent) carrier changes no bit.
    let (mut e0, mut link0) = Engine::with_link(plan(&base).0, SEED, LinkCapacity::default());
    let (l0, r0) = run(&mut e0, &mut link0, 300, |_, _| {});
    assert_eq!(bits(&l), bits(&l0));
    assert_eq!(bits(&r), bits(&r0));
    assert!(l.iter().any(|x| x.abs() > 0.05));
}

#[test]
fn swaps_that_add_and_remove_routes_and_modules_allocate_nothing() {
    let base = gens(&["osc1", "osc2"]);
    let with_wt = with_routes(
        gens(&["osc1", "osc2", "wt1"]),
        &[route("osc2", "wt1.pm", 40.0), route("osc1", "wt1.pm", 10.0)],
    );
    let reordered = with_routes(
        gens(&["wt1", "osc1", "osc2"]),
        &[route("osc2", "wt1.pm", 40.0), route("osc1", "wt1.pm", 10.0)],
    );
    let (mut e, mut link) = Engine::with_link(plan(&base).0, SEED, LinkCapacity::default());
    // A module and routes added (a crossfade), the module moved in the patch and its routes with it (the same modules
    // and routes, found by id: none), then everything removed again (a crossfade).
    let mut queue = vec![plan(&with_wt).0, plan(&reordered).0, plan(&base).0];
    let mut sent = 0;
    run(&mut e, &mut link, 1_400, |k, link| {
        if k % 400 == 100 && sent < 3 && link.send_plan(queue.remove(0)).is_ok() {
            sent += 1;
        }
    });
    assert_eq!(e.stats().swaps, 3);
    assert_eq!(e.stats().crossfades, 2);
}

/// The segments an audio depth ramp takes (`DEPTH_SMOOTHING_MS` at the test rate).
fn ramp_segments() -> usize {
    (f64::from(DEPTH_SMOOTHING_MS) * f64::from(SR) / 1000.0 / SEG as f64).round() as usize
}

/// Run an engine one segment at a time with nothing playing, sending `next` (and nothing else) before segment
/// `send_at`; after every segment `watch` reads the engine. Allocation-guarded like [`run`].
fn step_through<T>(
    e: &mut Engine,
    link: &mut Link,
    segments: usize,
    send_at: usize,
    mut next: Option<Plan>,
    mut watch: impl FnMut(&Engine) -> T,
) -> Vec<T> {
    let mut seen = Vec::new();
    for k in 0..segments {
        if k == send_at {
            link.send_plan(next.take().unwrap()).unwrap();
        }
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        assert_no_alloc(|| e.process(&mut l, &mut r, &[]));
        link.collect();
        seen.push(watch(e));
    }
    seen
}

#[test]
fn a_route_added_by_a_swap_ramps_in_from_zero() {
    let base = gens(&["osc1", "wt1"]);
    let routed = with_routes(base.clone(), &[route("osc1", "wt1.pm", 80.0)]);
    let (new_plan, map) = plan(&routed);
    let id = map.depth(&raddr("osc1", "wt1.pm")).expect("an audio route");
    let (mut e, mut link) = Engine::with_link(plan(&base).0, SEED, LinkCapacity::default());
    let n = ramp_segments();
    let seen = step_through(&mut e, &mut link, 10 + n + 4, 10, Some(new_plan), |e| {
        e.plan().route_depth(id)
    });
    assert_eq!(seen[9], None, "no route before the swap");
    let target = 80.0f32 / 100.0 * 100.0;
    let ramp: Vec<f32> = seen[10..10 + n].iter().map(|d| d.unwrap()).collect();
    assert!(
        ramp[0] > 0.0 && ramp[0] < target / 4.0,
        "starts near 0, not at {target}: {}",
        ramp[0]
    );
    assert!(
        ramp.windows(2).all(|w| w[0] < w[1]),
        "rises every segment: {ramp:?}"
    );
    assert_eq!(
        ramp[n - 1].to_bits(),
        target.to_bits(),
        "lands on the depth"
    );
    assert!(
        seen[10 + n..]
            .iter()
            .all(|d| d.unwrap().to_bits() == target.to_bits())
    );
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(e.stats().crossfades, 0, "a route added needs no crossfade");
}

#[test]
fn a_depth_that_moves_from_zero_ramps_in_and_one_that_changes_goes_on_from_where_it_was() {
    // Depth 0 is left out of the plan, so 0 -> 40 is a route that appears; 40 -> 90 is a route that stays.
    let base = gens(&["osc1", "wt1"]);
    let (zero, _) = plan(&with_routes(base.clone(), &[route("osc1", "wt1.pm", 0.0)]));
    let (forty, map) = plan(&with_routes(base.clone(), &[route("osc1", "wt1.pm", 40.0)]));
    let (ninety, _) = plan(&with_routes(base, &[route("osc1", "wt1.pm", 90.0)]));
    let id = map.depth(&raddr("osc1", "wt1.pm")).unwrap();
    let n = ramp_segments();
    let (mut e, mut link) = Engine::with_link(zero, SEED, LinkCapacity::default());
    let up = step_through(&mut e, &mut link, n + 3, 0, Some(forty), |e| {
        e.plan().route_depth(id)
    });
    let target = 40.0f32 / 100.0 * 100.0;
    assert!(up[0].unwrap() > 0.0 && up[0].unwrap() < target / 4.0);
    assert_eq!(up[n - 1].unwrap().to_bits(), target.to_bits());
    // Now 40 -> 90 across a swap: the first segment is a step up from 40, not a jump to 90.
    let down = step_through(&mut e, &mut link, n + 3, 0, Some(ninety), |e| {
        e.plan().route_depth(id)
    });
    let first = down[0].unwrap();
    assert!(
        first > target && first < 90.0 / 2.0,
        "went on from 40: {first}"
    );
    assert_eq!(
        down[n - 1].unwrap().to_bits(),
        (90.0f32 / 100.0 * 100.0).to_bits()
    );
    assert_eq!(e.stats().swaps, 2);
    assert_eq!(e.stats().crossfades, 0, "no module and no route went away");
}

#[test]
fn a_swap_that_changes_nothing_about_the_routes_leaves_their_depths_alone() {
    let base = with_routes(gens(&["osc1", "wt1"]), &[route("osc1", "wt1.pm", 55.0)]);
    let (first, map) = plan(&base);
    let id = map.depth(&raddr("osc1", "wt1.pm")).unwrap();
    let (mut e, mut link) = Engine::with_link(first, SEED, LinkCapacity::default());
    let same = step_through(&mut e, &mut link, 6, 2, Some(plan(&base).0), |e| {
        e.plan().route_depth(id).unwrap().to_bits()
    });
    assert!(same.iter().all(|b| *b == same[0]), "no ramp: {same:?}");
    assert_eq!(e.stats().crossfades, 0);
}

#[test]
fn a_depth_event_in_flight_goes_on_across_a_swap_to_the_same_depth() {
    let base = with_routes(gens(&["osc1", "wt1"]), &[route("osc1", "wt1.pm", 80.0)]);
    let landing = with_routes(gens(&["osc1", "wt1"]), &[route("osc1", "wt1.pm", 20.0)]);
    let (first, map) = plan(&base);
    let id = map.depth(&raddr("osc1", "wt1.pm")).unwrap();
    let n = ramp_segments();
    let (mut e, mut link) = Engine::with_link(first, SEED, LinkCapacity::default());
    link.send_depth(0, id, 20.0).unwrap();
    // The event ramps 80 -> 20; three segments in, the patch is swapped for one that says 20.
    let seen = step_through(&mut e, &mut link, n + 2, 3, Some(plan(&landing).0), |e| {
        e.plan().route_depth(id).unwrap()
    });
    assert!(
        seen.windows(2).all(|w| w[0] >= w[1]),
        "falls all the way: {seen:?}"
    );
    assert!(seen[2] > 20.0 && seen[2] < 80.0);
    assert_eq!(
        seen[n - 1].to_bits(),
        20.0f32.to_bits(),
        "lands when the event's ramp would have"
    );
}

#[test]
fn a_route_removed_by_a_swap_crossfades_and_a_silent_one_does_not() {
    let base = gens(&["osc1", "wt1"]);
    let routed = with_routes(base.clone(), &[route("osc1", "wt1.pm", 80.0)]);
    // Removed while audible: the old plan fades out with its route.
    let (mut e, mut link) = Engine::with_link(plan(&routed).0, SEED, LinkCapacity::default());
    step_through(&mut e, &mut link, 20, 5, Some(plan(&base).0), |_| ());
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(e.stats().crossfades, 1);
    assert!(!e.is_crossfading(), "the crossfade is over");
    // A depth moved to 0 across a recompile is the same swap.
    let zero = with_routes(base.clone(), &[route("osc1", "wt1.pm", 0.0)]);
    let (mut e, mut link) = Engine::with_link(plan(&routed).0, SEED, LinkCapacity::default());
    step_through(&mut e, &mut link, 20, 5, Some(plan(&zero).0), |_| ());
    assert_eq!(e.stats().crossfades, 1);
    // A route a depth event has already brought to 0 has nothing left to fade.
    let (first, map) = plan(&routed);
    let id = map.depth(&raddr("osc1", "wt1.pm")).unwrap();
    let (mut e, mut link) = Engine::with_link(first, SEED, LinkCapacity::default());
    link.send_depth(0, id, 0.0).unwrap();
    step_through(
        &mut e,
        &mut link,
        ramp_segments() + 5,
        ramp_segments() + 2,
        Some(plan(&base).0),
        |_| (),
    );
    assert_eq!(e.plan().route_depth(id), None, "the route is gone");
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(e.stats().crossfades, 0, "silent already: no crossfade");
}

#[test]
fn routes_found_by_id_survive_a_reorder_and_a_swap_without_routes_is_what_it_was() {
    // The same routes with the generators in another patch order: the routes are the same routes.
    let a = with_routes(
        gens(&["wt1", "osc1", "osc2"]),
        &[route("osc2", "wt1.pm", 40.0), route("osc1", "wt1.am", 25.0)],
    );
    let b = with_routes(
        gens(&["osc2", "osc1", "wt1"]),
        &[route("osc1", "wt1.am", 25.0), route("osc2", "wt1.pm", 40.0)],
    );
    let (first, _) = plan(&a);
    let (second, map_b) = plan(&b);
    let (pm_b, am_b) = (
        map_b.depth(&raddr("osc2", "wt1.pm")).unwrap(),
        map_b.depth(&raddr("osc1", "wt1.am")).unwrap(),
    );
    let (mut e, mut link) = Engine::with_link(first, SEED, LinkCapacity::default());
    step_through(&mut e, &mut link, 6, 2, Some(second), |_| ());
    assert_eq!(e.stats().crossfades, 0);
    // Four segments after the swap an unmatched route would still be ramping in (15 segments); these are on target.
    assert_eq!(e.plan().route_depth(pm_b), Some(40.0f32 / 100.0 * 100.0));
    assert_eq!(e.plan().route_depth(am_b), Some(25.0f32 / 100.0 * 100.0));
    // A swap between patches without audio routes: the bits of a render that swaps equal the bits of one that does
    // not (the audio code is not entered).
    let plain = gens(&["osc1", "osc2"]);
    let (mut s, mut slink) = Engine::with_link(plan(&plain).0, SEED, LinkCapacity::default());
    let (l1, r1) = run(&mut s, &mut slink, 120, |k, link| {
        if k == 40 {
            link.send_plan(plan(&plain).0).unwrap();
        }
    });
    let (mut t, mut tlink) = Engine::with_link(plan(&plain).0, SEED, LinkCapacity::default());
    let (l0, r0) = run(&mut t, &mut tlink, 120, |_, _| {});
    assert_eq!(s.stats().swaps, 1);
    assert_eq!(s.stats().crossfades, 0);
    assert_eq!(bits(&l1), bits(&l0));
    assert_eq!(bits(&r1), bits(&r0));
}

#[test]
fn depth_events_reach_audio_routes_and_stay_apart_from_control_routes() {
    let mut p = with_routes(
        gens(&["osc1", "wt1"]),
        &[
            route("lfo1", "osc1.level", -30.0),
            route("osc1", "wt1.pm", 80.0),
        ],
    );
    let mut lfo = ModuleInstance::new(id("lfo1"), ModuleKind::Lfo);
    lfo.values[LfoParam::Rate.index()] = 3.0;
    p.modulators.push(lfo);
    let (p0, map) = plan(&p);
    let control = map.depth(&raddr("lfo1", "osc1.level")).expect("control id");
    let audio = map.depth(&raddr("osc1", "wt1.pm")).expect("audio id");
    assert_ne!(control, audio);
    assert_eq!(p0.route_depth(control), Some(-30.0f32 / 100.0 * 100.0));
    assert_eq!(p0.route_depth(audio), Some(80.0f32 / 100.0 * 100.0));
    let (mut e, mut link) = Engine::with_link(p0, SEED, LinkCapacity::default());
    let mut seen = Vec::new();
    for k in 0..40 {
        if k == 2 {
            link.send_depth(0, audio, 20.0).unwrap();
            link.send_depth(7, audio, 99.0).unwrap(); // another plan's epoch: dropped
            link.send_depth(0, control, f32::NAN).unwrap(); // ignored
        }
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        assert_no_alloc(|| e.process(&mut l, &mut r, &[]));
        seen.push((
            e.plan().route_depth(audio).unwrap(),
            e.plan().route_depth(control).unwrap(),
        ));
    }
    let n = (f64::from(DEPTH_SMOOTHING_MS) * f64::from(SR) / 1000.0 / SEG as f64).round() as usize;
    assert!(
        seen[2].0 < 80.0 && seen[2].0 > 20.0,
        "ramping: {}",
        seen[2].0
    );
    assert_eq!(
        seen[2 + n - 1].0,
        20.0f32 / 100.0 * 100.0,
        "landed after one ramp"
    );
    assert_eq!(seen[39].0, 20.0f32 / 100.0 * 100.0);
    assert!(
        seen.iter().all(|s| s.1 == -30.0f32 / 100.0 * 100.0),
        "the control route did not move"
    );
    assert_eq!(e.stats().stale_events, 1);
    // A reset lands any ramp in flight at once.
    link.send_depth(0, audio, -60.0).unwrap();
    let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
    e.process(&mut l, &mut r, &[]);
    e.reset();
    assert_eq!(e.plan().route_depth(audio), Some(-60.0f32 / 100.0 * 100.0));
}

#[test]
fn a_patch_without_audio_routes_has_no_audio_depth_ids() {
    let mut p = gens(&["osc1", "wt1"]);
    p.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    let (_, map) = plan(&p);
    assert_eq!(map.depth(&raddr("osc1", "wt1.pm")), None);
    // Depth 0 is left out like a control route at depth 0: a depth change from or to 0 is a recompile.
    let (_, map) = plan(&with_routes(p, &[route("osc1", "wt1.pm", 0.0)]));
    assert_eq!(map.depth(&raddr("osc1", "wt1.pm")), None);
}

#[test]
fn audio_routes_and_the_stereo_voice_path_compose() {
    // The stereo path with mono content equals the mono path (lane SVP); an audio route into a silent carrier changes
    // nothing on either. So routed + forced stereo equals unrouted + mono, on both channels.
    let base = gens(&["wt1", "osc1", "osc2"]);
    let routed = with_routes(
        base.clone(),
        &[
            route("osc2", "wt1.pm", 40.0),
            route("osc1", "wt1.am", -25.0),
        ],
    );
    let frames = 8_000;
    let cfg = PrepareCfg::new(SR);
    let mut mono = Engine::new(compile(&base, &cfg).unwrap().0, SEED);
    let (stereo_plan, _) = compile_with(&routed, &cfg, StereoMode::Forced).unwrap();
    assert!(stereo_plan.is_stereo());
    let mut both = Engine::new(stereo_plan, SEED);
    let events = common::events(&chord());
    let (ml, mr) = common::drive(&mut mono, &events, frames, 480, |_, _, _| {});
    let (sl, sr) = common::drive(&mut both, &events, frames, 480, |_, _, _| {});
    assert!(ml.iter().any(|x| x.abs() > 0.05));
    assert_eq!(bits(&ml), bits(&sl));
    assert_eq!(bits(&mr), bits(&sr));
}
