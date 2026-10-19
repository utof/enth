//! Property tests of `enth_dsp::compressor` (P1a lane FX2): whatever the settings and the signal, the output is
//! finite and bounded, the host block size is not an input, and the static curve keeps its shape.
//!
//! Run with the gate's fixed seed (`PROPTEST_RNG_SEED` in `.cargo/config.toml`) and the default 256 cases.

use enth_dsp::compressor::{Compressor, CompressorParams, StaticCurve};
use proptest::prelude::*;

/// A value in `lo..hi`, or now and then NaN or an infinity: the core must clamp, never trust.
fn wild(lo: f32, hi: f32) -> impl Strategy<Value = f32> {
    prop_oneof![
        19 => lo..hi,
        1 => prop_oneof![Just(f32::NAN), Just(f32::INFINITY), Just(f32::NEG_INFINITY)],
    ]
}

fn any_params() -> impl Strategy<Value = CompressorParams> {
    (
        wild(-200.0, 60.0),
        wild(-2.0, 1500.0),
        wild(-5.0, 80.0),
        wild(-5.0, 3000.0),
        wild(-5.0, 20_000.0),
        wild(-100.0, 100.0),
        wild(-50.0, 150.0),
    )
        .prop_map(
            |(threshold_db, ratio, knee_db, attack_ms, release_ms, makeup_db, mix_pct)| {
                CompressorParams {
                    threshold_db,
                    ratio,
                    knee_db,
                    attack_ms,
                    release_ms,
                    makeup_db,
                    mix_pct,
                }
            },
        )
}

/// Finite samples of all sizes: full scale and beyond, tiny, denormal-range and exact zero.
fn sample() -> impl Strategy<Value = f32> {
    prop_oneof![
        6 => -4.0_f32..4.0,
        2 => -1.0e-3_f32..1.0e-3,
        1 => -1.0e-30_f32..1.0e-30,
        1 => Just(0.0_f32),
    ]
}

fn frames() -> impl Strategy<Value = Vec<(f32, f32)>> {
    prop::collection::vec((sample(), sample()), 1..700)
}

fn prepared(rate: f32, p: &CompressorParams) -> Compressor {
    let mut c = Compressor::new();
    c.prepare(rate);
    c.set_params(p);
    c
}

proptest! {
    #[test]
    fn output_is_finite_and_bounded_by_the_makeup_gain(
        p in any_params(),
        rate in prop_oneof![Just(44_100.0_f32), Just(48_000.0), Just(96_000.0), 8_000.0_f32..200_000.0],
        input in frames(),
    ) {
        let mut c = prepared(rate, &p);
        let used = c.params();
        // The wet path is at most the make-up gain; the dry path is 1. Their blend never exceeds the larger.
        let bound = f64::max(1.0, libm::pow(10.0, f64::from(used.makeup_db) / 20.0)) * 1.0001;
        for &(l, r) in &input {
            let (a, b) = c.tick(l, r);
            prop_assert!(a.is_finite() && b.is_finite());
            prop_assert!(f64::from(a.abs()) <= f64::from(l.abs()) * bound + 1e-30);
            prop_assert!(f64::from(b.abs()) <= f64::from(r.abs()) * bound + 1e-30);
            let gr = c.gain_reduction_db();
            prop_assert!(gr.is_finite() && gr >= 0.0);
        }
        let peak = c.take_peak_gain_reduction_db();
        prop_assert!(peak.is_finite() && peak >= 0.0);
    }

    #[test]
    fn the_block_size_is_not_an_input(
        p in any_params(),
        input in frames(),
        chunks in prop::collection::vec(1_usize..130, 1..40),
    ) {
        let (l, r): (Vec<f32>, Vec<f32>) = input.iter().copied().unzip();
        let n = l.len();
        let mut whole = prepared(48_000.0, &p);
        let (mut want_l, mut want_r) = (vec![0.0_f32; n], vec![0.0_f32; n]);
        whole.process(&l, &r, &mut want_l, &mut want_r);

        let mut parts = prepared(48_000.0, &p);
        let (mut got_l, mut got_r) = (vec![0.0_f32; n], vec![0.0_f32; n]);
        let (mut at, mut i) = (0, 0);
        while at < n {
            let end = (at + chunks[i % chunks.len()]).min(n);
            // The settings are re-applied before every chunk, as a module wrapper does every segment.
            parts.set_params(&p);
            parts.process(&l[at..end], &r[at..end], &mut got_l[at..end], &mut got_r[at..end]);
            at = end;
            i += 1;
        }
        let same = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits());
        prop_assert!(same(&want_l, &got_l) && same(&want_r, &got_r));
        prop_assert_eq!(whole.gain_reduction_db().to_bits(), parts.gain_reduction_db().to_bits());
    }

    #[test]
    fn the_static_curve_keeps_its_shape(
        threshold in -120.0_f32..24.0,
        ratio in 1.0_f32..1000.0,
        knee in 0.0_f32..48.0,
        a in -150.0_f32..60.0,
        b in -150.0_f32..60.0,
    ) {
        let curve = StaticCurve::new(threshold, ratio, knee);
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let (g_lo, g_hi) = (curve.gain_db(lo), curve.gain_db(hi));
        // Never a boost, and a louder input is never reduced less.
        prop_assert!(g_lo <= 0.0 && g_hi <= 0.0);
        prop_assert!(g_hi <= g_lo + 1e-5);
        // Never more than the hard-knee line extended to the lower end of the knee, and exactly zero below the knee.
        let slope = 1.0 - 1.0 / f64::from(ratio);
        let floor = -slope * (f64::from(hi) - f64::from(threshold) + f64::from(knee) / 2.0).max(0.0);
        prop_assert!(f64::from(g_hi) >= floor - 1e-4);
        if f64::from(hi) <= f64::from(threshold) - f64::from(knee) / 2.0 {
            prop_assert_eq!(g_hi, 0.0);
        }
        // A soft knee reduces at least as much as the hard knee at the same threshold (and the same above the knee).
        let hard = StaticCurve::new(threshold, ratio, 0.0).gain_db(hi);
        prop_assert!(g_hi <= hard + 1e-4);
    }
}
