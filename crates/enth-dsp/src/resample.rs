//! Zero-latency IIR oversampling by two-path polyphase all-pass half-bands (S2a lane DST; `docs/dsp/dist.md`).
//!
//! The shared oversampler that `dist` uses first and other aliasing modules (a driven filter, a high-index PM pair) may
//! reuse. It is written from the cited papers, with no code taken from any library:
//!
//! - **Structure:** Valenzuela and Constantinides, *Digital signal processing schemes for efficient interpolation and
//!   decimation*, IEE Proc. G 130(6), 1983. A half-band low-pass is the average of two all-pass filters,
//!   `H(z) = ½·[A0(z²) + z⁻¹·A1(z²)]`, where each `Ai` is a chain of first-order all-pass sections
//!   `(α + z⁻¹)/(1 + α·z⁻¹)` with real `α` in (0, 1). In the polyphase form both chains run at the *lower* rate, so
//!   an interpolator by two costs one pass through each chain per input sample, and a decimator the same.
//! - **Design:** the coefficients are the poles of an odd-order **elliptic half-band** (a power-complementary
//!   low-pass with `ωp + ωs = π`): its analogue prototype has all poles on one circle, `|s| = ρ`, so each complex pair
//!   maps through `z = (1 + s/ρ)/(1 − s/ρ)` to a pair `±j·√α` on the imaginary axis, and `α = (1 − σ)/(1 + σ)` where
//!   `σ = −Re(s)/ρ`. The poles come from Jacobi elliptic functions; `design` (in the tests) is that computation, and it
//!   reproduces the committed [`HB_STAGE1`] and [`HB_STAGE2`] to 1e-13. Sorting the `α` ascending and giving
//!   alternate ones to `A0` and `A1` puts the odd-order low-pass in the path. References: A. Antoniou, *Digital
//!   Signal Processing* (2005) and L. Gazsi, *Explicit formulas for lattice wave digital filters*, IEEE Trans.
//!   CAS 32(1), 1985 (the same filters from the wave-digital side); M. Lutovac, D. Tosic and B. Evans, *Filter Design
//!   for Signal Processing Using MATLAB and Mathematica* (2001, the half-band chapter) for the pole/coefficient
//!   relation; and the elliptic-function evaluation by the descending Landen (AGM) transformation of Abramowitz and
//!   Stegun 16.4.
//! - **Stages:** 2× is one stage ([`HB_STAGE1`], nine poles: four all-pass sections, passband to 0.2 of the 2×
//!   rate, at least 70 dB of stopband from 0.3). 4× is two: the first at the base rate, the second ([`HB_STAGE2`],
//!   seven poles, three sections, at least 76.5 dB) running at 2× on the first one's output, where its images are
//!   far from the signal and a short filter is enough.
//!
//! **Zero latency.** Nothing is reported and nothing looks ahead: the filters are causal IIR all-passes, so the
//! group delay is a frequency-dependent 2.3 to 2.8 base-rate samples for 2× (3.3 to 3.9 for 4×) from 50 Hz to 10 kHz
//! (ADR-0006: effects report zero latency). The magnitude is flat (±2e-6 dB) from DC to 19 kHz at 48 kHz.
//!
//! **Precision and determinism.** `f64` throughout, `+ − × ×0.5` only, with a fixed order, so the same bits come out
//! on every platform; states are flushed by the caller once per segment ([`Allpass::flush`]).
//!
//! **A matched dry path.** Mixing a distorted signal with the clean one needs the clean one to have the wet path's
//! phase. For the linear part of an oversampler the identity chain (up, nothing, down) is `½·[A0² + z⁻¹·A1²]`, and the
//! two halves are in phase to within 6e-4 rad, so `A0` applied twice reproduces its phase with unit magnitude
//! ([`PhaseChain`]).

use crate::util::flush::flush64;

/// One first-order all-pass section `(a + z⁻¹)/(1 + a·z⁻¹)`: `y[n] = a·(x[n] − y[n−1]) + x[n−1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Allpass {
    x1: f64,
    y1: f64,
}

