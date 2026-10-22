//! **Owner: lane BQ** (the biquad). Pre-declared by lane S0e (R4/R5, freeze point F0e). Notes: `docs/dsp/biquad.md`
//! (BQ's), the design in `docs/ear.md` ("Rendering and playback").
//!
//! An **offline RBJ peaking and shelf biquad** (Robert Bristow-Johnson's *Audio EQ Cookbook*), the one primitive for
//! R4's EQ games now (Frequency finder's EQ form) and for S2's EQ module later, which wraps it. Coefficients go through
//! [`util::math`](crate::util::math) (the only door to `libm`), so they are bit-exact on every platform. The band a
//! question carries is integers in `enth-music`; this file renders it. Not a voice module and not on the audio thread,
//! so the engine never calls it.
//!
//! **The shape.** [`Coeffs`] designs a section (`f64`, `a0` divided out) from a [`Kind`], a frequency, a gain in dB, a
//! `Q` and a sample rate; [`Biquad`] runs it over an `f32` buffer in place. Design and state are `f64`: a 20 Hz peak
//! at 48 kHz has poles within `1e-4` of the unit circle, which `f32` coefficients cannot place, and offline there is
//! nothing to save by rounding early. The direct form is **transposed direct form II**
//! (`y = b0·x + s1`, `s1 = b1·x − a1·y + s2`, `s2 = b2·x − a2·y`): two states, five products, written in one fixed
//! order with no `mul_add`, so every target (including a wasm32 build and a CPU with FMA) computes the same bits.
//!
//! **Determinism.** Transcendentals are `pow`, `sin` and `cos` from the pinned `libm`; everything else is `+ − × ÷`
//! and `sqrt`, all correctly rounded. The states and the output are flushed in code ([`flush64`]), so a decaying tail
//! reaches exactly `0.0` and never lingers in the subnormal range, whatever the CPU's FTZ/DAZ flags say. A band with
//! no gain designs [`Coeffs::IDENTITY`], and [`Biquad::process`] leaves the buffer alone for it.
//!
//! This is a new file: no existing output changes, and no existing golden moves. BQ adds its own golden
//! (`tests/biquad_golden.rs`, the same bits on the six golden targets) and tests the response against the cookbook's
//! analog prototypes to within `1e-6` dB (coefficients) and `0.1` dB (processed). It is distinct from the reverb's
//! private `reverb::biquad`, which stays as it is.
//!
//! Reference: R. Bristow-Johnson, *Cookbook formulae for audio EQ biquad filter coefficients* (the Audio EQ Cookbook,
//! W3C Audio EQ Cookbook note, 2005 revision): `peakingEQ`, `lowShelf` and `highShelf`, in their `Q` form, with
//! `A = 10^(dBgain/40)`. The equations only; no code is copied.

use core::f64::consts::TAU;

use crate::util::flush::flush64;
use crate::util::math;

/// The lowest centre (or corner) frequency designed, in Hz. A lower request is raised to it.
pub const MIN_FREQ_HZ: f64 = 1.0;

/// The highest centre (or corner) frequency designed, as a fraction of the sample rate. A higher request is lowered
/// to it, so `sin(w0)` never reaches zero and the section stays well conditioned.
pub const MAX_FREQ_FRACTION: f64 = 0.49;

/// The lowest `Q` designed. A lower request is raised to it.
pub const MIN_Q: f64 = 0.01;

/// The highest `Q` designed. A higher request is lowered to it.
pub const MAX_Q: f64 = 1000.0;

/// The largest gain magnitude designed, in dB. A larger request is clamped to it (`A` stays finite and well
/// conditioned: 120 dB is `A = 1000`).
pub const MAX_GAIN_DB: f64 = 120.0;

/// Which cookbook filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `peakingEQ`: a bell of `gain_db` at the centre, unity far away.
    Peak,
    /// `lowShelf`: `gain_db` below the corner, unity above, half the gain (in dB) at the corner.
    LowShelf,
    /// `highShelf`: unity below the corner, `gain_db` above, half the gain (in dB) at the corner.
    HighShelf,
}

