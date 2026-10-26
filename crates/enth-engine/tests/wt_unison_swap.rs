//! `wt` unison through plan swaps and under the allocation guard (S2a lane UNI): rendering allocates nothing, even
//! with every voice stolen and a stereo `wt` in audio routes; a swap to an equal plan carries the copies' state and
//! gives the same bits as no swap; a change of the copy count, or of the stereo flag with it, is a structural edit
//! that crossfades; and flipping the count over and over never allocates. (Debug builds only: the guard is the
//! debug allocator.)
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, bits, id};
use enth_engine::module::{PrepareCfg, SEG};
use enth_engine::{
    Engine, Event, EventKind, Link, LinkCapacity, NoteKey, Plan, StereoMode, compile_with,
};
use enth_patch::catalogue::{AdsrParam, VoiceParam, WtParam};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn plan(patch: &Patch) -> Plan {
    compile_with(patch, &PrepareCfg::new(SR), StereoMode::Auto)
        .expect("the patch compiles")
        .0
}

fn route(src: &str, dst: &str, depth: f32) -> Route {
    Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    }
}

fn wt(name: &str, set: &[(WtParam, f32)]) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Wt);
    for &(param, value) in set {
        m.values[param.index()] = value;
    }
    m
}

/// A gated patch with a `wt1` of `count` copies, a plain `wt2` that it phase-modulates (a route leaves the stereo
/// generator, so the right `post` buffer exists) and a feedback to give the copies' history something to carry.
fn patch(count: u32) -> Patch {
    let mut p = Patch::init();
    p.generators.clear();
    let env = &mut p.modulators[0].values;
    env[AdsrParam::Attack.index()] = 0.0;
    env[AdsrParam::Decay.index()] = 0.0;
    env[AdsrParam::Sustain.index()] = 100.0;
    env[AdsrParam::Release.index()] = 80.0;
    p.generators.push(wt(
        "wt1",
        &[
            (WtParam::Unison, count as f32),
            (WtParam::Detune, 40.0),
            (WtParam::Blend, 70.0),
            (WtParam::Spread, 80.0),
            (WtParam::Feedback, 25.0),
            (WtParam::Position, 55.0),
        ],
    ));
    p.generators
        .push(wt("wt2", &[(WtParam::PmIndex, 2.0), (WtParam::Semi, 7.0)]));
    p.routes.push(route("wt1", "wt2.pm", 40.0));
    p.voice.values[VoiceParam::Pan.index()] = 20.0;
    p
}

fn on(key: u8, id: i32) -> EventKind {
    EventKind::NoteOn {
        key: NoteKey::new(key, id),
        velocity: 0.8,
    }
}

fn off(key: u8, id: i32) -> EventKind {
    EventKind::NoteOff {
        key: NoteKey::new(key, id),
        velocity: 0.0,
    }
}

/// A plan sent at a segment.
struct Swap<'a> {
    at: usize,
    patch: &'a Patch,
}

