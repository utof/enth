//! The segment grid and the host adapter: event timing, block-size invariance, arrival order, voice end, pan,
//! seeds. These run on the stub modules (the reference saw and envelope), so they test the engine, not the DSP.

mod common;

use common::{SEED, bits, drive, engine, events, note, render, reset_saw_patch, saw_patch};
use enth_engine::{Event, EventKind, NoteKey, ParamId, PrepareCfg, SEG, compile};
use enth_patch::catalogue::{AdsrParam, MasterParam, OscParam, VoiceParam};
use enth_patch::{ParamAddr, Patch};
use proptest::prelude::*;

fn first_nonzero(x: &[f32]) -> Option<usize> {
    x.iter().position(|v| *v != 0.0)
}

#[test]
fn an_event_takes_effect_at_the_first_segment_boundary_at_or_after_its_frame() {
    // Start at 90 degrees, so the first sample of a note is not the saw's zero at phase 0.
    let mut patch = reset_saw_patch();
    patch.generators[0].values[OscParam::Phase.index()] = 90.0;
    for f in [0u64, 1, 31, 32, 33, 40, 63, 64, 100, 1000] {
        let (l, _, _) = render(&patch, &[note(60, f, f + 4800)], 4096, 64, SEED);
        let expected = f.div_ceil(SEG as u64) * SEG as u64;
        assert_eq!(
            first_nonzero(&l),
            Some(expected as usize),
            "note-on at frame {f}"
        );
    }
}

/// Fixed block sizes from the acceptance list, plus awkward ones: all bit-identical.
#[test]
fn the_host_block_size_does_not_change_the_output() {
    let patch = saw_patch();
    let notes = [
        note(60, 0, 9000),
        note(64, 1234, 5000),
        note(67, 1234, 20_001),
        note(72, 7777, 7778),
        note(48, 15_000, 30_000),
    ];
    let frames = 36_000;
    let (rl, rr, _) = render(&patch, &notes, frames, SEG, SEED);
    assert!(rl.iter().any(|x| *x != 0.0));
    for block in [
        1, 2, 3, 7, 31, 33, 48, 63, 64, 65, 100, 441, 512, 1000, 4096, 36_000,
    ] {
        let (l, r, _) = render(&patch, &notes, frames, block, SEED);
        assert_eq!(bits(&l), bits(&rl), "left, block {block}");
        assert_eq!(bits(&r), bits(&rr), "right, block {block}");
    }
}

/// Drive with a sequence of different block sizes, one per call.
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

proptest! {
    /// Random notes (off-grid times, overlaps, zero-length notes) and a random sequence of host block sizes give
    /// the same bits as 32-frame blocks.
    #[test]
    fn random_events_and_block_sequences_match_the_reference(
        raw in prop::collection::vec((0u64..6000, 0u64..3000, 36u8..96), 1..12),
        sizes in prop::collection::vec(1usize..700, 1..8),
    ) {
        let notes: Vec<_> = raw.iter().map(|&(on, len, key)| note(key, on, on + len)).collect();
        let ev = events(&notes);
        let patch = saw_patch();
        let reference = render_varying(&patch, &ev, 8000, &[SEG]);
        prop_assert_eq!(render_varying(&patch, &ev, 8000, &sizes), reference);
    }
}

#[test]
fn same_frame_events_apply_in_arrival_order() {
    // A note-on and its note-off at the same frame: the voice starts and is released at once.
    let (_, _, e) = render(&saw_patch(), &[note(60, 100, 100)], 48_000, 512, SEED);
    assert_eq!(
        e.active_voices(),
        &[] as &[usize],
        "released at once and ended"
    );
    // A parameter event before a note-on at the same frame reaches the note's start: osc1's start phase.
    let patch = reset_saw_patch();
    let (_, map) = compile(&patch, &PrepareCfg::new(48_000.0)).unwrap();
    let phase = map.get(&"osc1.phase".parse().unwrap()).unwrap();
    let key = NoteKey::new(60, 0);
    let on = EventKind::NoteOn { key, velocity: 1.0 };
    let set = EventKind::Param {
        id: phase,
        value: 180.0,
    };
    let run = |ev: &[(u64, EventKind)]| {
        let mut e =
            enth_engine::Engine::new(compile(&patch, &PrepareCfg::new(48_000.0)).unwrap().0, SEED);
        drive(&mut e, ev, 256, 64, |_, _, _| {}).0
    };
    let before = run(&[(0, set), (0, on)]);
    let after = run(&[(0, on), (0, set)]);
    let plain = run(&[(0, on)]);
    assert_ne!(bits(&before), bits(&plain), "the phase event came first");
    assert_eq!(
        bits(&after),
        bits(&plain),
        "the phase event came after the note started"
    );
}

