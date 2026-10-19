//! The voice manager and the modulation matrix through the engine (lane S2b): steal fades, mono and legato, glide,
//! voice-end reports, chokes, routes from velocity, key, macros and the mod wheel, smoothing, block-size invariance
//! with modulation and glide, and 40 s tails in every voice mode.

mod common;

use common::{SEED, SR, bits, drive, engine, every_kind_patch, note, reset_saw_patch};
use enth_engine::{Event, EventKind, NoteKey, ParamId, PrepareCfg, SEG, compile};
use enth_patch::catalogue::{AdsrParam, GlideMode, OscParam, VoiceMode, VoiceParam};
use enth_patch::{Patch, Route};
use proptest::prelude::*;

/// The reference saw (reset phase) through a gate envelope, so levels are constant while a key is held.
fn gate_patch() -> Patch {
    let mut p = reset_saw_patch();
    let env = &mut p.modulators[0].values;
    env[AdsrParam::Attack.index()] = 0.0;
    env[AdsrParam::Decay.index()] = 0.0;
    env[AdsrParam::Sustain.index()] = 100.0;
    env[AdsrParam::Release.index()] = 0.0;
    p
}

fn with_mode(mut p: Patch, mode: VoiceMode) -> Patch {
    p.voice.values[VoiceParam::Mode.index()] = mode.index() as f32;
    p
}

fn with_glide(mut p: Patch, ms: f32, mode: GlideMode) -> Patch {
    p.voice.values[VoiceParam::Glide.index()] = ms;
    p.voice.values[VoiceParam::GlideMode.index()] = mode.index() as f32;
    p
}

fn route(p: &mut Patch, src: &str, dst: &str, depth: f32) {
    p.routes.push(Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    });
}

/// Absolute-frame events for notes (`(key, on, off)`), sorted stably by frame.
fn notes_events(notes: &[(u8, u64, u64)]) -> Vec<(u64, EventKind)> {
    let mut ev = Vec::new();
    for (i, &(key, on, off)) in notes.iter().enumerate() {
        let k = NoteKey::new(key, i as i32);
        ev.push((
            on,
            EventKind::NoteOn {
                key: k,
                velocity: 0.8,
            },
        ));
        ev.push((
            off,
            EventKind::NoteOff {
                key: k,
                velocity: 0.0,
            },
        ));
    }
    ev.sort_by_key(|(f, _)| *f);
    ev
}

/// Render, recording after every 32-frame block: (active voices, `pitch_cur` of every active voice).
fn trace(
    patch: &Patch,
    ev: &[(u64, EventKind)],
    frames: usize,
) -> (Vec<f32>, Vec<(usize, Vec<f32>)>) {
    let mut e = engine(patch, SEED);
    let mut t = Vec::new();
    let (l, _) = drive(&mut e, ev, frames, SEG, |e, _, _| {
        let pitches = e
            .active_voices()
            .iter()
            .map(|&v| e.notes().pitch_cur[v])
            .collect();
        t.push((e.active_voices().len(), pitches));
    });
    (l, t)
}

#[test]
fn a_stolen_voice_fades_out_over_64_frames() {
    let mut p = gate_patch();
    p.voice.values[VoiceParam::Polyphony.index()] = 1.0;
    let a = notes_events(&[(60, 0, 48_000)]);
    let b = notes_events(&[(67, 3200, 48_000)]);
    let both = notes_events(&[(60, 0, 48_000), (67, 3200, 48_000)]);
    let mut e = engine(&p, SEED);
    let (l_a, _) = drive(&mut engine(&p, SEED), &a, 8000, 64, |_, _, _| {});
    let (l_b, _) = drive(&mut engine(&p, SEED), &b, 8000, 64, |_, _, _| {});
    let (l, _) = drive(&mut e, &both, 8000, 64, |_, _, _| {});
    assert_eq!(e.stats().stolen_voices, 1);
    assert_eq!(
        e.active_voices(),
        &[1],
        "the new note took the spare slot 1"
    );
    assert_eq!(bits(&l[..3200]), bits(&l_a[..3200]));
    // During the fade: A times a linear fade 1 → 0 over 64 frames, plus B.
    for j in 0..64 {
        let t = 3200 + j;
        let fade = 1.0 - (j + 1) as f32 / 64.0;
        let expected = l_a[t] * fade + l_b[t];
        assert!(
            (l[t] - expected).abs() < 1e-6,
            "frame {t}: {} vs {expected}",
            l[t]
        );
    }
    // After it: B alone, bit for bit (the slot does not change a voice's samples).
    assert_eq!(bits(&l[3264..]), bits(&l_b[3264..]));
}