impl Allpass {
    /// A section at rest.
    pub const ZERO: Allpass = Allpass { x1: 0.0, y1: 0.0 };

    /// Run one sample through the section with coefficient `a`.
    #[inline(always)]
    pub fn step(&mut self, a: f64, x: f64) -> f64 {
        let y = a * (x - self.y1) + self.x1;
        self.x1 = x;
        self.y1 = y;
        y
    }

    /// Put the section in the steady state of a constant input `x` (an all-pass has unity gain at DC, so the output
    /// is `x` too).
    #[inline(always)]
    pub fn prime(&mut self, x: f64) {
        self.x1 = x;
        self.y1 = x;
    }

    /// Flush tiny state values to zero (`util::flush`).
    #[inline(always)]
    pub fn flush(&mut self) {
        self.x1 = flush64(self.x1);
        self.y1 = flush64(self.y1);
    }

    /// How many state values are subnormal (test support).
    #[must_use]
    pub fn subnormals(&self) -> usize {
        usize::from(self.x1.is_subnormal()) + usize::from(self.y1.is_subnormal())
    }
}

/// The coefficients of one half-band stage: the sections of `A0` and of `A1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefs<const N0: usize, const N1: usize> {
    /// The sections of the branch that carries the even output samples of an interpolator.
    pub a0: [f64; N0],
    /// The sections of the branch that carries the odd ones.
    pub a1: [f64; N1],
}

/// The first stage: nine poles, four sections (`A0` has the 1st and 3rd coefficient of the sorted design, `A1` the
/// 2nd and 4th). Elliptic half-band with the passband edge at 0.2 of the oversampled rate: at least 70 dB of
/// stopband from 0.3, passband ripple 4e-7 dB. Designed by the method in the module docs (`design(9, 0.2)`).
pub const HB_STAGE1: Coefs<2, 2> = Coefs {
    a0: [0.07986642623635758, 0.5453236510711319],
    a1: [0.2838293448741102, 0.8344118914807372],
};

/// The second stage of 4×, running at 2× on the first one's output: seven poles, three sections, the passband edge at
/// 0.15 of the 4× rate (the first stage's stopband, 0.6 of the base rate, reflected about the 2× Nyquist is 1.4 of
/// it), at least 76.5 dB of stopband. `design(7, 0.15)`.
pub const HB_STAGE2: Coefs<2, 1> = Coefs {
    a0: [0.08143002317661638, 0.7097708001024805],
    a1: [0.3156598402166598],
};

/// The interpolating half of a stage: one input sample in, two samples at twice the rate out, in time order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Up<const N0: usize, const N1: usize> {
    a0: [Allpass; N0],
    a1: [Allpass; N1],
}

impl<const N0: usize, const N1: usize> Default for Up<N0, N1> {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<const N0: usize, const N1: usize> Up<N0, N1> {
    /// A stage at rest.
    pub const ZERO: Self = Self {
        a0: [Allpass::ZERO; N0],
        a1: [Allpass::ZERO; N1],
    };

    /// Interpolate: `[y[2n], y[2n+1]] = [A0·x, A1·x]` (unity gain: the polyphase form of `2·H(z)` on the
    /// zero-stuffed input).
    #[inline(always)]
    pub fn process(&mut self, c: &Coefs<N0, N1>, x: f64) -> [f64; 2] {
        let mut e = x;
        for (s, &a) in self.a0.iter_mut().zip(&c.a0) {
            e = s.step(a, e);
        }
        let mut o = x;
        for (s, &a) in self.a1.iter_mut().zip(&c.a1) {
            o = s.step(a, o);
        }
        [e, o]
    }

    /// Steady state for a constant input `x`.
    #[inline(always)]
    pub fn prime(&mut self, x: f64) {
        for s in &mut self.a0 {
            s.prime(x);
        }
        for s in &mut self.a1 {
            s.prime(x);
        }
    }

    /// Flush tiny state values.
    #[inline(always)]
    pub fn flush(&mut self) {
        for s in &mut self.a0 {
            s.flush();
        }
        for s in &mut self.a1 {
            s.flush();
        }
    }