#[test]
fn voices_end_below_minus_100_db_after_their_release() {
    let mut patch = saw_patch();
    patch.modulators[0].values[AdsrParam::Release.index()] = 100.0;
    let notes = [note(60, 0, 4800), note(67, 0, 9600)];
    let (l, _, e) = render(&patch, &notes, 48_000, 512, SEED);
    assert_eq!(e.active_voices(), &[] as &[usize]);
    let last = l.iter().rposition(|x| *x != 0.0).unwrap();
    // 9600 frames held + the release down to -100 dB (100 ms per 60 dB: about 167 ms), on the segment grid.
    assert!(last < 9600 + 9000, "last sound at {last}");
    let tail_peak = l[last.saturating_sub(SEG)..=last]
        .iter()
        .fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(tail_peak < 1e-5, "{tail_peak}");
}

#[test]
fn full_polyphony_steals_the_oldest_voice_with_a_fade() {
    let mut patch = saw_patch();
    patch.voice.values[VoiceParam::Polyphony.index()] = 2.0;
    let notes = [note(60, 0, 4800), note(62, 0, 4800), note(64, 0, 4800)];
    let (_, _, e) = render(&patch, &notes, 2048, 512, SEED);
    let s = e.stats();
    assert_eq!((s.notes, s.dropped_notes, s.stolen_voices), (3, 0, 1));
    // Slot 0 (the oldest) faded out; the third note took the spare slot 2.
    assert_eq!(e.active_voices(), &[1, 2]);
}

