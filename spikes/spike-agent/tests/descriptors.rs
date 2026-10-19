//! Descriptor sanity on synthetic signals whose answers are known: if these drift, the agent loop is measuring
//! the wrong thing (docs/spikes/agent-probe.md, "Why the descriptors are the probe").

use spike_agent::descriptors::{Describer, Descriptors, Want};
use std::f64::consts::PI;

const SR: f64 = 48_000.0;

fn describe(x: &[f32], hold: Option<f64>) -> Descriptors {
    Describer::new(SR as f32).describe(x, hold, Want::ALL)
}

fn sine(freq: f64, amp: f64, seconds: f64) -> Vec<f32> {
    (0..(SR * seconds) as usize)
        .map(|i| (amp * (2.0 * PI * freq * i as f64 / SR).sin()) as f32)
        .collect()
}

/// Deterministic white noise in -1..1 (xorshift).
fn noise(amp: f32, seconds: f64) -> Vec<f32> {
    let mut s = 0x1234_5678_9abc_def0u64;
    (0..(SR * seconds) as usize)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            amp * ((s >> 11) as f64 / (1u64 << 52) as f64 - 1.0) as f32
        })
        .collect()
}

fn cents(a: f64, b: f64) -> f64 {
    1200.0 * (a / b).log2()
}

#[test]
fn sine_pitch_brightness_and_level_are_right() {
    let d = describe(&sine(440.0, 0.5, 1.0), None);
    let f0 = d.f0_hz.expect("a sine has a pitch");
    assert!(cents(f0, 440.0).abs() < 3.0, "f0 {f0}");
    assert_eq!(d.note.as_deref(), Some("A4"));
    assert!(d.stability.unwrap() > 0.98);
    let c = d.centroid_hz.unwrap();
    assert!(
        (c - 440.0).abs() < 25.0,
        "a sine's centroid is its own frequency: {c}"
    );
    assert!(d.noise_ratio.unwrap() < 0.01, "{:?}", d.noise_ratio);
    assert!((d.peak_dbfs + 6.02).abs() < 0.1, "{}", d.peak_dbfs);
    assert!(!d.clipping && !d.silent);
}

#[test]
fn pitch_is_right_across_the_range() {
    for (f, name) in [
        (65.406, "C2"),
        (130.813, "C3"),
        (261.626, "C4"),
        (1046.5, "C6"),
    ] {
        let d = describe(&sine(f, 0.4, 1.0), None);
        let got = d.f0_hz.unwrap_or_else(|| panic!("no pitch for {f}"));
        assert!(cents(got, f).abs() < 5.0, "{f} Hz read as {got}");
        assert_eq!(d.note.as_deref(), Some(name));
    }
}

#[test]
fn loudness_matches_bs1770_for_a_1khz_sine() {
    // A full-scale 997 Hz sine reads -3.01 LUFS, so amplitude 0.1 reads -23.01.
    let d = describe(&sine(997.0, 0.1, 3.0), None);
    let l = d.lufs_i.expect("loud enough to measure");
    assert!((l + 23.0).abs() < 0.3, "{l}");
}

#[test]
fn centroid_of_two_known_sines() {
    let mut x = sine(1000.0, 0.25, 1.0);
    for (a, b) in x.iter_mut().zip(sine(3000.0, 0.25, 1.0)) {
        *a += b;
    }
    let c = describe(&x, None).centroid_hz.unwrap();
    assert!(
        (c - 2000.0).abs() < 60.0,
        "equal amplitudes centre at 2 kHz: {c}"
    );
    let mut y = sine(1000.0, 0.3, 1.0);
    for (a, b) in y.iter_mut().zip(sine(3000.0, 0.15, 1.0)) {
        *a += b;
    }
    let c = describe(&y, None).centroid_hz.unwrap();
    assert!(
        (c - 1666.7).abs() < 60.0,
        "1 : 0.5 amplitudes centre at 1667 Hz: {c}"
    );
}

