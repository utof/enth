//! The P1a G1 agent-eval findings this crate answers (`docs/p1a-g1.md`, "Eval findings to act on"), each on
//! signals with a known answer:
//!
//! - finding 3: a sub oscillator an octave below a C2 (32.7 Hz) left no pitch; it now reads as the sub, the
//!   waveform's period, by YIN's long-frame fallback and by the tonal pitch;
//! - finding 6: a filter LFO is read as a brightness sweep (`modulation.filter_sweep`) at any depth, an amplitude
//!   LFO is not; a filter envelope is a one-shot sweep;
//! - finding 7: a tone under noise keeps a pitch (`tonal.f0_hz`) long after YIN loses it;
//! - the goal-design note on tremolo: attack and sustain are read with the tremolo fitted out, as with a beat.
//!
//! The clips are short so the debug build stays quick; `docs/dsp/analysis.md` records the wider tables.

mod common;

use common::raw_envelope;
use enth_analysis::descriptors::{Describer, Descriptors, Feature, NoteTiming};
use enth_analysis::sweep::{SweepDir, SweepKind};
use enth_analysis::test_signals::{
    Adsr, CutoffLfo, SR, ToyPatch, ToyPerf, Wave, mix, oscillator, render_toy, render_toy_lfo,
    sine, tremolo, white_noise,
};
use enth_analysis::units::cents;

fn describe(x: &[f32], hold_s: f64) -> Descriptors {
    Describer::new(SR)
        .describe(x, NoteTiming::held(hold_s))
        .expect("finite test signal")
}

#[test]
fn a_sub_octave_reads_as_the_sub() {
    // bass_sub's structure: a saw at C2 over a sine an octave below (C1, 32.7 Hz, under the main YIN's 46.9 Hz)
    let x = mix(
        &oscillator(SR, Wave::Saw, 65.406, 1.5, 0.0),
        &sine(SR, 32.703, 0.5, 1.5),
    );
    let d = describe(&x, 1.0);
    let p = d.pitch.as_ref().expect("a pitch");
    assert!(
        cents(p.f0_hz, 32.703).abs() < 5.0 && p.note == "C1",
        "{p:?}"
    );
    let t = d.tonal.as_ref().expect("a tonal pitch");
    assert!(cents(t.f0_hz, 32.703).abs() < 5.0, "{t:?}");
    // a plain C1 saw too
    let d = describe(&oscillator(SR, Wave::Saw, 32.703, 1.5, 0.0), 1.0);
    assert!(cents(d.get(Feature::Pitch).unwrap(), 32.703).abs() < 5.0);
    // and noise still has none of either (the fallback does not invent a period)
    let d = describe(&white_noise(SR, 0.3, 1.0, 9), 0.3);
    assert!(d.pitch.is_none() && d.tonal.is_none(), "{d:?}");
}

#[test]
fn the_tonal_pitch_survives_noise_that_hides_pitch() {
    // A3 under white noise at several ratios. YIN's pitch goes at noisiness about 0.23 (the eval saw 0.2 to 0.4);
    // the tonal pitch stays within a few cents until the tone is about 4 % of the power.
    let mut lost = 0;
    for wave in [Wave::Triangle, Wave::Saw, Wave::Square] {
        for amp in [0.0, 0.1, 0.3, 0.5, 0.8, 1.2] {
            let tone = oscillator(SR, wave, 220.0, 0.8, 0.0);
            let mut x = white_noise(SR, amp, 0.8, 7);
            for (a, b) in x.iter_mut().zip(&tone) {
                *a += 0.3 * b;
            }
            let d = describe(&x, 0.6);
            let n = d.noise_ratio.unwrap();
            let t = d
                .tonal
                .as_ref()
                .unwrap_or_else(|| panic!("{wave:?} noise {amp}: no tonal pitch at noisiness {n}"));
            assert!(
                cents(t.f0_hz, 220.0).abs() < 3.0,
                "{wave:?} noise {amp}: {t:?}"
            );
            assert!((t.share - (1.0 - n)).abs() < 0.1, "{t:?} vs noisiness {n}");
            if n >= 0.4 {
                assert!(d.pitch.is_none(), "YIN still reads {:?} at {n}", d.pitch);
                lost += 1;
            }
        }
    }
    assert!(lost >= 9, "{lost}");
}

