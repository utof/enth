//! Contract tests of `enth_dsp::ott` (S2a lane OTT): what the module *is*, measured against closed forms and not
//! against a previous render. The Linkwitz–Riley split against its transfer functions, the recombined bands against a
//! flat magnitude (and the phase of the two allpasses), the per-band static curves against the documented equations
//! and a steady sine's output level against them, depth as intensity, mix, silence, NaN, a 40 s tail, latency, the
//! native = wasm32 hash and `assert_no_alloc`. Timing of the follower (63 % times) is in the module's own tests.
//!
//! Test oracles are written in `f64` with `libm` (the workspace bans `std`'s transcendentals everywhere, tests too).
//! `cargo test -p enth-dsp --test ott -- --nocapture` prints the tables `docs/dsp/ott.md` records.
#![allow(
    clippy::print_stderr,
    reason = "the sweeps print the tables docs/dsp/ott.md records"
)]

use assert_no_alloc::assert_no_alloc;
use core::fmt::Write as _;
use enth_dsp::ott::{
    BAND_SPECS, BANDS, BandCurve, KNEE_DB, LIFT_CEILING_ABOVE_THRESHOLD_DB, Ott, OttParams,
    UP_CAP_DB, UP_FADE_BOTTOM_DB, UP_FADE_TOP_DB,
};

// `AllocDisabler` exists only in debug builds (assert_no_alloc's `disable_release` default): release builds run the
// closure unchecked.
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const FS: f32 = 48_000.0;
const FSD: f64 = 48_000.0;
const TAU: f64 = core::f64::consts::TAU;
const SEG: usize = 32;

// ---------------------------------------------------------------------------------------------------------------
// Helpers

type C = (f64, f64);

fn cmul(a: C, b: C) -> C {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}
fn cdiv(a: C, b: C) -> C {
    let d = b.0 * b.0 + b.1 * b.1;
    ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
}
fn cabs(a: C) -> f64 {
    libm::hypot(a.0, a.1)
}
fn carg(a: C) -> f64 {
    libm::atan2(a.1, a.0)
}
fn db(x: f64) -> f64 {
    20.0 * libm::log10(x)
}
fn undb(x: f64) -> f64 {
    libm::pow(10.0, x / 20.0)
}

/// The analog Butterworth section `1/(s² + √2·s + 1)` at `s = j·u` (the bilinear frequency `u = tan(πf/fs) /
/// tan(πfc/fs)`): the TPT state-variable filter's exact response.
fn h2(f: f64, fc: f64) -> C {
    let u =
        libm::tan(core::f64::consts::PI * f / FSD) / libm::tan(core::f64::consts::PI * fc / FSD);
    cdiv((1.0, 0.0), (1.0 - u * u, core::f64::consts::SQRT_2 * u))
}
/// LR4 low-pass `H2²`.
fn lp4(f: f64, fc: f64) -> C {
    let h = h2(f, fc);
    cmul(h, h)
}
/// LR4 high-pass `u⁴·H2²`.
fn hp4(f: f64, fc: f64) -> C {
    let u =
        libm::tan(core::f64::consts::PI * f / FSD) / libm::tan(core::f64::consts::PI * fc / FSD);
    let u4 = u * u * u * u;
    let h = lp4(f, fc);
    (h.0 * u4, h.1 * u4)
}
/// The allpass LR4's two branches sum to: `(s² − √2·s + 1)/(s² + √2·s + 1)`.
fn ap2(f: f64, fc: f64) -> C {
    let h = h2(f, fc);
    let u =
        libm::tan(core::f64::consts::PI * f / FSD) / libm::tan(core::f64::consts::PI * fc / FSD);
    cmul((1.0 - u * u, -core::f64::consts::SQRT_2 * u), h)
}

fn params(f: impl FnOnce(&mut OttParams)) -> OttParams {
    let mut p = OttParams::DEFAULT;
    f(&mut p);
    p
}

fn ott_with(p: &OttParams) -> Ott {
    let mut o = Ott::new();
    o.prepare(FS);
    o.set_params(p);
    o
}

/// Demodulate `y` against `cos`/`sin` at `f` over whole cycles: the complex amplitude (a unit sine → 1 at phase 0)
/// of the component at `f`, with `n0` the sample index of `y[0]` (so phases are those of a sine from sample 0).
fn demod(y: &[f64], f: f64, n0: usize) -> C {
    let (mut re, mut im) = (0.0, 0.0);
    for (i, v) in y.iter().enumerate() {
        let t = TAU * f * (n0 + i) as f64 / FSD;
        re += v * libm::sin(t);
        im += v * libm::cos(t);
    }
    let k = 2.0 / y.len() as f64;
    // y = A·sin(t + φ) = A·cos φ·sin t + A·sin φ·cos t → re = A cos φ, im = A sin φ.
    (re * k, im * k)
}