    /// Subnormal state values (test support).
    #[must_use]
    pub fn subnormals(&self) -> usize {
        self.a0
            .iter()
            .chain(&self.a1)
            .map(Allpass::subnormals)
            .sum()
    }
}

/// The decimating half of a stage: two samples in (time order), one out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Down<const N0: usize, const N1: usize> {
    a0: [Allpass; N0],
    a1: [Allpass; N1],
    /// The odd branch's output of the previous pair (the `z⁻¹` between the branches).
    odd: f64,
}

impl<const N0: usize, const N1: usize> Default for Down<N0, N1> {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<const N0: usize, const N1: usize> Down<N0, N1> {
    /// A stage at rest.
    pub const ZERO: Self = Self {
        a0: [Allpass::ZERO; N0],
        a1: [Allpass::ZERO; N1],
        odd: 0.0,
    };

    /// Decimate the pair `(even, odd)`: `y[n] = ½·(A0·v[2n] + A1·v[2n−1])`.
    #[inline(always)]
    pub fn process(&mut self, c: &Coefs<N0, N1>, even: f64, odd: f64) -> f64 {
        let mut e = even;
        for (s, &a) in self.a0.iter_mut().zip(&c.a0) {
            e = s.step(a, e);
        }
        let mut o = odd;
        for (s, &a) in self.a1.iter_mut().zip(&c.a1) {
            o = s.step(a, o);
        }
        let y = 0.5 * e + 0.5 * self.odd;
        self.odd = o;
        y
    }

    /// Steady state for a constant input `x` (every sample of every pair equal to `x`).
    #[inline(always)]
    pub fn prime(&mut self, x: f64) {
        for s in &mut self.a0 {
            s.prime(x);
        }
        for s in &mut self.a1 {
            s.prime(x);
        }
        self.odd = x;
    }

    /// Flush tiny state values.
    #[inline(always)]
    pub fn flush(&mut self) {
        for s in &mut self.a0 {
            s.flush();
        }
        for s in &mut self.a1 {
            s.flush();
        }
        self.odd = flush64(self.odd);
    }

    /// Subnormal state values (test support).
    #[must_use]
    pub fn subnormals(&self) -> usize {
        self.a0
            .iter()
            .chain(&self.a1)
            .map(Allpass::subnormals)
            .sum::<usize>()
            + usize::from(self.odd.is_subnormal())
    }
}

/// An oversampler by 1, 2 or 4 around a memoryless or stateful per-sample function: up, the function at the high
/// rate, down. The factor is a const generic of [`Oversampler::process`], so each factor compiles to straight code.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Oversampler {
    up1: Up<2, 2>,
    down1: Down<2, 2>,
    up2: Up<2, 1>,
    down2: Down<2, 1>,
}

impl Oversampler {
    /// An oversampler at rest.
    pub const ZERO: Self = Self {
        up1: Up::ZERO,
        down1: Down::ZERO,
        up2: Up::ZERO,
        down2: Down::ZERO,
    };

    /// One base-rate sample through `f` at `K` times the rate (`K` is 1, 2 or 4): `f` is called `K` times, in time
    /// order, on the interpolated samples, and the results are decimated. `K` = 1 is `f(x)`.
    #[inline(always)]
    pub fn process<const K: usize>(&mut self, x: f64, f: &mut impl FnMut(f64) -> f64) -> f64 {
        match K {
            1 => f(x),
            2 => {
                let [e, o] = self.up1.process(&HB_STAGE1, x);
                let fe = f(e);
                let fo = f(o);
                self.down1.process(&HB_STAGE1, fe, fo)
            }
            _ => {
                let [e, o] = self.up1.process(&HB_STAGE1, x);
                let [e0, e1] = self.up2.process(&HB_STAGE2, e);
                let [o0, o1] = self.up2.process(&HB_STAGE2, o);
                let fe0 = f(e0);
                let fe1 = f(e1);
                let fo0 = f(o0);
                let fo1 = f(o1);
                let fe = self.down2.process(&HB_STAGE2, fe0, fe1);
                let fo = self.down2.process(&HB_STAGE2, fo0, fo1);
                self.down1.process(&HB_STAGE1, fe, fo)
            }
        }
    }