/// A chord held from segment 0 (a fourth note at 100, released at 400), 900 segments, every `process` call under
/// `assert_no_alloc`; `swaps` are sent at their segments.
fn run(base: &Patch, swaps: &[Swap<'_>], capacity: LinkCapacity) -> (Vec<f32>, Vec<f32>, Engine) {
    let (mut e, mut link): (Engine, Link) = Engine::with_link(plan(base), SEED, capacity);
    let mut pending: Vec<(usize, Plan)> = swaps.iter().map(|s| (s.at, plan(s.patch))).collect();
    let segments = 900;
    let (mut left, mut right) = (vec![0.0; segments * SEG], vec![0.0; segments * SEG]);
    for k in 0..segments {
        if let Some(i) = pending.iter().position(|(at, _)| *at == k) {
            let (_, p) = pending.remove(i);
            link.send_plan(p).expect("room for a plan");
        }
        link.collect();
        let kinds = match k {
            0 => vec![on(48, 0), on(55, 1), on(60, 2)],
            100 => vec![on(64, 3)],
            400 => vec![off(48, 0), off(55, 1), off(60, 2), off(64, 3)],
            _ => Vec::new(),
        };
        let evs: Vec<Event> = kinds
            .into_iter()
            .map(|kind| Event { offset: 0, kind })
            .collect();
        let (l, r) = (
            &mut left[k * SEG..(k + 1) * SEG],
            &mut right[k * SEG..(k + 1) * SEG],
        );
        assert_no_alloc(|| e.process(l, r, &evs));
    }
    link.collect();
    (left, right, e)
}

#[test]
fn rendering_a_stereo_wt_in_routes_allocates_nothing_even_with_every_voice_stolen() {
    let mut p = patch(7);
    p.voice.values[VoiceParam::Polyphony.index()] = 4.0;
    let (mut e, _link) = Engine::with_link(plan(&p), SEED, LinkCapacity::default());
    assert!(e.plan().is_stereo());
    let mut left = vec![0.0; 64];
    let mut right = vec![0.0; 64];
    let mut heard = false;
    for block in 0..400 {
        let evs: Vec<Event> = if block < 40 {
            vec![Event {
                offset: 0,
                kind: on(36 + (block as u8 * 3) % 50, block),
            }]
        } else {
            Vec::new()
        };
        assert_no_alloc(|| e.process(&mut left, &mut right, &evs));
        heard |= left.iter().any(|x| *x != 0.0);
    }
    assert!(heard);
    assert!(e.stats().stolen_voices > 0, "steals happened");
    assert_eq!(e.stats().dropped_notes, 0);
}

#[test]
fn a_swap_to_an_equal_plan_carries_the_copies_and_changes_no_bit() {
    let base = patch(5);
    let cap = LinkCapacity::default();
    let (l0, r0, e0) = run(&base, &[], cap);
    let (l1, r1, e1) = run(
        &base,
        &[Swap {
            at: 200,
            patch: &base,
        }],
        cap,
    );
    assert_eq!((e0.stats().swaps, e1.stats().swaps), (0, 1));
    assert_eq!(
        e1.stats().crossfades,
        0,
        "nothing structural changed: instant"
    );
    assert!(l0.iter().any(|x| x.abs() > 0.05));
    assert_eq!(bits(&l1), bits(&l0), "left");
    assert_eq!(bits(&r1), bits(&r0), "right");
}

#[test]
fn an_edit_of_the_unison_knobs_keeps_the_copies_going_and_does_not_crossfade() {
    // detune, blend and spread are parameters (modulatable), so a swap that changes them is instant and the copies
    // keep their phases: the first segments after the swap continue the old render (no jump of the waveform).
    let base = patch(5);
    let mut edit = base.clone();
    edit.generators[0].values[WtParam::Detune.index()] = 55.0;
    edit.generators[0].values[WtParam::Spread.index()] = 30.0;
    let cap = LinkCapacity::default();
    let (l0, _, _) = run(&base, &[], cap);
    let (l1, r1, e1) = run(
        &edit,
        &[Swap {
            at: 200,
            patch: &edit,
        }],
        cap,
    );
    let (l2, r2, e2) = run(
        &base,
        &[Swap {
            at: 200,
            patch: &edit,
        }],
        cap,
    );
    assert_eq!(e2.stats().crossfades, 0);
    assert_eq!(e1.stats().crossfades, 0);
    // Before the swap the render is the base's.
    let cut = 200 * SEG;
    assert_eq!(bits(&l2[..cut]), bits(&l0[..cut]));
    // A render of the edit from the start differs from one switched to it mid-note (the copies had other phases)...
    assert_ne!(bits(&l1), bits(&l2));
    // ... and the switched render is continuous across the swap: no sample jump larger than the signal's own slope.
    let mut worst = 0.0f32;
    for i in cut - 4..cut + 4 {
        worst = worst.max((l2[i + 1] - l2[i]).abs());
    }
    let slope = (cut - 400..cut)
        .map(|i| (l0[i + 1] - l0[i]).abs())
        .fold(0.0, f32::max);
    assert!(
        worst < 4.0 * slope + 0.02,
        "a jump of {worst} at the swap, slope {slope}"
    );
    assert!(r2.iter().any(|x| x.abs() > 0.05));
    assert!(r1.iter().all(|x| x.is_finite()));
}

#[test]
fn a_change_of_the_copy_count_crossfades_and_never_allocates() {
    let cap = LinkCapacity::default();
    for (from, to) in [(4u32, 5u32), (7, 2), (1, 4), (4, 1), (1, 1)] {
        let base = patch(from);
        let next = patch(to);
        let (l, r, e) = run(
            &base,
            &[Swap {
                at: 200,
                patch: &next,
            }],
            cap,
        );
        let structural = from != to;
        assert_eq!(e.stats().swaps, 1, "{from} -> {to}");
        assert_eq!(
            e.stats().crossfades,
            u64::from(structural),
            "{from} -> {to}: crossfades"
        );
        assert!(!e.is_crossfading());
        assert_eq!(e.plan().is_stereo(), to >= 2, "{from} -> {to}: the flag");
        assert!(l.iter().chain(&r).all(|x| x.is_finite()), "{from} -> {to}");
        assert!(
            r.iter().any(|x| x.abs() > 0.05),
            "{from} -> {to}: sound after the swap"
        );
        // The raised-cosine fade is smooth: no step between consecutive samples beyond twice the loudest step the
        // signal makes on either side of the swap (the old and the new plan play the same notes).
        let step = |x: &[f32], range: std::ops::Range<usize>| {
            range
                .map(|i| (x[i + 1] - x[i]).abs())
                .fold(0.0f32, f32::max)
        };
        let reference = step(&l, 100 * SEG..199 * SEG).max(step(&l, 600 * SEG..700 * SEG));
        let at = step(&l, 200 * SEG..202 * SEG + 300);
        assert!(
            at <= 2.5 * reference + 0.01,
            "{from} -> {to}: a step of {at} at the swap, the signal's own {reference}"
        );
    }
}

#[test]
fn flipping_the_copy_count_over_and_over_never_allocates() {
    let a = patch(3);
    let b = patch(6);
    let c = patch(1);
    // A return ring of one: a crossfade's old plan waits in the pending slot until collected.
    let cap = LinkCapacity {
        commands: 64,
        returns: 1,
    };
    let swaps: Vec<Swap<'_>> = (0..30)
        .map(|i| Swap {
            at: 10 + 28 * i,
            patch: match i % 3 {
                0 => &b,
                1 => &c,
                _ => &a,
            },
        })
        .collect();
    let (l, r, e) = run(&a, &swaps, cap);
    assert!(
        e.stats().swaps >= 20,
        "the swaps went through: {:?}",
        e.stats()
    );
    assert_eq!(e.stats().crossfades, e.stats().swaps);
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
    assert!(l.iter().any(|x| x.abs() > 0.05));
}