#[test]
fn noise_is_noisy_and_has_no_pitch() {
    let d = describe(&noise(0.3, 1.0), None);
    assert!(d.noise_ratio.unwrap() > 0.9, "{:?}", d.noise_ratio);
    assert!(
        d.f0_hz.is_none(),
        "white noise has no steady pitch: {:?}",
        d.f0_hz
    );
    let c = d.centroid_hz.unwrap();
    assert!(
        (9_000.0..15_000.0).contains(&c),
        "white noise centres near 12 kHz: {c}"
    );
    // a tone plus a little noise sits in between
    let mut mix = sine(440.0, 0.3, 1.0);
    for (a, b) in mix.iter_mut().zip(noise(0.2, 1.0)) {
        *a += b;
    }
    let m = describe(&mix, None).noise_ratio.unwrap();
    assert!(m > 0.05 && m < 0.9, "{m}");
}

fn ramped_sine(attack_s: f64) -> Vec<f32> {
    let mut x = sine(440.0, 0.5, 1.0);
    for (i, s) in x.iter_mut().enumerate() {
        *s *= (i as f64 / SR / attack_s).min(1.0) as f32;
    }
    x
}

#[test]
fn attack_time_of_a_linear_ramp() {
    // from 1 % to -1 dB (89 %) of a linear ramp is 88 % of its length; the 8 ms window blurs short ramps
    for (ramp, lo, hi) in [(0.2, 165.0, 190.0), (0.1, 80.0, 96.0), (0.03, 22.0, 31.0)] {
        let a = describe(&ramped_sine(ramp), None).attack_ms.unwrap();
        assert!((lo..hi).contains(&a), "ramp {ramp}s read as {a} ms");
    }
    let click = describe(&ramped_sine(0.001), None).attack_ms.unwrap();
    assert!(click < 6.0, "{click}");
}

/// Sine with a linear attack, exponential decay to a sustain level, held until `hold`, then an exponential release.
fn adsr_sine(sustain: f64, tau_decay: f64, hold: f64, tau_release: f64, seconds: f64) -> Vec<f32> {
    let attack = 0.005;
    let mut out = Vec::new();
    let mut level_at_off = 0.0;
    for i in 0..(SR * seconds) as usize {
        let t = i as f64 / SR;
        let env = if t < attack {
            t / attack
        } else if t < hold {
            let e = sustain + (1.0 - sustain) * (-(t - attack) / tau_decay).exp();
            level_at_off = e;
            e
        } else {
            level_at_off * (-(t - hold) / tau_release).exp()
        };
        out.push((0.5 * env * (2.0 * PI * 330.0 * t).sin()) as f32);
    }
    out
}

#[test]
fn sustain_decay_and_release_of_a_known_envelope() {
    // sustain 0.25 = -12.04 dB; decay tau 50 ms -> 90 % of the way down at 2.3 tau = 115 ms; release tau 50 ms ->
    // -40 dB at 4.6 tau = 230 ms
    let x = adsr_sine(0.25, 0.05, 1.0, 0.05, 2.0);
    let d = describe(&x, Some(1.0));
    let s = d.sustain_db.unwrap();
    assert!((s + 12.04).abs() < 0.6, "sustain {s}");
    let dec = d.decay_ms.unwrap();
    assert!((95.0..140.0).contains(&dec), "decay {dec}");
    let rel = d.release_ms.unwrap();
    assert!((205.0..255.0).contains(&rel), "release {rel}");
    assert!(!d.release_truncated);
    let dur = d.duration_ms.unwrap();
    // 1 s held, then 40 dB below the peak from the 0.25 held level is ln(25) = 3.2 tau = 160 ms later
    assert!((1130.0..1200.0).contains(&dur), "duration {dur}");
    assert!(!d.duration_truncated);
}

#[test]
fn a_flat_note_has_no_decay_and_a_pluck_has_no_sustain() {
    let flat = describe(&adsr_sine(1.0, 0.05, 1.0, 0.05, 2.0), Some(1.0));
    assert_eq!(flat.decay_ms, Some(0.0));
    assert!(flat.sustain_db.unwrap() > -1.0);
    let pluck = describe(&adsr_sine(0.0, 0.05, 1.0, 0.05, 2.0), Some(1.0));
    assert!(pluck.sustain_db.unwrap() < -60.0, "{:?}", pluck.sustain_db);
    // already silent at note-off: nothing left to release
    assert_eq!(pluck.release_ms, Some(0.0));
    let dur = pluck.duration_ms.unwrap();
    assert!(
        (200.0..320.0).contains(&dur),
        "a pluck with tau 50 ms is within 40 dB for about 4.6 tau: {dur}"
    );
}

