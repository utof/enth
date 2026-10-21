//! Lane F1S: the seam between FE's intake and `create` (the F1s judge's M3 and its checklist), and the judge's FE
//! probes as regressions (`probe_fe_limits`, `probe_fe_held_pitched` in
//! `~/code/enth-scratch/r23/f1s-judge/probe/judge_probe.rs`): a held noise-like sound keeps its body (M4), the engine's
//! square is not read as clipped (should-fix 1), a held pad raises no phrase warning (should-fix 13).

mod create_support;

use create_support::{load, set, target_patch};
use enth_analysis::intake::trim::SEGMENT_FRAMES;
use enth_analysis::intake::{FloorSource, Intake, Warning};
use enth_patch::Patch;
use enth_render::{DEFAULT_SEED, Note, Performance, RenderSpec, render};
use enth_solver::create::{SCORE_S, Target};

const SR: u32 = 48_000;

fn mid(p: &Patch, perf: &Performance, sr: u32) -> Vec<f32> {
    render(p, perf, &RenderSpec::new(sr, DEFAULT_SEED))
        .unwrap()
        .mid()
}

fn held(key: u8, on_s: f64, duration_s: f64) -> Performance {
    Performance {
        notes: vec![Note::new(key, 0.8, on_s, 10.0)],
        duration_s,
        ..Performance::default()
    }
}

/// The first sample whose magnitude is above zero.
fn first_sound(x: &[f32]) -> usize {
    x.iter().position(|v| *v != 0.0).unwrap()
}

/// `trim::SEGMENT_FRAMES` is the engine's segment: a note sounds from the first multiple of it at or after its frame
/// (the render grid of docs/design.md §5), so a pre-roll of whole segments puts a candidate's note-on on the grid.
#[test]
fn the_intakes_segment_is_the_engines() {
    let p = target_patch();
    let at = |frame: usize| {
        let perf = Performance {
            notes: vec![Note::new(69, 0.8, frame as f64 / f64::from(SR), 0.2)],
            duration_s: 0.25,
            ..Performance::default()
        };
        first_sound(&mid(&p, &perf, SR))
    };
    let s = SEGMENT_FRAMES;
    let base = at(s);
    assert_eq!(
        at(1),
        base,
        "a note at frame 1 waits for the first boundary"
    );
    assert_eq!(at(s - 1), base);
    assert_eq!(at(s + 1), base + s, "one frame later is one segment later");
    assert_eq!(at(2 * s), base + s);
    assert_eq!(at(5 * s), base + 4 * s);
}

/// The intake of an engine render, mapped by `Target::from_intake` and rendered again with the same patch, matches
/// the clip sample for sample from its first sample to the note-off: the onset sits on the render grid and the
/// performance plays it there, whatever the attack. (The note-off is FE's inference, `infer_note_off`, and lands
/// within a few ms of the truth, so the release need not match sample for sample.)
#[test]
fn a_rendered_note_round_trips_sample_aligned() {
    let slow = {
        let mut p = target_patch();
        set(&mut p, "env1.attack", 120.0);
        p
    };
    for (sr, lead_segments, key, p) in [
        (SR, 150usize, 57u8, target_patch()),
        (44_100, 77, 64, target_patch()),
        (22_050, 33, 45, target_patch()),
        (SR, 151, 50, slow.clone()),
        (44_100, 3, 69, slow),
    ] {
        let on = (lead_segments * SEGMENT_FRAMES) as f64 / f64::from(sr);
        let perf = Performance {
            notes: vec![Note::new(key, 100.0 / 127.0, on, on + 0.4)],
            duration_s: on + 1.2,
            ..Performance::default()
        };
        let file = mid(&p, &perf, sr);
        let it = Intake::new(&file, 1, sr).unwrap();
        let t = Target::from_intake(&it, DEFAULT_SEED);
        assert_eq!(t.perf.key, key, "{sr} Hz");
        let on_frame = (t.perf.on_s * f64::from(sr)).round() as usize;
        assert_eq!(
            on_frame % SEGMENT_FRAMES,
            0,
            "{sr} Hz: the note-on is on the grid"
        );
        let again = render(&p, &t.performance(), &t.render_spec())
            .unwrap()
            .mid();
        assert_eq!(again.len(), t.audio.len(), "{sr} Hz");
        // From the clip's first sample to the note-off: the same samples.
        let off = on_frame + (0.4 * f64::from(sr)) as usize;
        let differ = (0..off).find(|&i| again[i] != t.audio[i]);
        assert_eq!(differ, None, "{sr} Hz, key {key}: first difference");
        assert_eq!(first_sound(&again), first_sound(&t.audio), "{sr} Hz");
        assert!(
            (t.perf.hold_s - 0.4).abs() < 0.03,
            "{sr} Hz, key {key}: hold {}",
            t.perf.hold_s
        );
    }
}