    /// Steady state for a constant input `x` whose image through the function is the constant `fx`: the up stages hold
    /// `x`, the down stages `fx`.
    #[inline]
    pub fn prime(&mut self, x: f64, fx: f64) {
        self.up1.prime(x);
        self.up2.prime(x);
        self.down1.prime(fx);
        self.down2.prime(fx);
    }

    /// Flush tiny state values.
    #[inline]
    pub fn flush(&mut self) {
        self.up1.flush();
        self.up2.flush();
        self.down1.flush();
        self.down2.flush();
    }

    /// Subnormal state values (test support).
    #[must_use]
    pub fn subnormals(&self) -> usize {
        self.up1.subnormals()
            + self.up2.subnormals()
            + self.down1.subnormals()
            + self.down2.subnormals()
    }
}

/// The most sections a [`PhaseChain`] has.
pub const PHASE_SECTIONS: usize = 5;

/// A fixed chain of all-pass sections whose phase is that of the linear part of the `dist` wet path at one
/// oversampling factor: the oversampler's identity chain (`A0` twice for 2× and 4×) followed by one section that
/// carries the rest, the half-sample delay of the first-order ADAA at the oversampled rate and, for 4×, the second
/// stage. The trailing coefficient is a least-squares fit of the phase over 50 Hz to 12 kHz (maximum error 1.3° at
/// 2×, 2.1° at 1×, 2.6° at 4×, `phase_chains_follow_the_linear_wet_path` in the tests). The magnitude is exactly 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhaseChain {
    s: [Allpass; PHASE_SECTIONS],
}

/// The phase chain's coefficients for oversampling by `factor` (1, 2 or 4; anything else gives 4's), and how many
/// sections they use.
#[must_use]
pub const fn phase_chain_coefs(factor: usize) -> ([f64; PHASE_SECTIONS], usize) {
    let [p0, p1] = [HB_STAGE1.a0[0], HB_STAGE1.a0[1]];
    match factor {
        1 => ([PHASE_ADAA_1, 0.0, 0.0, 0.0, 0.0], 1),
        2 => ([p0, p0, p1, p1, PHASE_ADAA_2], 5),
        _ => ([p0, p0, p1, p1, PHASE_ADAA_4], 5),
    }
}

/// The last section for the wet path without oversampling: the first-order ADAA is a two-tap average, a delay of half a
/// sample.
pub const PHASE_ADAA_1: f64 = 0.3933182303573387;
/// The last section for 2×.
pub const PHASE_ADAA_2: f64 = 0.6521487138786072;
/// The last section for 4×.
pub const PHASE_ADAA_4: f64 = -0.10712356954513495;

impl PhaseChain {
    /// A chain at rest.
    pub const ZERO: Self = Self {
        s: [Allpass::ZERO; PHASE_SECTIONS],
    };

    /// One sample through the first `n` sections with coefficients `c` (from [`phase_chain_coefs`]).
    #[inline(always)]
    pub fn step(&mut self, c: &[f64; PHASE_SECTIONS], n: usize, x: f64) -> f64 {
        let mut y = x;
        for (s, &a) in self.s.iter_mut().zip(c).take(n) {
            y = s.step(a, y);
        }
        y
    }

    /// Steady state for a constant input `x`.
    #[inline(always)]
    pub fn prime(&mut self, x: f64) {
        for s in &mut self.s {
            s.prime(x);
        }
    }

    /// Flush tiny state values.
    #[inline(always)]
    pub fn flush(&mut self) {
        for s in &mut self.s {
            s.flush();
        }
    }

