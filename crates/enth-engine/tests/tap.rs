//! The telemetry tap (P1b lane E+): read-only, lock-free and allocation-free; it never changes the output; overflow
//! drops and never blocks. Every `process` call runs under `assert_no_alloc`.
#![cfg(debug_assertions)]

mod common;

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, bits, every_kind_patch, id};
use enth_engine::{
    Engine, Event, EventKind, Link, LinkCapacity, NoteKey, ParamId, Plan, PlanMap, PrepareCfg, SEG,
    Tap, TapCapacity, compile,
};
use enth_patch::catalogue::VoiceParam;
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn plan(patch: &Patch) -> (Plan, PlanMap) {
    compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles")
}

fn route(p: &mut Patch, src: &str, dst: &str, depth: f32) {
    p.routes.push(Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    });
}

/// Every kind, 16 voices, routes from voice and global sources into voice and global destinations.
fn routed_patch() -> Patch {
    let mut p = every_kind_patch(300.0);
    p.voice.values[VoiceParam::Polyphony.index()] = 16.0;
    route(&mut p, "lfo1", "filter1.cutoff", 20.0);
    route(&mut p, "env1", "osc2.level", -30.0);
    route(&mut p, "velocity", "noise1.level", 40.0);
    route(&mut p, "key", "voice.pan", 50.0);
    route(&mut p, "lfo2", "comp1.mix", -40.0);
    route(&mut p, "macro1", "reverb1.wet", 30.0);
    route(&mut p, "modwheel", "osc1.fine", 25.0);
    p
}

fn on(key: u8, note_id: i32) -> EventKind {
    EventKind::NoteOn {
        key: NoteKey::new(key, note_id),
        velocity: 0.7,
    }
}

fn off(key: u8, note_id: i32) -> EventKind {
    EventKind::NoteOff {
        key: NoteKey::new(key, note_id),
        velocity: 0.0,
    }
}

/// The performance: 16 notes, released at segment 200.
fn events(k: usize) -> Vec<EventKind> {
    match k {
        0 => (0..16).map(|i| on(40 + 3 * i as u8, i)).collect(),
        200 => (0..16).map(|i| off(40 + 3 * i as u8, i)).collect(),
        _ => Vec::new(),
    }
}

/// Render `blocks` blocks of `block` frames under `assert_no_alloc`, calling `after(engine, left, right)` after each.
fn render(
    engine: &mut Engine,
    link: &mut Link,
    blocks: usize,
    block: usize,
    mut before: impl FnMut(usize, &mut Link),
    mut after: impl FnMut(&Engine, &[f32], &[f32]),
) -> (Vec<f32>, Vec<f32>) {
    let mut left = vec![0.0; blocks * block];
    let mut right = vec![0.0; blocks * block];
    for k in 0..blocks {
        before(k, link);
        // Blocks are whole segments here: the events of the segment the block starts with.
        let ev: Vec<Event> = events(k * block / SEG)
            .into_iter()
            .map(|kind| Event { offset: 0, kind })
            .collect();
        let (l, r) = (
            &mut left[k * block..(k + 1) * block],
            &mut right[k * block..(k + 1) * block],
        );
        assert_no_alloc(|| engine.process(l, r, &ev));
        after(engine, l, r);
        link.collect();
    }
    (left, right)
}

#[test]
fn the_tap_never_changes_the_output() {
    let run = |tap: Option<Arc<Tap>>| {
        let (mut e, mut link) =
            Engine::with_link(plan(&routed_patch()).0, SEED, LinkCapacity::default());
        if let Some(t) = tap {
            t.set_enabled(true);
            e.attach_tap(t);
        }
        render(&mut e, &mut link, 600, 64, |_, _| {}, |_, _, _| {})
    };
    let (l0, r0) = run(None);
    let tap = Tap::new(TapCapacity::default());
    let (l1, r1) = run(Some(Arc::clone(&tap)));
    assert!(tap.generation() > 0, "it published");
    assert_eq!(bits(&l1), bits(&l0));
    assert_eq!(bits(&r1), bits(&r0));
    // A tiny tap that overflows everywhere changes nothing either.
    let small = Tap::new(TapCapacity {
        params: 1,
        modulators: 1,
        scope: 32,
    });
    let (l2, _) = run(Some(small));
    assert_eq!(bits(&l2), bits(&l0));
}

#[test]
fn the_tap_carries_the_modulated_values_of_the_display_voice() {
    let (pl, map) = plan(&routed_patch());
    let ids: Vec<ParamId> = pl.tap_params().to_vec();
    let mods: Vec<String> = pl
        .tap_modulators()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(ids.len(), 7, "one per routed destination");
    assert_eq!(mods, ["env1", "lfo1", "lfo2"]);
    assert!(map.addr(ids[0]).is_some());
    let tap = Tap::new(TapCapacity::default());
    tap.set_enabled(true);
    let (mut e, mut link) = Engine::with_link(pl, SEED, LinkCapacity::default());
    e.attach_tap(Arc::clone(&tap));
    let mut frame = tap.frame();
    let mut checked = 0;
    let mut held = Vec::new();
    render(
        &mut e,
        &mut link,
        150,
        64,
        |_, _| {},
        |e, _, _| {
            tap.read(&mut frame);
            held.push(frame.held);
            assert_eq!(frame.epoch, 0);
            assert_eq!(frame.voices as usize, e.active_voices().len());
            assert_eq!(frame.params_len, ids.len());
            assert_eq!(frame.modulators_len, 3);
            assert_eq!(frame.stats, e.stats());
            let Some(v) = frame.display_voice else { return };
            let v = v as usize;
            // The newest held voice.
            let newest = e
                .active_voices()
                .iter()
                .copied()
                .max_by_key(|&v| e.notes().note_index[v])
                .unwrap();
            assert_eq!(v, newest);
            for (k, &id) in ids.iter().enumerate() {
                assert_eq!(
                    frame.params[k].to_bits(),
                    e.value(id, v).unwrap().to_bits(),
                    "{}",
                    map.addr(id).unwrap()
                );
            }
            checked += 1;
        },
    );
    assert!(checked > 100);
    assert_eq!(
        held[99], 16,
        "every key down until the release at block 100"
    );
    assert_eq!(held[100], 0);
    assert!(frame.peaks[0] > 0.0 && frame.peaks[1] > 0.0);
}

