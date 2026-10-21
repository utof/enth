//! The intake end to end: FE's acceptance on synthetic signals with known answers (docs/phase-3.md, FE's row): trim
//! within 5 ms, bandwidth within 1/3 octave, the note right on at least 95 % of pitched clips, a chord warning, short
//! clips kept. The toy voice of `test_signals` stands in for the engine (this crate cannot depend on it); lane FE's
//! measurement on the engine's own renders and DEG's chain is in docs/r3.md.

use super::*;
use crate::test_signals::{Rng, ToyPatch, ToyPerf, Wave, mix, random_toy, render_toy, white_noise};
use crate::units::{hz_to_midi, midi_to_hz};
use realfft::num_complex::Complex32;

const SR: u32 = 48_000;
const SRF: f64 = 48_000.0;

/// A random toy note at a random MIDI key from 28 to 96, and its key.
fn random_note(rng: &mut Rng) -> (Vec<f32>, u8, ToyPatch) {
    let (mut p, mut perf) = random_toy(rng);
    let key = (28.0 + (rng.next_f64() * 69.0).floor()).min(96.0);
    perf.note_hz = midi_to_hz(key);
    p.detune_ct = if rng.next_f64() < 0.5 {
        0.0
    } else {
        rng.uniform(0.0, 25.0)
    };
    // the clip's RMS sets the noise level below, so the level here is free
    let x = render_toy(SRF, &p, &perf, (0.0, 0.37), 0.0);
    (x, key as u8, p)
}

fn rms(x: &[f32]) -> f64 {
    libm::sqrt(x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64)
}

/// `x` after `lead_s` of silence (or of the noise), with white noise at `snr_db` below its RMS (none for `None`).
fn place(x: &[f32], lead_s: f64, snr_db: Option<f64>, seed: u64) -> Vec<f32> {
    let lead = (lead_s * SRF) as usize;
    let mut y = vec![0.0f32; lead];
    y.extend_from_slice(x);
    y.resize(y.len() + (0.3 * SRF) as usize, 0.0);
    match snr_db {
        None => y,
        Some(snr) => {
            // uniform noise in ±a has RMS a/√3
            let a = rms(x) * libm::pow(10.0, -snr / 20.0) * libm::sqrt(3.0);
            mix(&y, &white_noise(SRF, a, y.len() as f64 / SRF, seed))
        }
    }
}

#[test]
fn the_onset_lands_within_5_ms() {
    let mut rng = Rng(0x00FE_0001);
    for (snr, need) in [(None, 1.0), (Some(30.0), 0.95), (Some(20.0), 0.95)] {
        let (mut ok, mut n, mut worst) = (0usize, 0usize, 0.0f64);
        for i in 0..60 {
            let (x, _, p) = random_note(&mut rng);
            // slow attacks under noise surface late by the time they take to clear it; judged on attacks up to 20 ms
            if snr.is_some() && p.attack_ms > 20.0 {
                continue;
            }
            let lead = rng.uniform(0.0, 0.5);
            let y = place(&x, lead, snr, 100 + i);
            let r = Intake::new(&y, 1, SR).unwrap().report;
            let err = (r.span.onset_s - lead).abs();
            worst = worst.max(err);
            n += 1;
            if err <= 0.005 {
                ok += 1;
            }
        }
        let share = ok as f64 / n as f64;
        eprintln!(
            "onset, snr {snr:?}: {ok}/{n} within 5 ms, worst {:.1} ms",
            worst * 1000.0
        );
        assert!(share >= need, "snr {snr:?}: {ok}/{n}, worst {worst}");
    }
}

#[test]
fn the_clip_starts_with_the_preroll_and_the_tail_is_trimmed_in_noise() {
    let p = ToyPatch {
        sustain_db: -60.0,
        decay_ms: 400.0,
        ..ToyPatch::default()
    };
    let x = render_toy(SRF, &p, &ToyPerf::default(), (0.0, 0.37), 0.0);
    let y = place(&x, 0.25, Some(20.0), 3);
    let it = Intake::new(&y, 1, SR).unwrap();
    let r = &it.report;
    assert!((r.perf.on_s - 256.0 / SRF).abs() < 1e-12);
    let onset = (r.span.onset_s * SRF) as usize;
    assert_eq!(&it.audio[256..356], &y[onset..onset + 100]);
    let floor = r.degradation.noise_floor_dbfs.expect("a floor");
    assert!(r.degradation.snr_db.unwrap() > 15.0, "{r:?}");
    // the 2 s clip and its 0.55 s of padding: the pluck has sunk into the noise long before
    assert!(r.span.signal_end_s < 1.2, "{:?}", r.span);
    assert!(r.perf.duration_s < 1.2 && !r.span.capped, "{:?}", r.perf);
    assert!(floor < -20.0);
    // and the clean one keeps its whole audible decay
    let c = Intake::new(&place(&x, 0.25, None, 3), 1, SR)
        .unwrap()
        .report;
    assert!(
        c.degradation.noise_floor_dbfs.is_none() && c.span.signal_end_s > r.span.signal_end_s,
        "{c:?}"
    );
}

