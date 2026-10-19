//! Cytomic/Simper TPT state-variable filter (LP, BP, HP, notch) (P1a lane D2). Notes: `docs/dsp/svf.md`.
//!
//! The structure is Andrew Simper's *Linear Trapezoidal Integrated State Variable Filter* (Cytomic, 2013, typo fix
//! 2016), the same equations as Zavalishin's TPT SVF: two trapezoidal integrators in a zero-delay loop, solved in
//! closed form, so there is no one-sample delay in the feedback and the response is the bilinear transform of the
//! analogue 12 dB/octave prototype with its cutoff pre-warped (`g = tan(π·fc/fs)`). Written from the paper's
//! equations; no code is copied.
//!
//! ```text
//! per sample:  v3 = v0 − ic2
//!              v1 = a1·ic1 + a2·v3                 band
//!              v2 = ic2 + a2·ic1 + a3·v3           low
//!              ic1 = 2·v1 − ic1;   ic2 = 2·v2 − ic2
//! outputs:     lp = v2;  bp = k·v1 (unity peak);  hp = v0 − k·v1 − v2;  notch = v0 − k·v1
//! with:        g = tan(π·fc/fs),  k = 1/Q,  a1 = 1/(1 + g·(g + k)),  a2 = g·a1,  a3 = g·a2
//! ```
//!
//! Two things make it usable inside the engine's 32-frame segments:
//!
//! - **Coefficients are interpolated, not stepped.** The caller computes the coefficients for the *end* of a segment
//!   (one `tanf`) and [`process_block`] ramps `a1, a2, a3, k` linearly from the previous segment's values to those
//!   across the block, three adds and no division per sample. Report 02 measured 13–23 dB less error than stepping,
//!   and the filter stays stable along the interpolation path (eigenvalues of the update matrix ≤ 0.99995 in
//!   magnitude between 20 Hz and 21.6 kHz at Q 25).
//! - **Drive is a continuous soft saturator in front of the filter** ([`drive_amount`]): `x / sqrt(1 + a·x²)`, which
//!   is the identity, bit for bit, at 0 dB.
//!
//! Determinism: the only transcendental is `tanf` (and `exp2`/`expm1` for the parameter maps), through
//! `util::math`; the per-sample code is `+ − × ÷` and `sqrt`, all correctly rounded in IEEE 754. No `mul_add`.
//! Recursive state is flushed once per block.

use crate::util::flush::flush;
use crate::util::math;

/// The lowest Q: `resonance = 0 %` (critically damped, no peak).
pub const Q_MIN: f32 = 0.5;
/// The highest Q: `resonance = 100 %` (a 28 dB peak at the cutoff).
pub const Q_MAX: f32 = 25.0;
/// `log2(Q_MAX / Q_MIN)`: the resonance map is `Q = Q_MIN · 2^(LOG2_Q_RANGE · resonance)`.
pub const LOG2_Q_RANGE: f64 = 5.643_856_189_774_724;

/// The lowest cutoff the filter computes (the catalogue's minimum): lower values, from key tracking, are clamped.
pub const MIN_CUTOFF_HZ: f32 = 20.0;
/// The highest cutoff as a fraction of the sample rate: 21.6 kHz at 48 kHz, where `g = tan(0.45π) = 6.3`.
pub const MAX_CUTOFF_FRACTION: f32 = 0.45;

/// The filter response, in the catalogue's order (`FilterMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Low-pass, 12 dB/octave.
    Lp,
    /// Band-pass with unity gain at the cutoff.
    Bp,
    /// High-pass, 12 dB/octave.
    Hp,
    /// Notch: low-pass plus high-pass.
    Notch,
}

const LP: u8 = 0;
const BP: u8 = 1;
const HP: u8 = 2;
const NOTCH: u8 = 3;