#[test]
fn mono_plays_one_voice_and_follows_the_last_note() {
    let p = with_mode(gate_patch(), VoiceMode::Mono);
    let ev = notes_events(&[(60, 0, 9600), (64, 2400, 4800), (67, 3200, 4000)]);
    let (_, t) = trace(&p, &ev, 12_000);
    assert!(t.iter().all(|(n, _)| *n <= 1), "one voice at most");
    let pitch_at = |frame: usize| t[frame / SEG].1.first().copied();
    assert_eq!(pitch_at(1000), Some(60.0));
    assert_eq!(pitch_at(3000), Some(64.0));
    assert_eq!(pitch_at(3500), Some(67.0));
    assert_eq!(pitch_at(4500), Some(64.0), "back to the newest held key");
    assert_eq!(pitch_at(6000), Some(60.0));
    assert_eq!(
        pitch_at(11_000),
        None,
        "released, and the gate envelope ends it at once"
    );
}

#[test]
fn mono_retriggers_the_envelope_and_legato_does_not() {
    // The reference envelope (attack 5 ms, decay 300 ms to 70 %): a retrigger climbs back towards full level.
    let p = reset_saw_patch();
    let ev = notes_events(&[(60, 0, 24_000), (60, 12_000, 24_000)]);
    let render = |mode| {
        let mut e = engine(&with_mode(p.clone(), mode), SEED);
        drive(&mut e, &ev, 16_000, 512, |_, _, _| {}).0
    };
    let (mono, legato) = (render(VoiceMode::Mono), render(VoiceMode::Legato));
    assert_eq!(bits(&mono[..12_000]), bits(&legato[..12_000]));
    let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, y| m.max(y.abs()));
    let (pm, pl) = (peak(&mono[12_400..13_000]), peak(&legato[12_400..13_000]));
    assert!(pm > pl * 1.2, "mono retriggered: {pm} vs legato {pl}");
    // In poly mode the same notes are two voices.
    let mut e = engine(&p, SEED);
    drive(&mut e, &ev, 13_000, 512, |_, _, _| {});
    assert_eq!(e.active_voices().len(), 2);
}

#[test]
fn glide_is_linear_in_semitones_at_constant_time() {
    // 100 ms at 48 kHz = 150 segments, whatever the interval.
    let p = with_glide(
        with_mode(gate_patch(), VoiceMode::Legato),
        100.0,
        GlideMode::Legato,
    );
    let ev = notes_events(&[(60, 0, 20_000), (72, 9600, 20_000), (48, 30_000, 40_000)]);
    let (_, t) = trace(&p, &ev, 40_000);
    let start = 9600 / SEG;
    for j in 0..150 {
        let got = t[start + j].1[0];
        let want = 60.0 + 12.0 * (j + 1) as f32 / 150.0;
        assert!((got - want).abs() < 1e-3, "segment {j}: {got} vs {want}");
    }
    assert_eq!(t[start + 149].1[0], 72.0, "lands exactly");
    assert_eq!(t[start + 300].1[0], 72.0);
    // Legato glide mode: a note with no key held jumps.
    assert_eq!(t[30_000 / SEG + 1].1[0], 48.0);
    // Always: it glides from the previous note, in poly mode too (a fresh voice starts at the old pitch).
    let p = with_glide(gate_patch(), 100.0, GlideMode::Always);
    let ev = notes_events(&[(60, 0, 3200), (72, 9600, 20_000)]);
    let (_, t) = trace(&p, &ev, 20_000);
    let first = &t[9600 / SEG].1;
    assert_eq!(first.len(), 1);
    assert!((first[0] - (60.0 + 12.0 / 150.0)).abs() < 1e-4, "{first:?}");
}