#[test]
fn constant_power_pan_minus_3_db_at_centre_and_hard_sides() {
    let notes = [note(60, 0, 4800)];
    let (l, r, _) = render(&saw_patch(), &notes, 4096, 512, SEED);
    assert_eq!(bits(&l), bits(&r), "centre: identical channels");
    let mut left_only = saw_patch();
    left_only.voice.values[VoiceParam::Pan.index()] = -100.0;
    let (l2, r2, _) = render(&left_only, &notes, 4096, 512, SEED);
    assert!(
        r2.iter().all(|x| *x == 0.0),
        "hard left: silent right channel"
    );
    // Centre gain is cos(pi/4) of the hard-left gain (1): -3 dB.
    let i = l.iter().position(|x| x.abs() > 0.1).unwrap();
    let ratio = f64::from(l[i]) / f64::from(l2[i]);
    assert!(
        (ratio - core::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6,
        "{ratio}"
    );
}

#[test]
fn master_level_and_parameter_events() {
    let notes = [note(60, 0, 48_000)];
    let mut quiet = saw_patch();
    quiet.master[MasterParam::Level.index()] = -6.0;
    let (loud, _, _) = render(&saw_patch(), &notes, 4096, 512, SEED);
    let (soft, _, _) = render(&quiet, &notes, 4096, 512, SEED);
    let g = enth_dsp::util::math::db_to_gain(-6.0);
    for (a, b) in loud.iter().zip(&soft).skip(64) {
        assert_eq!((a * g).to_bits(), b.to_bits());
    }
    // The same through a value event at frame 0: master.level is smoothed linearly over 20 ms (30 segments), then
    // it is the exact value again.
    let (_, map) = compile(&saw_patch(), &PrepareCfg::new(48_000.0)).unwrap();
    let id = map.get(&ParamAddr::master(MasterParam::Level)).unwrap();
    assert!(matches!(id, ParamId::Global(_)));
    let mut ev = events(&notes);
    ev.insert(0, (0, EventKind::Param { id, value: -6.0 }));
    let mut e = engine(&saw_patch(), SEED);
    let (via_event, _) = drive(&mut e, &ev, 4096, 512, |_, _, _| {});
    assert_eq!(bits(&via_event[30 * SEG..]), bits(&soft[30 * SEG..]));
    assert_ne!(
        bits(&via_event[..30 * SEG]),
        bits(&soft[..30 * SEG]),
        "a ramp, not a step"
    );
    for (i, ((v, l), s)) in via_event.iter().zip(&loud).zip(&soft).enumerate().skip(64) {
        assert!(
            v.abs() <= l.abs() && v.abs() >= s.abs() - 1e-7,
            "frame {i}: between 0 and -6 dB"
        );
    }
    // Unknown ids and NaN are ignored, never a panic.
    let mut e = engine(&saw_patch(), SEED);
    let junk = [
        (
            0,
            EventKind::Param {
                id: ParamId::Global(999),
                value: 1.0,
            },
        ),
        (
            0,
            EventKind::Param {
                id: ParamId::Voice(999),
                value: 1.0,
            },
        ),
        (
            0,
            EventKind::Param {
                id,
                value: f32::NAN,
            },
        ),
    ];
    let mut all = junk.to_vec();
    all.extend(events(&notes));
    let (l, _) = drive(&mut e, &all, 4096, 512, |_, _, _| {});
    assert_eq!(bits(&l), bits(&loud));
}

#[test]
fn the_render_seed_moves_free_phases_and_nothing_else() {
    let notes = [note(60, 0, 4800), note(64, 0, 4800)];
    let (a, _, _) = render(&saw_patch(), &notes, 4096, 512, SEED);
    let (a2, _, _) = render(&saw_patch(), &notes, 4096, 512, SEED);
    let (b, _, _) = render(&saw_patch(), &notes, 4096, 512, SEED + 1);
    assert_eq!(bits(&a), bits(&a2), "same seed, same bits");
    assert_ne!(bits(&a), bits(&b), "free phases follow the render seed");
    let (c, _, _) = render(&reset_saw_patch(), &notes, 4096, 512, SEED);
    let (d, _, _) = render(&reset_saw_patch(), &notes, 4096, 512, SEED + 1);
    assert_eq!(bits(&c), bits(&d), "reset phases ignore the render seed");
    // Seeded mode: the module's own seed parameter replaces the render seed.
    let mut seeded = saw_patch();
    seeded.generators[0].values[OscParam::PhaseMode.index()] =
        enth_patch::catalogue::PhaseMode::Seeded.index() as f32;
    let (s1, _, _) = render(&seeded, &notes, 4096, 512, SEED);
    let (s2, _, _) = render(&seeded, &notes, 4096, 512, SEED + 1);
    assert_eq!(bits(&s1), bits(&s2), "seeded phases ignore the render seed");
    seeded.generators[0].values[OscParam::Seed.index()] = 9.0;
    let (s3, _, _) = render(&seeded, &notes, 4096, 512, SEED);
    assert_ne!(bits(&s1), bits(&s3), "but follow the seed parameter");
}

#[test]
fn odd_host_calls_never_panic() {
    let mut e = engine(&saw_patch(), SEED);
    let on = Event {
        offset: 10_000, // past the block: taken as its last frame
        kind: EventKind::NoteOn {
            key: NoteKey::new(60, 0),
            velocity: 7.0, // clamped
        },
    };
    let (mut l, mut r) = (vec![1.0; 100], vec![1.0; 60]);
    e.process(&mut l, &mut r, &[on]);
    assert!(
        l[60..].iter().all(|x| *x == 0.0),
        "the longer channel's tail is zeroed"
    );
    e.process(&mut [], &mut [], &[on]); // empty block with events
    let (mut l, mut r) = (vec![0.0; 4096], vec![0.0; 4096]);
    e.process(&mut l, &mut r, &[]);
    assert!(l.iter().all(|x| x.is_finite()));
    assert_eq!(e.active_voices().len(), 2);
}
