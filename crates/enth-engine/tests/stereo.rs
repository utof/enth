//! The stereo voice path (lane SVP, S2a): with the plan's flag forced on and mono content, both channels equal the
//! mono render bit for bit (G1, every voice mode, every voice-effect kind, pans, steals, glide, routes, at several
//! host blocks); a swap that keeps the flag, one that turns it on and one that turns it off all give the mono
//! render's bits, with no allocation anywhere.
#![cfg(debug_assertions)]

mod common;

#[path = "../benches/common/g1.rs"]
mod g1_patch;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{Note, SEED, SR, bits, drive, events, id, note};
use enth_engine::module::{PrepareCfg, SEG};
use enth_engine::{
    Engine, Event, EventKind, Link, LinkCapacity, NoteKey, Plan, StereoMode, compile, compile_with,
};
use enth_patch::catalogue::{FilterMode, GlideMode, LfoParam, SvfParam, VoiceMode, VoiceParam};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn plan(patch: &Patch, mode: StereoMode) -> Plan {
    compile_with(patch, &PrepareCfg::new(SR), mode)
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

fn filter(name: &str, mode: FilterMode, cutoff: f32, resonance: f32) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Svf);
    m.values[SvfParam::Mode.index()] = mode.index() as f32;
    m.values[SvfParam::Cutoff.index()] = cutoff;
    m.values[SvfParam::Resonance.index()] = resonance;
    m.values[SvfParam::Keytrack.index()] = 50.0;
    m
}

/// The offline render of `notes` through `patch` with the flag as `mode` says.
fn render(
    patch: &Patch,
    mode: StereoMode,
    notes: &[Note],
    frames: usize,
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let p = plan(patch, mode);
    assert_eq!(p.is_stereo(), mode == StereoMode::Forced);
    let mut e = Engine::new(p, SEED);
    drive(&mut e, &events(notes), frames, block, |_, _, _| {})
}

/// Mono against forced stereo, both channels, bit for bit, at three host blocks.
fn assert_equal_to_mono(name: &str, patch: &Patch, notes: &[Note], frames: usize) {
    for block in [32, 100, 512] {
        let (ml, mr) = render(patch, StereoMode::Auto, notes, frames, block);
        let (sl, sr) = render(patch, StereoMode::Forced, notes, frames, block);
        assert!(
            ml.iter().any(|x| *x != 0.0),
            "{name}: the mono render is silent, so the comparison would prove nothing"
        );
        assert_eq!(bits(&sl), bits(&ml), "{name}: left, block {block}");
        assert_eq!(bits(&sr), bits(&mr), "{name}: right, block {block}");
    }
}

fn chord(len: u64) -> Vec<Note> {
    vec![
        note(48, 0, len),
        note(55, 0, len),
        note(60, 200, len),
        note(64, 2_000, len + 4_000),
    ]
}

#[test]
fn the_flag_is_clear_for_ordinary_patches_and_forcing_sets_it() {
    for p in [
        Patch::init(),
        common::every_kind_patch(400.0),
        g1_patch::g1_patch(),
    ] {
        assert!(!plan(&p, StereoMode::Auto).is_stereo());
        assert!(!compile(&p, &PrepareCfg::new(SR)).unwrap().0.is_stereo());
        assert!(plan(&p, StereoMode::Forced).is_stereo());
    }
}

#[test]
fn stereo_with_zero_spread_equals_mono_on_g1() {
    let patch = g1_patch::g1_patch();
    // The bench's chord: sixteen notes, held, then released.
    let notes: Vec<Note> = (0..16)
        .map(|i| note(36 + (i as u8 * 5) % 60, 0, 24_000))
        .collect();
    assert_equal_to_mono("g1", &patch, &notes, 48_000);
}