/// The damping `k = 1/Q` for a resonance of `percent` (0–100, clamped): `Q = 0.5 · 50^(percent/100)`, so
/// `k = 2 · 50^(−percent/100)`.
///
/// The curve is exponential in Q on purpose. The peak at the cutoff is about `Q` for `Q ≥ 1`, so equal steps of the
/// knob are equal steps in decibels (about 0.34 dB per percent from Q = 1 at 17.7 % up to 28 dB at 100 %), where a
/// linear-in-`k` map spends half its travel between Q 0.5 and 1 and has no audible peak at 50 %. Landmarks:
/// 0 % Q 0.50, 10 % (the default) Q 0.74 (within 0.5 dB of the maximally flat 0.707), 25 % Q 1.33, 50 % Q 3.54,
/// 75 % Q 9.4, 100 % Q 25.
#[must_use]
pub fn k_from_resonance(percent: f32) -> f32 {
    let r = f64::from(percent.clamp(0.0, 100.0)) * 0.01;
    (2.0 * math::exp2(-LOG2_Q_RANGE * r)) as f32
}

/// The Q for a resonance of `percent` (see [`k_from_resonance`]).
#[must_use]
pub fn q_from_resonance(percent: f32) -> f32 {
    let r = f64::from(percent.clamp(0.0, 100.0)) * 0.01;
    (f64::from(Q_MIN) * math::exp2(LOG2_Q_RANGE * r)) as f32
}

/// The saturator amount `a = G² − 1` for a drive of `db` decibels (`G = 10^(dB/20)`): 0 for `db <= 0` (and for NaN),
/// which makes the saturator the identity. Computed in `f64` and rounded once.
#[must_use]
pub fn drive_amount(db: f32) -> f32 {
    if db > 0.0 {
        // G² − 1 = 10^(dB/10) − 1 = expm1(dB · ln(10)/10).
        math::expm1(f64::from(db) * 0.230_258_509_299_404_6) as f32
    } else {
        0.0
    }
}

/// The soft saturator in front of the filter: `x / sqrt(1 + a·x²)` for `a = `[`drive_amount`]. Odd, monotonic,
/// unity gain at small signals, the identity (bit for bit) at `a = 0`, and bounded by `1/sqrt(a)`: gain `G` into
/// the sigmoid `u/sqrt(1 + u²)`, normalised back by `1/G`, offset so that 0 dB is transparent.
#[inline(always)]
#[must_use]
pub fn saturate(x: f32, a: f32) -> f32 {
    x / (1.0 + a.max(0.0) * x * x).sqrt()
}

/// The filter coefficients for one cutoff and damping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    /// `1 / (1 + g·(g + k))`.
    pub a1: f32,
    /// `g · a1`.
    pub a2: f32,
    /// `g · a2`.
    pub a3: f32,
    /// The damping `1/Q`.
    pub k: f32,
}

impl Coeffs {
    /// Coefficients for `cutoff_hz` (clamped to [`MIN_CUTOFF_HZ`]`..=`[`MAX_CUTOFF_FRACTION`]` · sample_rate`; NaN
    /// becomes the minimum) and damping `k` (> 0) at `sample_rate`. One `tanf`.
    #[must_use]
    pub fn new(cutoff_hz: f32, k: f32, sample_rate: f32) -> Self {
        let hi = MAX_CUTOFF_FRACTION * sample_rate;
        let fc = if cutoff_hz >= MIN_CUTOFF_HZ {
            cutoff_hz.min(hi)
        } else {
            MIN_CUTOFF_HZ
        };
        let g = math::tanf(core::f32::consts::PI * fc / sample_rate);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        Self { a1, a2, a3, k }
    }
}

/// The two integrator states of one filter.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    /// First integrator (band-pass memory).
    pub ic1: f32,
    /// Second integrator (low-pass memory).
    pub ic2: f32,
}

impl State {
    /// A silent filter.
    pub const ZERO: Self = Self { ic1: 0.0, ic2: 0.0 };

    /// Flush tiny values to zero (once per block is enough: a feed-forward voice that ends at −100 dB).
    #[inline]
    pub fn flush(&mut self) {
        self.ic1 = flush(self.ic1);
        self.ic2 = flush(self.ic2);
    }

