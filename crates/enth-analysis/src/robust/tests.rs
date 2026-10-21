//! The robust distance on synthetic signals with known answers: v1.1 where nothing is damaged, and each kind of damage
//! stops paying for its copy. Lane OBJ's measurement on the engine (40 dev targets × 4 degradations, DEG's chain) is
//! in docs/r3.md.

use super::*;
use crate::features::{FeatureCfg, FeatureExtractor};
use crate::intake::Intake;
use crate::pooled::{metric_v1_1, pool};
use crate::test_signals::{SR, ToyPatch, ToyPerf, mix, render_toy, white_noise};

fn fx() -> FeatureExtractor {
    FeatureExtractor::new(FeatureCfg::default())
}

fn toy(p: &ToyPatch, perf: &ToyPerf) -> Vec<f32> {
    render_toy(SR, p, perf, (0.0, 0.37), 0.0)
}

/// Held to the clip's end: nothing after the sound to compare.
const HELD: ToyPerf = ToyPerf {
    note_hz: 220.0,
    dur_s: 1.0,
    hold_s: 1.0,
};

/// A pluck that has died away well before the end of its 1.5 s clip.
fn pluck() -> ToyPatch {
    ToyPatch {
        attack_ms: 2.0,
        decay_ms: 150.0,
        sustain_db: -80.0,
        release_ms: 60.0,
        ..ToyPatch::default()
    }
}

const PLUCK: ToyPerf = ToyPerf {
    note_hz: 220.0,
    dur_s: 1.5,
    hold_s: 0.3,
};

fn v11(fx: &mut FeatureExtractor, a: &[f32], b: &[f32]) -> f64 {
    let (pa, pb) = (pool(&fx.extract(a)), pool(&fx.extract(b)));
    metric_v1_1(&pa, &pb)
}

fn robust(fx: &mut FeatureExtractor, target: &[f32], damage: &Damage, cand: &[f32]) -> f64 {
    let t = RobustTarget::new(fx, target, damage, 3.0);
    let c = t.candidate(fx, cand);
    t.distance(&c)
}

/// An additive saw at `f0` whose partials stop at `top_hz`, each scaled by `shape(f)`.
fn additive(f0: f64, top_hz: f64, seconds: f64, shape: impl Fn(f64) -> f64) -> Vec<f32> {
    let n = (seconds * SR) as usize;
    let mut out = vec![0.0f32; n];
    let mut h = 1u32;
    while f0 * f64::from(h) <= top_hz {
        let f = f0 * f64::from(h);
        let a = 0.3 * shape(f) / f64::from(h);
        for (i, s) in out.iter_mut().enumerate() {
            *s += (a * libm::sin(2.0 * core::f64::consts::PI * f * i as f64 / SR)) as f32;
        }
        h += 1;
    }
    out
}

#[test]
fn without_damage_it_is_metric_v1_1_bit_for_bit() {
    let mut fx = fx();
    let base = ToyPatch::default();
    let t = toy(&base, &HELD);
    let cands = [
        toy(&base, &HELD),
        toy(
            &ToyPatch {
                cutoff_hz: 4000.0,
                ..base
            },
            &HELD,
        ),
        toy(
            &ToyPatch {
                attack_ms: 80.0,
                release_ms: 900.0,
                ..base
            },
            &HELD,
        ),
        toy(&pluck(), &HELD),
    ];
    for c in &cands {
        let want = v11(&mut fx, &t, c);
        assert_eq!(robust(&mut fx, &t, &Damage::default(), c), want);
        // damage present but every setting off: v1.1 again
        let damage = Damage {
            bandwidth_hz: Some(3000.0),
            noise_floor_dbfs: Some(-50.0),
            clipped_s: vec![(0.1, 0.4)],
            ends_in_floor: true,
            onset_s: Some(0.005),
        };
        let rt = RobustTarget::with_settings(&mut fx, &t, &damage, 3.0, &Settings::off());
        assert_eq!(rt.extra_samples(), 0);
        assert_eq!(rt.distance(&rt.candidate(&mut fx, c)), want);
    }
    assert_eq!(robust(&mut fx, &t, &Damage::default(), &t), 0.0);
}