#[test]
fn the_scope_holds_the_newest_mono_samples_and_a_small_ring_keeps_only_the_newest() {
    for capacity in [32, 4096] {
        let tap = Tap::new(TapCapacity {
            scope: capacity,
            ..TapCapacity::default()
        });
        tap.set_enabled(true);
        let (mut e, mut link) =
            Engine::with_link(plan(&routed_patch()).0, SEED, LinkCapacity::default());
        e.attach_tap(Arc::clone(&tap));
        let mut scope = vec![0.0f32; 2048];
        let (l, r) = render(&mut e, &mut link, 20, 512, |_, _| {}, |_, _, _| {});
        let written = tap.scope(&mut scope);
        assert_eq!(written, 20 * 512);
        let n = capacity.min(scope.len());
        let mono: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
        assert_eq!(
            bits(&scope[scope.len() - n..]),
            bits(&mono[mono.len() - n..])
        );
        assert!(
            scope[..scope.len() - n].iter().all(|x| *x == 0.0),
            "older than the ring"
        );
    }
}

#[test]
fn overflow_drops_and_counts_and_never_allocates() {
    // One parameter and one modulator of room for seven routed parameters and three modulators.
    let tap = Tap::new(TapCapacity {
        params: 1,
        modulators: 1,
        scope: 32,
    });
    tap.set_enabled(true);
    let (mut e, mut link) =
        Engine::with_link(plan(&routed_patch()).0, SEED, LinkCapacity::default());
    e.attach_tap(Arc::clone(&tap));
    let mut frame = tap.frame();
    render(&mut e, &mut link, 10, 4096, |_, _| {}, |_, _, _| {});
    tap.read(&mut frame);
    assert_eq!(frame.params_len, 1);
    assert_eq!(frame.modulators_len, 1);
    assert_eq!(
        frame.dropped,
        10 * (6 + 2),
        "six parameters and two modulators per publish"
    );
    assert_eq!(tap.capacity().scope, 32);
}

#[test]
fn silence_stops_publishing_and_the_wake_handshake_works() {
    let tap = Tap::new(TapCapacity::default());
    // The saw and the default envelope (400 ms release): silent soon after its note.
    let (mut e, _link) =
        Engine::with_link(plan(&common::saw_patch()).0, SEED, LinkCapacity::default());
    e.attach_tap(Arc::clone(&tap));
    let (mut l, mut r) = ([0.0; 64], [0.0; 64]);
    let note = [Event {
        offset: 0,
        kind: on(60, 99),
    }];
    // Disabled: nothing is published.
    assert_no_alloc(|| e.process(&mut l, &mut r, &note));
    assert_eq!(tap.generation(), 0);
    tap.set_enabled(true);
    let mut gens = Vec::new();
    let release = [Event {
        offset: 0,
        kind: off(60, 99),
    }];
    assert_no_alloc(|| e.process(&mut l, &mut r, &release));
    for _ in 0..4000 {
        assert_no_alloc(|| e.process(&mut l, &mut r, &[]));
        gens.push(tap.generation());
    }
    assert_eq!(e.active_voices().len(), 0);
    let last = *gens.last().unwrap();
    let stopped = gens.iter().position(|&g| g == last).unwrap();
    assert!(
        stopped < 3900,
        "the generation stops once silence fills the scope"
    );
    // The editor sleeps on the last generation it drew; the next publish with sound wakes it once.
    assert!(!tap.sleep(last - 1), "newer data: stay awake");
    assert!(tap.sleep(last));
    assert!(!tap.take_wake());
    assert_no_alloc(|| e.process(&mut l, &mut r, &note));
    assert!(tap.take_wake(), "woken");
    assert!(!tap.take_wake(), "once");
    assert_eq!(tap.generation(), last + 1);
}

#[test]
fn a_plan_swap_changes_the_layout_with_the_epoch() {
    let tap = Tap::new(TapCapacity::default());
    tap.set_enabled(true);
    let base = routed_patch();
    let mut edited = base.clone();
    edited
        .modulators
        .push(ModuleInstance::new(id("lfo3"), ModuleKind::Lfo));
    route(&mut edited, "lfo3", "osc1.level", 10.0);
    let (p1, _) = plan(&edited);
    let n1 = p1.tap_params().len();
    let (mut e, mut link) = Engine::with_link(plan(&base).0, SEED, LinkCapacity::default());
    e.attach_tap(Arc::clone(&tap));
    let mut p1 = Some(p1);
    let mut frame = tap.frame();
    let mut seen = Vec::new();
    render(
        &mut e,
        &mut link,
        40,
        64,
        |k, link| {
            if k == 10 {
                link.send_plan(p1.take().unwrap()).unwrap();
            }
        },
        |_, _, _| {
            tap.read(&mut frame);
            seen.push((frame.epoch, frame.params_len, frame.modulators_len));
        },
    );
    assert_eq!(seen[9], (0, 7, 3));
    assert_eq!(seen[10], (1, n1, 4), "published with the new plan's order");
    assert_eq!(n1, 8);
}