#[test]
fn voice_ends_and_cuts_are_reported_for_note_end() {
    let mut p = gate_patch();
    p.voice.values[VoiceParam::Polyphony.index()] = 1.0;
    let ev = notes_events(&[(60, 0, 1000), (62, 2000, 3000), (64, 2500, 4000)]);
    let mut e = engine(&p, SEED);
    let mut ends = Vec::new();
    drive(&mut e, &ev, 8000, 100, |e, _, _| {
        ends.extend(e.note_ends().iter().map(|n| (n.key.key, n.frame)));
    });
    // The gate envelope's release lasts the 1 ms minimum ramp, rounded up to whole segments (lane D2).
    let release = u64::from(enth_engine::modules::adsr::min_ramp_steps(48_000.0)) * SEG as u64;
    // 60: its note-off lands at 1024 (the grid) and the gate envelope is silent `release` frames later.
    // 62 is stolen at 2528 by 64 and faded out by 2592. 64: note-off at 4000 (on the grid), silent `release` later.
    assert_eq!(
        ends,
        [(60, 1024 + release), (62, 2528 + 64), (64, 4000 + release)]
    );
}

#[test]
fn a_choke_fades_the_note_in_64_frames() {
    let p = gate_patch();
    let mut ev = notes_events(&[(60, 0, 48_000), (64, 0, 48_000)]);
    ev.push((
        3200,
        EventKind::Choke {
            key: NoteKey {
                note_id: -1,
                ..NoteKey::new(60, 0)
            },
        },
    ));
    ev.sort_by_key(|(f, _)| *f);
    let mut e = engine(&p, SEED);
    let (l, _) = drive(&mut e, &ev, 6400, 256, |_, _, _| {});
    let (l64, _) = drive(
        &mut engine(&p, SEED),
        &notes_events(&[(64, 0, 48_000)]),
        6400,
        256,
        |_, _, _| {},
    );
    assert_eq!(e.active_voices(), &[1]);
    assert_eq!(e.stats().stolen_voices, 0, "a choke is not a steal");
    assert_eq!(bits(&l[3264..]), bits(&l64[3264..]));
}