#[test]
fn stereo_with_zero_spread_equals_mono_for_voice_effects_pans_and_routes() {
    // A resonant low-pass with keytrack, a cutoff LFO, key to pan: every part of the per-voice chain and the balance.
    let mut routed = Patch::init();
    routed.voice_fx = vec![filter("filter1", FilterMode::Lp, 700.0, 70.0)];
    routed
        .modulators
        .push(ModuleInstance::new(id("lfo1"), ModuleKind::Lfo));
    routed.modulators[1].values[LfoParam::Rate.index()] = 6.0;
    routed.routes = vec![
        route("lfo1", "filter1.cutoff", 60.0),
        route("key", "voice.pan", 80.0),
    ];
    routed.voice.values[VoiceParam::Pan.index()] = 20.0;
    assert_equal_to_mono("routed", &routed, &chord(20_000), 36_000);

    // Two instances of one kind, then every kind a voice chain takes (the S2a ones are pass-through stubs).
    let mut chain = Patch::init();
    chain.voice_fx = vec![
        filter("filter1", FilterMode::Lp, 900.0, 60.0),
        filter("filter2", FilterMode::Hp, 200.0, 30.0),
        ModuleInstance::new(id("dist1"), ModuleKind::Dist),
        ModuleInstance::new(id("comb1"), ModuleKind::Comb),
    ];
    assert_equal_to_mono("two filters, dist, comb", &chain, &chord(20_000), 36_000);
    let mut other = Patch::init();
    other.voice_fx = vec![
        ModuleInstance::new(id("formant1"), ModuleKind::Formant),
        filter("filter1", FilterMode::Bp, 1_200.0, 50.0),
        ModuleInstance::new(id("ladder1"), ModuleKind::Ladder),
        filter("filter2", FilterMode::Notch, 800.0, 40.0),
    ];
    assert_equal_to_mono("formant, bp, ladder, notch", &other, &chord(20_000), 36_000);

    // Pans at the ends and off centre, each with a filter that has state.
    for pan in [-100.0, -37.0, 0.0, 64.0, 100.0] {
        let mut p = Patch::init();
        p.voice_fx = vec![filter("filter1", FilterMode::Lp, 1_500.0, 50.0)];
        p.voice.values[VoiceParam::Pan.index()] = pan;
        if pan == 100.0 || pan == -100.0 {
            // Hard right silences the left channel: compare on a patch that still has sound.
            let (ml, mr) = render(&p, StereoMode::Auto, &chord(10_000), 20_000, 100);
            let (sl, sr) = render(&p, StereoMode::Forced, &chord(10_000), 20_000, 100);
            assert_eq!(bits(&sl), bits(&ml), "pan {pan}: left");
            assert_eq!(bits(&sr), bits(&mr), "pan {pan}: right");
            continue;
        }
        assert_equal_to_mono(&format!("pan {pan}"), &p, &chord(10_000), 20_000);
    }
}

