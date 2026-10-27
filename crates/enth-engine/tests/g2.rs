//! The G2 reference patch (S2a lane G2B, `benches/common/g2.rs`, 4 voices) and its 8-voice stress case: each compiles
//! to a stereo plan, plays all its voices, stays finite and bounded, renders the same bits at any host block, and its
//! swaps (an instant edit and a crossfading one, delay, chorus and OTT state among what moves or is copied) run under
//! `assert_no_alloc`. That the test host's fixtures and the golden case are these patches is checked by
//! `tests/golden/tests/g2.rs` (the engine is a core crate and does no file I/O).
#![cfg(debug_assertions)]

mod common;

#[path = "../benches/common/g2.rs"]
mod g2_patch;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, bits};
use enth_engine::module::PrepareCfg;
use enth_engine::{Engine, Event, EventKind, LinkCapacity, NoteKey, Plan, compile};
use enth_patch::catalogue::{Oversample, VoiceParam};
use enth_patch::{ModuleKind, Patch};
use g2_patch::{G2, KEYS, g2_patch, stress_patch};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn plan(patch: &Patch) -> Plan {
    compile(patch, &PrepareCfg::new(SR))
        .expect("the patch compiles")
        .0
}

fn on(key: u8, id: i32) -> Event {
    Event {
        offset: 0,
        kind: EventKind::NoteOn {
            key: NoteKey::new(key, id),
            velocity: 0.8,
        },
    }
}

/// The first `voices` notes of the bass chord.
fn chord(voices: u32) -> Vec<Event> {
    KEYS.iter()
        .take(voices as usize)
        .enumerate()
        .map(|(i, &k)| on(k, i as i32))
        .collect()
}

/// Render `frames` frames of the held chord (as many notes as the patch has voices) in `block`-frame calls.
fn render(patch: &Patch, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let mut engine = Engine::new(plan(patch), SEED);
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    let events = chord(patch.voice.values[VoiceParam::Polyphony.index()] as u32);
    let mut first = true;
    for (l, r) in left.chunks_mut(block).zip(right.chunks_mut(block)) {
        engine.process(l, r, if first { &events } else { &[] });
        first = false;
    }
    (left, right)
}

#[test]
fn g2_is_a_stereo_plan_that_plays_all_its_voices() {
    for (name, patch, voices) in [
        ("reference", g2_patch(), 4.0),
        ("stress", stress_patch(), 8.0),
    ] {
        assert_eq!(
            patch.voice.values[VoiceParam::Polyphony.index()],
            voices,
            "{name}: voices"
        );
        assert!(
            plan(&patch).is_stereo(),
            "{name}: a unison wt makes the plan stereo"
        );
        let (l, r) = render(&patch, 24_000, 64);
        let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(
            l.iter().chain(&r).all(|x| x.is_finite()),
            "{name}: every sample is finite"
        );
        assert!(peak > 0.05, "{name}: the patch sounds (peak {peak})");
        assert!(peak < 2.0, "{name}: the chain stays bounded (peak {peak})");
        assert_ne!(bits(&l), bits(&r), "{name}: the spread widens the voices");
    }
}

#[test]
fn the_output_does_not_depend_on_the_host_block() {
    for patch in [g2_patch(), stress_patch()] {
        let (l1, r1) = render(&patch, 9_600, 32);
        let (l2, r2) = render(&patch, 9_600, 100);
        assert_eq!(bits(&l1), bits(&l2), "left");
        assert_eq!(bits(&r1), bits(&r2), "right");
    }
}

#[test]
fn the_variants_the_bench_measures_all_compile() {
    for (name, g, stereo) in [
        ("reference", G2::default(), true),
        ("stress", G2::stress(), true),
        (
            "dist off",
            G2 {
                oversample: Oversample::Off,
                ..G2::stress()
            },
            true,
        ),
        (
            "dist x4",
            G2 {
                oversample: Oversample::X4,
                ..G2::stress()
            },
            true,
        ),
        (
            "no ladder",
            G2 {
                ladder: false,
                ..G2::stress()
            },
            true,
        ),
        (
            "no formant",
            G2 {
                formant: false,
                ..G2::stress()
            },
            true,
        ),
        (
            "3 copies",
            G2 {
                copies: 3,
                ..G2::stress()
            },
            true,
        ),
        (
            "1 copy",
            G2 {
                copies: 1,
                ..G2::stress()
            },
            false,
        ),
        (
            "6 voices",
            G2 {
                voices: 6,
                ..G2::stress()
            },
            true,
        ),
    ] {
        let p = g.patch();
        assert_eq!(plan(&p).is_stereo(), stereo, "{name}");
        let (l, r) = render(&p, 4_800, 64);
        assert!(
            l.iter().chain(&r).all(|x| x.is_finite()),
            "{name}: finite output"
        );
    }
}

/// Swap `edit` into `base` after 0.25 s of the held chord, run on for 0.25 s more, all under `assert_no_alloc`;
/// returns whether the engine was crossfading at some point.
fn swap_under_the_guard(base: G2, edit: &Patch) -> bool {
    let (mut engine, mut link) =
        Engine::with_link(plan(&base.patch()), SEED, LinkCapacity::default());
    let (mut left, mut right) = (vec![0.0; 64], vec![0.0; 64]);
    let events = chord(base.voices);
    let blocks = 12_000 / 64;
    assert_no_alloc(|| engine.process(&mut left, &mut right, &events));
    for _ in 1..blocks {
        assert_no_alloc(|| engine.process(&mut left, &mut right, &[]));
    }
    link.send_plan(plan(edit)).expect("one plan in flight");
    let mut crossfaded = false;
    for _ in 0..blocks {
        assert_no_alloc(|| engine.process(&mut left, &mut right, &[]));
        crossfaded |= engine.is_crossfading();
        assert!(
            left.iter().chain(&right).all(|x| x.is_finite()),
            "finite across the swap"
        );
    }
    assert!(!engine.is_crossfading(), "the crossfade ends");
    // The old plan goes back on the return ring and is freed here, off the audio thread.
    assert_eq!(link.collect(), 1, "the old plan came back");
    crossfaded
}

#[test]
fn an_instant_swap_and_a_crossfading_swap_allocate_nothing() {
    for base in [G2::default(), G2::stress()] {
        assert!(
            !swap_under_the_guard(base, &base.value_edit()),
            "a value edit swaps instantly ({} voices)",
            base.voices
        );
        assert!(
            swap_under_the_guard(base, &base.module_added()),
            "an added module crossfades ({} voices)",
            base.voices
        );
    }
}

#[test]
fn the_reference_has_the_modules_the_brief_names() {
    let p = g2_patch();
    let kinds: Vec<ModuleKind> = p
        .generators
        .iter()
        .chain(&p.voice_fx)
        .chain(&p.modulators)
        .chain(&p.fx)
        .map(|m| m.kind)
        .collect();
    for k in [
        ModuleKind::Wt,
        ModuleKind::Osc,
        ModuleKind::Dist,
        ModuleKind::Ladder,
        ModuleKind::Formant,
        ModuleKind::Mseg,
        ModuleKind::Lfo,
        ModuleKind::Ott,
        ModuleKind::Chorus,
        ModuleKind::Delay,
    ] {
        assert!(kinds.contains(&k), "G2 uses {k:?}");
    }
    assert_eq!(
        p.voice.values[VoiceParam::Polyphony.index()],
        4.0,
        "four voices: the reference"
    );
}