/// Feed `amp·sin(2π f n/fs)` (both channels) through `o` in `SEG`-frame calls for `frames` frames and return the
/// complex amplitude of the left output at `f` over the last `measure` frames (whole cycles: `f` is an integer Hz and
/// `measure` a multiple of `fs`/gcd).
fn through(o: &mut Ott, f: f64, amp: f64, frames: usize, measure: usize) -> C {
    let mut out = Vec::with_capacity(measure);
    let mut l = [0.0_f32; SEG];
    let mut r = [0.0_f32; SEG];
    let mut n = 0;
    while n < frames {
        for i in 0..SEG {
            let x = (amp * libm::sin(TAU * f * (n + i) as f64 / FSD)) as f32;
            l[i] = x;
            r[i] = x;
        }
        o.process_in_place(&mut l, &mut r);
        if n + SEG > frames - measure {
            out.extend(l.iter().map(|&v| f64::from(v)));
        }
        n += SEG;
    }
    // `out` may start up to SEG frames early; keep exactly the last `measure` frames.
    let start = out.len() - measure;
    demod(&out[start..], f, frames - measure)
}

/// The log-spaced integer frequencies 20 Hz … 20 kHz.
fn freqs(n: usize) -> Vec<f64> {
    let mut v: Vec<f64> = (0..n)
        .map(|i| libm::round(20.0 * libm::pow(1000.0, i as f64 / (n - 1) as f64)))
        .collect();
    v.extend([
        20.0, 50.0, 100.0, 250.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0, 20_000.0,
    ]);
    v.sort_by(f64::total_cmp);
    v.dedup();
    v
}

// ---------------------------------------------------------------------------------------------------------------
// The split

#[test]
fn each_band_is_the_linkwitz_riley_response_of_its_closed_form() {
    for &(f1, f2) in &[
        (88.3_f32, 2_500.0_f32),
        (200.0, 6_000.0),
        (40.0, 500.0),
        (500.0, 10_000.0),
    ] {
        let p = params(|p| {
            p.low_split_hz = f1;
            p.high_split_hz = f2;
        });
        let (f1, f2) = (f64::from(f1), f64::from(f2));
        let mut worst = (0.0_f64, 0.0_f64);
        for f in freqs(41) {
            let mut o = ott_with(&p);
            // The split is linear: drive it with a sine, read the three bands with `split_frame`.
            let n = 96_000;
            let mut b: [Vec<f64>; BANDS] = [Vec::new(), Vec::new(), Vec::new()];
            for i in 0..n {
                let x = libm::sin(TAU * f * i as f64 / FSD);
                let s = o.split_frame(x, x);
                if i >= n - 48_000 {
                    for (k, v) in b.iter_mut().enumerate() {
                        v.push(s[k][0]);
                    }
                }
            }
            let want = [
                cmul(lp4(f, f1), ap2(f, f2)),
                cmul(hp4(f, f1), lp4(f, f2)),
                cmul(hp4(f, f1), hp4(f, f2)),
            ];
            for k in 0..BANDS {
                let got = demod(&b[k], f, n - 48_000);
                let err = cabs((got.0 - want[k].0, got.1 - want[k].1));
                worst.0 = worst.0.max(err);
                assert!(
                    err < 2e-7,
                    "split {f1}/{f2}, band {k} at {f} Hz: got {got:?}, want {:?} (|err| {err:e})",
                    want[k]
                );
            }
            worst.1 = worst.1.max(0.0);
        }
        eprintln!(
            "LR4 bands vs closed form, splits {f1}/{f2}: worst complex error {:.2e}",
            worst.0
        );
    }
}