    /// Whether either integrator holds a subnormal number (test support).
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.ic1.is_subnormal() || self.ic2.is_subnormal()
    }

    /// One sample through the filter with fixed coefficients, in `mode` (not the fast path; the engine uses
    /// [`process_block`]).
    #[inline]
    pub fn tick(&mut self, mode: Mode, c: &Coeffs, x: f32) -> f32 {
        match mode {
            Mode::Lp => self.step::<LP>(c, x),
            Mode::Bp => self.step::<BP>(c, x),
            Mode::Hp => self.step::<HP>(c, x),
            Mode::Notch => self.step::<NOTCH>(c, x),
        }
    }

    #[inline(always)]
    fn step<const MODE: u8>(&mut self, c: &Coeffs, x: f32) -> f32 {
        self.step_with::<MODE>(c.a1, c.a2, c.a3, c.k, x)
    }

    #[inline(always)]
    fn step_with<const MODE: u8>(&mut self, a1: f32, a2: f32, a3: f32, k: f32, x: f32) -> f32 {
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match MODE {
            LP => v2,
            BP => k * v1,
            HP => x - k * v1 - v2,
            _ => x - k * v1,
        }
    }
}

/// Filter `io` in place. The coefficients ramp linearly from `from` (the previous block's end) to `to` across the
/// block (the last sample uses `to`, up to rounding), and the saturator amount ramps from `drive_from` to
/// `drive_to` (see [`drive_amount`]); with both at zero the saturator is skipped entirely, which is bit-identical to
/// running it. The states are flushed at the end.
pub fn process_block<const N: usize>(
    state: &mut State,
    mode: Mode,
    from: &Coeffs,
    to: &Coeffs,
    drive_from: f32,
    drive_to: f32,
    io: &mut [f32; N],
) {
    // A ramp between equal coefficients adds exact zeros: skipping it is bit-identical and saves four adds a sample.
    let ramp = from != to;
    let drive = drive_from > 0.0 || drive_to > 0.0;
    macro_rules! go {
        ($m:expr) => {
            match (ramp, drive) {
                (false, false) => {
                    run::<$m, false, false, N>(state, from, to, drive_from, drive_to, io)
                }
                (true, false) => {
                    run::<$m, true, false, N>(state, from, to, drive_from, drive_to, io)
                }
                (false, true) => {
                    run::<$m, false, true, N>(state, from, to, drive_from, drive_to, io)
                }
                (true, true) => run::<$m, true, true, N>(state, from, to, drive_from, drive_to, io),
            }
        };
    }
    match mode {
        Mode::Lp => go!(LP),
        Mode::Bp => go!(BP),
        Mode::Hp => go!(HP),
        Mode::Notch => go!(NOTCH),
    }
    state.flush();
}

