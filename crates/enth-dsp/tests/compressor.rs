//! Contract tests of `enth_dsp::compressor` (P1a lane FX2): the static curve against its closed form and against the
//! Web Audio formula, step responses, a steady sine, silence, the stereo link, block-size invariance, the
//! native = wasm32 hash and `assert_no_alloc`. White-box tests of the follower and the flush live in the module.
//!
//! The same file runs on `wasm32-wasip1` under wasmtime (docs/dsp/compressor.md): `HASH_GOLDEN` is one constant for
//! every platform.
//!
//! Test oracles are written in `f64` with `libm` (the workspace bans `std`'s transcendentals everywhere, tests too).

use assert_no_alloc::assert_no_alloc;
use enth_dsp::compressor::{Compressor, CompressorParams, StaticCurve};

// `AllocDisabler` exists only in debug builds (assert_no_alloc's `disable_release` default): release builds run the
// closure unchecked.
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const FS: f32 = 48_000.0;
const TAU: f64 = core::f64::consts::TAU;

// ---------------------------------------------------------------------------------------------------------------
// Helpers

fn params(
    threshold_db: f32,
    ratio: f32,
    knee_db: f32,
    attack_ms: f32,
    release_ms: f32,
) -> CompressorParams {
    CompressorParams {
        threshold_db,
        ratio,
        knee_db,
        attack_ms,
        release_ms,
        makeup_db: 0.0,
        mix_pct: 100.0,
    }
}

fn compressor(p: &CompressorParams, fs: f32) -> Compressor {
    let mut c = Compressor::new();
    c.prepare(fs);
    c.set_params(p);
    c
}

fn db(x: f64) -> f64 {
    20.0 * libm::log10(x)
}

fn lin(db: f64) -> f64 {
    libm::pow(10.0, db / 20.0)
}

/// Spacing of `f32` around `x`.
fn ulp(x: f32) -> f64 {
    let bits = x.abs().to_bits();
    f64::from(f32::from_bits(bits + 1) - f32::from_bits(bits))
}

/// xorshift32: a tiny seeded generator so the tests need no RNG crate.
struct Xorshift32(u32);

impl Xorshift32 {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in `[-1, 1)`, exact in `f32`.
    fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 8_388_608.0 - 1.0
    }
}

/// `amp·sin(2π·freq·n/fs)`.
fn sine(amp: f32, freq: f64, fs: f64, n: usize) -> f32 {
    (f64::from(amp) * libm::sin(TAU * freq * n as f64 / fs)) as f32
}