#[test]
fn routes_from_velocity_key_macros_and_the_mod_wheel() {
    let notes = [note(60, 0, 48_000)];
    let ev = common::events(&notes); // velocity 0.8
    let render = |p: &Patch, extra: &[(u64, EventKind)]| {
        let mut all = ev.clone();
        all.extend_from_slice(extra);
        all.sort_by_key(|(f, _)| *f);
        drive(&mut engine(p, SEED), &all, 9600, 512, |_, _, _| {})
    };
    // velocity: level 50 % + 50 % of the range · 0.8 = 90 %.
    let mut by_velocity = gate_patch();
    by_velocity.generators[0].values[OscParam::Level.index()] = 50.0;
    route(&mut by_velocity, "velocity", "osc1.level", 50.0);
    let mut at_90 = gate_patch();
    at_90.generators[0].values[OscParam::Level.index()] = 90.0;
    let (a, _) = render(&by_velocity, &[]);
    let (b, _) = render(&at_90, &[]);
    let diff = a
        .iter()
        .zip(&b)
        .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
    assert!(diff < 1e-5, "{diff}");
    // key: (60 − 60)/120 = 0 at C4; at C9 (120) +0.5 · 100 % of the pan range: hard right.
    let mut by_key = gate_patch();
    route(&mut by_key, "key", "voice.pan", 100.0);
    let hi = [note(120, 0, 48_000)];
    let (l, r) = drive(
        &mut engine(&by_key, SEED),
        &common::events(&hi),
        4096,
        512,
        |_, _, _| {},
    );
    assert!(l.iter().all(|x| *x == 0.0) && r.iter().any(|x| *x != 0.0));
    // macro1 (patch value 0.5) on semi at 12.5 %: +6 semitones, like semi = 6.
    let mut by_macro = gate_patch();
    by_macro.macros[0].value = 0.5;
    route(&mut by_macro, "macro1", "osc1.semi", 12.5);
    let mut at_6 = gate_patch();
    at_6.generators[0].values[OscParam::Semi.index()] = 6.0;
    let (a, _) = render(&by_macro, &[]);
    let (b, _) = render(&at_6, &[]);
    assert_eq!(bits(&a), bits(&b));
    // The mod wheel and a macro event: a route at −100 % on the level silences the voice after the 10 ms ramp.
    for (src, event) in [
        ("modwheel", EventKind::ModWheel { value: 1.0 }),
        (
            "macro8",
            EventKind::Macro {
                index: 7,
                value: 1.0,
            },
        ),
    ] {
        let mut p = gate_patch();
        route(&mut p, src, "osc1.level", -100.0);
        let (l, _) = render(&p, &[(4800, event)]);
        let loud = l[4000..4800].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let ramp = &l[4800..4800 + 480];
        let after = l[4800 + 512..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(loud > 0.3, "{src}");
        assert!(
            ramp.iter().any(|x| x.abs() > 0.05),
            "{src}: smoothed, not a step"
        );
        assert_eq!(after, 0.0, "{src}: silent once the ramp is over");
    }
}

#[test]
fn a_route_whose_source_rests_at_zero_changes_nothing_here() {
    // osc1.fine is Linear −100..100 ct at 0: routed, it is from_norm(to_norm(0) + 0) = from_norm(0.5) = 0 exactly,
    // and every unrouted parameter keeps its exact plain value, so the bits are those of the unrouted patch.
    let mut p = gate_patch();
    route(&mut p, "modwheel", "osc1.fine", 50.0);
    let notes = [note(60, 0, 48_000)];
    let (a, _) = drive(
        &mut engine(&p, SEED),
        &common::events(&notes),
        4096,
        512,
        |_, _, _| {},
    );
    let (b, _) = drive(
        &mut engine(&gate_patch(), SEED),
        &common::events(&notes),
        4096,
        512,
        |_, _, _| {},
    );
    assert_eq!(bits(&a), bits(&b));
}

/// Drive with a sequence of different block sizes, one per call; both channels as bits.
fn render_varying(
    patch: &Patch,
    ev: &[(u64, EventKind)],
    frames: usize,
    sizes: &[usize],
) -> Vec<u32> {
    let mut e = engine(patch, SEED);
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let (mut start, mut next, mut k) = (0usize, 0usize, 0usize);
    while start < frames {
        let end = (start + sizes[k % sizes.len()].max(1)).min(frames);
        k += 1;
        let mut chunk = Vec::new();
        while let Some(&(f, kind)) = ev.get(next) {
            if f >= end as u64 {
                break;
            }
            chunk.push(Event {
                offset: (f - start as u64) as u32,
                kind,
            });
            next += 1;
        }
        e.process(&mut left[start..end], &mut right[start..end], &chunk);
        start = end;
    }
    left.iter().chain(&right).map(|x| x.to_bits()).collect()
}

/// A patch with routes from every built-in source, glide, and three voices (so steals happen).
fn modulated_patch(mode: VoiceMode) -> Patch {
    let mut p = with_glide(with_mode(reset_saw_patch(), mode), 30.0, GlideMode::Always);
    p.voice.values[VoiceParam::Polyphony.index()] = 3.0;
    p.macros[0].value = 0.25;
    route(&mut p, "velocity", "osc1.level", -40.0);
    route(&mut p, "key", "voice.pan", 80.0);
    route(&mut p, "macro1", "osc1.fine", 30.0);
    route(&mut p, "modwheel", "osc1.semi", 10.0);
    route(&mut p, "env1", "env1.sustain", -20.0);
    p
}

proptest! {
    /// Random notes, mod-wheel, macro and automation events and a random sequence of host block sizes give the same
    /// bits as 32-frame blocks, with routes, smoothing, glide and steals, in every voice mode.
    #[test]
    fn modulation_and_glide_do_not_depend_on_the_host_block_size(
        raw in prop::collection::vec((0u64..6000, 0u64..3000, 36u8..96, 0.0f32..1.0), 1..10),
        wheel in prop::collection::vec((0u64..8000, 0.0f32..1.0), 0..4),
        sizes in prop::collection::vec(1usize..700, 1..8),
        mode in 0usize..3,
    ) {
        let mode = [VoiceMode::Poly, VoiceMode::Mono, VoiceMode::Legato][mode];
        let patch = modulated_patch(mode);
        let (_, map) = compile(&patch, &PrepareCfg::new(SR)).unwrap();
        let level = map.get(&"osc1.level".parse().unwrap()).unwrap();
        assert!(matches!(level, ParamId::Voice(_)));
        let mut ev = Vec::new();
        for (i, &(on, len, key, vel)) in raw.iter().enumerate() {
            let k = NoteKey::new(key, i as i32);
            ev.push((on, EventKind::NoteOn { key: k, velocity: vel }));
            ev.push((on + len, EventKind::NoteOff { key: k, velocity: 0.0 }));
        }
        for (i, &(f, v)) in wheel.iter().enumerate() {
            ev.push((f, EventKind::ModWheel { value: v }));
            ev.push((f + 7, EventKind::Macro { index: (i % 8) as u8, value: 1.0 - v }));
            ev.push((f + 11, EventKind::Param { id: level, value: 100.0 * v }));
        }
        ev.sort_by_key(|(f, _)| *f);
        let reference = render_varying(&patch, &ev, 9000, &[SEG]);
        prop_assert_eq!(render_varying(&patch, &ev, 9000, &sizes), reference);
    }
}

#[test]
fn a_40_s_tail_ends_with_no_voice_in_every_mode() {
    for mode in [VoiceMode::Poly, VoiceMode::Mono, VoiceMode::Legato] {
        let mut p = with_glide(
            with_mode(every_kind_patch(3000.0), mode),
            50.0,
            GlideMode::Always,
        );
        p.voice.values[VoiceParam::Polyphony.index()] = 2.0;
        route(&mut p, "velocity", "filter1.cutoff", 30.0);
        route(&mut p, "lfo1", "osc1.fine", 10.0);
        route(&mut p, "lfo2", "reverb1.wet", 10.0);
        let notes: Vec<_> = (0..12)
            .map(|i| note(48 + (i * 5 % 24) as u8, i * 2000, i * 2000 + 7000))
            .collect();
        let mut e = engine(&p, SEED);
        let mut subnormal_out = 0;
        let mut worst_state = 0;
        drive(
            &mut e,
            &common::events(&notes),
            40 * 48_000,
            4096,
            |e, l, r| {
                subnormal_out += l.iter().chain(r).filter(|x| x.is_subnormal()).count();
                worst_state = worst_state.max(e.subnormals());
            },
        );
        assert_eq!(e.active_voices().len(), 0, "{mode:?}: voices left");
        assert_eq!(subnormal_out, 0, "{mode:?}");
        assert_eq!(worst_state, 0, "{mode:?}");
        if mode == VoiceMode::Poly {
            assert!(e.stats().stolen_voices > 0, "the phrase steals");
        }
    }
}