#[test]
fn the_note_is_right_on_95_percent_of_pitched_clips() {
    let mut rng = Rng(0x00FE_0002);
    for snr in [None, Some(20.0)] {
        let (mut ok, mut n) = (0usize, 0usize);
        let mut misses = Vec::new();
        for i in 0..100 {
            let (x, key, _) = random_note(&mut rng);
            let y = place(&x, 0.05, snr, 500 + i);
            let r = Intake::new(&y, 1, SR).unwrap().report;
            n += 1;
            if r.perf.key == key {
                ok += 1;
            } else {
                misses.push((key, r.perf.key, r.perf.f0_hz.map(hz_to_midi)));
            }
        }
        let share = ok as f64 / n as f64;
        eprintln!("note, snr {snr:?}: {ok}/{n}; misses (true, got, midi) {misses:?}");
        assert!(share >= 0.95, "snr {snr:?}: {ok}/{n}: {misses:?}");
    }
}

#[test]
fn chords_warn_and_single_notes_do_not() {
    let note = |wave, midi: f64| {
        let p = ToyPatch {
            wave,
            cutoff_hz: 4000.0,
            ..ToyPatch::default()
        };
        let perf = ToyPerf {
            note_hz: midi_to_hz(midi),
            ..ToyPerf::default()
        };
        render_toy(SRF, &p, &perf, (0.0, 0.37), 0.0)
    };
    let mut warned = 0;
    let mut total = 0;
    for wave in Wave::ALL {
        for root in [43.0, 50.0, 57.0, 64.0] {
            for shape in [
                &[0.0, 4.0, 7.0][..],
                &[0.0, 3.0, 7.0],
                &[0.0, 4.0, 7.0, 11.0],
                &[0.0, 3.0, 6.0],
            ] {
                let mut x: Vec<f32> = Vec::new();
                for s in shape {
                    x = mix(&x, &note(wave, root + s));
                }
                let r = Intake::new(&x, 1, SR).unwrap().report;
                total += 1;
                if r.warnings
                    .iter()
                    .any(|w| matches!(w, Warning::Chord { .. }))
                {
                    warned += 1;
                }
            }
            let single = Intake::new(&note(wave, root), 1, SR).unwrap().report;
            assert!(
                !single
                    .warnings
                    .iter()
                    .any(|w| matches!(w, Warning::Chord { .. })),
                "{wave:?} {root}: {:?}",
                single.warnings
            );
        }
    }
    eprintln!("chords warned: {warned}/{total}");
    assert!(
        f64::from(warned) >= 0.9 * f64::from(total),
        "{warned}/{total}"
    );
}

#[test]
fn short_sounds_are_kept() {
    for ms in [0.5, 2.0, 10.0, 40.0] {
        let n = ((ms / 1000.0) * SRF).max(1.0) as usize;
        let x: Vec<f32> = (0..n)
            .map(|i| 0.5 * libm::sinf(i as f32 * 0.3 + 0.5))
            .collect();
        // alone in the file, and inside a longer silent file
        for lead in [0usize, 4800] {
            let mut y = vec![0.0f32; lead];
            y.extend_from_slice(&x);
            let it = Intake::new(&y, 1, SR).unwrap_or_else(|e| panic!("{ms} ms: {e}"));
            assert!(it.audio.len() >= 4800, "{ms} ms");
            let on = 256;
            assert_eq!(&it.audio[on..on + n], &x[..], "{ms} ms, lead {lead}");
            assert!(it.report.perf.hold_s > 0.0 && it.report.perf.duration_s >= 0.1);
        }
    }
}

#[test]
fn a_codec_cut_off_is_reported_and_a_low_rate_is_not() {
    use crate::fft::FftPlanner;
    let p = ToyPatch {
        cutoff_hz: 16_000.0,
        ..ToyPatch::default()
    };
    let x = render_toy(SRF, &p, &ToyPerf::default(), (0.0, 0.37), 0.0);
    // brick-wall at 11 kHz over the whole clip
    let n = x.len() & !1;
    let mut pl = FftPlanner::new(FftPath::Scalar);
    let (fw, inv) = (pl.forward(n), pl.inverse(n));
    let mut buf = x[..n].to_vec();
    let mut spec = vec![Complex32::new(0.0, 0.0); fw.bins()];
    fw.forward(&mut buf, &mut spec, &mut fw.make_scratch());
    let cut = (11_000.0 / (SRF / n as f64)) as usize;
    for c in spec.iter_mut().skip(cut) {
        *c = Complex32::new(0.0, 0.0);
    }
    let mut y = vec![0.0; n];
    inv.inverse(&mut spec, &mut y, &mut inv.make_scratch());
    let y: Vec<f32> = y.iter().map(|v| v / n as f32).collect();
    let got = Intake::new(&y, 1, SR)
        .unwrap()
        .report
        .degradation
        .bandwidth_hz
        .unwrap();
    assert!(libm::log2(got / 11_000.0).abs() < 1.0 / 3.0, "{got}");
    assert!(
        Intake::new(&x, 1, SR)
            .unwrap()
            .report
            .degradation
            .bandwidth_hz
            .is_none()
    );
    // the same voice rendered at 16 kHz: its Nyquist is its band, not a cut-off
    let z = render_toy(16_000.0, &p, &ToyPerf::default(), (0.0, 0.37), 0.0);
    let r = Intake::new(&z, 1, 16_000).unwrap().report;
    assert_eq!(r.sample_rate, 16_000);
    assert!(r.degradation.bandwidth_hz.is_none(), "{:?}", r.degradation);
}