/// FNV-1a 64 over the little-endian bytes of `f32` bit patterns.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn f32(&mut self, x: f32) {
        for b in x.to_bits().to_le_bytes() {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// The compressor's equations in `f64` end to end (JOS structure, Giannoulis knee): an independent reference for
/// trajectories. Returns the gain reduction in dB (`>= 0`) after each input level.
struct Reference {
    env: f64,
    gs: f64,
    k_attack: f64,
    k_release: f64,
    k_smooth: f64,
    curve: fn(f64, f64, f64, f64) -> f64,
    threshold: f64,
    ratio: f64,
    knee: f64,
}

impl Reference {
    fn new(p: &CompressorParams, fs: f64) -> Self {
        let k = |ms: f64| {
            if ms <= 0.0 {
                1.0
            } else {
                -libm::expm1(-1.0 / (ms * 0.001 * fs))
            }
        };
        Self {
            env: 0.0,
            gs: 0.0,
            k_attack: k(f64::from(p.attack_ms)),
            k_release: k(f64::from(p.release_ms)),
            k_smooth: k(0.5 * f64::from(p.attack_ms)),
            curve: giannoulis_gain_db,
            threshold: f64::from(p.threshold_db),
            ratio: f64::from(p.ratio),
            knee: f64::from(p.knee_db),
        }
    }
    fn tick(&mut self, e: f64) -> f64 {
        let k = if e > self.env {
            self.k_attack
        } else {
            self.k_release
        };
        self.env += k * (e - self.env);
        let level = db(self.env.max(1e-10));
        let g = (self.curve)(level, self.threshold, self.knee, self.ratio);
        self.gs += self.k_smooth * (g - self.gs);
        -self.gs
    }
}

/// Giannoulis, Massberg and Reiss (2012), eq. 4, as the gain change `y − x` in dB (`<= 0`):
/// `y = x` if `2(x−T) < −W`; `y = x + (1/R − 1)(x − T + W/2)² / (2W)` if `2|x−T| <= W`; `y = T + (x−T)/R` above.
fn giannoulis_gain_db(x: f64, t: f64, w: f64, r: f64) -> f64 {
    let y = if 2.0 * (x - t) < -w {
        x
    } else if w > 0.0 && 2.0 * (x - t).abs() <= w {
        let u = x - t + w / 2.0;
        x + (1.0 / r - 1.0) * u * u / (2.0 * w)
    } else {
        t + (x - t) / r
    };
    y - x
}

/// The gain computer of web-audio-api-rs `DynamicsCompressorNode`, as a test oracle.
///
/// Source: <https://github.com/orottier/web-audio-api-rs/blob/03d0cb28983896be5cde38d3961a47bdefb8ea8f/src/node/dynamics_compressor.rs>
/// (lines 356-426, the "Gain Computer stage"), MIT, "Copyright (c) 2021 Otto Rottier"; the file was written by
/// contributor b-ma in July 2022 from the Web Audio spec's compression curve and Giannoulis et al. (2012), eq. 4.
/// Only this arithmetic is reproduced, in `f64` and in dB (nothing else of the file: no detector, no look-ahead, no
/// make-up). Its quirk is kept: when the knee is not zero the node moves the threshold up by `knee/2`, so its
/// `threshold` parameter is our threshold minus half the knee. Returns the attenuation `x − y` in dB, `>= 0`.
fn web_audio_attenuation_db(sample_db: f64, threshold: f64, knee: f64, ratio: f64) -> f64 {
    let threshold = if knee > 0.0 {
        threshold + knee / 2.0
    } else {
        threshold
    };
    let half_knee = knee / 2.0;
    let knee_partial = (1.0 / ratio - 1.0) / (2.0 * knee);
    let attenuated = if sample_db <= threshold - half_knee {
        sample_db
    } else if sample_db <= threshold + half_knee {
        let u = sample_db - threshold + half_knee;
        sample_db + u * u * knee_partial
    } else {
        threshold + (sample_db - threshold) / ratio
    };
    sample_db - attenuated
}

/// Tolerance for a static-curve comparison: 1e-6 dB, or half an `f32` ulp of the result when that is larger (only
/// for reductions above 32 dB, where one ulp of `f32` itself exceeds 1e-6).
fn curve_tolerance(expected: f64) -> f64 {
    (0.5 * ulp(expected as f32) * 1.001).max(1e-6)
}

// ---------------------------------------------------------------------------------------------------------------
// The static curve

#[test]
fn static_curve_table() {
    // (threshold, ratio, knee, [(level, gain change)]) worked by hand from the closed form; every entry is an exact
    // binary fraction. Threshold -24 dB, ratio 4: slope 1/R - 1 = -0.75.
    type Rows = &'static [(f32, f32)];
    #[rustfmt::skip]
    let table: &[(f32, f32, f32, Rows)] = &[
        // Hard knee: 0 up to the threshold, then -0.75 dB per dB.
        (-24.0, 4.0, 0.0, &[(-60.0, 0.0), (-24.0, 0.0), (-18.0, -4.5), (-12.0, -9.0), (-6.0, -13.5), (0.0, -18.0), (6.0, -22.5)]),
        // 6 dB knee: edges at -27 and -21; -0.75·(d + 3)²/12 inside, -0.75·d above.
        (-24.0, 4.0, 6.0, &[(-30.0, 0.0), (-27.0, 0.0), (-25.5, -0.140_625), (-24.0, -0.562_5), (-22.5, -1.265_625), (-21.0, -2.25), (-18.0, -4.5)]),
        // 12 dB knee: edges at -30 and -18.
        (-24.0, 4.0, 12.0, &[(-36.0, 0.0), (-30.0, 0.0), (-27.0, -0.281_25), (-24.0, -1.125), (-21.0, -2.531_25), (-18.0, -4.5), (-6.0, -13.5)]),
        // Ratio 2, threshold -18, 4 dB knee: slope -0.5.
        (-18.0, 2.0, 4.0, &[(-30.0, 0.0), (-20.0, 0.0), (-18.0, -0.25), (-16.0, -1.0), (-12.0, -3.0), (0.0, -9.0)]),
        // Ratio 1 never reduces; ratio 1000 is nearly a wall.
        (-24.0, 1.0, 6.0, &[(-60.0, 0.0), (-24.0, 0.0), (0.0, 0.0)]),
        (-24.0, 1000.0, 0.0, &[(-30.0, 0.0), (0.0, -23.976)]),
    ];
    for &(t, r, w, rows) in table {
        let curve = StaticCurve::new(t, r, w);
        for &(level, expected) in rows {
            let got = curve.gain_db(level);
            assert!(
                (got - expected).abs() <= 1e-6,
                "T {t} R {r} W {w}: gain at {level} dB is {got}, expected {expected}"
            );
            assert!((curve.output_db(level) - (level + expected)).abs() <= 1e-5);
        }
    }
}

#[test]
fn hard_knee_is_jos_max_curve() {
    // JOS compression_gain_mono: G = max(level - thresh, 0) · (1/ratio - 1).
    for &t in &[-60.0_f32, -24.0, -6.0, 0.0] {
        for &r in &[1.0_f32, 1.5, 2.0, 4.0, 8.0, 20.0] {
            let curve = StaticCurve::new(t, r, 0.0);
            for i in 0..=1200 {
                let level = -90.0 + 0.1 * i as f32;
                let jos = (f64::from(level) - f64::from(t)).max(0.0) * (1.0 / f64::from(r) - 1.0);
                let got = f64::from(curve.gain_db(level));
                assert!(
                    (got - jos).abs() <= curve_tolerance(jos),
                    "T {t} R {r} level {level}: {got} vs {jos}"
                );
            }
        }
    }
}

#[test]
fn static_curve_matches_the_closed_form_giannoulis_equations() {
    let mut worst = 0.0_f64;
    for &t in &[-60.0_f32, -48.5, -36.0, -24.0, -12.25, -6.0, -0.5, 0.0] {
        for &r in &[1.0_f32, 1.5, 2.0, 3.0, 4.0, 8.0, 12.0, 20.0, 100.0] {
            for &w in &[0.0_f32, 0.5, 1.0, 3.0, 6.0, 12.0, 24.0, 48.0] {
                let curve = StaticCurve::new(t, r, w);
                for i in 0..=10_600 {
                    let level = -100.0 + 0.01 * i as f32;
                    let expected = giannoulis_gain_db(
                        f64::from(level),
                        f64::from(t),
                        f64::from(w),
                        f64::from(r),
                    );
                    let got = f64::from(curve.gain_db(level));
                    let err = (got - expected).abs();
                    worst = worst.max(err / curve_tolerance(expected));
                    assert!(
                        err <= curve_tolerance(expected),
                        "T {t} R {r} W {w} level {level}: {got} vs {expected} (error {err})"
                    );
                }
            }
        }
    }
    assert!(worst <= 1.0);
}

/// The Web Audio node's gain computer, within 1e-6 dB (report 02 measured 9.5e-7 in `f32`; this implementation
/// evaluates in `f64` and rounds once, so the bound is half an `f32` ulp of the result).
#[test]
fn static_curve_matches_web_audio_api_rs_within_1e6_db() {
    let (mut worst, mut worst_below_32) = (0.0_f64, 0.0_f64);
    // `threshold` below is the Web Audio node's parameter; ours is that plus half the knee (all values are exact in
    // f32, so no rounding enters before the comparison).
    for &wa_threshold in &[-60.0_f32, -48.5, -36.0, -24.0, -12.25, -6.0, -0.5, 0.0] {
        for &ratio in &[1.0_f32, 1.5, 2.0, 4.0, 8.0, 12.0, 20.0] {
            for &knee in &[0.0_f32, 1.0, 3.0, 6.0, 12.0, 24.0, 30.0] {
                let ours_threshold = if knee > 0.0 {
                    wa_threshold + knee / 2.0
                } else {
                    wa_threshold
                };
                let curve = StaticCurve::new(ours_threshold, ratio, knee);
                for i in 0..=10_600 {
                    let level = -100.0 + 0.01 * i as f32;
                    let wa = -web_audio_attenuation_db(
                        f64::from(level),
                        f64::from(wa_threshold),
                        f64::from(knee),
                        f64::from(ratio),
                    );
                    let got = f64::from(curve.gain_db(level));
                    let err = (got - wa).abs();
                    worst = worst.max(err);
                    if wa.abs() < 32.0 {
                        worst_below_32 = worst_below_32.max(err);
                    }
                    assert!(
                        err <= curve_tolerance(wa),
                        "WA T {wa_threshold} R {ratio} W {knee} level {level}: ours {got}, web-audio {wa}"
                    );
                }
            }
        }
    }
    println!(
        "largest |ours - web-audio-api-rs|: {worst:e} dB overall, {worst_below_32:e} dB where the reduction is under 32 dB"
    );
    // Above 32 dB of reduction one f32 ulp of the result alone is 3.8e-6 dB, so only half an ulp is promised there.
    assert!(
        worst_below_32 < 1e-6,
        "worst deviation from the Web Audio formula below 32 dB: {worst_below_32} dB"
    );
    assert!(
        worst < 2e-6,
        "worst deviation from the Web Audio formula: {worst} dB"
    );
}

#[test]
fn static_curve_is_non_positive_monotone_continuous_and_bounded_in_slope() {
    for &t in &[-60.0_f32, -24.0, -6.0] {
        for &r in &[1.0_f32, 2.0, 4.0, 20.0] {
            for &w in &[0.0_f32, 6.0, 24.0] {
                let curve = StaticCurve::new(t, r, w);
                let max_slope = 1.0 - 1.0 / f64::from(r);
                let (mut prev_level, mut prev) = (-120.0_f32, curve.gain_db(-120.0));
                for i in 1..=14_400 {
                    let level = -120.0 + 0.01 * i as f32;
                    let g = curve.gain_db(level);
                    assert!(g <= 0.0, "positive gain change {g} at {level}");
                    assert!(g <= prev + 1e-6, "not monotone at {level}: {prev} then {g}");
                    // The reduction grows by at most (1 - 1/R) dB per dB: the curve is continuous and never steeper
                    // than its hard-knee asymptote. (Levels are f32, so use their actual spacing.)
                    let step = f64::from(level) - f64::from(prev_level);
                    let rounding = ulp(g) + ulp(prev) + 1e-6;
                    assert!(
                        f64::from(prev - g) <= max_slope * step + rounding,
                        "jump at {level}: {prev} then {g}"
                    );
                    (prev_level, prev) = (level, g);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Behaviour

fn run_stereo(c: &mut Compressor, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut out_l = vec![0.0; l.len()];
    let mut out_r = vec![0.0; r.len()];
    c.process(l, r, &mut out_l, &mut out_r);
    (out_l, out_r)
}

#[test]
fn silence_gives_exact_zeros_and_no_nan() {
    for makeup in [0.0_f32, 12.0, -12.0] {
        for &(ratio, knee, att) in &[
            (4.0_f32, 6.0_f32, 10.0_f32),
            (20.0, 0.0, 0.0),
            (1.0, 48.0, 100.0),
        ] {
            let p = CompressorParams {
                makeup_db: makeup,
                ..params(-60.0, ratio, knee, att, 50.0)
            };
            let mut c = compressor(&p, FS);
            let zeros = vec![0.0_f32; 4096];
            let (l, r) = run_stereo(&mut c, &zeros, &zeros);
            assert!(
                l.iter().chain(&r).all(|&x| x == 0.0),
                "silence in must be silence out (makeup {makeup})"
            );
            assert_eq!(c.gain_reduction_db(), 0.0);
            assert_eq!(c.take_peak_gain_reduction_db(), 0.0);
        }
    }
}

#[test]
fn silence_after_a_loud_passage_decays_without_nan_or_subnormals() {
    let mut c = compressor(&params(-30.0, 8.0, 6.0, 5.0, 300.0), FS);
    let mut rng = Xorshift32(0x1234_5678);
    for _ in 0..48_000 {
        let (a, b) = c.tick(rng.unit(), rng.unit());
        assert!(a.is_finite() && b.is_finite());
    }
    for _ in 0..FS as usize * 20 {
        let (a, b) = c.tick(0.0, 0.0);
        assert!(a == 0.0 && b == 0.0);
        assert!(c.gain_reduction_db().is_finite());
        assert!(!c.gain_reduction_db().is_subnormal());
    }
    assert_eq!(
        c.gain_reduction_db(),
        0.0,
        "the meter returns to exactly zero"
    );
}

#[test]
fn non_finite_input_does_not_poison_the_state() {
    let mut c = compressor(&CompressorParams::DEFAULT, FS);
    for _ in 0..1000 {
        c.tick(0.9, 0.9);
    }
    for &(a, b) in &[
        (f32::NAN, 0.1),
        (0.1, f32::NAN),
        (f32::NAN, f32::NAN),
        (f32::INFINITY, 0.0),
        (f32::NEG_INFINITY, 0.3),
    ] {
        let _ = c.tick(a, b);
        assert!(c.gain_reduction_db().is_finite());
    }
    for _ in 0..1000 {
        let (a, b) = c.tick(0.5, -0.5);
        assert!(
            a.is_finite() && b.is_finite(),
            "the compressor must recover on the next finite frame"
        );
    }
}

#[test]
fn below_threshold_it_is_transparent_bit_for_bit() {
    // Threshold -24 dB, knee 6: nothing below -27 dB. A -40 dBFS sine leaves the compressor untouched.
    let mut c = compressor(&params(-24.0, 4.0, 6.0, 10.0, 200.0), FS);
    let l: Vec<f32> = (0..9600).map(|n| sine(0.01, 997.0, 48_000.0, n)).collect();
    let r: Vec<f32> = (0..9600).map(|n| sine(0.007, 331.0, 48_000.0, n)).collect();
    let (ol, or) = run_stereo(&mut c, &l, &r);
    assert!(ol.iter().zip(&l).all(|(a, b)| a.to_bits() == b.to_bits()));
    assert!(or.iter().zip(&r).all(|(a, b)| a.to_bits() == b.to_bits()));
    assert_eq!(c.gain_reduction_db(), 0.0);
}

#[test]
fn mix_zero_is_the_dry_input_and_mix_blends_linearly() {
    let l: Vec<f32> = (0..9600).map(|n| sine(0.8, 220.0, 48_000.0, n)).collect();
    let r: Vec<f32> = (0..9600).map(|n| sine(0.6, 330.0, 48_000.0, n)).collect();
    let base = params(-30.0, 8.0, 6.0, 2.0, 100.0);
    let (wet_l, _) = run_stereo(&mut compressor(&base, FS), &l, &r);
    let (dry_l, dry_r) = run_stereo(
        &mut compressor(
            &CompressorParams {
                mix_pct: 0.0,
                ..base
            },
            FS,
        ),
        &l,
        &r,
    );
    assert!(
        dry_l
            .iter()
            .zip(&l)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "mix 0 % is bit-identical"
    );
    assert!(
        dry_r
            .iter()
            .zip(&r)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    let (mix_l, _) = run_stereo(
        &mut compressor(
            &CompressorParams {
                mix_pct: 35.0,
                ..base
            },
            FS,
        ),
        &l,
        &r,
    );
    assert!(
        wet_l.iter().zip(&l).any(|(w, d)| (w - d).abs() > 0.05),
        "the test signal must actually be compressed"
    );
    for ((m, w), d) in mix_l.iter().zip(&wet_l).zip(&l) {
        let expected = 0.65 * d + 0.35 * w;
        assert!((m - expected).abs() <= 1e-5, "mix 35 %: {m} vs {expected}");
    }
}

#[test]
fn makeup_gain_is_added_after_the_curve_to_the_wet_path_only() {
    // Quiet input, nothing compressed: the output is the input times the make-up.
    let l: Vec<f32> = (0..4800).map(|n| sine(0.01, 440.0, 48_000.0, n)).collect();
    for &m in &[-12.0_f32, 0.0, 6.0, 24.0] {
        let p = CompressorParams {
            makeup_db: m,
            ..params(-24.0, 4.0, 6.0, 10.0, 200.0)
        };
        let (out, _) = run_stereo(&mut compressor(&p, FS), &l, &l);
        let k = lin(f64::from(m));
        for (o, i) in out.iter().zip(&l) {
            assert!(
                (f64::from(*o) - f64::from(*i) * k).abs()
                    <= 2e-6 * k.max(1.0) * f64::from(i.abs()).max(1e-3)
            );
        }
        assert_eq!(c_gain_reduction(&p, &l), 0.0);
    }
    // At mix 50 % the make-up applies to half of the signal: gain = 0.5 + 0.5·10^(6/20).
    let p = CompressorParams {
        makeup_db: 6.0,
        mix_pct: 50.0,
        ..params(-24.0, 4.0, 6.0, 10.0, 200.0)
    };
    let (out, _) = run_stereo(&mut compressor(&p, FS), &l, &l);
    let k = 0.5 + 0.5 * lin(6.0);
    assert!((f64::from(out[100]) - f64::from(l[100]) * k).abs() < 1e-6);
}

fn c_gain_reduction(p: &CompressorParams, l: &[f32]) -> f32 {
    let mut c = compressor(p, FS);
    let _ = run_stereo(&mut c, l, l);
    c.gain_reduction_db()
}

#[test]
fn ratio_one_never_reduces() {
    let l: Vec<f32> = (0..9600).map(|n| sine(1.0, 100.0, 48_000.0, n)).collect();
    let (out, _) = run_stereo(
        &mut compressor(&params(-60.0, 1.0, 12.0, 1.0, 10.0), FS),
        &l,
        &l,
    );
    assert!(out.iter().zip(&l).all(|(a, b)| a.to_bits() == b.to_bits()));
}

#[test]
fn the_louder_channel_drives_both_gains() {
    // Gain reduction depends on max(|L|, |R|) alone: a centred mono signal reads the same as the same signal on
    // one channel (JOS's |L| + |R| link would read it 6 dB hot), and a quieter partner changes nothing.
    let p = params(-24.0, 4.0, 6.0, 5.0, 100.0);
    let x: Vec<f32> = (0..24_000)
        .map(|n| sine(0.7, 523.25, 48_000.0, n))
        .collect();
    let quiet: Vec<f32> = x.iter().map(|v| 0.5 * v).collect();
    let zero = vec![0.0_f32; x.len()];
    let mut meters = Vec::new();
    for (l, r) in [
        (&x, &x),
        (&x, &zero),
        (&zero, &x),
        (&x, &quiet),
        (&quiet, &x),
    ] {
        let mut c = compressor(&p, FS);
        let mut trace = Vec::new();
        for (a, b) in l.iter().zip(r) {
            c.tick(*a, *b);
            trace.push(c.gain_reduction_db().to_bits());
        }
        meters.push(trace);
    }
    for m in &meters[1..] {
        assert_eq!(
            &meters[0], m,
            "the gain reduction trace depends only on max(|L|, |R|)"
        );
    }
    // And the one gain multiplies both channels.
    let mut c = compressor(&p, FS);
    let (ol, or) = run_stereo(&mut c, &x, &quiet);
    for (n, (((o_l, o_r), a), b)) in ol.iter().zip(&or).zip(&x).zip(&quiet).enumerate() {
        if a.abs() > 0.1 {
            let (gl, gr) = (
                f64::from(*o_l) / f64::from(*a),
                f64::from(*o_r) / f64::from(*b),
            );
            assert!(
                (gl - gr).abs() <= 1e-6 * gl.abs(),
                "frame {n}: {gl} vs {gr}"
            );
        }
    }
}

#[test]
fn zero_latency_and_block_size_does_not_matter() {
    let mut c = compressor(&params(-30.0, 6.0, 6.0, 3.0, 80.0), FS);
    assert_eq!(c.latency_samples(), 0);
    // An impulse comes out on the same frame it goes in.
    let (a, _) = c.tick(0.9, 0.0);
    assert!(a != 0.0 && a.abs() <= 0.9);

    let mut rng = Xorshift32(0xdead_beef);
    let n = 10_007;
    let l: Vec<f32> = (0..n)
        .map(|i| 0.8 * rng.unit() * sine(1.0, 3.0, 48_000.0, i).abs())
        .collect();
    let r: Vec<f32> = (0..n).map(|_| 0.3 * rng.unit()).collect();
    let p = params(-30.0, 6.0, 6.0, 3.0, 80.0);
    let (ref_l, ref_r) = run_stereo(&mut compressor(&p, FS), &l, &r);
    for &chunk in &[1_usize, 7, 32, 63, 64, 65, 512, 4096] {
        let mut c = compressor(&p, FS);
        let (mut out_l, mut out_r) = (vec![0.0_f32; n], vec![0.0_f32; n]);
        let mut at = 0;
        while at < n {
            let end = (at + chunk).min(n);
            c.process(
                &l[at..end],
                &r[at..end],
                &mut out_l[at..end],
                &mut out_r[at..end],
            );
            at = end;
        }
        assert!(
            out_l
                .iter()
                .zip(&ref_l)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "chunk {chunk}"
        );
        assert!(
            out_r
                .iter()
                .zip(&ref_r)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "chunk {chunk}"
        );
    }
    // In place gives the same bits as separate buffers.
    let (mut in_l, mut in_r) = (l.clone(), r.clone());
    compressor(&p, FS).process_in_place(&mut in_l, &mut in_r);
    assert!(
        in_l.iter()
            .zip(&ref_l)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    assert!(
        in_r.iter()
            .zip(&ref_r)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
}

#[test]
fn mismatched_buffer_lengths_never_panic() {
    let mut c = compressor(&CompressorParams::DEFAULT, FS);
    let (l, r) = (vec![0.5_f32; 100], vec![0.5_f32; 60]);
    let (mut ol, mut or) = (vec![0.0_f32; 80], vec![0.0_f32; 200]);
    c.process(&l, &r, &mut ol, &mut or);
    assert!(ol[..60].iter().all(|&x| x != 0.0));
    assert!(
        ol[60..].iter().all(|&x| x == 0.0),
        "only min(len) frames are written"
    );
    c.process(&[], &[], &mut [], &mut []);
}

#[test]
fn reset_restores_the_initial_state_and_prepare_resets_too() {
    let p = params(-30.0, 6.0, 6.0, 3.0, 800.0);
    let mut rng = Xorshift32(7);
    let l: Vec<f32> = (0..5000).map(|_| 0.9 * rng.unit()).collect();
    let fresh = run_stereo(&mut compressor(&p, FS), &l, &l);
    let mut used = compressor(&p, FS);
    let _ = run_stereo(&mut used, &l, &l);
    assert!(used.gain_reduction_db() > 1.0);
    used.reset();
    assert_eq!(used.gain_reduction_db(), 0.0);
    assert_eq!(used.take_peak_gain_reduction_db(), 0.0);
    assert_eq!(
        run_stereo(&mut used, &l, &l),
        fresh,
        "reset gives the output of a fresh compressor"
    );
    used.prepare(FS);
    assert_eq!(
        run_stereo(&mut used, &l, &l),
        fresh,
        "prepare resets as well"
    );
}

#[test]
fn the_meter_reads_gain_reduction_and_its_peak() {
    let mut c = compressor(&params(-24.0, 4.0, 0.0, 1.0, 50.0), FS);
    assert_eq!(c.gain_reduction_db(), 0.0);
    for _ in 0..4800 {
        c.tick(0.5, 0.5);
    }
    let held = c.gain_reduction_db();
    assert!(
        (held - 13.484).abs() < 0.01,
        "settled GR at -6 dBFS over -24 dB, 4:1: {held}"
    );
    assert!(c.take_peak_gain_reduction_db() >= held);
    // After the signal stops the instantaneous reading falls, and the next peak read restarts from the current value.
    for _ in 0..24_000 {
        c.tick(0.0, 0.0);
    }
    assert!(c.gain_reduction_db() < held);
    let peak = c.take_peak_gain_reduction_db();
    assert!(
        peak >= held - 1e-3,
        "the peak since the last read still holds the previous value"
    );
    assert!(c.take_peak_gain_reduction_db() <= c.gain_reduction_db() + 1e-6);
}

#[test]
fn out_of_range_and_non_finite_params_are_clamped_not_trusted() {
    let wild = CompressorParams {
        threshold_db: f32::NEG_INFINITY,
        ratio: f32::NAN,
        knee_db: 1.0e9,
        attack_ms: -3.0,
        release_ms: f32::INFINITY,
        makeup_db: 1.0e9,
        mix_pct: f32::NAN,
    };
    let mut c = compressor(&wild, FS);
    assert_eq!(c.params(), wild.sanitized());
    let mut rng = Xorshift32(99);
    for _ in 0..10_000 {
        let (a, b) = c.tick(rng.unit(), rng.unit());
        assert!(a.is_finite() && b.is_finite());
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Time behaviour: steady sine and step responses

/// First frame (1-based count) at which `meter()` reaches `target`, advancing the compressor with `input`.
fn frames_until(c: &mut Compressor, input: f32, target: f32, rising: bool, limit: usize) -> usize {
    for n in 1..=limit {
        c.tick(input, input);
        let gr = c.gain_reduction_db();
        if (rising && gr >= target) || (!rising && gr <= target) {
            return n;
        }
    }
    panic!("the gain reduction never crossed {target} within {limit} frames");
}

/// Gain-reduction attack: DC step from silence to 0.5 (-6.02 dBFS) into T = -24 dB, 4:1, hard knee. The static
/// reduction at that level is 13.485 dB.
fn attack_t63_samples(attack_ms: f32, release_ms: f32, fs: f32, amp: f32) -> (usize, f64) {
    let p = params(-24.0, 4.0, 0.0, attack_ms, release_ms);
    let mut c = compressor(&p, fs);
    let final_gr = -f64::from(StaticCurve::new(-24.0, 4.0, 0.0).gain_db(db(f64::from(amp)) as f32));
    let target = ((1.0 - libm::exp(-1.0)) * final_gr) as f32;
    (frames_until(&mut c, amp, target, true, 5_000_000), final_gr)
}

/// Release: hold the step long enough to settle, drop to silence, count frames until 63.2 % of the reduction is gone.
fn release_t63_samples(attack_ms: f32, release_ms: f32, fs: f32, amp: f32) -> (usize, f32) {
    let p = params(-24.0, 4.0, 0.0, attack_ms, release_ms);
    let mut c = compressor(&p, fs);
    for _ in 0..(fs as usize * 4) {
        c.tick(amp, amp);
    }
    let start = c.gain_reduction_db();
    let target = ((libm::exp(-1.0)) * f64::from(start)) as f32;
    (frames_until(&mut c, 0.0, target, false, 20_000_000), start)
}

#[test]
fn step_response_attack_63_percent_times() {
    // Documented rule (docs/dsp/compressor.md): for a step 18 dB over a hard threshold the gain reduction reaches
    // 63.2 % of its final value after 1.25 attack times, because the gain smoother (time constant attack/2) is
    // cascaded behind the follower. Golden sample counts at 48 kHz, ±1.
    #[rustfmt::skip]
    let goldens: &[(f32, usize)] = &[(1.0, 60), (5.0, 301), (10.0, 601), (30.0, 1804), (100.0, 6013)];
    for &(attack_ms, golden) in goldens {
        let (n, final_gr) = attack_t63_samples(attack_ms, 100.0, FS, 0.5);
        assert!((final_gr - 13.484).abs() < 0.001);
        assert!(
            n.abs_diff(golden) <= 1,
            "attack {attack_ms} ms: {n} samples, golden {golden}"
        );
        let ratio = n as f64 / (f64::from(attack_ms) * 48.0);
        assert!(
            (ratio - 1.25).abs() < 0.02,
            "attack {attack_ms} ms: t63 is {ratio:.3} attack times"
        );
    }
}

#[test]
fn step_response_release_63_percent_times() {
    // After release the envelope falls 8.686 dB per release time, linear in dB, so the reduction (a fixed share of the
    // level above the threshold) recovers 0.632·d/8.686 release times after the attack/2 smoother lag.
    #[rustfmt::skip]
    let goldens: &[(f32, f32, usize)] = &[(1.0, 100.0, 6305), (5.0, 100.0, 6401), (10.0, 200.0, 12801), (30.0, 200.0, 13281), (100.0, 1000.0, 65204)];
    for &(attack_ms, release_ms, golden) in goldens {
        let (n, _) = release_t63_samples(attack_ms, release_ms, FS, 0.5);
        assert!(
            n.abs_diff(golden) <= 1,
            "attack {attack_ms} release {release_ms}: {n} samples, golden {golden}"
        );
        let d = db(0.5) + 24.0; // dB over the threshold when the signal stops
        let predicted_ms = (1.0 - libm::exp(-1.0)) * d / (20.0 * libm::log10(core::f64::consts::E))
            * f64::from(release_ms)
            + 0.5 * f64::from(attack_ms);
        let measured_ms = n as f64 / 48.0;
        assert!(
            (measured_ms - predicted_ms).abs() <= 0.005 * predicted_ms + 0.1,
            "release {release_ms} ms: measured {measured_ms:.2} ms, predicted {predicted_ms:.2} ms"
        );
    }
}

#[test]
fn a_release_shorter_than_half_the_attack_is_not_reachable() {
    // The gain smoother runs at attack/2 for falls too (JOS's kneesmooth), so with a 100 ms attack a 10 ms release
    // still takes about 61 ms to recover 63 %: never less than attack/2, and within 5 % of the rule of thumb.
    let (n, _) = release_t63_samples(100.0, 10.0, FS, 0.5);
    let measured_ms = n as f64 / 48.0;
    let rule_ms = (1.0 - libm::exp(-1.0)) * (db(0.5) + 24.0) / 8.685_889_6 * 10.0 + 50.0;
    assert!(measured_ms >= 50.0, "{measured_ms}");
    assert!((measured_ms - 60.7).abs() < 0.2, "{measured_ms}");
    assert!(
        (measured_ms - rule_ms).abs() <= 0.05 * rule_ms,
        "{measured_ms} vs rule {rule_ms}"
    );
}

#[test]
fn step_responses_do_not_depend_on_the_sample_rate() {
    for &fs in &[44_100.0_f32, 96_000.0, 192_000.0] {
        let (a, _) = attack_t63_samples(10.0, 200.0, fs, 0.5);
        let (r, _) = release_t63_samples(10.0, 200.0, fs, 0.5);
        let (a_ms, r_ms) = (
            a as f64 / f64::from(fs) * 1000.0,
            r as f64 / f64::from(fs) * 1000.0,
        );
        assert!((a_ms - 12.52).abs() < 0.05, "{fs}: attack t63 {a_ms} ms");
        assert!((r_ms - 266.7).abs() < 0.5, "{fs}: release t63 {r_ms} ms");
    }
}

#[test]
fn the_f32_core_follows_an_f64_reference_of_the_same_equations() {
    // Trajectory agreement over a bursty signal: the gain reduction stays within a few 1e-4 dB of a double-precision
    // evaluation of the documented difference equations.
    let p = params(-30.0, 5.0, 8.0, 4.0, 120.0);
    let mut c = compressor(&p, FS);
    let mut reference = Reference::new(&p, f64::from(FS));
    let mut rng = Xorshift32(2024);
    let mut worst = 0.0_f64;
    for n in 0..96_000 {
        let burst = if (n / 6000) % 2 == 0 { 0.9 } else { 0.02 };
        let (l, r) = (burst * rng.unit(), burst * rng.unit());
        c.tick(l, r);
        let e = f64::from(l.abs().max(r.abs()));
        let expected = reference.tick(e);
        worst = worst.max((f64::from(c.gain_reduction_db()) - expected).abs());
    }
    assert!(
        worst < 1e-3,
        "worst gain reduction difference from the f64 reference: {worst} dB"
    );
}

#[test]
fn the_step_response_matches_the_f64_reference_trajectory() {
    for &(att, rel, amp) in &[
        (10.0_f32, 200.0_f32, 0.5_f32),
        (1.0, 50.0, 1.0),
        (30.0, 400.0, 0.1),
    ] {
        let p = params(-24.0, 4.0, 0.0, att, rel);
        let mut c = compressor(&p, FS);
        let mut reference = Reference::new(&p, f64::from(FS));
        let mut worst = 0.0_f64;
        for n in 0..(FS as usize * 2) {
            let x = if n < FS as usize { amp } else { 0.0 };
            c.tick(x, x);
            worst =
                worst.max((f64::from(c.gain_reduction_db()) - reference.tick(f64::from(x))).abs());
        }
        assert!(
            worst < 1e-3,
            "attack {att} release {rel} amp {amp}: worst difference {worst} dB"
        );
    }
}

#[test]
fn a_steady_sine_settles_just_under_the_static_reduction_at_its_peak() {
    // 1 kHz sine at -6.02 dBFS into T = -24 dB, 4:1, hard knee, 10 ms / 200 ms. The static reduction at the peak
    // is 13.484 dB; the follower sits a little below the peak between cycles, so the reduction is a little less.
    let p = params(-24.0, 4.0, 0.0, 10.0, 200.0);
    let mut c = compressor(&p, FS);
    let mut reference = Reference::new(&p, f64::from(FS));
    let total = FS as usize * 3;
    let (mut gr_min, mut gr_max, mut gr_sum, mut out_peak) = (f32::MAX, 0.0_f32, 0.0_f64, 0.0_f32);
    let mut worst = 0.0_f64;
    for n in 0..total {
        let x = sine(0.5, 1000.0, f64::from(FS), n);
        let (y, y_r) = c.tick(x, x);
        assert_eq!(y.to_bits(), y_r.to_bits());
        let expected = reference.tick(f64::from(x.abs()));
        if n >= total - FS as usize {
            let gr = c.gain_reduction_db();
            gr_min = gr_min.min(gr);
            gr_max = gr_max.max(gr);
            gr_sum += f64::from(gr);
            out_peak = out_peak.max(y.abs());
            worst = worst.max((f64::from(gr) - expected).abs());
        }
    }
    let mean = gr_sum / f64::from(FS);
    assert!(worst < 1e-3, "difference from the f64 reference: {worst}");
    assert!(
        gr_max <= 13.485,
        "never more than the static reduction at the peak: {gr_max}"
    );
    assert!(gr_min > 12.5, "{gr_min}");
    // Report 02 measured 12.93 dB for a sine at exactly -6 dBFS (13.5 dB static); this one is 0.02 dB quieter.
    assert!((mean - 12.91).abs() < 0.01, "mean gain reduction {mean} dB");
    assert!(gr_max - gr_min < 0.05, "ripple {} dB", gr_max - gr_min);
    let out_db = db(f64::from(out_peak));
    assert!(
        (out_db - (-6.0206 - mean)).abs() < 0.05,
        "output peak {out_db} dBFS"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Determinism: one hash on every platform

/// Hash of the scenario below. The same constant must hold natively (debug and release, `-C target-cpu=native`) and
/// on `wasm32-wasip1` under wasmtime (docs/dsp/compressor.md has the commands).
const HASH_GOLDEN: u64 = 0xc1e3_9483_7f03_ee91;

fn scenario_hash() -> u64 {
    const SEGMENT: usize = 12_288; // 192 blocks, 0.256 s
    const BLOCK: usize = 64;
    let mut c = Compressor::new();
    c.prepare(FS);
    let mut rng = Xorshift32(0x0e17_5eed);
    let mut hash = Fnv::new();
    let mut n = 0_usize;
    let mut block_index = 0_u32;
    let sample_sets = [
        params(-24.0, 4.0, 6.0, 10.0, 200.0),
        params(-40.0, 12.0, 0.0, 0.5, 40.0),
        params(-12.0, 2.0, 24.0, 40.0, 1000.0),
        params(-30.0, 20.0, 12.0, 0.0, 0.0),
    ];
    let mut in_l = [0.0_f32; BLOCK];
    let mut in_r = [0.0_f32; BLOCK];
    let mut out_l = [0.0_f32; BLOCK];
    let mut out_r = [0.0_f32; BLOCK];
    for segment in 0..8_usize {
        for _ in 0..SEGMENT / BLOCK {
            for (i, (slot_l, slot_r)) in in_l.iter_mut().zip(in_r.iter_mut()).enumerate() {
                let t = n + i;
                let (l, r) = match segment {
                    0 => (
                        sine(0.7, 220.0, f64::from(FS), t),
                        sine(0.35, 331.0, f64::from(FS), t),
                    ),
                    1 => (0.0, 0.0),
                    2 => (0.5 * rng.unit(), 0.5 * rng.unit()),
                    3 => {
                        // 10 ms bursts of a 1 kHz sine every 50 ms over a -80 dBFS floor
                        let on = (t % 2400) < 480;
                        (
                            if on {
                                sine(0.9, 1000.0, f64::from(FS), t)
                            } else {
                                1e-4 * rng.unit()
                            },
                            1e-4 * rng.unit(),
                        )
                    }
                    4 => {
                        // a 60 dB amplitude ramp on 440 Hz
                        let k = (t - 4 * SEGMENT) as f64 / SEGMENT as f64;
                        (
                            sine(1.0, 440.0, f64::from(FS), t) * lin(-60.0 * (1.0 - k)) as f32,
                            0.0,
                        )
                    }
                    5 => (
                        if (t - 5 * SEGMENT) < SEGMENT / 2 {
                            0.7
                        } else {
                            0.0
                        },
                        0.0,
                    ),
                    6 => (
                        if (t - 6 * SEGMENT) < SEGMENT / 2 {
                            1.0e-40
                        } else {
                            0.0
                        },
                        1.0e-39,
                    ),
                    _ => (
                        if (t / 100).is_multiple_of(2) {
                            1.0
                        } else {
                            -1.0
                        },
                        0.9 * rng.unit(),
                    ),
                };
                *slot_l = l;
                *slot_r = r;
            }
            // Settings change every block: a slow threshold sweep and a different set every 7th block.
            let mut p = sample_sets[(block_index as usize / 7) % sample_sets.len()];
            p.threshold_db += 6.0 * libm::sinf(block_index as f32 * 0.05);
            p.makeup_db = 3.0 * libm::cosf(block_index as f32 * 0.031);
            p.mix_pct = 100.0 - 25.0 * f32::from(u8::from(block_index.is_multiple_of(13)));
            c.set_params(&p);
            c.process(&in_l, &in_r, &mut out_l, &mut out_r);
            for (&a, &b) in out_l.iter().zip(&out_r) {
                hash.f32(a);
                hash.f32(b);
            }
            hash.f32(c.gain_reduction_db());
            hash.f32(c.take_peak_gain_reduction_db());
            n += BLOCK;
            block_index += 1;
        }
    }
    hash.0
}

#[test]
fn render_hash_is_the_same_on_every_platform() {
    let h = scenario_hash();
    // Visible with `--nocapture`, so a native run and a wasmtime run can be compared by eye or by `diff`.
    println!("compressor-hash {h:016x}");
    assert_eq!(
        h, HASH_GOLDEN,
        "compressor render hash changed: update HASH_GOLDEN only for a deliberate change"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Real time

#[test]
fn process_allocates_nothing() {
    let mut c = Compressor::new();
    c.prepare(FS);
    let mut rng = Xorshift32(5);
    let l: Vec<f32> = (0..1024).map(|_| rng.unit()).collect();
    let r: Vec<f32> = (0..1024).map(|_| rng.unit()).collect();
    let (mut out_l, mut out_r) = (vec![0.0_f32; 1024], vec![0.0_f32; 1024]);
    let (mut io_l, mut io_r) = (l.clone(), r.clone());
    let p = params(-30.0, 6.0, 9.0, 2.0, 90.0);
    let wild = CompressorParams {
        threshold_db: f32::NAN,
        ratio: -1.0,
        ..p
    };
    let mut sink = 0.0_f32;
    assert_no_alloc(|| {
        for round in 0..64_u32 {
            // Changing settings, resetting and preparing happen on the audio thread too.
            let mut q = p;
            q.threshold_db -= round as f32 * 0.25;
            c.set_params(&q);
            c.process(&l, &r, &mut out_l, &mut out_r);
            c.process_in_place(&mut io_l, &mut io_r);
            sink += c.gain_reduction_db() + c.take_peak_gain_reduction_db();
            if round % 16 == 15 {
                c.set_params(&wild);
                c.reset();
                c.prepare(FS * 2.0);
                c.prepare(FS);
            }
        }
        for (&x, &y) in l.iter().zip(&r) {
            let (a, b) = c.tick(x, y);
            sink += a + b;
        }
    });
    assert!(sink.is_finite());
}