#[inline(always)]
fn run<const MODE: u8, const RAMP: bool, const DRIVE: bool, const N: usize>(
    state: &mut State,
    from: &Coeffs,
    to: &Coeffs,
    drive_from: f32,
    drive_to: f32,
    io: &mut [f32; N],
) {
    let n = N as f32;
    let (mut a1, mut a2, mut a3, mut k) = if RAMP {
        (from.a1, from.a2, from.a3, from.k)
    } else {
        (to.a1, to.a2, to.a3, to.k)
    };
    let (d1, d2, d3, dk) = if RAMP {
        (
            (to.a1 - a1) / n,
            (to.a2 - a2) / n,
            (to.a3 - a3) / n,
            (to.k - k) / n,
        )
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    let mut a = drive_from;
    let da = (drive_to - drive_from) / n;
    for x in io.iter_mut() {
        if RAMP {
            a1 += d1;
            a2 += d2;
            a3 += d3;
            k += dk;
        }
        let mut v0 = *x;
        if DRIVE {
            a += da;
            v0 = saturate(v0, a);
        }
        *x = state.step_with::<MODE>(a1, a2, a3, k, v0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::rng::Rng;

    const SR: f32 = 48_000.0;

    /// The exact digital transfer function of the TPT SVF at frequency `f`, by complex arithmetic in `f64`:
    /// `s = (z − 1)/((z + 1)·g)` and the analogue prototype `1/(s² + k·s + 1)` (lp), `k·s/(...)` (bp), `s²/(...)`
    /// (hp), `(s² + 1)/(...)` (notch). Returns the magnitude.
    fn analytic(mode: Mode, fc: f64, k: f64, sr: f64, f: f64) -> f64 {
        let g = math::tan(core::f64::consts::PI * fc / sr);
        let w = 2.0 * core::f64::consts::PI * f / sr;
        // z = e^{jw}; s = (z−1)/(z+1)/g = j·tan(w/2)/g.
        let t = math::tan(w / 2.0);
        let sj = t / g; // s = j·sj
        // Denominator 1 − sj² + j·k·sj.
        let den_re = 1.0 - sj * sj;
        let den_im = k * sj;
        let den = (den_re * den_re + den_im * den_im).sqrt();
        let num = match mode {
            Mode::Lp => 1.0,
            Mode::Bp => k * sj.abs(),
            Mode::Hp => sj * sj,
            Mode::Notch => (1.0 - sj * sj).abs(),
        };
        num / den
    }

    /// |H| measured with a sine of frequency `f` through the filter: the amplitude of the settled output.
    fn measured(mode: Mode, fc: f32, k: f32, f: f64) -> f64 {
        let c = Coeffs::new(fc, k, SR);
        let mut s = State::ZERO;
        let w = 2.0 * core::f64::consts::PI * f / f64::from(SR);
        let total = 96_000usize;
        let (mut re, mut im, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..total {
            let x = math::sin(w * i as f64) as f32;
            let y = f64::from(s.tick(mode, &c, x));
            if i >= total / 2 {
                // Correlate with the input sinusoid over whole-sample support (an exact DFT bin estimate).
                re += y * math::sin(w * i as f64);
                im += y * math::cos(w * i as f64);
                n += 1.0;
            }
        }
        2.0 * (re * re + im * im).sqrt() / n
    }

    fn db(x: f64) -> f64 {
        20.0 * math::log10(x.max(1e-30))
    }

    #[test]
    fn resonance_map_landmarks() {
        assert_eq!(k_from_resonance(0.0), 2.0);
        assert!((k_from_resonance(100.0) - 0.04).abs() < 1e-6);
        assert!((q_from_resonance(0.0) - 0.5).abs() < 1e-7);
        assert!((q_from_resonance(100.0) - 25.0).abs() < 1e-4);
        // Landmarks documented in docs/dsp/svf.md.
        for (pct, q) in [(10.0f32, 0.740), (25.0, 1.330), (50.0, 3.536), (75.0, 9.40)] {
            assert!(
                (q_from_resonance(pct) - q).abs() < 0.01,
                "{pct}: {}",
                q_from_resonance(pct)
            );
        }
        // Q = 1 at 17.7 % and a peak that grows evenly in decibels above it.
        assert!((q_from_resonance(17.7) - 1.0).abs() < 0.01);
        let step = |a: f32, b: f32| db(f64::from(q_from_resonance(b) / q_from_resonance(a)));
        assert!(
            (step(20.0, 30.0) - step(80.0, 90.0)).abs() < 1e-3,
            "even dB steps"
        );
        // k * Q = 1 everywhere, monotone, and out-of-range input is clamped.
        let mut last = f32::MAX;
        for i in 0..=100 {
            let r = i as f32;
            let (k, q) = (k_from_resonance(r), q_from_resonance(r));
            assert!((k * q - 1.0).abs() < 1e-6, "{r}");
            assert!(k < last);
            last = k;
        }
        assert_eq!(k_from_resonance(-5.0), k_from_resonance(0.0));
        assert_eq!(k_from_resonance(500.0), k_from_resonance(100.0));
    }

    #[test]
    fn the_saturator_is_transparent_at_zero_and_bounded_above() {
        assert_eq!(drive_amount(0.0), 0.0);
        assert_eq!(drive_amount(-3.0), 0.0);
        assert_eq!(drive_amount(f32::NAN), 0.0);
        for x in [-3.0f32, -0.5, 0.0, 0.123, 1.0, 40.0] {
            assert_eq!(saturate(x, 0.0).to_bits(), x.to_bits());
        }
        // 12 dB: G = 3.981, a = G² − 1 = 14.85, the limit 1/sqrt(a) = 0.2595.
        let a = drive_amount(12.0);
        assert!((a - 14.848).abs() < 0.01, "{a}");
        assert!(saturate(1000.0, a) <= 1.0 / a.sqrt());
        assert!((saturate(1000.0, a) - 1.0 / a.sqrt()).abs() < 1e-4);
        // Unity small-signal gain, odd, monotonic.
        assert!((saturate(1e-3, a) / 1e-3 - 1.0).abs() < 1e-4);
        let mut last = -1.0f32;
        for i in -100..=100 {
            let x = i as f32 * 0.05;
            let y = saturate(x, a);
            assert_eq!(y.to_bits(), (-saturate(-x, a)).to_bits(), "odd at {x}");
            assert!(y >= last);
            last = y;
        }
        // 24 dB: a = 250.
        assert!((drive_amount(24.0) - 250.19).abs() < 0.1);
    }

    #[test]
    fn response_at_dc_cutoff_and_nyquist_matches_the_analytic_value_within_0_1_db() {
        for &fc in &[300.0f32, 1_000.0, 5_000.0, 15_000.0] {
            for &res in &[0.0f32, 10.0, 50.0, 100.0] {
                let k = k_from_resonance(res);
                for mode in [Mode::Lp, Mode::Bp, Mode::Hp, Mode::Notch] {
                    // The analytic |H| at the cutoff is the analogue prototype's: Q for lp, 1 for bp, Q for hp,
                    // 0 for the notch. Nyquist gives lp 0, bp 0, hp 1, notch 1; DC gives lp 1, bp 0, hp 0, notch 1.
                    // Frequencies where the response is exactly 0 are compared against a floor instead.
                    for &f in &[
                        f64::from(fc),
                        f64::from(fc) * 0.5,
                        f64::from(fc) * 1.7,
                        23_990.0,
                        40.0,
                    ] {
                        let want = analytic(mode, f64::from(fc), f64::from(k), f64::from(SR), f);
                        let got = measured(mode, fc, k, f);
                        if want < 1e-3 {
                            assert!(
                                got < 2e-3,
                                "{mode:?} fc {fc} res {res} f {f}: {got} vs {want}"
                            );
                        } else {
                            assert!(
                                (db(got) - db(want)).abs() < 0.1,
                                "{mode:?} fc {fc} res {res} f {f}: {:.3} dB vs {:.3} dB",
                                db(got),
                                db(want)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dc_and_exact_nyquist_points() {
        // DC: lp and notch pass it at unity, bp and hp block it. Nyquist (alternating +1/−1): lp and bp block it,
        // hp and notch pass it at unity. Both are exact properties of the bilinear transform (z = ±1).
        let c = Coeffs::new(2_000.0, k_from_resonance(30.0), SR);
        for (mode, dc, nyq) in [
            (Mode::Lp, 1.0, 0.0),
            (Mode::Bp, 0.0, 0.0),
            (Mode::Hp, 0.0, 1.0),
            (Mode::Notch, 1.0, 1.0),
        ] {
            let mut s = State::ZERO;
            let mut y = 0.0;
            for _ in 0..48_000 {
                y = s.tick(mode, &c, 1.0);
            }
            assert!((y - dc).abs() < 1e-4, "{mode:?} DC {y}");
            let mut s = State::ZERO;
            for i in 0..48_000 {
                y = s.tick(mode, &c, if i % 2 == 0 { 1.0 } else { -1.0 });
            }
            assert!((y.abs() - nyq).abs() < 1e-3, "{mode:?} Nyquist {y}");
        }
    }

    #[test]
    fn the_cutoff_is_clamped_and_nan_is_safe() {
        let lo = Coeffs::new(MIN_CUTOFF_HZ, 1.0, SR);
        assert_eq!(Coeffs::new(0.1, 1.0, SR), lo);
        assert_eq!(Coeffs::new(f32::NAN, 1.0, SR), lo);
        assert_eq!(Coeffs::new(-5.0, 1.0, SR), lo);
        let hi = Coeffs::new(MAX_CUTOFF_FRACTION * SR, 1.0, SR);
        assert_eq!(Coeffs::new(40_000.0, 1.0, SR), hi);
        assert_eq!(Coeffs::new(f32::INFINITY, 1.0, SR), hi);
        for c in [lo, hi] {
            assert!(c.a1.is_finite() && c.a2.is_finite() && c.a3.is_finite());
            assert!(c.a1 > 0.0 && c.a1 <= 1.0);
        }
    }

    /// FNV-1a over the bit patterns: a pin for the impulse responses below.
    fn fnv(xs: &[f32]) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for x in xs {
            for b in x.to_bits().to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    fn impulse_response(mode: Mode, fc: f32, res: f32) -> Vec<f32> {
        let c = Coeffs::new(fc, k_from_resonance(res), SR);
        let mut s = State::ZERO;
        (0..4096)
            .map(|i| s.tick(mode, &c, if i == 0 { 1.0 } else { 0.0 }))
            .collect()
    }

    /// The acceptance pin: impulse-response hashes for 4 modes × 3 cutoffs × 2 resonances. The same bits on every
    /// platform: the only transcendentals are libm's `tanf` and `exp2`, everything else is IEEE arithmetic. The
    /// native hashes were checked equal under wasmtime (wasm32-wasip1) when they were written down.
    #[test]
    fn impulse_response_hashes_are_pinned() {
        let mut got = Vec::new();
        for mode in [Mode::Lp, Mode::Bp, Mode::Hp, Mode::Notch] {
            for fc in [200.0f32, 2_000.0, 12_000.0] {
                for res in [10.0f32, 90.0] {
                    got.push(fnv(&impulse_response(mode, fc, res)));
                }
            }
        }
        // Distinct responses, distinct hashes.
        let mut sorted = got.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 24);
        assert_eq!(got, PINNED_IR_HASHES, "{got:#018x?}");
    }

    const PINNED_IR_HASHES: [u64; 24] = [
        0xc5e594c7161b6c4f,
        0xf18888ac2c22cce8,
        0xfec765a5b446ba0a,
        0x10255532baa7043d,
        0xc2eb72709e504bc9,
        0x0256567055bb7a33,
        0xb3136c1b2834f273,
        0x3d01b02e3be26b5c,
        0xa4131b1ba1d67d85,
        0x5c5d0c2c3115953c,
        0x44ff6c5324c77f58,
        0x49220d63d1aa4074,
        0x968d0e8d079b04a1,
        0xa01008308bc69a9f,
        0x097bdfe41bd3f694,
        0x014f5de037c10bf9,
        0xc0b2987235f4b51d,
        0x74b27dfa51d9b09d,
        0xe8d1fc535382dd57,
        0xefbc8e6a559b49a5,
        0xcdd56af958be3d32,
        0x1a49072a0bac728c,
        0x4e0d5dac6a6864aa,
        0x05b6ac4fead3d23a,
    ];

    #[test]
    fn interpolating_to_the_same_coefficients_equals_fixed_coefficients() {
        // With from == to the ramp is flat: identical bits to the plain per-sample loop.
        let c = Coeffs::new(1_800.0, k_from_resonance(40.0), SR);
        let mut rng = Rng::new(5);
        let input: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32());
        for mode in [Mode::Lp, Mode::Bp, Mode::Hp, Mode::Notch] {
            let mut a = State::ZERO;
            let mut b = State::ZERO;
            let mut block = input;
            process_block(&mut a, mode, &c, &c, 0.0, 0.0, &mut block);
            let plain: Vec<f32> = input.iter().map(|&x| b.tick(mode, &c, x)).collect();
            assert_eq!(block.to_vec(), plain, "{mode:?}");
            assert_eq!(a, b);
        }
    }

    #[test]
    fn the_drive_path_is_transparent_at_zero_amount() {
        // Forcing the saturating loop with a == 0 must equal the plain loop, bit for bit, in every mode.
        let c = Coeffs::new(900.0, k_from_resonance(60.0), SR);
        let mut rng = Rng::new(6);
        let input: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32() * 3.0);
        macro_rules! check {
            ($mode:expr, $m:expr) => {{
                let (mut s1, mut s2) = (State::ZERO, State::ZERO);
                let (mut plain, mut sat) = (input, input);
                process_block(&mut s1, $mode, &c, &c, 0.0, 0.0, &mut plain);
                run::<$m, false, true, 32>(&mut s2, &c, &c, 0.0, 0.0, &mut sat);
                s2.flush();
                assert_eq!(plain, sat, "{:?}", $mode);
                assert_eq!(s1, s2);
            }};
        }
        check!(Mode::Lp, LP);
        check!(Mode::Bp, BP);
        check!(Mode::Hp, HP);
        check!(Mode::Notch, NOTCH);
    }

    #[test]
    fn drive_adds_harmonics_and_keeps_small_signals_unchanged() {
        let c = Coeffs::new(18_000.0, 1.0, SR);
        let a = drive_amount(24.0);
        let w = 2.0 * core::f32::consts::PI * 1_000.0 / SR;
        let mut s_clean = State::ZERO;
        let mut s_drive = State::ZERO;
        // Small signal (0.001): the same output within 0.1 %.
        let mut worst = 0.0f32;
        for i in 0..4800 {
            let x = 0.001 * math::sinf(w * i as f32);
            let mut b1 = [x; 1];
            let mut b2 = [x; 1];
            process_block(&mut s_clean, Mode::Lp, &c, &c, 0.0, 0.0, &mut b1);
            process_block(&mut s_drive, Mode::Lp, &c, &c, a, a, &mut b2);
            if i > 100 {
                worst = worst.max((b1[0] - b2[0]).abs());
            }
        }
        assert!(worst < 2e-5, "{worst}");
        // Full scale: a 1 kHz sine through 24 dB of drive grows a third harmonic (odd symmetry), where the clean
        // path has essentially none.
        let third = |drive: f32| {
            let mut s = State::ZERO;
            let (mut re1, mut im1, mut re3, mut im3) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let total = 4800usize;
            for i in 0..total {
                let t = i as f64 / 48_000.0;
                let x = math::sin(2.0 * core::f64::consts::PI * 1_000.0 * t) as f32;
                let mut b = [x; 1];
                process_block(&mut s, Mode::Lp, &c, &c, drive, drive, &mut b);
                let y = f64::from(b[0]);
                re1 += y * math::sin(2.0 * core::f64::consts::PI * 1_000.0 * t);
                im1 += y * math::cos(2.0 * core::f64::consts::PI * 1_000.0 * t);
                re3 += y * math::sin(2.0 * core::f64::consts::PI * 3_000.0 * t);
                im3 += y * math::cos(2.0 * core::f64::consts::PI * 3_000.0 * t);
            }
            (re3 * re3 + im3 * im3).sqrt() / (re1 * re1 + im1 * im1).sqrt()
        };
        assert!(third(0.0) < 0.002, "{}", third(0.0));
        assert!(third(a) > 0.1, "{}", third(a));
    }

    #[test]
    fn stays_finite_and_settles_under_audio_rate_cutoff_modulation_at_full_resonance() {
        // The acceptance stress: random cutoff every block between 20 Hz and 21.6 kHz at Q 25, white noise in.
        let k = k_from_resonance(100.0);
        let mut rng = Rng::new(0x1234_5678);
        for mode in [Mode::Lp, Mode::Bp, Mode::Hp, Mode::Notch] {
            for drive in [0.0f32, 24.0] {
                let a = drive_amount(drive);
                let mut s = State::ZERO;
                let mut prev = Coeffs::new(1_000.0, k, SR);
                let (mut peak, mut sum) = (0.0f32, 0.0f64);
                let blocks = 15_000; // 10 s
                for _ in 0..blocks {
                    let u = rng.unit_f32();
                    let fc = 20.0 * math::exp2f(u * math::log2f(0.45 * SR / 20.0));
                    let next = Coeffs::new(fc, k, SR);
                    let mut block: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32() * 0.5);
                    process_block(&mut s, mode, &prev, &next, a, a, &mut block);
                    for y in block {
                        assert!(y.is_finite(), "{mode:?} drive {drive}");
                        peak = peak.max(y.abs());
                        sum += f64::from(y);
                    }
                    prev = next;
                }
                // Bounded: the input is ±0.5 and the worst gain is Q = 25 at a resonant peak, but a random walk
                // of the cutoff cannot pump it (report 02 measured a peak of 2.9).
                assert!(peak < 25.0, "{mode:?} drive {drive}: peak {peak}");
                // No stuck DC: the mean of a zero-mean input stays far below the signal.
                let mean = sum / f64::from(blocks) / 32.0;
                assert!(mean.abs() < 0.05, "{mode:?} drive {drive}: mean {mean}");
                // And with the input removed and the cutoff parked, the state dies out to exact zero.
                let mut quiet = 0;
                for _ in 0..200_000 {
                    let mut block = [0.0f32; 32];
                    process_block(&mut s, mode, &prev, &prev, a, a, &mut block);
                    if s == State::ZERO {
                        break;
                    }
                    quiet += 1;
                }
                assert_eq!(s, State::ZERO, "{mode:?} drive {drive}: still ringing");
                // 20 Hz at Q 25 rings for tens of seconds at worst; the park cutoff here is whatever the walk ended
                // on, so only bound the time.
                assert!(quiet < 200_000);
            }
        }
    }

    #[test]
    fn an_impulse_at_the_highest_cutoff_and_full_resonance_decays() {
        let c = Coeffs::new(0.45 * SR, k_from_resonance(100.0), SR);
        let mut s = State::ZERO;
        let mut last = 0usize;
        for i in 0..48_000 {
            let y = s.tick(Mode::Lp, &c, if i == 0 { 1.0 } else { 0.0 });
            assert!(y.is_finite());
            if y.abs() > 1e-6 {
                last = i;
            }
        }
        // Report 02: below 1e-6 within 42 ms.
        assert!(last < 48 * 60, "rings until sample {last}");
    }

    #[test]
    fn an_interpolated_jump_is_smoother_than_a_stepped_one() {
        // Zipper: a 110 Hz saw through a cutoff jump from 200 Hz to 8 kHz at Q 5. The error against a per-sample
        // exact-coefficient reference is smaller when the coefficients are ramped across the block (report 02: 13 to
        // 23 dB less on sweeps).
        let k = k_from_resonance(70.0);
        let saw = |i: usize| ((i % 436) as f32 / 436.0) * 2.0 - 1.0;
        let fc_at = |i: usize| -> f32 {
            let t = (i as f32 / 32.0).min(1.0);
            200.0 * math::exp2f(t * math::log2f(8_000.0 / 200.0))
        };
        let mut s_ref = State::ZERO;
        let (mut s_step, mut s_ramp) = (State::ZERO, State::ZERO);
        let c0 = Coeffs::new(200.0, k, SR);
        let c1 = Coeffs::new(8_000.0, k, SR);
        let mut reference = [0.0f32; 32];
        for (i, r) in reference.iter_mut().enumerate() {
            *r = s_ref.tick(Mode::Lp, &Coeffs::new(fc_at(i + 1), k, SR), saw(i));
        }
        let mut stepped = [0.0f32; 32];
        for (i, o) in stepped.iter_mut().enumerate() {
            *o = s_step.tick(Mode::Lp, &c1, saw(i));
        }
        let mut ramped: [f32; 32] = core::array::from_fn(saw);
        process_block(&mut s_ramp, Mode::Lp, &c0, &c1, 0.0, 0.0, &mut ramped);
        let err = |a: &[f32; 32]| {
            a.iter()
                .zip(&reference)
                .map(|(x, y)| f64::from(x - y) * f64::from(x - y))
                .sum::<f64>()
        };
        assert!(
            err(&ramped) < err(&stepped),
            "{} vs {}",
            err(&ramped),
            err(&stepped)
        );
    }

    #[test]
    fn states_are_flushed_per_block_and_never_subnormal() {
        let mut s = State {
            ic1: 3e-16,
            ic2: -2e-17,
        };
        let c = Coeffs::new(1_000.0, 1.0, SR);
        let mut block = [0.0f32; 32];
        process_block(&mut s, Mode::Lp, &c, &c, 0.0, 0.0, &mut block);
        assert_eq!(s, State::ZERO);
        assert!(!s.has_subnormal());
        assert!(block.iter().all(|y| y.to_bits() == 0 || y.abs() >= 1e-30));
    }
}