#[test]
fn a_filter_lfo_is_a_sweep_and_an_amp_lfo_is_not() {
    let p = ToyPatch {
        cutoff_hz: 900.0,
        resonance_q: 1.5,
        sustain_db: 0.0,
        ..ToyPatch::default()
    };
    let perf = ToyPerf {
        note_hz: 110.0,
        dur_s: 3.0,
        hold_s: 2.6,
    };
    // a shallow wobble (no tremolo reading at all) and a deep one (also a tremolo)
    for (oct, trem) in [(0.25, false), (1.0, true)] {
        let lfo = CutoffLfo {
            rate_hz: 2.0,
            depth_oct: oct,
        };
        let d = describe(
            &render_toy_lfo(SR, &p, &perf, (0.0, 0.37), 0.0, Some(lfo)),
            perf.hold_s,
        );
        let s = d.modulation.filter_sweep.expect("a filter sweep");
        assert_eq!(s.kind, SweepKind::Lfo);
        assert!((s.hz.unwrap() / 2.0 - 1.0).abs() < 0.02, "{s:?}");
        assert!(s.depth_oct > 0.2 && s.depth_oct < 3.0 * oct, "{s:?}");
        assert_eq!(d.modulation.amp_lfo.is_some(), trem, "{:?}", d.modulation);
    }
    // the same voice with an amplitude LFO: a tremolo, and the brightness does not move
    let mut x = render_toy(SR, &p, &perf, (0.0, 0.37), 0.0);
    tremolo(&mut x, SR, 2.0, 0.5);
    let d = describe(&x, perf.hold_s);
    assert!(d.modulation.filter_sweep.is_none(), "{:?}", d.modulation);
    let t = d.modulation.amp_lfo.expect("a tremolo");
    assert!((t.hz / 2.0 - 1.0).abs() < 0.03, "{t:?}");
}

#[test]
fn a_filter_envelope_is_a_one_shot_sweep() {
    let perf = ToyPerf {
        note_hz: 220.0,
        dur_s: 1.5,
        hold_s: 1.2,
    };
    let pluck = |amount: f64| ToyPatch {
        cutoff_hz: 500.0,
        env_amount_oct: amount,
        env_decay_ms: 300.0,
        sustain_db: -60.0,
        decay_ms: 800.0,
        ..ToyPatch::default()
    };
    let d = describe(&render_toy(SR, &pluck(4.0), &perf, (0.0, 0.37), 0.0), 1.2);
    let s = d.modulation.filter_sweep.expect("a one-shot sweep");
    assert_eq!((s.kind, s.dir), (SweepKind::Env, Some(SweepDir::Down)));
    assert!(s.depth_oct > 1.0 && s.hz.is_none(), "{s:?}");
    assert!(s.ms.is_some_and(|ms| ms > 50.0 && ms < 500.0), "{s:?}");
    let d = describe(&render_toy(SR, &pluck(0.0), &perf, (0.0, 0.37), 0.0), 1.2);
    assert!(d.modulation.filter_sweep.is_none(), "{:?}", d.modulation);
}

#[test]
fn detuned_oscillators_are_not_a_filter_lfo() {
    // Beating swings the centroid at the beat rate and its multiples; that is not a filter.
    for (note, det) in [(110.0, 20.0), (220.0, 30.0), (440.0, 10.0)] {
        let p = ToyPatch {
            detune_ct: det,
            cutoff_hz: 1500.0,
            sustain_db: -3.0,
            ..ToyPatch::default()
        };
        let perf = ToyPerf {
            note_hz: note,
            dur_s: 2.5,
            hold_s: 2.0,
        };
        let d = describe(&render_toy(SR, &p, &perf, (0.0, 0.37), 0.0), 2.0);
        assert!(d.detune.is_some(), "{note} {det}");
        assert!(
            d.modulation.filter_sweep.is_none(),
            "{note} {det}: {:?}",
            d.modulation
        );
    }
}

#[test]
fn tremolo_no_longer_bends_attack_and_sustain() {
    // A 30 ms attack to a full sustain under a 4 Hz tremolo (6 dB) that starts in a trough and is released in one
    // (at 2.0 s). Read raw, the attack runs on to the first tremolo crest at 125 ms and the sustain is the trough's.
    let env = Adsr {
        attack_s: 0.03,
        decay_s: 0.2,
        sustain: 1.0,
        release_s: 0.3,
    };
    let hold = 2.0;
    let mut twin = oscillator(SR, Wave::Saw, 220.0, 2.6, 0.0);
    env.apply(&mut twin, SR, hold);
    let mut x = twin.clone();
    for (i, s) in x.iter_mut().enumerate() {
        let ph = 2.0 * core::f64::consts::PI * 4.0 * i as f64 / SR + core::f64::consts::PI;
        *s *= (1.0 - 0.25 * (1.0 - libm::cos(ph))) as f32;
    }
    let t = describe(&twin, hold).envelope.unwrap();
    let d = describe(&x, hold);
    assert!(d.modulation.amp_lfo.is_some(), "{:?}", d.modulation);
    let e = d.envelope.unwrap();
    let raw = raw_envelope(&x, hold);
    assert!(
        (e.attack_ms - t.attack_ms).abs() <= 5.0,
        "corrected {e:?} twin {t:?}"
    );
    assert!(
        (e.sustain_db.unwrap() - t.sustain_db.unwrap()).abs() < 1.5,
        "corrected {e:?} twin {t:?}"
    );
    assert!(e.beat_corrected);
    // what it read before the fix
    assert!(raw.attack_ms > t.attack_ms + 50.0, "raw {raw:?} twin {t:?}");
    assert!(
        raw.sustain_db.unwrap() < t.sustain_db.unwrap() - 4.0,
        "raw {raw:?} twin {t:?}"
    );
}