#[test]
fn the_target_is_zero_from_itself_under_the_band_clipping_and_the_end() {
    let mut fx = fx();
    // (a confirmed floor adds the noise to the candidate, which is clean by assumption: the next test)
    let t = toy(&pluck(), &PLUCK);
    let damage = Damage {
        bandwidth_hz: Some(4000.0),
        noise_floor_dbfs: None,
        clipped_s: vec![(0.05, 0.2)],
        ends_in_floor: true,
        onset_s: Some(0.0),
    };
    let rt = RobustTarget::new(&mut fx, &t, &damage, 3.0);
    assert!(rt.extra_samples() > 0);
    assert_eq!(rt.distance(&rt.candidate(&mut fx, &t)), 0.0);
    // the same sound rendered longer (it has ended): still nothing to charge
    let mut longer = t.clone();
    longer.resize(t.len() + rt.extra_samples(), 0.0);
    assert_eq!(rt.distance(&rt.candidate(&mut fx, &longer)), 0.0);
    // and it is deterministic
    let c = rt.candidate(&mut fx, &toy(&ToyPatch::default(), &PLUCK));
    assert_eq!(rt.distance(&c), rt.distance(&c));
}

#[test]
fn a_codec_cut_is_not_matched() {
    // The upload lost everything above 4 kHz. The true sound (full band) should beat a darker one that imitates the
    // cut; v1.1 prefers the imitation.
    let mut fx = fx();
    let target = additive(220.0, 4_000.0, 1.0, |_| 1.0);
    let truth = additive(220.0, 16_000.0, 1.0, |_| 1.0);
    let dark = additive(220.0, 16_000.0, 1.0, |f| {
        1.0 / (1.0 + libm::pow(f / 3_000.0, 8.0))
    });
    let damage = Damage {
        bandwidth_hz: Some(4_000.0),
        ..Damage::default()
    };
    let (rt, rd) = (
        robust(&mut fx, &target, &damage, &truth),
        robust(&mut fx, &target, &damage, &dark),
    );
    let (vt, vd) = (v11(&mut fx, &target, &truth), v11(&mut fx, &target, &dark));
    assert!(vd < vt, "v1.1 rewards the imitation: {vd} {vt}");
    assert!(rt < rd, "robust prefers the truth: {rt} {rd}");
    assert!(
        rt < 0.2 * vt,
        "the cut costs the truth almost nothing: {rt} {vt}"
    );
    // the level alignment: without it, the energy above the cut lowers the truth's kept bands
    let no_align = RobustTarget::with_settings(
        &mut fx,
        &target,
        &damage,
        3.0,
        &Settings {
            level_align: false,
            ..Settings::default()
        },
    );
    let ra = no_align.distance(&no_align.candidate(&mut fx, &truth));
    assert!(rt < ra, "{rt} {ra}");
}

#[test]
fn a_confirmed_floor_stops_paying_for_noise() {
    let mut fx = fx();
    let clean = toy(&pluck(), &PLUCK);
    let noise = white_noise(SR, 0.01, PLUCK.dur_s, 7);
    let target = mix(&clean, &noise);
    let floor = 20.0 * libm::log10(0.01 / libm::sqrt(3.0));
    let damage = Damage {
        noise_floor_dbfs: Some(floor),
        ..Damage::default()
    };
    // the clean sound, against the same sound with its own noise (another realisation) and a long ringing release
    // that fills the floor the way a search does
    let other_noise = mix(&clean, &white_noise(SR, 0.01, PLUCK.dur_s, 99));
    let ring = toy(
        &ToyPatch {
            release_ms: 2_500.0,
            sustain_db: -30.0,
            ..pluck()
        },
        &PLUCK,
    );
    let (r_clean, r_ring) = (
        robust(&mut fx, &target, &damage, &clean),
        robust(&mut fx, &target, &damage, &ring),
    );
    let (v_clean, v_ring) = (v11(&mut fx, &target, &clean), v11(&mut fx, &target, &ring));
    assert!(
        v_ring < v_clean,
        "v1.1 pays for the ringing: {v_ring} {v_clean}"
    );
    assert!(r_clean < r_ring, "robust does not: {r_clean} {r_ring}");
    assert!(r_clean < 0.25 * v_clean, "{r_clean} {v_clean}");
    // the noise itself is not worth copying any more
    let r_other = robust(&mut fx, &target, &damage, &other_noise);
    assert!(r_clean < r_other + 0.05, "{r_clean} {r_other}");
    // without a (confirmed) floor nothing is clamped: v1.1
    assert_eq!(
        robust(&mut fx, &target, &Damage::default(), &clean),
        v_clean
    );
}