/// `from_intake` per the judge's checklist: the clip cut to the scored window, the rate, the seed, the performance
/// (note-on in clip time, the hold, the window's length), the bandwidth, and the floor only when confirmed.
#[test]
fn from_intake_maps_the_report_and_scores_three_seconds() {
    let p = target_patch();
    // A long held note: the intake keeps 5 s, the target scores 3.
    let file = mid(&p, &held(52, 0.2, 7.0), SR);
    let it = Intake::new(&file, 1, SR).unwrap();
    let r = &it.report;
    assert!(
        it.audio.len() > (4.9 * f64::from(SR)) as usize,
        "the intake keeps 5 s"
    );
    let t = Target::from_intake(&it, 7);
    assert!(
        (SCORE_S - 3.0).abs() < 1e-12,
        "the coordinator's 3 s window"
    );
    assert_eq!(t.audio.len(), (SCORE_S * f64::from(SR)).round() as usize);
    assert_eq!(t.audio[..], it.audio[..t.audio.len()]);
    assert_eq!(
        (t.perf.duration_s * f64::from(SR)).round() as usize,
        t.audio.len()
    );
    assert_eq!(t.sample_rate, r.sample_rate);
    assert_eq!(t.render_seed, 7);
    assert_eq!(
        (
            t.perf.key,
            t.perf.velocity,
            t.perf.on_s,
            t.perf.hold_s,
            t.perf.f0_hz
        ),
        (
            r.perf.key,
            r.perf.velocity,
            r.perf.on_s,
            r.perf.hold_s,
            r.perf.f0_hz
        )
    );
    assert_eq!(t.bandwidth_hz, r.degradation.bandwidth_hz);
    assert!(
        !t.perf.release_heard(),
        "held past the window: no release to search"
    );
    let rendered = render(&p, &t.performance(), &t.render_spec()).unwrap();
    assert_eq!(
        rendered.frames(),
        t.audio.len(),
        "a candidate renders the window exactly"
    );
    // Another window (the CLI's --score-seconds), and a clip shorter than the window, scored whole.
    let five = Target::from_intake_scored(&it, 7, 5.0);
    assert_eq!(five.audio.len(), it.audio.len().min(5 * SR as usize));
    let short = mid(&p, &Performance::note(60, 0.8, 0.3, 0.9), SR);
    let it = Intake::new(&short, 1, SR).unwrap();
    let t = Target::from_intake(&it, 7);
    assert_eq!(t.audio, it.audio);
    assert_eq!(t.perf.duration_s, it.report.perf.duration_s);
    assert!(t.perf.release_heard());

    // The floor goes to the target only when confirmed: a pluck in white noise with a noise lead-in.
    let mut noisy = vec![0.0f32; SR as usize / 2];
    noisy.extend(mid(&p, &Performance::note(57, 0.8, 0.3, 1.5), SR));
    let mut state = 0x2545_f491_4f6c_dd1du64;
    for v in &mut noisy {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *v += 0.003 * ((state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32;
    }
    let it = Intake::new(&noisy, 1, SR).unwrap();
    let d = &it.report.degradation;
    assert!(d.floor_confirmed, "{d:?}");
    assert!(matches!(
        d.floor_source,
        Some(FloorSource::LeadIn | FloorSource::Both)
    ));
    assert_eq!(
        Target::from_intake(&it, 1).noise_floor_dbfs,
        d.noise_floor_dbfs
    );
}

/// The F1s judge's M4: white and pink noise decaying to a 10–40 % sustain held to the end of a 3 s file was cut to its
/// attack (a floor of −15 to −37 dBFS read from the sustain, a clip of 0.15–0.21 s, a hold of 0.05–0.11 s). The quiet
/// stretch alone no longer moves the trim: the floor is reported unconfirmed, and the body is kept.
#[test]
fn a_held_noise_keeps_its_body() {
    let noise = r#"{ "schema": 1, "meta": { "name": "n" }, "voice": { "amp_env": "env1" },
      "generators": [ { "id": "noise1", "kind": "noise", "params": { "color": "white" } } ],
      "modulators": [ { "id": "env1", "kind": "adsr" } ] }"#;
    for (sustain, color) in [(25.0f32, 0.0f32), (40.0, 0.0), (25.0, 1.0), (10.0, 1.0)] {
        let mut p = load(noise);
        set(&mut p, "noise1.color", color);
        set(&mut p, "env1.attack", 3.0);
        set(&mut p, "env1.decay", 250.0);
        set(&mut p, "env1.sustain", sustain);
        let it = Intake::new(&mid(&p, &held(60, 0.0, 3.0), SR), 1, SR).unwrap();
        let r = &it.report;
        let d = &r.degradation;
        let case = format!("sustain {sustain} color {color}: {r:?}");
        assert!(r.perf.hold_s >= 2.5, "{case}");
        assert!(r.perf.duration_s >= 2.9, "{case}");
        assert!(!d.floor_confirmed && d.snr_db.is_none(), "{case}");
        if d.noise_floor_dbfs.is_some() {
            assert_eq!(d.floor_source, Some(FloorSource::Quiet), "{case}");
        }
        let t = Target::from_intake(&it, DEFAULT_SEED);
        assert_eq!(t.noise_floor_dbfs, None, "{case}");
        assert!(!t.perf.release_heard(), "{case}");
    }
}

/// A pitched note decaying to a held sustain, its pitch within 8 % of a mains harmonic (so the floor reader's mains
/// tolerance lets its sustain pass as a background), with no lead-in: the floor is not confirmed, because the
/// sustain is the body's own note, and the body is kept. A pluck that decays into white noise with no lead-in has a
/// floor confirmed by its note, and its noisy tail is trimmed.
#[test]
fn a_held_note_near_a_mains_harmonic_keeps_its_body_and_a_pluck_in_noise_is_trimmed() {
    let base = load(include_str!("../../../content/templates/init.patch.json"));
    for key in [43u8, 50, 55] {
        let mut p = base.clone();
        set(&mut p, "env1.attack", 3.0);
        set(&mut p, "env1.decay", 300.0);
        set(&mut p, "env1.sustain", 30.0);
        let it = Intake::new(&mid(&p, &held(key, 0.0, 3.0), SR), 1, SR).unwrap();
        let r = &it.report;
        assert!(r.perf.hold_s >= 2.9, "key {key}: {r:?}");
        assert!(!r.degradation.floor_confirmed, "key {key}: {r:?}");
    }
    let mut x = mid(&target_patch(), &Performance::note(57, 0.8, 0.3, 2.5), SR);
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for v in &mut x {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *v += 0.002 * ((state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32;
    }
    let it = Intake::new(&x, 1, SR).unwrap();
    let d = &it.report.degradation;
    assert_eq!(d.floor_source, Some(FloorSource::Quiet), "{d:?}");
    assert!(d.floor_confirmed && d.snr_db.is_some(), "{d:?}");
    assert!(it.report.perf.duration_s < 1.5, "{:?}", it.report.perf);
}

/// Should-fix 1: the engine's unfiltered pulse at 50 % (a square) has flat tops, and was read as clipped at a share of
/// 0.81–0.84. A saw driven into a hard clip still is.
#[test]
fn the_engines_square_is_not_clipped_and_a_clipped_saw_is() {
    let base = load(include_str!("../../../content/templates/init.patch.json"));
    for key in [33u8, 45, 57, 69, 81] {
        let mut p = base.clone();
        set(&mut p, "osc1.wave", 3.0);
        set(&mut p, "env1.attack", 2.0);
        set(&mut p, "env1.sustain", 100.0);
        set(&mut p, "env1.release", 100.0);
        let it = Intake::new(&mid(&p, &Performance::note(key, 0.8, 1.0, 1.5), SR), 1, SR).unwrap();
        assert_eq!(it.report.degradation.clipping, None, "square at key {key}");
    }
    for key in [33u8, 45, 57] {
        let mut p = base.clone();
        set(&mut p, "env1.sustain", 100.0);
        let x = mid(&p, &Performance::note(key, 0.8, 1.0, 1.5), SR);
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let clipped: Vec<f32> = x
            .iter()
            .map(|v| (4.0 * v / peak).clamp(-1.0, 1.0))
            .collect();
        let it = Intake::new(&clipped, 1, SR).unwrap();
        let c = it.report.degradation.clipping.expect("a clipped saw");
        assert!(c.share > 0.2, "saw at key {key}: {c:?}");
    }
}

/// Pitched held sounds to the end of the file keep their whole length and raise no floor; should-fix 13: the pad held
/// at C5 raised a spurious `Phrase { onsets: 2 }` (its slow swell and its LFO opening the filter).
#[test]
fn a_held_pad_keeps_its_length_and_is_one_note() {
    let pad = load(include_str!(
        "../../../content/templates/pad_2osc.patch.json"
    ));
    for (key, seed) in [(36u8, 3), (48, 3), (72, 3), (72, DEFAULT_SEED), (84, 3)] {
        let x = render(&pad, &held(key, 0.0, 3.0), &RenderSpec::new(SR, seed))
            .unwrap()
            .mid();
        let it = Intake::new(&x, 1, SR).unwrap();
        let r = &it.report;
        assert_eq!(r.degradation.noise_floor_dbfs, None, "key {key}: {r:?}");
        assert!(r.perf.hold_s > 2.9, "key {key}: {r:?}");
        assert!(
            !r.warnings
                .iter()
                .any(|w| matches!(w, Warning::Phrase { .. })),
            "key {key}: {:?} (onsets {:?})",
            r.warnings,
            r.perf.onsets_s
        );
    }
}