/// The coefficients of one section with `a0` divided out: `H(z) = (b0 + b1 z⁻¹ + b2 z⁻²) / (1 + a1 z⁻¹ + a2 z⁻²)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    /// Feed-forward, `z⁰`.
    pub b0: f64,
    /// Feed-forward, `z⁻¹`.
    pub b1: f64,
    /// Feed-forward, `z⁻²`.
    pub b2: f64,
    /// Feedback, `z⁻¹` (sign as in the cookbook: `H` has `1 + a1 z⁻¹ + a2 z⁻²` below).
    pub a1: f64,
    /// Feedback, `z⁻²`.
    pub a2: f64,
}

impl Coeffs {
    /// The neutral section: `H(z) = 1`.
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Design a section.
    ///
    /// `freq_hz` is the peak's centre or the shelf's corner (clamped to [`MIN_FREQ_HZ`] and
    /// [`MAX_FREQ_FRACTION`]` · sample_rate`), `gain_db` the boost or cut (clamped to ±[`MAX_GAIN_DB`]), `q` the
    /// quality factor (clamped to [`MIN_Q`]..[`MAX_Q`]; the shelves use the cookbook's `Q` form, `α = sin w0 / 2Q`,
    /// where `Q = 1/√2` is the shelf slope `S = 1`). A gain of exactly zero, a non-finite argument or a sample rate
    /// too low to hold [`MIN_FREQ_HZ`] designs [`Coeffs::IDENTITY`]. A designed section is stable for every
    /// argument.
    #[must_use]
    pub fn design(kind: Kind, freq_hz: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        if !(freq_hz.is_finite() && gain_db.is_finite() && q.is_finite() && sample_rate.is_finite())
        {
            return Self::IDENTITY;
        }
        let highest = MAX_FREQ_FRACTION * sample_rate;
        let gain_db = gain_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
        if gain_db == 0.0 || highest <= MIN_FREQ_HZ {
            return Self::IDENTITY;
        }
        let f0 = freq_hz.clamp(MIN_FREQ_HZ, highest);
        let q = q.clamp(MIN_Q, MAX_Q);

        let a = math::pow(10.0, gain_db / 40.0);
        let w0 = TAU * (f0 / sample_rate);
        let sin_w0 = math::sin(w0);
        let cos_w0 = math::cos(w0);
        let alpha = sin_w0 / (2.0 * q);

        let (b0, b1, b2, a0, a1, a2) = match kind {
            Kind::Peak => {
                let boost = alpha * a;
                let cut = alpha / a;
                (
                    1.0 + boost,
                    -2.0 * cos_w0,
                    1.0 - boost,
                    1.0 + cut,
                    -2.0 * cos_w0,
                    1.0 - cut,
                )
            }
            Kind::LowShelf => {
                let ap1 = a + 1.0;
                let am1 = a - 1.0;
                let t = 2.0 * a.sqrt() * alpha;
                (
                    a * ((ap1 - am1 * cos_w0) + t),
                    2.0 * a * (am1 - ap1 * cos_w0),
                    a * ((ap1 - am1 * cos_w0) - t),
                    (ap1 + am1 * cos_w0) + t,
                    -2.0 * (am1 + ap1 * cos_w0),
                    (ap1 + am1 * cos_w0) - t,
                )
            }
            Kind::HighShelf => {
                let ap1 = a + 1.0;
                let am1 = a - 1.0;
                let t = 2.0 * a.sqrt() * alpha;
                (
                    a * ((ap1 + am1 * cos_w0) + t),
                    -2.0 * a * (am1 + ap1 * cos_w0),
                    a * ((ap1 + am1 * cos_w0) - t),
                    (ap1 - am1 * cos_w0) + t,
                    2.0 * (am1 - ap1 * cos_w0),
                    (ap1 - am1 * cos_w0) - t,
                )
            }
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// The cookbook's `peakingEQ`: [`Coeffs::design`] with [`Kind::Peak`].
    #[must_use]
    pub fn peak(freq_hz: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        Self::design(Kind::Peak, freq_hz, gain_db, q, sample_rate)
    }

    /// The cookbook's `lowShelf`: [`Coeffs::design`] with [`Kind::LowShelf`].
    #[must_use]
    pub fn low_shelf(freq_hz: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        Self::design(Kind::LowShelf, freq_hz, gain_db, q, sample_rate)
    }

    /// The cookbook's `highShelf`: [`Coeffs::design`] with [`Kind::HighShelf`].
    #[must_use]
    pub fn high_shelf(freq_hz: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        Self::design(Kind::HighShelf, freq_hz, gain_db, q, sample_rate)
    }

    /// Whether this is exactly [`Coeffs::IDENTITY`].
    #[must_use]
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// The complex response `H(e^{jw})` at `freq_hz` as `(re, im)`, evaluated in `f64` from the coefficients. For
    /// tests and displays; a frequency outside `0..=sample_rate / 2` aliases, as the filter does.
    #[must_use]
    pub fn response(&self, freq_hz: f64, sample_rate: f64) -> (f64, f64) {
        let w = TAU * (freq_hz / sample_rate);
        let (c1, s1) = (math::cos(w), math::sin(w));
        let (c2, s2) = (math::cos(2.0 * w), math::sin(2.0 * w));
        let num_re = (self.b0 + self.b1 * c1) + self.b2 * c2;
        let num_im = -(self.b1 * s1 + self.b2 * s2);
        let den_re = (1.0 + self.a1 * c1) + self.a2 * c2;
        let den_im = -(self.a1 * s1 + self.a2 * s2);
        let norm = den_re * den_re + den_im * den_im;
        (
            (num_re * den_re + num_im * den_im) / norm,
            (num_im * den_re - num_re * den_im) / norm,
        )
    }

    /// The magnitude response `20·log10 |H(e^{jw})|` in dB at `freq_hz` (see [`Coeffs::response`]).
    #[must_use]
    pub fn magnitude_db(&self, freq_hz: f64, sample_rate: f64) -> f64 {
        let w = TAU * (freq_hz / sample_rate);
        let (c1, s1) = (math::cos(w), math::sin(w));
        let (c2, s2) = (math::cos(2.0 * w), math::sin(2.0 * w));
        let num_re = (self.b0 + self.b1 * c1) + self.b2 * c2;
        let num_im = self.b1 * s1 + self.b2 * s2;
        let den_re = (1.0 + self.a1 * c1) + self.a2 * c2;
        let den_im = self.a1 * s1 + self.a2 * s2;
        let num = num_re * num_re + num_im * num_im;
        let den = den_re * den_re + den_im * den_im;
        10.0 * math::log10(num / den)
    }
}

/// One biquad section running over `f32` audio: [`Coeffs`] plus two `f64` states (transposed direct form II).
#[derive(Clone, Debug, PartialEq)]
pub struct Biquad {
    coeffs: Coeffs,
    s1: f64,
    s2: f64,
}

impl Biquad {
    /// A section with the given coefficients and cleared state.
    #[must_use]
    pub const fn new(coeffs: Coeffs) -> Self {
        Self {
            coeffs,
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// The coefficients.
    #[must_use]
    pub const fn coeffs(&self) -> &Coeffs {
        &self.coeffs
    }

    /// Clear the state (a new clip).
    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    /// One sample. A non-finite input poisons the state until [`Biquad::reset`], as for any recursive filter.
    #[inline(always)]
    #[must_use]
    pub fn tick(&mut self, input: f32) -> f32 {
        let c = &self.coeffs;
        let x = f64::from(input);
        let y = c.b0 * x + self.s1;
        let s1 = (c.b1 * x - c.a1 * y) + self.s2;
        let s2 = c.b2 * x - c.a2 * y;
        self.s1 = flush64(s1);
        self.s2 = flush64(s2);
        flush64(y) as f32
    }

    /// Filter `buf` in place, continuing from the state the last call left (so a clip may be processed in blocks of
    /// any size with the same bits). An identity section leaves the buffer untouched, bit for bit. No allocation.
    pub fn process(&mut self, buf: &mut [f32]) {
        if self.coeffs.is_identity() {
            return;
        }
        for sample in buf {
            *sample = self.tick(*sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::Strategy as _;

    const KINDS: [Kind; 3] = [Kind::Peak, Kind::LowShelf, Kind::HighShelf];

    // -----------------------------------------------------------------------------------------------------------
    // The reference: the cookbook's analog prototypes under its bilinear transform
    // -----------------------------------------------------------------------------------------------------------

    /// A complex number, for the reference only.
    #[derive(Clone, Copy)]
    struct C(f64, f64);

    impl C {
        fn add(self, o: C) -> C {
            C(self.0 + o.0, self.1 + o.1)
        }
        fn mul(self, o: C) -> C {
            C(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
        }
        fn scale(self, k: f64) -> C {
            C(self.0 * k, self.1 * k)
        }
        fn abs2(self) -> f64 {
            self.0 * self.0 + self.1 * self.1
        }
    }

    /// The cookbook's analog prototype (in its `Q` form, corner at `s = j1`) at the normalised `s = j·x`:
    ///
    /// ```text
    /// peakingEQ  H(s) =       (s² + s·(A/Q) + 1) / (s² + s/(A·Q) + 1)
    /// lowShelf   H(s) = A ·   (s² + s·(√A/Q) + A) / (A·s² + s·(√A/Q) + 1)
    /// highShelf  H(s) = A ·   (A·s² + s·(√A/Q) + 1) / (s² + s·(√A/Q) + A)
    /// ```
    ///
    /// returned as dB. `x = tan(w/2) / tan(w0/2)` is the bilinear transform with the corner pre-warped to `w0`.
    fn prototype_db(kind: Kind, gain_db: f64, q: f64, x: f64) -> f64 {
        let a = libm::pow(10.0, gain_db / 40.0);
        let s = C(0.0, x);
        let s2 = s.mul(s);
        let poly = |c2: f64, c1: f64, c0: f64| s2.scale(c2).add(s.scale(c1)).add(C(c0, 0.0));
        let (num, den, k) = match kind {
            Kind::Peak => (poly(1.0, a / q, 1.0), poly(1.0, 1.0 / (a * q), 1.0), 1.0),
            Kind::LowShelf => {
                let r = a.sqrt() / q;
                (poly(1.0, r, a), poly(a, r, 1.0), a)
            }
            Kind::HighShelf => {
                let r = a.sqrt() / q;
                (poly(a, r, 1.0), poly(1.0, r, a), a)
            }
        };
        10.0 * libm::log10(k * k * num.abs2() / den.abs2())
    }

    /// The cookbook's response in dB at `f`: the prototype at the pre-warped frequency.
    fn cookbook_db(kind: Kind, f0: f64, gain_db: f64, q: f64, sr: f64, f: f64) -> f64 {
        let x =
            libm::tan(core::f64::consts::PI * f / sr) / libm::tan(core::f64::consts::PI * f0 / sr);
        prototype_db(kind, gain_db, q, x)
    }

    /// `n` frequencies, log-spaced from `lo` to `hi`.
    fn log_grid(lo: f64, hi: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| lo * libm::pow(hi / lo, i as f64 / (n - 1) as f64))
            .collect()
    }

    const GAINS: [f64; 8] = [-24.0, -12.0, -3.0, -0.01, 0.01, 3.0, 12.0, 24.0];
    const QS: [f64; 7] = [
        0.1,
        0.5,
        core::f64::consts::FRAC_1_SQRT_2,
        1.0,
        4.0,
        8.0,
        25.0,
    ];
    const SRS: [f64; 3] = [44_100.0, 48_000.0, 96_000.0];
    const FREQS: [f64; 7] = [20.0, 63.0, 250.0, 1_000.0, 4_000.0, 12_000.0, 20_000.0];

    #[test]
    fn coefficients_follow_the_cookbook_prototypes_through_the_bilinear_transform() {
        // The digital filter is the analog prototype under s = j·tan(w/2)/tan(w0/2); f64 design, f64 evaluation, so
        // the two agree to rounding. This is the independent check of the transcribed equations.
        let mut worst = 0.0f64;
        let mut cases = 0;
        for kind in KINDS {
            for sr in SRS {
                for f0 in FREQS {
                    for gain in GAINS {
                        for q in QS {
                            let c = Coeffs::design(kind, f0, gain, q, sr);
                            for f in log_grid(5.0, 0.499 * sr, 24) {
                                let got = c.magnitude_db(f, sr);
                                let want = cookbook_db(kind, f0, gain, q, sr, f);
                                worst = worst.max((got - want).abs());
                                cases += 1;
                                assert!(
                                    (got - want).abs() < 1e-6,
                                    "{kind:?} {f0} Hz {gain} dB Q {q} sr {sr} at {f}: {got} vs {want}"
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(cases > 50_000, "{cases}");
        println!("max |coefficients - prototype| over {cases} points: {worst:.3e} dB");
    }

    #[test]
    fn landmarks_of_the_responses() {
        let sr = 48_000.0;
        for (f0, gain, q) in [
            (1_000.0, 12.0, 1.0),
            (250.0, -9.5, 0.3),
            (8_000.0, 24.0, 8.0),
            (60.0, -24.0, 4.0),
            (16_000.0, 6.0, 0.7),
        ] {
            // A peak is exactly `gain` at its centre and unity at DC and at Nyquist.
            let p = Coeffs::peak(f0, gain, q, sr);
            assert!(
                (p.magnitude_db(f0, sr) - gain).abs() < 1e-9,
                "peak centre {f0}"
            );
            assert!(p.magnitude_db(0.0, sr).abs() < 1e-9);
            assert!(p.magnitude_db(sr / 2.0, sr).abs() < 1e-9);

            // A low shelf is `gain` at DC, half of it (in dB) at the corner, unity at Nyquist.
            let l = Coeffs::low_shelf(f0, gain, q, sr);
            assert!(
                (l.magnitude_db(0.0, sr) - gain).abs() < 1e-9,
                "low shelf DC {f0}"
            );
            assert!(
                (l.magnitude_db(f0, sr) - gain / 2.0).abs() < 1e-9,
                "low shelf corner {f0}"
            );
            assert!(l.magnitude_db(sr / 2.0, sr).abs() < 1e-9);

            // A high shelf mirrors it.
            let h = Coeffs::high_shelf(f0, gain, q, sr);
            assert!(h.magnitude_db(0.0, sr).abs() < 1e-9);
            assert!(
                (h.magnitude_db(f0, sr) - gain / 2.0).abs() < 1e-9,
                "high shelf corner {f0}"
            );
            assert!(
                (h.magnitude_db(sr / 2.0, sr) - gain).abs() < 1e-9,
                "high shelf Nyquist {f0}"
            );
        }
    }

    #[test]
    fn opposite_gains_are_inverse_sections() {
        // The cookbook's boost and cut are exact inverses: H(+g)·H(-g) = 1 at every frequency.
        let sr = 44_100.0;
        for kind in KINDS {
            for (f0, gain, q) in [(300.0, 9.0, 1.0), (5_000.0, 24.0, 0.4), (40.0, 3.0, 12.0)] {
                let up = Coeffs::design(kind, f0, gain, q, sr);
                let down = Coeffs::design(kind, f0, -gain, q, sr);
                for f in log_grid(10.0, 20_000.0, 40) {
                    let sum = up.magnitude_db(f, sr) + down.magnitude_db(f, sr);
                    assert!(sum.abs() < 1e-9, "{kind:?} {f0} {gain} at {f}: {sum}");
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------------
    // The processed response: the impulse response through `Biquad::process`, read at many frequencies
    // -----------------------------------------------------------------------------------------------------------

    /// The impulse response of `c` through [`Biquad::process`] in `f32`, run until the last chunk is below `1e-10`
    /// (or `cap` samples). Returns it and whether it settled.
    fn impulse_response(c: Coeffs, cap: usize) -> (Vec<f32>, bool) {
        let mut b = Biquad::new(c);
        let mut h = vec![0.0f32; 4096];
        h[0] = 1.0;
        b.process(&mut h);
        let mut settled = false;
        while h.len() < cap {
            let mut chunk = vec![0.0f32; 4096];
            b.process(&mut chunk);
            let quiet = chunk.iter().all(|v| v.abs() < 1e-10);
            h.extend_from_slice(&chunk);
            if quiet {
                settled = true;
                break;
            }
        }
        (h, settled)
    }

    /// `|DTFT(h)|` in dB at `f` (a rotating phasor, re-synchronised from `sin`/`cos` every 1,024 samples).
    fn dtft_db(h: &[f32], f: f64, sr: f64) -> f64 {
        let w = TAU * (f / sr);
        let (rc, rs) = (math::cos(w), -math::sin(w));
        let (mut re, mut im) = (0.0f64, 0.0f64);
        let (mut pc, mut ps) = (1.0f64, 0.0f64);
        for (n, &v) in h.iter().enumerate() {
            if n % 1024 == 0 {
                let ph = w * n as f64;
                pc = math::cos(ph);
                ps = -math::sin(ph);
            }
            let v = f64::from(v);
            re += v * pc;
            im += v * ps;
            (pc, ps) = (pc * rc - ps * rs, pc * rs + ps * rc);
        }
        10.0 * math::log10(re * re + im * im)
    }

    #[test]
    fn the_processed_response_is_within_a_tenth_of_a_decibel_of_the_cookbook() {
        let sr = 48_000.0;
        let mut worst = 0.0f64;
        let (mut tested, mut skipped) = (0, 0);
        for kind in KINDS {
            for f0 in [20.0, 100.0, 1_000.0, 8_000.0, 16_000.0, 20_000.0] {
                for gain in [-24.0, -6.0, 6.0, 24.0] {
                    for q in [0.1, core::f64::consts::FRAC_1_SQRT_2, 4.0, 25.0] {
                        let c = Coeffs::design(kind, f0, gain, q, sr);
                        let (h, settled) = impulse_response(c, 1 << 19);
                        if !settled {
                            // The longest rings (a 20 Hz boost at Q 25 takes minutes): the coefficient test covers them.
                            skipped += 1;
                            continue;
                        }
                        tested += 1;
                        let mut probes = log_grid(20.0, 0.45 * sr, 33);
                        probes.push(f0);
                        probes.push(0.0);
                        for f in probes {
                            let got = dtft_db(&h, f, sr);
                            let want = cookbook_db(kind, f0, gain, q, sr, f);
                            worst = worst.max((got - want).abs());
                            assert!(
                                (got - want).abs() < 0.1,
                                "{kind:?} {f0} Hz {gain} dB Q {q} at {f}: {got} vs {want}"
                            );
                        }
                    }
                }
            }
        }
        assert!(
            tested >= 250 && skipped <= 20,
            "tested {tested}, skipped {skipped}"
        );
        println!(
            "max |processed - cookbook| over {tested} sections: {worst:.3e} dB ({skipped} too long to settle)"
        );
    }

    #[test]
    fn a_low_band_with_a_narrow_q_is_exact_in_f64_where_f32_would_not_be() {
        // 40 Hz, Q 8 at 96 kHz: poles within 1e-3 of the unit circle. The processed response still matches.
        let sr = 96_000.0;
        let c = Coeffs::peak(40.0, 12.0, 8.0, sr);
        let (h, settled) = impulse_response(c, 1 << 20);
        assert!(settled);
        for f in log_grid(20.0, 2_000.0, 25) {
            let got = dtft_db(&h, f, sr);
            let want = cookbook_db(Kind::Peak, 40.0, 12.0, 8.0, sr, f);
            assert!((got - want).abs() < 0.01, "at {f}: {got} vs {want}");
        }
    }

    // -----------------------------------------------------------------------------------------------------------
    // Contracts
    // -----------------------------------------------------------------------------------------------------------

    fn noise(n: usize, seed: u64) -> Vec<f32> {
        let mut rng = crate::util::rng::Rng::new(seed);
        (0..n).map(|_| rng.bipolar_f32() * 0.5).collect()
    }

    #[test]
    fn no_gain_is_the_identity_bit_for_bit() {
        for kind in KINDS {
            for f0 in [20.0, 1_000.0, 19_000.0] {
                let c = Coeffs::design(kind, f0, 0.0, 1.0, 48_000.0);
                assert_eq!(c, Coeffs::IDENTITY);
                assert!(c.is_identity());
            }
        }
        let mut b = Biquad::new(Coeffs::peak(1_000.0, 0.0, 1.0, 48_000.0));
        let original = [
            0.25f32,
            -0.0,
            1e-30,
            f32::NAN,
            f32::INFINITY,
            -3.5,
            f32::MIN_POSITIVE / 4.0,
        ];
        let mut buf = original;
        b.process(&mut buf);
        for (a, b) in buf.iter().zip(original) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        // The identity section run sample by sample is exact for ordinary values too.
        let mut b = Biquad::new(Coeffs::IDENTITY);
        for x in [0.25f32, -0.9, 0.0, 1.0] {
            assert_eq!(b.tick(x).to_bits(), x.to_bits());
        }
    }

    #[test]
    fn bad_arguments_are_clamped_or_neutral() {
        let sr = 48_000.0;
        let base = Coeffs::peak(1_000.0, 6.0, 1.0, sr);
        assert!(!base.is_identity());
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(Coeffs::peak(bad, 6.0, 1.0, sr).is_identity());
            assert!(Coeffs::peak(1_000.0, bad, 1.0, sr).is_identity());
            assert!(Coeffs::peak(1_000.0, 6.0, bad, sr).is_identity());
            assert!(Coeffs::peak(1_000.0, 6.0, 1.0, bad).is_identity());
        }
        // A sample rate that cannot hold the lowest frequency.
        assert!(Coeffs::peak(1.0, 6.0, 1.0, 2.0).is_identity());
        assert!(Coeffs::peak(1.0, 6.0, 1.0, -48_000.0).is_identity());
        // Clamps.
        assert_eq!(
            Coeffs::peak(0.0, 6.0, 1.0, sr),
            Coeffs::peak(MIN_FREQ_HZ, 6.0, 1.0, sr)
        );
        assert_eq!(
            Coeffs::peak(-5.0, 6.0, 1.0, sr),
            Coeffs::peak(MIN_FREQ_HZ, 6.0, 1.0, sr)
        );
        assert_eq!(
            Coeffs::peak(1e9, 6.0, 1.0, sr),
            Coeffs::peak(MAX_FREQ_FRACTION * sr, 6.0, 1.0, sr)
        );
        assert_eq!(
            Coeffs::peak(1_000.0, 6.0, 0.0, sr),
            Coeffs::peak(1_000.0, 6.0, MIN_Q, sr)
        );
        assert_eq!(
            Coeffs::peak(1_000.0, 6.0, 1e9, sr),
            Coeffs::peak(1_000.0, 6.0, MAX_Q, sr)
        );
        assert_eq!(
            Coeffs::peak(1_000.0, 500.0, 1.0, sr),
            Coeffs::peak(1_000.0, MAX_GAIN_DB, 1.0, sr)
        );
        assert_eq!(
            Coeffs::peak(1_000.0, -500.0, 1.0, sr),
            Coeffs::peak(1_000.0, -MAX_GAIN_DB, 1.0, sr)
        );
    }

    #[test]
    fn the_host_block_size_is_not_an_input() {
        let input = noise(20_000, 7);
        for kind in KINDS {
            let c = Coeffs::design(kind, 700.0, 9.0, 1.5, 48_000.0);
            let mut whole = input.clone();
            Biquad::new(c).process(&mut whole);
            for block in [1usize, 7, 32, 100, 4_096, 20_000] {
                let mut b = Biquad::new(c);
                let mut chunked = input.clone();
                for piece in chunked.chunks_mut(block) {
                    b.process(piece);
                }
                assert!(
                    whole
                        .iter()
                        .zip(&chunked)
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                    "{kind:?} block {block}"
                );
            }
            // `tick` is `process`.
            let mut b = Biquad::new(c);
            let ticked: Vec<f32> = input.iter().map(|&x| b.tick(x)).collect();
            assert!(
                whole
                    .iter()
                    .zip(&ticked)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
        }
    }

    #[test]
    fn reset_restarts_the_clip() {
        let input = noise(5_000, 3);
        let c = Coeffs::low_shelf(400.0, -10.0, 0.9, 48_000.0);
        let mut b = Biquad::new(c);
        let mut first = input.clone();
        b.process(&mut first);
        assert_ne!((b.s1, b.s2), (0.0, 0.0));
        b.reset();
        assert_eq!(b, Biquad::new(c));
        let mut second = input.clone();
        b.process(&mut second);
        assert!(
            first
                .iter()
                .zip(&second)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }

    #[test]
    fn a_decaying_tail_reaches_exact_zero_without_subnormals() {
        for kind in KINDS {
            // A resonant boost: a long ring, then silence for the rest.
            let c = Coeffs::design(kind, 1_000.0, 12.0, 4.0, 48_000.0);
            let mut b = Biquad::new(c);
            let mut buf = vec![0.0f32; 600_000];
            buf[..64].copy_from_slice(&noise(64, 11));
            b.process(&mut buf);
            let tail = &buf[buf.len() - 4_096..];
            assert!(
                tail.iter().all(|v| v.to_bits() == 0),
                "{kind:?} tail is not +0.0"
            );
            assert_eq!((b.s1.to_bits(), b.s2.to_bits()), (0, 0), "{kind:?} state");
            assert!(
                buf.iter().all(|v| *v == 0.0 || v.abs() >= 1e-15),
                "{kind:?} a tiny output"
            );
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        /// Whatever band a question carries (the ranges of docs/challenge.md) and whatever the sample rate: the
        /// section is stable, hits its landmarks, and matches the cookbook at a random probe.
        #[test]
        fn any_band_is_stable_and_follows_the_cookbook(
            kind in 0usize..3,
            sr in proptest::sample::select(vec![32_000.0f64, 44_100.0, 48_000.0, 88_200.0, 96_000.0]),
            f0_unit in 0.0f64..1.0,
            gain_cdb in (1i32..=2_400).prop_flat_map(|g| proptest::prelude::prop_oneof![
                proptest::prelude::Just(g),
                proptest::prelude::Just(-g),
            ]),
            q_milli in 100u32..=25_000,
            probe in 20.0f64..15_000.0,
        ) {
            // Log-uniform from 20 Hz to 0.45 of the sample rate (the clamp is tested apart).
            let f0 = 20.0 * libm::pow(0.45 * sr / 20.0, f0_unit);
            let kind = KINDS[kind];
            let gain = f64::from(gain_cdb) / 100.0;
            let q = f64::from(q_milli) / 1000.0;
            let c = Coeffs::design(kind, f0, gain, q, sr);
            proptest::prop_assert!(c.a2.abs() < 1.0 && c.a1.abs() < 1.0 + c.a2, "unstable {c:?}");
            let got = c.magnitude_db(probe, sr);
            let want = cookbook_db(kind, f0, gain, q, sr, probe);
            proptest::prop_assert!((got - want).abs() < 1e-6, "{got} vs {want}");
            match kind {
                Kind::Peak => proptest::prop_assert!((c.magnitude_db(f0, sr) - gain).abs() < 1e-6),
                Kind::LowShelf => proptest::prop_assert!((c.magnitude_db(0.0, sr) - gain).abs() < 1e-6),
                Kind::HighShelf => proptest::prop_assert!((c.magnitude_db(sr / 2.0, sr) - gain).abs() < 1e-6),
            }
        }
    }

    #[test]
    fn extreme_sections_stay_finite_and_bounded() {
        // Stable for every argument (see `design`): the corners of the ranges, an impulse and then a loud tone.
        for kind in KINDS {
            for sr in [8_000.0, 44_100.0, 192_000.0] {
                for f0 in [0.0, 1.0, 20.0, 20_000.0, 1e9] {
                    for gain in [-120.0, -24.0, 24.0, 120.0] {
                        for q in [0.0, 0.1, 25.0, 1e6] {
                            let c = Coeffs::design(kind, f0, gain, q, sr);
                            let mut b = Biquad::new(c);
                            let mut buf = vec![0.0f32; 20_000];
                            buf[0] = 1.0;
                            b.process(&mut buf);
                            assert!(
                                buf.iter().all(|v| v.is_finite()),
                                "{kind:?} {sr} {f0} {gain} {q}"
                            );
                            assert!(
                                c.a2.abs() < 1.0 && c.a1.abs() < 1.0 + c.a2,
                                "{kind:?} {sr} {f0} {gain} {q}: unstable {c:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