#[test]
fn damage_from_report_uses_only_a_confirmed_floor_and_hard_clipping() {
    // A note after 0.4 s of noise: the lead-in confirms the floor.
    let clean = toy(&pluck(), &PLUCK);
    let mut x = vec![0.0f32; (0.4 * SR) as usize];
    x.extend_from_slice(&clean);
    let x = mix(&x, &white_noise(SR, 0.003, x.len() as f64 / SR, 3));
    let it = Intake::new(&x, 1, 48_000).unwrap();
    let d = &it.report.degradation;
    assert!(d.floor_confirmed && d.noise_floor_dbfs.is_some(), "{d:?}");
    let damage = Damage::from_report(&it.report);
    assert_eq!(damage.noise_floor_dbfs, d.noise_floor_dbfs);
    assert!(damage.ends_in_floor, "{:?}", it.report.span);
    // unconfirmed: no floor
    let mut report = it.report.clone();
    report.degradation.floor_confirmed = false;
    assert_eq!(Damage::from_report(&report).noise_floor_dbfs, None);
    // the clipped spans, in clip time
    report.degradation.clipping = Some(crate::intake::Clipping {
        level_dbfs: -1.0,
        share: 0.1,
        spans: vec![crate::intake::ClipSpan {
            start_s: 0.01,
            end_s: 0.05,
        }],
    });
    assert_eq!(Damage::from_report(&report).clipped_s, vec![(0.01, 0.05)]);
    // a capped clip, or a window that cuts it, does not end in the floor
    report.span.capped = true;
    assert!(!Damage::from_report(&report).ends_in_floor);
    let clip_s = it.report.perf.duration_s;
    let windowed = Damage::from_report(&it.report).windowed(0.5 * clip_s, clip_s);
    assert!(!windowed.ends_in_floor);
    let whole = Damage::from_report(&it.report)
        .windowed(it.report.perf.duration_s, it.report.perf.duration_s);
    assert!(whole.ends_in_floor);
}

#[test]
fn clipped_blocks_weigh_less_only_when_asked() {
    let mut fx = fx();
    let base = ToyPatch::default();
    let t = toy(&base, &HELD);
    // a candidate that differs from the target only in its first 0.3 s
    let other = toy(
        &ToyPatch {
            cutoff_hz: 6000.0,
            ..base
        },
        &HELD,
    );
    let cut = (0.3 * SR) as usize;
    let mut c = other[..cut].to_vec();
    c.extend_from_slice(&t[cut..]);
    let plain = robust(&mut fx, &t, &Damage::default(), &c);
    let clipped = Damage {
        clipped_s: vec![(0.0, 0.3)],
        ..Damage::default()
    };
    // the definition compares them like the rest (the dev measurement found no gain)
    assert_eq!(robust(&mut fx, &t, &clipped, &c), plain);
    let gentle = Settings {
        clip_weight: 0.25,
        ..Settings::default()
    };
    let rt = RobustTarget::with_settings(&mut fx, &t, &clipped, 3.0, &gentle);
    let r = rt.distance(&rt.candidate(&mut fx, &c));
    assert!(r < 0.6 * plain, "{r} {plain}");
}

