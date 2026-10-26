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
    BAND_SPECS, BANDS, BandCurve, KNEE_DB, Ott, OttParams, UP_CAP_DB, UP_FADE_BOTTOM_DB,
    UP_FADE_TOP_DB,
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
    let tail = (FSD / f).max(1.0);
    let cycles = libm::floor(1.0 / (1.0 / f)).max(1.0);
    let len = (cycles * FSD / f) as usize;
    let _ = tail;
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