#[test]
fn the_unity_bands_sum_flat_within_a_tenth_of_a_db_and_the_phase_is_two_allpasses() {
    // depth 0: every band at 0 dB, so the module is the split and the sum alone.
    for &(f1, f2) in &[
        (88.3_f32, 2_500.0_f32),
        (40.0, 500.0),
        (500.0, 10_000.0),
        (250.0, 1_500.0),
    ] {
        let p = params(|p| {
            p.depth = 0.0;
            p.low_split_hz = f1;
            p.high_split_hz = f2;
        });
        let mut worst_db = 0.0_f64;
        let mut worst_phase = 0.0_f64;
        let mut table = String::new();
        for f in freqs(61) {
            let mut o = ott_with(&p);
            let got = through(&mut o, f, 0.5, 96_000, 48_000);
            let g = cabs(got) / 0.5;
            let want = cmul(ap2(f, f64::from(f1)), ap2(f, f64::from(f2)));
            let dphase = {
                let d = carg(got) - carg(want);
                libm::atan2(libm::sin(d), libm::cos(d)).abs()
            };
            worst_db = worst_db.max(db(g).abs());
            worst_phase = worst_phase.max(dphase);
            assert!(db(g).abs() < 0.1, "{f1}/{f2} at {f} Hz: {:.4} dB", db(g));
            assert!(
                dphase < 1e-3,
                "{f1}/{f2} at {f} Hz: phase off by {dphase} rad"
            );
            if [
                20.0, 50.0, 100.0, 250.0, 1000.0, 2500.0, 5000.0, 10000.0, 20000.0,
            ]
            .contains(&f)
            {
                let _ = writeln!(
                    table,
                    "  {f:>7.0} Hz  {:+.5} dB  {:7.1} deg",
                    db(g),
                    carg(want).to_degrees()
                );
            }
            for k in 0..BANDS {
                assert_eq!(
                    o.band_gain_db(k),
                    0.0,
                    "depth 0: band {k} gain is exactly 0 dB"
                );
            }
        }
        eprintln!(
            "flat sum, splits {f1}/{f2}: worst magnitude error {worst_db:.5} dB, phase vs two allpasses {worst_phase:.2e} rad\n{table}"
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The static curves

/// The documented curve in `f64`, written from the equations (Giannoulis et al. 2012, eq. 4, mirrored for upward).
fn curve_db(spec: &enth_dsp::ott::BandSpec, level: f64, a_down: f64, a_up: f64) -> f64 {
    let w = f64::from(KNEE_DB);
    // Reduction/boost for `d` dB beyond a threshold with slope `s = 1/R − 1` (<= 0): g = s·(d + w/2)²/(2w) in the knee,
    // s·d above it, 0 below it.
    let g = |d: f64, ratio: f64| {
        let s = 1.0 / ratio - 1.0;
        if 2.0 * d <= -w {
            0.0
        } else if 2.0 * d < w {
            s * (d + 0.5 * w) * (d + 0.5 * w) / (2.0 * w)
        } else {
            s * d
        }
    };
    let down = g(
        level - f64::from(spec.down_threshold_db),
        f64::from(spec.down_ratio),
    );
    let lift = -g(
        f64::from(spec.up_threshold_db) - level,
        f64::from(spec.up_ratio),
    );
    let lift = lift.min(f64::from(UP_CAP_DB));
    let fade = ((level - f64::from(UP_FADE_BOTTOM_DB))
        / f64::from(UP_FADE_TOP_DB - UP_FADE_BOTTOM_DB))
    .clamp(0.0, 1.0);
    a_down * down + a_up * lift * fade
}

#[test]
fn the_static_curve_is_the_documented_equation() {
    let mut worst = 0.0_f64;
    for (b, spec) in BAND_SPECS.iter().enumerate() {
        let c = BandCurve::new(spec);
        for i in 0..=14_000 {
            let level = -140.0 + f64::from(i) * 0.01;
            for &(ad, au) in &[(1.0, 1.0), (0.3, 0.3), (1.0, 0.0), (0.0, 1.0), (0.0, 0.0)] {
                let got = f64::from(c.gain_db(level as f32, ad as f32, au as f32));
                let want = curve_db(spec, f64::from(level as f32), ad, au);
                worst = worst.max((got - want).abs());
                assert!(
                    (got - want).abs() < 2e-5,
                    "band {b}, {level} dB, a=({ad}, {au}): {got} vs {want}"
                );
            }
        }
    }
    eprintln!("static curve vs the documented equation: worst error {worst:.2e} dB");
    // The properties the doc states.
    for spec in &BAND_SPECS {
        let c = BandCurve::new(spec);
        assert!(
            c.down_db(-200.0) == 0.0 && c.up_db(-200.0) == 0.0,
            "silence is not lifted"
        );
        assert!(c.up_db(UP_FADE_BOTTOM_DB) == 0.0, "no lift at the floor");
        assert!(
            c.up_db(-60.0) > 0.0 && c.up_db(-20.0) == 0.0,
            "lift only below the upward threshold"
        );
        assert!(
            c.down_db(-5.0) < 0.0 && c.down_db(-60.0) == 0.0,
            "reduction only above the downward threshold"
        );
        for l in (-120..=24).map(|l| l as f32 * 0.5 - 20.0) {
            assert!(c.up_db(l) <= UP_CAP_DB && c.up_db(l) >= 0.0 && c.down_db(l) <= 0.0);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// A steady sine through each band

/// The compressor's follower in `f64` on `amp·|sin|` sampled like the module sees it: the level (dB) a steady sine
/// settles at. The reference for "what the detector reads" (it sits about 1 dB under the peak, the same offset the
/// compressor's documentation records), written from the equations and independent of the module's code.
fn follower_level_db(f: f64, amp: f64, attack_ms: f64, release_ms: f64) -> f64 {
    let ka = 1.0 - libm::exp(-1.0 / (attack_ms * 1e-3 * FSD));
    let kr = 1.0 - libm::exp(-1.0 / (release_ms * 1e-3 * FSD));
    let n = (FSD * 4.0) as usize;
    let mut env = 0.0;
    let (mut sum, mut cnt) = (0.0, 0);
    // Average over the last whole number of cycles (at most one second).
    let cycles = libm::floor(f).max(1.0);
    let len = (cycles * FSD / f) as usize;
    for i in 0..n {
        let e = (amp * libm::sin(TAU * f * i as f64 / FSD)).abs();
        env += (if e > env { ka } else { kr }) * (e - env);
        if i >= n - len {
            sum += env;
            cnt += 1;
        }
    }
    db(sum / f64::from(cnt))
}

/// Which band a test sine lives in, with the splits that isolate it (the neighbours see it at least 80 dB down, so
/// the band's own curve is all that shapes the output).
struct Probe {
    band: usize,
    name: &'static str,
    hz: f64,
    low_split: f32,
    high_split: f32,
}

const PROBES: [Probe; 3] = [
    Probe {
        band: 0,
        name: "low",
        hz: 30.0,
        low_split: 500.0,
        high_split: 10_000.0,
    },
    Probe {
        band: 1,
        name: "mid",
        hz: 1_000.0,
        low_split: 40.0,
        high_split: 10_000.0,
    },
    Probe {
        band: 2,
        name: "high",
        hz: 15_000.0,
        low_split: 40.0,
        high_split: 500.0,
    },
];

/// The output level (dB) of a settled sine of `p_db` dBFS.
fn settled_out_db(o: &mut Ott, probe: &Probe, p_db: f64, settle: usize, measure: usize) -> f64 {
    let amp = undb(p_db);
    let c = through(o, probe.hz, amp, settle + measure, measure);
    db(cabs(c))
}

#[test]
fn a_steady_sine_in_each_band_follows_the_documented_curve() {
    let levels: Vec<f64> = (0..=25).map(|i| -100.0 + 4.0 * f64::from(i)).collect();
    for probe in &PROBES {
        let spec = &BAND_SPECS[probe.band];
        let p = params(|p| {
            p.depth = 100.0;
            p.low_split_hz = probe.low_split;
            p.high_split_hz = probe.high_split;
        });
        let mut table = format!(
            "{} band, {} Hz, depth 100:\n  in dBFS   out dBFS   gain dB   curve dB   error dB\n",
            probe.name, probe.hz
        );
        let mut worst = 0.0_f64;
        for direction in 0..2 {
            let mut o = ott_with(&p);
            let order: Vec<f64> = if direction == 0 {
                levels.clone()
            } else {
                levels.iter().rev().copied().collect()
            };
            for &lvl in &order {
                let out = settled_out_db(&mut o, probe, lvl, 120_000, 24_000);
                let seen = follower_level_db(
                    probe.hz,
                    undb(lvl),
                    f64::from(spec.attack_ms),
                    f64::from(spec.release_ms),
                );
                let want = curve_db(spec, seen, 1.0, 1.0);
                let err = (out - lvl) - want;
                worst = worst.max(err.abs());
                if direction == 0 {
                    let _ = writeln!(
                        table,
                        "  {lvl:8.1}  {out:9.2}  {:8.2}  {want:9.2}  {err:9.3}",
                        out - lvl
                    );
                }
                assert!(
                    err.abs() < 0.5,
                    "{} band at {lvl} dBFS ({direction}): out {out}, want gain {want}",
                    probe.name
                );
            }
        }
        eprintln!("{table}  worst error {worst:.3} dB (rising and falling sweeps)\n");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Intensity, the plain gains and mix

/// A mid-band sine at `p_db` through the mid probe's splits with `set` applied: its settled output gain in dB.
fn mid_gain_with(p_db: f64, set: impl FnOnce(&mut OttParams)) -> f64 {
    let probe = &PROBES[1];
    let p = params(|p| {
        p.low_split_hz = probe.low_split;
        p.high_split_hz = probe.high_split;
        set(p);
    });
    let mut o = ott_with(&p);
    settled_out_db(&mut o, probe, p_db, 96_000, 24_000) - p_db
}

#[test]
fn depth_scales_both_curves_linearly_in_db_and_the_two_knobs_pick_one() {
    let spec = &BAND_SPECS[1];
    // -60 dBFS is in the upward region, -4 dBFS in the downward one (the detector reads about 1 dB lower).
    let seen = |p: f64| {
        follower_level_db(
            PROBES[1].hz,
            undb(p),
            f64::from(spec.attack_ms),
            f64::from(spec.release_ms),
        )
    };
    let full_up = curve_db(spec, seen(-60.0), 0.0, 1.0);
    let full_down = curve_db(spec, seen(-4.0), 1.0, 0.0);
    assert!(full_up > 15.0 && full_down < -5.0, "{full_up} {full_down}");
    let mut worst = 0.0_f64;
    for &depth in &[0.0_f32, 10.0, 30.0, 50.0, 75.0, 100.0] {
        let d = f64::from(depth) / 100.0;
        let up = mid_gain_with(-60.0, |p| p.depth = depth);
        let down = mid_gain_with(-4.0, |p| p.depth = depth);
        worst = worst
            .max((up - d * full_up).abs())
            .max((down - d * full_down).abs());
        assert!(
            (up - d * full_up).abs() < 0.06,
            "depth {depth}: upward {up} vs {}",
            d * full_up
        );
        assert!(
            (down - d * full_down).abs() < 0.06,
            "depth {depth}: downward {down} vs {}",
            d * full_down
        );
    }
    // `upward` and `downward` scale their own curve (times depth) and leave the other alone.
    for &k in &[0.0_f32, 25.0, 60.0] {
        let kd = f64::from(k) / 100.0;
        let up = mid_gain_with(-60.0, |p| {
            p.depth = 100.0;
            p.upward = k;
        });
        let up_other = mid_gain_with(-60.0, |p| {
            p.depth = 100.0;
            p.downward = k;
        });
        let down = mid_gain_with(-4.0, |p| {
            p.depth = 100.0;
            p.downward = k;
        });
        let down_other = mid_gain_with(-4.0, |p| {
            p.depth = 100.0;
            p.upward = k;
        });
        assert!((up - kd * full_up).abs() < 0.06, "upward {k}: {up}");
        assert!(
            (up_other - full_up).abs() < 0.06,
            "downward {k} must not touch the lift: {up_other}"
        );
        assert!((down - kd * full_down).abs() < 0.06, "downward {k}: {down}");
        assert!(
            (down_other - full_down).abs() < 0.06,
            "upward {k} must not touch the reduction: {down_other}"
        );
    }
    eprintln!(
        "depth and the knobs: gain is depth x curve, worst deviation {worst:.3} dB (upward {full_up:.2} dB at -60, downward {full_down:.2} dB at -4)"
    );
}

#[test]
fn the_plain_gains_are_plain() {
    // depth 0 leaves the bands alone, so each gain is its own dB, and they add.
    for (name, set, want) in [
        (
            "input +6",
            (|p: &mut OttParams| p.input_db = 6.0) as fn(&mut OttParams),
            6.0,
        ),
        ("output -9", |p| p.output_db = -9.0, -9.0),
        ("mid_gain +4", |p| p.mid_gain_db = 4.0, 4.0),
        (
            "input +6 output -9 mid +4",
            |p| {
                p.input_db = 6.0;
                p.output_db = -9.0;
                p.mid_gain_db = 4.0;
            },
            1.0,
        ),
    ] {
        let g = mid_gain_with(-30.0, |p| {
            p.depth = 0.0;
            set(p);
        });
        assert!((g - want).abs() < 0.02, "{name}: {g} dB, want {want}");
    }
    // The other bands' gains do not reach a mid sine (the splits isolate it).
    let g = mid_gain_with(-30.0, |p| {
        p.depth = 0.0;
        p.low_gain_db = 12.0;
        p.high_gain_db = -12.0;
    });
    assert!(g.abs() < 0.05, "{g}");
    // Each band's gain on its own sine.
    for probe in &PROBES {
        let p = params(|p| {
            p.depth = 0.0;
            p.low_split_hz = probe.low_split;
            p.high_split_hz = probe.high_split;
            match probe.band {
                0 => p.low_gain_db = -7.0,
                1 => p.mid_gain_db = -7.0,
                _ => p.high_gain_db = -7.0,
            }
        });
        let mut o = ott_with(&p);
        let g = settled_out_db(&mut o, probe, -20.0, 96_000, 24_000) + 20.0;
        assert!((g + 7.0).abs() < 0.05, "{} band gain -7: {g}", probe.name);
    }
}

fn noise(seed: u32, n: usize, amp: f32) -> Vec<f32> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            amp * ((x >> 8) as f32 / 8_388_608.0 - 1.0)
        })
        .collect()
}

fn run(p: &OttParams, l: &[f32], r: &[f32], chunk: usize) -> (Vec<f32>, Vec<f32>) {
    let mut o = ott_with(p);
    let (mut yl, mut yr) = (l.to_vec(), r.to_vec());
    for (a, b) in yl.chunks_mut(chunk).zip(yr.chunks_mut(chunk)) {
        o.process_in_place(a, b);
    }
    (yl, yr)
}

#[test]
fn mix_zero_is_the_input_bit_for_bit_and_mix_blends_the_two() {
    let (l, r) = (noise(1, 20_000, 0.8), noise(2, 20_000, 0.5));
    for depth in [0.0, 30.0, 100.0] {
        let zero = params(|p| {
            p.mix = 0.0;
            p.depth = depth;
        });
        let (yl, yr) = run(&zero, &l, &r, SEG);
        assert!(
            yl.iter()
                .zip(&l)
                .chain(yr.iter().zip(&r))
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "depth {depth}: mix 0 passes the input bit for bit"
        );
        let full = params(|p| p.depth = depth);
        let half = params(|p| {
            p.depth = depth;
            p.mix = 50.0;
        });
        let (wl, _) = run(&full, &l, &r, SEG);
        let (hl, _) = run(&half, &l, &r, SEG);
        for i in 0..l.len() {
            assert_eq!(hl[i], l[i] * 0.5 + wl[i] * 0.5, "mix 50 % at frame {i}");
        }
    }
}

#[test]
fn a_dry_blend_notches_the_crossovers_which_is_why_depth_is_not_dry_wet() {
    // depth 0 and unity gains: the wet path is A1·A2·x, so mix 50 % gives (1 + A1·A2)/2 against the input: a null where
    // the two allpasses together turn the phase by 180 degrees. Measured against that closed form.
    let (f1, f2) = (88.3_f64, 2_500.0_f64);
    let p = params(|p| {
        p.depth = 0.0;
        p.mix = 50.0;
    });
    let mut out = String::new();
    for f in [
        50.0, 80.0, 88.0, 100.0, 200.0, 1_000.0, 2_000.0, 2_500.0, 3_000.0, 8_000.0,
    ] {
        let mut o = ott_with(&p);
        let got = through(&mut o, f, 0.5, 96_000, 48_000);
        let h = cmul(ap2(f, f1), ap2(f, f2));
        let want = cabs((f64::midpoint(1.0, h.0), 0.5 * h.1));
        let g = cabs(got) / 0.5;
        let _ = writeln!(
            out,
            "  {f:>7.0} Hz  measured {:+8.2} dB  closed form {:+8.2} dB",
            db(g),
            db(want)
        );
        assert!(
            (db(g) - db(want)).abs() < 0.1 || (g < 0.01 && want < 0.01),
            "{f} Hz: {} vs {}",
            db(g),
            db(want)
        );
    }
    eprintln!("mix 50 % at depth 0 (the dry blend of a band split):\n{out}");
}

// ---------------------------------------------------------------------------------------------------------------
// Silence, bad input, the tail, latency, block size

#[test]
fn silence_in_gives_exact_zeros_for_any_setting() {
    for p in [
        OttParams::DEFAULT,
        params(|p| {
            p.depth = 100.0;
            p.input_db = 24.0;
            p.output_db = 24.0;
            p.low_gain_db = 24.0;
            p.mid_gain_db = 24.0;
            p.high_gain_db = 24.0;
            p.mix = 100.0;
        }),
    ] {
        let z = vec![0.0_f32; 4_096];
        let (yl, yr) = run(&p, &z, &z, SEG);
        assert!(yl.iter().chain(&yr).all(|x| x.to_bits() == 0));
    }
}

#[test]
fn non_finite_input_does_not_poison_the_state() {
    let p = params(|p| p.depth = 100.0);
    let n = 8_192;
    let clean = noise(7, n, 0.6);
    let mut dirty = clean.clone();
    for i in (100..140).step_by(3) {
        dirty[i] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY][(i / 3) % 3];
    }
    // The wet path treats them as zero: the same bits as feeding zeros there, and all finite.
    let mut zeroed = clean.clone();
    for i in (100..140).step_by(3) {
        zeroed[i] = 0.0;
    }
    let (a, _) = run(&p, &dirty, &dirty, SEG);
    let (b, _) = run(&p, &zeroed, &zeroed, SEG);
    assert!(a.iter().all(|x| x.is_finite()));
    assert_eq!(
        a.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
        b.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
    );
    // At mix 0 the dry sample passes as it is (a NaN stays a NaN for that frame), and the next ones are fine.
    let dry = params(|p| p.mix = 0.0);
    let (c, _) = run(&dry, &dirty, &dirty, SEG);
    assert!(c[100].is_nan() && c[200].is_finite());
}

#[test]
fn a_forty_second_tail_holds_no_subnormal_and_reaches_idle() {
    for time in [100.0_f32, 400.0] {
        let p = params(|p| {
            p.depth = 100.0;
            p.time = time;
            p.input_db = 12.0;
        });
        let mut o = ott_with(&p);
        let mut rng = noise(11, 48_000, 0.9);
        let mut quiet = vec![0.0_f32; SEG];
        let mut max_sub = 0;
        let mut idle_at = None;
        // One second of loud noise, then 40 s of silence (the tail), checked every second.
        for chunk in rng.chunks_mut(SEG) {
            let mut r = chunk.to_vec();
            o.process_in_place(chunk, &mut r);
        }
        for sec in 0..40 {
            for _ in 0..(48_000 / SEG) {
                quiet.fill(0.0);
                let mut r = vec![0.0_f32; SEG];
                o.process_in_place(&mut quiet, &mut r);
                assert!(quiet.iter().all(|x| x.is_finite() && !x.is_subnormal()));
            }
            max_sub = max_sub.max(o.count_subnormals());
            if idle_at.is_none() && o.is_idle() {
                idle_at = Some(sec);
            }
        }
        assert_eq!(max_sub, 0, "time {time} %: subnormal values in the state");
        assert!(idle_at.is_some(), "time {time} %: never idle in 40 s");
        eprintln!(
            "tail at time {time} %: idle after {} s, 0 subnormals",
            idle_at.unwrap() + 1
        );
    }
}

#[test]
fn latency_is_zero_an_impulse_leaves_on_the_frame_it_enters() {
    let o = Ott::new();
    assert_eq!(o.latency_samples(), 0);
    for p in [
        params(|p| p.depth = 0.0),
        params(|p| p.depth = 100.0),
        OttParams::DEFAULT,
    ] {
        let mut l = vec![0.0_f32; 256];
        l[0] = 1.0;
        let mut r = l.clone();
        let mut o = ott_with(&p);
        o.process_in_place(&mut l[..SEG], &mut r[..SEG]);
        assert!(
            l[0].abs() > 0.1,
            "the first frame already carries the impulse: {}",
            l[0]
        );
        // And it is causal: nothing before an impulse placed later.
        let mut l = vec![0.0_f32; 256];
        l[100] = 1.0;
        let mut r = l.clone();
        let mut o = ott_with(&p);
        for (a, b) in l.chunks_mut(SEG).zip(r.chunks_mut(SEG)) {
            o.process_in_place(a, b);
        }
        assert!(l[..100].iter().all(|x| *x == 0.0) && l[100].abs() > 0.1);
    }
}

#[test]
fn the_block_size_is_not_an_input_in_whole_segments() {
    let (l, r) = (noise(3, 24_000, 0.7), noise(4, 24_000, 0.4));
    let p = params(|p| p.depth = 100.0);
    let reference = run(&p, &l, &r, SEG);
    for chunk in [64, 128, 480, 960, 24_000] {
        assert!(run(&p, &l, &r, chunk) == reference, "chunk {chunk}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The platform hash and allocation

/// FNV-1a 64 over the little-endian bytes of `f32` bit patterns.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn f32(&mut self, x: f32) {
        for b in x.to_bits().to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// Hash of the scenario below. The same constant must hold natively (debug and release, `-C target-cpu=native`) and on
/// `wasm32-wasip1` under wasmtime (docs/dsp/ott.md has the commands).
const HASH_GOLDEN: u64 = 0x2e24_790f_2cbd_7392;

fn scenario_hash() -> u64 {
    const SEGMENT: usize = 9_600; // 300 calls of 32 frames, 0.2 s
    let mut o = Ott::new();
    o.prepare(FS);
    let mut rng = noise(0x0e17_5eed, 1, 1.0);
    let _ = &mut rng;
    let mut state = 0x0e17_5eed_u32;
    let mut unit = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state >> 8) as f32 / 8_388_608.0 - 1.0
    };
    let sets = [
        OttParams::DEFAULT,
        params(|p| {
            p.depth = 100.0;
            p.time = 30.0;
            p.input_db = 9.0;
            p.low_split_hz = 140.0;
            p.high_split_hz = 5_000.0;
            p.mix = 80.0;
        }),
        params(|p| {
            p.depth = 70.0;
            p.time = 400.0;
            p.upward = 40.0;
            p.low_gain_db = -4.0;
            p.mid_gain_db = 3.0;
            p.high_gain_db = 6.0;
            p.output_db = -3.0;
        }),
        params(|p| {
            p.depth = 100.0;
            p.downward = 0.0;
            p.low_split_hz = 60.0;
            p.high_split_hz = 800.0;
        }),
    ];
    let mut hash = Fnv::new();
    let mut n = 0_usize;
    let mut prev = sets[0];
    for segment in 0..8_usize {
        for call in 0..SEGMENT / SEG {
            let mut l = [0.0_f32; SEG];
            let mut r = [0.0_f32; SEG];
            for i in 0..SEG {
                let t = n + i;
                let (a, b) = match segment {
                    0 => (
                        (0.7 * libm::sin(TAU * 55.0 * t as f64 / FSD)) as f32,
                        (0.4 * libm::sin(TAU * 1_310.0 * t as f64 / FSD)) as f32,
                    ),
                    1 => (0.0, 0.0),
                    2 => (0.5 * unit(), 0.5 * unit()),
                    3 => {
                        let on = (t % 2_400) < 480;
                        (
                            if on {
                                (0.9 * libm::sin(TAU * 1_000.0 * t as f64 / FSD)) as f32
                            } else {
                                1e-4 * unit()
                            },
                            1e-4 * unit(),
                        )
                    }
                    4 => {
                        let k = (t - 4 * SEGMENT) as f64 / SEGMENT as f64;
                        (
                            (libm::sin(TAU * 440.0 * t as f64 / FSD) * undb(-70.0 * (1.0 - k)))
                                as f32,
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
                        0.9 * unit(),
                    ),
                };
                l[i] = a;
                r[i] = b;
            }
            // Settings change every call: a different set every 40th call, ramped from the previous call's.
            let mut cur = sets[(call / 40 + segment) % sets.len()];
            cur.depth = (cur.depth + 0.25 * call as f32).min(100.0);
            o.set_ramp(&prev, &cur);
            prev = cur;
            o.process_in_place(&mut l, &mut r);
            for i in 0..SEG {
                hash.f32(l[i]);
                hash.f32(r[i]);
            }
            for b in 0..BANDS {
                hash.f32(o.band_gain_db(b));
            }
            n += SEG;
        }
    }
    hash.0
}

#[test]
fn render_hash_is_the_same_on_every_platform() {
    let h = scenario_hash();
    // Visible with `--nocapture`, so a native run and a wasmtime run can be compared by eye or by `diff`.
    eprintln!("ott-hash {h:016x}");
    assert_eq!(
        h, HASH_GOLDEN,
        "ott render hash changed: update HASH_GOLDEN only for a deliberate change"
    );
}

#[test]
fn process_allocates_nothing() {
    let mut o = Ott::new();
    o.prepare(FS);
    let l = noise(5, 1_024, 0.8);
    let r = noise(6, 1_024, 0.8);
    let (mut a, mut b) = (l.clone(), r.clone());
    let wild = OttParams {
        depth: f32::NAN,
        time: -1.0,
        low_split_hz: 1e9,
        ..OttParams::DEFAULT
    };
    let mut sink = 0.0_f32;
    assert_no_alloc(|| {
        for round in 0..64_u32 {
            let from = params(|p| p.depth = 10.0 + round as f32);
            let to = params(|p| {
                p.depth = 20.0 + round as f32;
                p.time = 20.0 + 10.0 * round as f32;
                p.low_split_hz = 50.0 + round as f32;
            });
            o.set_ramp(&from, &to);
            for (x, y) in a.chunks_mut(SEG).zip(b.chunks_mut(SEG)) {
                o.process_in_place(x, y);
            }
            sink += o.band_gain_db(0) + o.band_level_db(1);
            if round % 16 == 15 {
                o.set_params(&wild);
                o.reset();
                o.prepare(FS * 2.0);
                o.prepare(FS);
                a.copy_from_slice(&l);
                b.copy_from_slice(&r);
            }
        }
    });
    assert!(sink.is_finite());
}

/// The peak (linear) of the left output over the 100 ms after a step from `from_db` to `to_db` dBFS of a sine at `hz`.
fn step_peak(p: &OttParams, hz: f64, from_db: f64, to_db: f64) -> f64 {
    let mut o = ott_with(p);
    let mut l = [0.0_f32; SEG];
    let mut r = [0.0_f32; SEG];
    let jump = 48_000;
    let mut peak = 0.0_f64;
    let mut n = 0;
    while n < jump + 4_800 {
        for i in 0..SEG {
            let a = if n + i < jump {
                undb(from_db)
            } else {
                undb(to_db)
            };
            let x = (a * libm::sin(TAU * hz * (n + i) as f64 / FSD)) as f32;
            l[i] = x;
            r[i] = x;
        }
        o.process_in_place(&mut l, &mut r);
        for (i, &y) in l.iter().enumerate() {
            if n + i >= jump {
                peak = peak.max(f64::from(y).abs());
            }
        }
        n += SEG;
    }
    peak
}

/// No look-ahead means a lift computed for a quiet passage meets the first cycles of a loud note. The onset guard
/// holds those cycles to the louder of the band's upward threshold + 20 dB and what the same step does at depth 0
/// (the crossovers' own onset ripple: +0.7 dB on a mid-band sine), plus half a dB. Without the guard a -60 -> -6 dBFS
/// step at depth 100 left the mid band at +15 dBFS.
#[test]
fn a_note_after_a_gap_is_not_thrown_far_over_its_input() {
    for (band, hz, ls, hs) in [
        (0, 60.0, 500.0, 10_000.0),
        (1, 1_000.0, 40.0, 10_000.0),
        (2, 8_000.0, 40.0, 500.0),
    ] {
        let ceiling_db =
            f64::from(BAND_SPECS[band].up_threshold_db + LIFT_CEILING_ABOVE_THRESHOLD_DB);
        for &(from_db, to_db) in &[(-60.0_f64, -6.0_f64), (-40.0, -6.0), (-60.0, -20.0)] {
            let flat = db(step_peak(
                &params(|p| {
                    p.depth = 0.0;
                    p.low_split_hz = ls;
                    p.high_split_hz = hs;
                }),
                hz,
                from_db,
                to_db,
            ));
            for depth in [30.0_f32, 100.0] {
                let p = params(|p| {
                    p.depth = depth;
                    p.low_split_hz = ls;
                    p.high_split_hz = hs;
                });
                let peak = db(step_peak(&p, hz, from_db, to_db));
                let allowed = flat.max(ceiling_db) + 0.5;
                assert!(
                    peak <= allowed,
                    "band {band} {hz} Hz depth {depth}: {from_db} -> {to_db} dBFS peaks at {peak:+.2} dBFS, over {allowed:+.2} (flat split {flat:+.2})"
                );
            }
        }
    }
}

/// The worst deviation (dB, signed) of a tone's settled output gain from its own band's curve over a rising level sweep
/// of -100 to 0 dBFS, and the level it occurs at.
fn worst_deviation(probe: &Probe, p: &OttParams) -> (f64, f64) {
    let spec = &BAND_SPECS[probe.band];
    let mut o = ott_with(p);
    let mut worst = (0.0_f64, 0.0_f64);
    for i in 0..=25 {
        let lvl = -100.0 + 4.0 * f64::from(i);
        let out = settled_out_db(&mut o, probe, lvl, 72_000, 24_000);
        let seen = follower_level_db(
            probe.hz,
            undb(lvl),
            f64::from(spec.attack_ms),
            f64::from(spec.release_ms),
        );
        let err = (out - lvl) - curve_db(spec, seen, 1.0, 1.0);
        if err.abs() > worst.0.abs() {
            worst = (err, lvl);
        }
    }
    worst
}

/// The band split is not a brick wall, so a tone near a crossover also lands in the neighbouring band, which lifts it
/// (upward compression is its job) and adds it back. At the default crossovers the output of a tone then deviates from
/// its own band's curve, by up to 6 dB in the upward region and by the most within an octave or two of a crossover;
/// with isolating splits (the test above) it does not. This records the deviation, so the page's table is checked.
#[test]
fn at_the_default_splits_the_neighbours_lift_the_leakage_and_the_deviation_is_the_recorded_one() {
    // (band, tone Hz, deviation dB, at dBFS) measured 2026-10-26; the tolerance is a quarter of a dB.
    let recorded: [(usize, f64, f64, f64); 6] = [
        (0, 40.0, 5.35, -40.0),
        (0, 60.0, 5.84, -48.0),
        (1, 200.0, 5.01, -40.0),
        (1, 1_000.0, 2.23, -44.0),
        (1, 500.0, 0.63, -8.0),
        (2, 6_000.0, 4.88, -36.0),
    ];
    let p = params(|p| p.depth = 100.0);
    for (band, hz, dev, at) in recorded {
        let probe = Probe {
            band,
            name: "default splits",
            hz,
            low_split: 88.3,
            high_split: 2_500.0,
        };
        let (got, level) = worst_deviation(&probe, &p);
        eprintln!(
            "band {band}, {hz} Hz, default splits: worst deviation {got:+.2} dB at {level} dBFS"
        );
        assert!(
            (got - dev).abs() < 0.25 && (level - at).abs() < 0.5,
            "band {band}, {hz} Hz: {got:+.2} dB at {level} dBFS, recorded {dev:+.2} at {at}"
        );
    }
}