#[test]
fn ringing_on_after_the_sound_is_charged() {
    // The upload ends in silence 0.6 s in. A candidate equal to it inside the clip but ringing on past it (a 220 Hz
    // tone decaying from -30 dB) is charged only when the comparison looks past the clip.
    let mut fx = fx();
    let perf = ToyPerf {
        note_hz: 220.0,
        dur_s: 0.6,
        hold_s: 0.2,
    };
    let t = toy(&pluck(), &perf);
    let extra = (0.5 * SR) as usize;
    let mut same = t.clone();
    same.resize(t.len() + extra, 0.0);
    let mut ring = same.clone();
    let peak = t.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    for (i, v) in ring.iter_mut().enumerate().skip(t.len()) {
        let s = (i - t.len()) as f64 / SR;
        *v += (f64::from(peak)
            * 0.03
            * libm::exp(-s / 0.3)
            * libm::sin(2.0 * core::f64::consts::PI * 220.0 * s)) as f32;
    }
    let ends = Damage {
        ends_in_floor: true,
        ..Damage::default()
    };
    let rt = RobustTarget::new(&mut fx, &t, &ends, 3.0);
    assert_eq!(rt.extra_samples(), extra);
    let (d_same, d_ring) = (
        rt.distance(&rt.candidate(&mut fx, &same)),
        rt.distance(&rt.candidate(&mut fx, &ring)),
    );
    assert_eq!(d_same, 0.0);
    assert!(d_ring > 0.1, "{d_ring}");
    // inside the clip the two are the same sound
    let n = t.len();
    assert_eq!(v11(&mut fx, &t, &ring[..n]), 0.0);
    // without the silence after the sound, or past the window, nothing is charged
    let plain = RobustTarget::new(&mut fx, &t, &Damage::default(), 3.0);
    assert_eq!(plain.extra_samples(), 0);
    assert_eq!(plain.distance(&plain.candidate(&mut fx, &ring[..n])), 0.0);
    let capped = RobustTarget::new(&mut fx, &t, &ends, 0.6);
    assert_eq!(capped.extra_samples(), 0);
}

#[test]
fn a_slow_attack_that_surfaced_late_gets_its_lead_back() {
    // A 300 ms attack under noise, cut as the intake cuts it when the rise clears the noise late: the clip starts
    // 256 samples (the pre-roll) before a late onset.
    let p = ToyPatch {
        attack_ms: 300.0,
        ..ToyPatch::default()
    };
    let perf = ToyPerf {
        note_hz: 220.0,
        dur_s: 1.5,
        hold_s: 1.2,
    };
    let amp = 0.004;
    let floor = 20.0 * libm::log10(amp / libm::sqrt(3.0));
    let noisy = mix(&toy(&p, &perf), &white_noise(SR, amp, perf.dur_s, 5));
    let on = 256.0 / SR;
    for late_ms in [8.0, 15.0, 25.0, 35.0] {
        let late = late_ms / 1000.0;
        let cut = (late * SR) as usize - 256;
        let lead = attack_lead(&noisy[cut..], SR, on, floor);
        assert!(
            (lead - late).abs() < 0.3 * late + 0.003,
            "{late_ms} ms late: {lead}"
        );
    }
    // the onset found exactly (pre-roll of noise before it): no lead worth having
    let mut exact = white_noise(SR, amp, on, 9);
    exact.extend_from_slice(&noisy);
    assert!(attack_lead(&exact, SR, on, floor) < 0.006);
    // a fast attack: none
    let fast = mix(
        &toy(
            &ToyPatch {
                attack_ms: 3.0,
                ..ToyPatch::default()
            },
            &perf,
        ),
        &white_noise(SR, amp, perf.dur_s, 5),
    );
    let mut fast_clip = white_noise(SR, amp, on, 11);
    fast_clip.extend_from_slice(&fast);
    assert_eq!(attack_lead(&fast_clip, SR, on, floor), 0.0);
    // only with a confirmed floor
    let late_clip = &noisy[(0.02 * SR) as usize - 256..];
    let with = Damage {
        noise_floor_dbfs: Some(floor),
        onset_s: Some(on),
        ..Damage::default()
    };
    let without = Damage {
        onset_s: Some(on),
        ..Damage::default()
    };
    let mut fx = fx();
    assert!(RobustTarget::new(&mut fx, late_clip, &with, 3.0).lead_s() > 0.01);
    assert_eq!(
        RobustTarget::new(&mut fx, late_clip, &without, 3.0).lead_s(),
        0.0
    );
}