#[test]
fn stereo_is_mixed_and_cancellation_is_flagged() {
    let x = render_toy(
        SRF,
        &ToyPatch::default(),
        &ToyPerf::default(),
        (0.0, 0.37),
        0.0,
    );
    let same: Vec<f32> = x.iter().flat_map(|v| [*v, *v]).collect();
    let r = Intake::new(&same, 2, SR).unwrap();
    assert_eq!(r.report.channels, 2);
    assert!(
        r.report
            .warnings
            .iter()
            .all(|w| !matches!(w, Warning::ChannelsCancel { .. }))
    );
    let flipped: Vec<f32> = x.iter().flat_map(|v| [*v, -0.9 * *v]).collect();
    let r = Intake::new(&flipped, 2, SR).unwrap();
    assert!(
        r.report
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::ChannelsCancel { loss_db } if *loss_db > 6.0)),
        "{:?}",
        r.report.warnings
    );
}

#[test]
fn refusals_name_their_reason() {
    assert_eq!(Intake::new(&[], 1, SR), Err(IntakeError::Empty));
    assert_eq!(Intake::new(&[0.1], 0, SR), Err(IntakeError::NoChannels));
    assert_eq!(
        Intake::new(&[0.1], 1, 4_000),
        Err(IntakeError::SampleRate(4_000))
    );
    assert_eq!(
        Intake::new(&[0.1], 1, 192_000),
        Err(IntakeError::SampleRate(192_000))
    );
    assert_eq!(
        Intake::new(&[0.1, f32::NAN], 1, SR),
        Err(IntakeError::NonFinite { index: 1 })
    );
    assert_eq!(
        Intake::new(&vec![1e-6; 4800], 1, SR),
        Err(IntakeError::Silent)
    );
    let bad = IntakeCfg {
        max_len_s: 0.2,
        ..IntakeCfg::default()
    };
    assert!(matches!(
        Intake::with_cfg(&[0.1], 1, SR, &bad),
        Err(IntakeError::BadConfig(_))
    ));
    // one quiet sample is a sound
    assert!(Intake::new(&[1e-3], 1, SR).is_ok());
}

#[test]
fn a_long_file_is_capped_and_says_so() {
    let p = ToyPatch {
        sustain_db: -3.0,
        ..ToyPatch::default()
    };
    let perf = ToyPerf {
        dur_s: 9.0,
        hold_s: 8.5,
        ..ToyPerf::default()
    };
    let x = render_toy(SRF, &p, &perf, (0.0, 0.37), 0.0);
    let r = Intake::new(&x, 1, SR).unwrap();
    assert_eq!(r.audio.len(), 5 * SR as usize);
    assert!(r.report.span.capped);
    assert!(
        r.report
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::Capped { .. }))
    );
    // a held note cut by the cap has no note-off: it is held to the end
    assert!(
        r.report.perf.off_s.is_none() && r.report.perf.hold_s > 4.9,
        "{:?}",
        r.report.perf
    );
}

/// The report's shape is emitted (`enth.create/1` embeds it): a fixture pins its field names, tags and whole numbers.
/// Floats are masked, so the fixture does not pin their last bits (the tests above check the values). This crate has
/// no JSON reader (`serde_json` is not among its dependencies, and the lock file is the coordinator's), so the read-back
/// is the consumer's test (`enth.create/1`, lane CLC).
#[test]
fn the_report_shape_is_pinned() {
    let p = ToyPatch {
        sustain_db: -12.0,
        ..ToyPatch::default()
    };
    let x = render_toy(SRF, &p, &ToyPerf::default(), (0.0, 0.37), 0.0);
    let mut y = place(&x, 0.1, Some(30.0), 11);
    for v in &mut y {
        *v = v.clamp(-0.4, 0.4);
    }
    let mut two: Vec<f32> = y.iter().flat_map(|v| [*v, -0.9 * *v]).collect();
    two.truncate(2 * (6.0 * SRF) as usize);
    let r = Intake::new(&y, 1, SR).unwrap().report;
    assert!(
        r.degradation.noise_floor_dbfs.is_some() && r.degradation.clipping.is_some(),
        "{r:?}"
    );
    let w = Intake::new(&two, 2, SR).unwrap().report.warnings;
    insta::with_settings!({filters => vec![(r"-?\d+\.\d+(e-?\d+)?", "[f]")]}, {
        insta::assert_json_snapshot!("report", r);
        insta::assert_json_snapshot!("warnings", w);
    });
}