#[test]
fn stereo_with_zero_spread_equals_mono_in_every_voice_mode_with_steals_and_glide() {
    let overlapping = vec![
        note(48, 0, 6_000),
        note(55, 2_000, 9_000),
        note(52, 4_000, 12_000),
        note(60, 10_000, 16_000),
    ];
    for (name, mode, glide, polyphony) in [
        ("mono", VoiceMode::Mono, 80.0, 8.0),
        ("legato", VoiceMode::Legato, 60.0, 8.0),
        ("poly glide", VoiceMode::Poly, 40.0, 8.0),
        ("steals", VoiceMode::Poly, 0.0, 2.0),
    ] {
        let mut p = Patch::init();
        p.voice_fx = vec![filter("filter1", FilterMode::Lp, 1_800.0, 65.0)];
        p.voice.values[VoiceParam::Mode.index()] = mode.index() as f32;
        p.voice.values[VoiceParam::Glide.index()] = glide;
        p.voice.values[VoiceParam::GlideMode.index()] = GlideMode::Always.index() as f32;
        p.voice.values[VoiceParam::Polyphony.index()] = polyphony;
        let burst: Vec<Note> = if name == "steals" {
            (0..9)
                .map(|i| note(40 + 3 * i as u8, 400 * i, 14_000))
                .collect()
        } else {
            overlapping.clone()
        };
        assert_equal_to_mono(name, &p, &burst, 26_000);
    }
    // Every kind in the patch, with the reverb and the compressor after the voices.
    assert_equal_to_mono(
        "every kind",
        &common::every_kind_patch(500.0),
        &chord(20_000),
        36_000,
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Swap carry

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

/// A patch whose voice chain has state worth carrying: a resonant filter whose cutoff an LFO moves, plus an unrelated
/// second filter that the "added module" edit can add or remove.
fn base_patch() -> Patch {
    let mut p = Patch::init();
    p.voice_fx = vec![filter("filter1", FilterMode::Lp, 700.0, 75.0)];
    p.modulators
        .push(ModuleInstance::new(id("lfo1"), ModuleKind::Lfo));
    p.routes = vec![route("lfo1", "filter1.cutoff", 60.0)];
    p.voice.values[VoiceParam::Pan.index()] = -30.0;
    p
}

/// What a swap test does at segment `at`: send the plan built from this patch with this flag.
struct Swap<'a> {
    at: usize,
    patch: &'a Patch,
    mode: StereoMode,
}

/// A chord held from segment 0 (a fourth note at 100, released at 400), 900 segments, every `process` call under
/// `assert_no_alloc`; `swaps` are sent at their segments. Returns the output and the engine.
fn run_with_swaps(
    base: &Patch,
    start: StereoMode,
    swaps: &[Swap<'_>],
    capacity: LinkCapacity,
) -> (Vec<f32>, Vec<f32>, Engine) {
    let (mut e, mut link): (Engine, Link) = Engine::with_link(plan(base, start), SEED, capacity);
    let mut pending: Vec<(usize, Plan)> = swaps
        .iter()
        .map(|s| (s.at, plan(s.patch, s.mode)))
        .collect();
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

fn same_bits(name: &str, got: &(Vec<f32>, Vec<f32>), want: &(Vec<f32>, Vec<f32>)) {
    assert!(want.0.iter().any(|x| *x != 0.0), "{name}: silent reference");
    assert_eq!(bits(&got.0), bits(&want.0), "{name}: left");
    assert_eq!(bits(&got.1), bits(&want.1), "{name}: right");
}

#[test]
fn a_stereo_plan_recompiled_keeps_its_state_bit_for_bit_and_does_not_crossfade() {
    let base = base_patch();
    let cap = LinkCapacity::default();
    let (l0, r0, e0) = run_with_swaps(&base, StereoMode::Forced, &[], cap);
    let swap = Swap {
        at: 200,
        patch: &base,
        mode: StereoMode::Forced,
    };
    let (l1, r1, e1) = run_with_swaps(&base, StereoMode::Forced, &[swap], cap);
    assert_eq!((e0.stats().swaps, e1.stats().swaps), (0, 1));
    assert_eq!(e1.stats().crossfades, 0, "the flag did not change: instant");
    assert!(e1.plan().is_stereo());
    same_bits(
        "recompile",
        &(l1.clone(), r1.clone()),
        &(l0.clone(), r0.clone()),
    );
    // ... and both are the mono render's bits (zero spread).
    let (lm, rm, _) = run_with_swaps(&base, StereoMode::Auto, &[], cap);
    same_bits("stereo = mono", &(l1, r1), &(lm, rm));
}

#[test]
fn an_edit_that_keeps_the_flag_gives_the_mono_edits_bits() {
    let base = base_patch();
    let mut edit = base.clone();
    edit.voice_fx[0].values[SvfParam::Cutoff.index()] = 2_400.0;
    edit.voice.values[VoiceParam::Pan.index()] = 45.0;
    let cap = LinkCapacity::default();
    let swap = |mode| Swap {
        at: 200,
        patch: &edit,
        mode,
    };
    let mono = run_with_swaps(&base, StereoMode::Auto, &[swap(StereoMode::Auto)], cap);
    let st = run_with_swaps(&base, StereoMode::Forced, &[swap(StereoMode::Forced)], cap);
    assert_eq!(st.2.stats().crossfades, 0);
    assert_eq!(mono.2.stats().crossfades, 0);
    same_bits("edit", &(st.0, st.1), &(mono.0, mono.1));
}

#[test]
fn adding_and_removing_a_voice_effect_while_stereo_gives_the_mono_bits() {
    let base = base_patch();
    let mut added = base.clone();
    added
        .voice_fx
        .push(filter("filter2", FilterMode::Hp, 150.0, 40.0));
    let cap = LinkCapacity::default();
    for (name, from, to) in [("add", &base, &added), ("remove", &added, &base)] {
        let swap = |mode| Swap {
            at: 200,
            patch: to,
            mode,
        };
        let mono = run_with_swaps(from, StereoMode::Auto, &[swap(StereoMode::Auto)], cap);
        let st = run_with_swaps(from, StereoMode::Forced, &[swap(StereoMode::Forced)], cap);
        assert_eq!(st.2.stats().crossfades, 1, "{name}: a module came or went");
        assert!(!st.2.is_crossfading());
        same_bits(name, &(st.0, st.1), &(mono.0, mono.1));
    }
}

#[test]
fn turning_the_flag_on_or_off_crossfades_and_gives_the_mono_bits() {
    let base = base_patch();
    let cap = LinkCapacity::default();
    let want = run_with_swaps(&base, StereoMode::Auto, &[], cap);
    for (name, start, to) in [
        ("on", StereoMode::Auto, StereoMode::Forced),
        ("off", StereoMode::Forced, StereoMode::Auto),
    ] {
        let swap = Swap {
            at: 200,
            patch: &base,
            mode: to,
        };
        let (l, r, e) = run_with_swaps(&base, start, &[swap], cap);
        assert_eq!(e.stats().swaps, 1);
        assert_eq!(
            e.stats().crossfades,
            1,
            "{name}: a flag change is a structural change"
        );
        assert!(!e.is_crossfading());
        assert_eq!(e.plan().is_stereo(), to == StereoMode::Forced);
        assert_eq!(e.plans_held(), 1, "{name}: the old plan went back");
        // Mono content: the right channel of the new stereo plan continued from the left state, and the dropped one
        // changes nothing, so there is no discontinuity at all, not even in the fade.
        same_bits(name, &(l, r), &(want.0.clone(), want.1.clone()));
    }
}

#[test]
fn toggling_the_flag_over_and_over_never_allocates_and_never_changes_the_bits() {
    let base = base_patch();
    // A return ring of one: a crossfade's old plan waits in the pending slot until collected.
    let cap = LinkCapacity {
        commands: 64,
        returns: 1,
    };
    let want = run_with_swaps(&base, StereoMode::Auto, &[], cap);
    let toggles: Vec<Swap<'_>> = (0..30)
        .map(|i| Swap {
            at: 10 + 28 * i,
            patch: &base,
            mode: if i % 2 == 0 {
                StereoMode::Forced
            } else {
                StereoMode::Auto
            },
        })
        .collect();
    let (l, r, e) = run_with_swaps(&base, StereoMode::Auto, &toggles, cap);
    assert!(
        e.stats().swaps >= 20,
        "the swaps went through: {:?}",
        e.stats()
    );
    assert_eq!(e.stats().crossfades, e.stats().swaps);
    same_bits("toggles", &(l, r), &want_pair(&want));
}

fn want_pair(w: &(Vec<f32>, Vec<f32>, Engine)) -> (Vec<f32>, Vec<f32>) {
    (w.0.clone(), w.1.clone())
}

#[test]
fn rendering_with_the_flag_on_allocates_nothing_even_with_every_voice_stolen() {
    // 40 notes into polyphony 4 with a four-effect chain: steals and cuts through the stereo balance.
    let mut p = Patch::init();
    p.voice_fx = vec![
        filter("filter1", FilterMode::Lp, 1_000.0, 60.0),
        filter("filter2", FilterMode::Hp, 150.0, 20.0),
        ModuleInstance::new(id("dist1"), ModuleKind::Dist),
        ModuleInstance::new(id("comb1"), ModuleKind::Comb),
    ];
    p.voice.values[VoiceParam::Polyphony.index()] = 4.0;
    let (mut e, _link) =
        Engine::with_link(plan(&p, StereoMode::Forced), SEED, LinkCapacity::default());
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
fn a_40_s_release_tail_through_the_stereo_path_has_no_subnormals_and_every_voice_ends() {
    // tests/tail.rs for a forced-stereo plan: the right instances' state is scanned too (`Engine::subnormals`).
    let patch = common::every_kind_patch(3_000.0);
    let mut e = Engine::new(plan(&patch, StereoMode::Forced), SEED);
    let notes = vec![
        note(48, 0, 48_000),
        note(55, 0, 48_000),
        note(60, 960, 48_000),
        note(64, 1_920, 52_800),
    ];
    let (mut output, mut state, mut non_finite, mut calls) = (0, 0, 0, 0usize);
    drive(&mut e, &events(&notes), 40 * 48_000, 512, |e, l, r| {
        output += l.iter().chain(r).filter(|x| x.is_subnormal()).count();
        non_finite += l.iter().chain(r).filter(|x| !x.is_finite()).count();
        if calls.is_multiple_of(16) {
            state = state.max(e.subnormals());
        }
        calls += 1;
    });
    state = state.max(e.subnormals());
    assert_eq!((output, state, non_finite), (0, 0, 0));
    assert_eq!(e.active_voices().len(), 0, "voices must end below -100 dB");
}