    /// Subnormal state values (test support).
    #[must_use]
    pub fn subnormals(&self) -> usize {
        self.s.iter().map(Allpass::subnormals).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Test-only maths straight from the pinned crate, as `biquad.rs`'s tests do: the designer is a reference
    // computation, not part of the rendered signal.
    use libm::{atan2, cos, hypot, log10, sin};

    /// The design method of the module docs: the `α` of an odd-order elliptic half-band of order `n` (`n` odd, at least
    /// 3) with its passband edge at `fp` of the oversampled rate (0 < fp < 0.25), sorted ascending, and the stopband
    /// attenuation in dB. Real arithmetic only: Jacobi elliptic functions by the descending Landen transformation
    /// (Abramowitz and Stegun 16.4), the real pole by bisection.
    mod design {
        use core::f64::consts::PI;
        use libm::{asin, cos, hypot, log10, sin, tan};

        /// `K(m)` by the arithmetic-geometric mean.
        fn ellipk(m: f64) -> f64 {
            let (mut a, mut b) = (1.0, (1.0 - m).sqrt());
            for _ in 0..60 {
                if (a - b).abs() < 1e-16 {
                    break;
                }
                (a, b) = (0.5 * a + 0.5 * b, (a * b).sqrt());
            }
            PI / (2.0 * a)
        }

        /// `sn, cn, dn (u | m)`.
        fn ellipj(u: f64, m: f64) -> (f64, f64, f64) {
            let mut a = vec![1.0];
            let mut b = vec![(1.0 - m).sqrt()];
            let mut c = vec![m.sqrt()];
            let mut n = 0;
            while c[n].abs() > 1e-16 && n < 30 {
                let (an, bn) = (a[n], b[n]);
                a.push(0.5 * an + 0.5 * bn);
                c.push(0.5 * (an - bn));
                b.push((an * bn).sqrt());
                n += 1;
            }
            let mut phi = f64::from(1u32 << n) * a[n] * u;
            let mut phi1 = phi;
            for i in (1..=n).rev() {
                phi1 = phi;
                phi = 0.5 * phi + 0.5 * asin((c[i] * sin(phi) / a[i]).clamp(-1.0, 1.0));
            }
            let (sn, cn) = (sin(phi), cos(phi));
            let dn = if n > 0 { cn / cos(phi1 - phi) } else { 1.0 };
            (sn, cn, dn)
        }

        pub fn alphas(n: usize, fp: f64) -> (Vec<f64>, f64) {
            assert!(n % 2 == 1 && n >= 3 && fp > 0.0 && fp < 0.25);
            let rho = 1.0 / tan(PI * fp);
            let k = 1.0 / (rho * rho);
            let m = k * k;
            let (kk, kp) = (ellipk(m), ellipk(1.0 - m));
            // The real pole sits at −ρ: sc(v0 | 1 − m) = ρ.
            let (mut lo, mut hi) = (0.0, kp * (1.0 - 1e-12));
            for _ in 0..200 {
                let mid = 0.5 * lo + 0.5 * hi;
                let (s, c, _) = ellipj(mid, 1.0 - m);
                if s / c < rho {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let v0 = 0.5 * lo + 0.5 * hi;
            let (sv, cv, dv) = ellipj(v0, 1.0 - m);
            let mut alphas = Vec::new();
            for j in (2..n).step_by(2) {
                let (s, c, d) = ellipj(j as f64 * kk / n as f64, m);
                let den = 1.0 - (d * sv) * (d * sv);
                let re = -(c * d * sv * cv) / den;
                let im = -(s * dv) / den;
                assert!(
                    (hypot(re, im) - rho).abs() < 1e-9 * rho,
                    "poles on a circle"
                );
                let sigma = -re / rho;
                alphas.push((1.0 - sigma) / (1.0 + sigma));
            }
            alphas.sort_by(f64::total_cmp);
            // The stopband level, from the frequency response at the stopband edge ωs = π − ωp.
            let edge = 0.5 - fp;
            let att = -20.0 * log10(response(&alphas, edge));
            (alphas, att)
        }

        /// `|H|` of the half-band built from the sorted `α` at `f` (a fraction of the oversampled rate), with the
        /// even-indexed coefficients in `A0` and the odd-indexed in `A1`.
        pub fn response(alphas: &[f64], f: f64) -> f64 {
            let w = 2.0 * PI * f;
            let (c2, s2) = (cos(-2.0 * w), sin(-2.0 * w));
            let mut a = [(1.0, 0.0), (1.0, 0.0)];
            for (i, &al) in alphas.iter().enumerate() {
                // (α + z⁻²)/(1 + α z⁻²) with z⁻² = c2 + j s2.
                let (nr, ni) = (al + c2, s2);
                let (dr, di) = (1.0 + al * c2, al * s2);
                let q = dr * dr + di * di;
                let (qr, qi) = ((nr * dr + ni * di) / q, (ni * dr - nr * di) / q);
                let (ar, ai) = a[i % 2];
                a[i % 2] = (ar * qr - ai * qi, ar * qi + ai * qr);
            }
            let (c1, s1) = (cos(w), -sin(w));
            let (br, bi) = (a[1].0 * c1 - a[1].1 * s1, a[1].0 * s1 + a[1].1 * c1);
            0.5 * hypot(a[0].0 + br, a[0].1 + bi)
        }
    }

    #[test]
    fn the_committed_coefficients_are_the_design() {
        let (a1, att1) = design::alphas(9, 0.2);
        let c1 = [
            HB_STAGE1.a0[0],
            HB_STAGE1.a1[0],
            HB_STAGE1.a0[1],
            HB_STAGE1.a1[1],
        ];
        for (got, want) in c1.iter().zip(&a1) {
            assert!((got - want).abs() < 1e-13, "{got} vs {want}");
        }
        assert!(att1 > 69.9, "{att1} dB");
        let (a2, att2) = design::alphas(7, 0.15);
        let c2 = [HB_STAGE2.a0[0], HB_STAGE2.a1[0], HB_STAGE2.a0[1]];
        for (got, want) in c2.iter().zip(&a2) {
            assert!((got - want).abs() < 1e-13, "{got} vs {want}");
        }
        assert!(att2 > 76.5, "{att2} dB");
    }

    /// Prints the design for the constants above and `docs/dsp/dist.md` (`cargo test -p enth-dsp --lib
    /// print_halfband_design -- --ignored --nocapture`).
    #[test]
    #[ignore = "prints the coefficient tables"]
    fn print_halfband_design() {
        for (n, fp) in [(9, 0.2), (7, 0.15)] {
            let (a, att) = design::alphas(n, fp);
            println!("order {n}, passband edge {fp} of the oversampled rate: {att:.2} dB");
            for x in a {
                println!("    {x:.17}");
            }
        }
    }

    #[test]
    fn the_half_bands_are_flat_in_the_passband_and_deep_in_the_stopband() {
        for (alphas, fp, stop_db, ripple_db) in [
            (
                vec![
                    HB_STAGE1.a0[0],
                    HB_STAGE1.a1[0],
                    HB_STAGE1.a0[1],
                    HB_STAGE1.a1[1],
                ],
                0.2,
                69.9,
                1e-5,
            ),
            (
                vec![HB_STAGE2.a0[0], HB_STAGE2.a1[0], HB_STAGE2.a0[1]],
                0.15,
                76.5,
                1e-5,
            ),
        ] {
            let mut worst_pass = 0f64;
            let mut best_stop = f64::INFINITY;
            for i in 0..=2000 {
                let f = 0.5 * f64::from(i) / 2000.0;
                let db = 20.0 * log10(design::response(&alphas, f));
                if f <= fp {
                    worst_pass = worst_pass.max(db.abs());
                }
                if f >= 0.5 - fp {
                    best_stop = best_stop.min(-db);
                }
                // Power complementary: |H(f)|² + |H(½ − f)|² = 1.
                let (r0, r1) = (
                    design::response(&alphas, f),
                    design::response(&alphas, 0.5 - f),
                );
                let p = r0 * r0 + r1 * r1;
                assert!((p - 1.0).abs() < 1e-12, "complementary at {f}: {p}");
            }
            assert!(worst_pass < ripple_db, "passband {worst_pass} dB");
            assert!(best_stop > stop_db, "stopband {best_stop} dB");
        }
    }

    /// The impulse response of `process::<K>` through the identity function, `n` samples.
    fn impulse<const K: usize>(n: usize) -> Vec<f64> {
        let mut os = Oversampler::ZERO;
        (0..n)
            .map(|i| os.process::<K>(if i == 0 { 1.0 } else { 0.0 }, &mut |v| v))
            .collect()
    }

    /// `(magnitude, phase)` of a finite response at `f` (a fraction of the base rate).
    fn dft(h: &[f64], f: f64) -> (f64, f64) {
        let (mut re, mut im) = (0.0, 0.0);
        for (n, &v) in h.iter().enumerate() {
            let a = -2.0 * core::f64::consts::PI * f * n as f64;
            re += v * cos(a);
            im += v * sin(a);
        }
        (hypot(re, im), atan2(im, re))
    }

    #[test]
    fn the_identity_chains_are_flat_and_have_no_latency() {
        for (k, h) in [(2, impulse::<2>(4096)), (4, impulse::<4>(4096))] {
            // Flat from 50 Hz to 19 kHz at 48 kHz (a fraction of the base rate).
            for hz in [50.0, 500.0, 2_000.0, 8_000.0, 15_000.0, 19_000.0] {
                let (mag, _) = dft(&h, hz / 48_000.0);
                assert!((20.0 * log10(mag)).abs() < 1e-4, "{k}x at {hz} Hz: {mag}");
            }
            // Causal with no look-ahead: the impulse response starts at once, and the early samples are already
            // the bulk of the energy (a linear-phase FIR would start with pre-ringing and a delay of tens).
            let energy: f64 = h.iter().map(|v| v * v).sum();
            let early: f64 = h[..16].iter().map(|v| v * v).sum();
            assert!(early > 0.9 * energy, "{k}x: {early} of {energy}");
            assert!(h[0].abs() > 0.0 || h[1].abs() > 0.0);
        }
    }

    #[test]
    fn the_group_delay_is_a_few_samples() {
        let delay = |h: &[f64], hz: f64| {
            let (f0, f1) = (hz / 48_000.0, hz * 1.001 / 48_000.0);
            let (_, p0) = dft(h, f0);
            let (_, p1) = dft(h, f1);
            let mut d = p0 - p1;
            if d > core::f64::consts::PI {
                d -= 2.0 * core::f64::consts::PI;
            } else if d < -core::f64::consts::PI {
                d += 2.0 * core::f64::consts::PI;
            }
            d / (2.0 * core::f64::consts::PI * (f1 - f0))
        };
        let h2 = impulse::<2>(4096);
        let h4 = impulse::<4>(4096);
        assert!(
            (delay(&h2, 100.0) - 2.29).abs() < 0.02,
            "{}",
            delay(&h2, 100.0)
        );
        assert!((delay(&h2, 10_000.0) - 2.78).abs() < 0.03);
        assert!(
            (delay(&h4, 100.0) - 3.32).abs() < 0.02,
            "{}",
            delay(&h4, 100.0)
        );
        assert!((delay(&h4, 10_000.0) - 3.85).abs() < 0.03);
    }

    /// The phase of a chain's response at `f` (a fraction of the base rate).
    fn chain_phase(factor: usize, f: f64) -> f64 {
        let (c, n) = phase_chain_coefs(factor);
        let w = 2.0 * core::f64::consts::PI * f;
        let (zr, zi) = (cos(w), -sin(w));
        let mut phase = 0.0;
        for &a in &c[..n] {
            // (a + z⁻¹)/(1 + a z⁻¹)
            let (nr, ni) = (a + zr, zi);
            let (dr, di) = (1.0 + a * zr, a * zi);
            phase += atan2(ni, nr) - atan2(di, dr);
        }
        phase
    }

    #[test]
    fn phase_chains_follow_the_linear_wet_path() {
        // The linearised wet path: the oversampler around a two-tap average, which is first-order ADAA of a straight
        // line (x + x1)/2 at the oversampled rate.
        fn wet<const K: usize>() -> Vec<f64> {
            let mut os = Oversampler::ZERO;
            let mut x1 = 0.0;
            (0..8192)
                .map(|i| {
                    let x = if i == 0 { 1.0 } else { 0.0 };
                    os.process::<K>(x, &mut |v| {
                        let y = 0.5 * v + 0.5 * x1;
                        x1 = v;
                        y
                    })
                })
                .collect()
        }
        for (factor, h, bound_deg) in [
            (1, wet::<1>(), 2.2),
            (2, wet::<2>(), 1.4),
            (4, wet::<4>(), 2.8),
        ] {
            let mut worst = 0f64;
            let mut prev = None;
            let mut offset = 0.0;
            let mut hz = 50.0;
            while hz <= 12_000.0 {
                let (_, p) = dft(&h, hz / 48_000.0);
                // Unwrap the measured phase along the sweep.
                let mut p = p + offset;
                if let Some(q) = prev {
                    while p - q > core::f64::consts::PI {
                        p -= 2.0 * core::f64::consts::PI;
                        offset -= 2.0 * core::f64::consts::PI;
                    }
                    while p - q < -core::f64::consts::PI {
                        p += 2.0 * core::f64::consts::PI;
                        offset += 2.0 * core::f64::consts::PI;
                    }
                }
                prev = Some(p);
                let c = chain_phase(factor, hz / 48_000.0);
                let mut e = (c - p).rem_euclid(2.0 * core::f64::consts::PI);
                if e > core::f64::consts::PI {
                    e -= 2.0 * core::f64::consts::PI;
                }
                worst = worst.max(e.abs());
                hz *= 1.05;
            }
            assert!(
                worst.to_degrees() < bound_deg,
                "{factor}x: {} degrees",
                worst.to_degrees()
            );
        }
    }

    #[test]
    fn a_primed_oversampler_holds_a_constant_from_the_first_sample() {
        for k in [1usize, 2, 4] {
            let mut os = Oversampler::ZERO;
            os.prime(0.7, 0.3);
            for _ in 0..8 {
                let y = match k {
                    1 => os.process::<1>(0.7, &mut |_| 0.3),
                    2 => os.process::<2>(0.7, &mut |_| 0.3),
                    _ => os.process::<4>(0.7, &mut |_| 0.3),
                };
                assert!((y - 0.3).abs() < 1e-15, "{k}x: {y}");
            }
        }
    }

    #[test]
    fn a_sine_survives_the_round_trip_at_the_base_amplitude() {
        // RMS over whole periods of the settled output, relative to the input's 1/sqrt 2 (a peak would depend on
        // the sampling phase against the oversampler's delay).
        for hz in [100.0, 1_000.0, 5_000.0, 12_000.0] {
            let w = 2.0 * core::f64::consts::PI * hz / 48_000.0;
            let mut os2 = Oversampler::ZERO;
            let mut os4 = Oversampler::ZERO;
            let (mut e2, mut e4, mut ex) = (0f64, 0f64, 0f64);
            for n in 0..48_000 {
                let x = sin(w * f64::from(n));
                let y2 = os2.process::<2>(x, &mut |v| v);
                let y4 = os4.process::<4>(x, &mut |v| v);
                if n >= 24_000 {
                    e2 += y2 * y2;
                    e4 += y4 * y4;
                    ex += x * x;
                }
            }
            let g2 = 10.0 * log10(e2 / ex);
            let g4 = 10.0 * log10(e4 / ex);
            assert!(g2.abs() < 1e-3, "{hz} Hz 2x: {g2} dB");
            assert!(g4.abs() < 1e-3, "{hz} Hz 4x: {g4} dB");
        }
    }

    #[test]
    fn flush_and_subnormal_counts() {
        let mut os = Oversampler::ZERO;
        for _ in 0..200 {
            let _ = os.process::<4>(1.0, &mut |v| v);
        }
        for _ in 0..40_000 {
            let _ = os.process::<4>(0.0, &mut |v| v);
            os.flush();
        }
        assert_eq!(os.subnormals(), 0);
        assert_eq!(os, {
            let mut o = os;
            o.flush();
            o
        });
        // Without flushing the tail decays into subnormals.
        let mut os = Oversampler::ZERO;
        for _ in 0..200 {
            let _ = os.process::<2>(1.0, &mut |v| v);
        }
        let mut seen = false;
        for _ in 0..60_000 {
            let _ = os.process::<2>(0.0, &mut |v| v);
            seen |= os.subnormals() > 0;
        }
        assert!(seen, "the control: an unflushed tail reaches subnormals");
    }
}