#[test]
fn a_tail_cut_off_by_the_clip_is_reported_as_a_lower_bound() {
    // release tau 0.5 s needs 2.3 s; the clip leaves 0.5 s after note-off
    let x = adsr_sine(1.0, 0.05, 1.0, 0.5, 1.5);
    let d = describe(&x, Some(1.0));
    assert!(d.release_truncated);
    let r = d.release_ms.unwrap();
    assert!(
        (450.0..520.0).contains(&r),
        "the bound is the time left in the clip: {r}"
    );
    assert!(d.duration_truncated);
}

#[test]
fn beating_rate_of_two_close_sines() {
    for (f2, expect) in [(223.0, 3.0), (226.0, 6.0), (221.5, 1.5)] {
        let mut x = sine(220.0, 0.25, 2.5);
        for (a, b) in x.iter_mut().zip(sine(f2, 0.25, 2.5)) {
            *a += b;
        }
        let d = describe(&x, None);
        let hz = d
            .beating_hz
            .unwrap_or_else(|| panic!("no beating found for {f2}"));
        assert!(
            (hz - expect).abs() < 0.3,
            "{f2} Hz: expected {expect}, got {hz}"
        );
        assert!(d.beat_depth_db.unwrap() > 6.0, "{:?}", d.beat_depth_db);
    }
}

#[test]
fn a_steady_single_tone_does_not_beat() {
    let d = describe(&sine(220.0, 0.4, 2.0), None);
    assert!(d.beating_hz.is_none(), "{:?}", d.beating_hz);
    // a plain fade is not beating either
    let mut x = sine(220.0, 0.4, 2.0);
    for (i, s) in x.iter_mut().enumerate() {
        *s *= (-(i as f64) / SR * 1.5).exp() as f32;
    }
    assert!(describe(&x, None).beating_hz.is_none());
}

#[test]
fn a_decay_into_a_sustain_level_is_not_beating() {
    let d = describe(&adsr_sine(0.25, 0.05, 1.0, 0.05, 2.0), Some(1.0));
    assert!(
        d.beating_hz.is_none(),
        "{:?} {:?}",
        d.beating_hz,
        d.beat_depth_db
    );
    let d = describe(&adsr_sine(0.5, 0.2, 1.8, 0.1, 2.5), Some(1.8));
    assert!(
        d.beating_hz.is_none(),
        "{:?} {:?}",
        d.beating_hz,
        d.beat_depth_db
    );
}

#[test]
fn beating_survives_an_envelope() {
    // two sines 4 Hz apart under a decay into a sustain level
    let env = adsr_sine(0.4, 0.15, 2.0, 0.1, 2.5);
    let x: Vec<f32> = env
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let t = i as f64 / SR;
            *e * (0.5 + 0.5 * (2.0 * PI * 4.0 * t).cos()) as f32
        })
        .collect();
    // (amplitude modulation at 4 Hz of a 330 Hz sine = components at 330 and 330 +- 4 Hz)
    let d = describe(&x, Some(2.0));
    let hz = d.beating_hz.expect("the 4 Hz wobble is found");
    assert!((hz - 4.0).abs() < 0.4, "{hz}");
}

#[test]
fn silence_and_tiny_clips_do_not_panic() {
    let d = describe(&vec![0.0; 48_000], Some(0.5));
    assert!(d.silent);
    assert!(d.centroid_hz.is_none() && d.f0_hz.is_none());
    for n in [0usize, 1, 100, 1500] {
        let _ = describe(&vec![0.1; n], Some(0.01));
    }
    let _ = describe(&sine(440.0, 0.5, 0.02), Some(0.01));
}

#[test]
fn descriptors_are_deterministic() {
    let x = sine(330.0, 0.4, 1.0);
    assert_eq!(describe(&x, Some(0.8)), describe(&x, Some(0.8)));
}
