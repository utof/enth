//! The biquads of the reverb: second-order high-pass, low-pass, low-shelf and high-shelf sections designed in
//! `f64`, run in `f32`.
//!
//! Derived from Signalsmith DSP `filters.h` (MIT, Copyright (c) 2021 Geraint Luff / Signalsmith Audio Ltd.):
//! `BiquadStatic` with `FreqSpec`, `qSpec`/`octaveSpec`, the `configure` branches for the four types used here, and
//! `lowpassQ`, `highpassQ`, `lowShelfQ`, `highShelf` with the one-sided bandwidth design. The Vicanek, cookbook,
//! band-pass, notch, peak and all-pass branches are not ported. See `THIRD_PARTY_NOTICES.md`.
//!
//! Upstream's coefficients are `float` members that are normalised in place, so the design rounds to `f32` first
//! and then multiplies by `1 / a0` (and rounds again); `b1 * 0.5` in the low-pass uses the rounded `b1`. This file
//! does the same, which is what keeps it bit-exact. All transcendentals go through `libm`.
//!
//! Change from upstream: the output of every section is flushed (`util::flush`) so a decaying reverb tail reaches
//! exactly zero instead of lingering in the subnormal range.

use core::f64::consts::PI;

use crate::util::flush::flush;

/// `defaultBandwidth`: 1.8999686269529916 octaves, equivalent to Q = sqrt(1/2). Used by the high shelf.
const DEFAULT_BANDWIDTH: f64 = 1.899_968_626_952_991_6;

/// Normalised coefficients of one section (`a0` divided out).
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Coeffs {
    a1: f32,
    a2: f32,
    b0: f32,
    b1: f32,
    b2: f32,
}

impl Coeffs {
    /// A neutral section (passes its input).
    pub(super) const IDENTITY: Self = Self {
        a1: 0.0,
        a2: 0.0,
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
    };
}

/// `FreqSpec`: the frequency terms shared by the designs.
struct FreqSpec {
    scaled_freq: f64,
    sin_w0: f64,
    cos_w0: f64,
    inv_2q: f64,
}

impl FreqSpec {
    fn new(freq: f64) -> Self {
        let scaled_freq = 1e-6f64.max(0.4999f64.min(freq));
        let w0 = 2.0 * PI * scaled_freq;
        Self {
            scaled_freq,
            cos_w0: libm::cos(w0),
            sin_w0: libm::sin(w0),
            inv_2q: 0.0,
        }
    }

    /// `qSpec`: `inv2Q = 0.5 / q`.
    fn with_q(freq: f64, q: f64) -> Self {
        let mut spec = Self::new(freq);
        spec.inv_2q = 0.5 / q;
        spec
    }

    /// `oneSidedCompQ`: compensate Q so that the lower boundary of the band lands where the bilinear transform
    /// puts it.
    fn one_sided_comp_q(&mut self) {
        let f1_factor = libm::sqrt(self.inv_2q * self.inv_2q + 1.0) - self.inv_2q;
        let ct_f1 = libm::tan(PI * self.scaled_freq * f1_factor);
        let inv_ct_f0 = (1.0 + self.cos_w0) / self.sin_w0;
        let k = ct_f1 * inv_ct_f0;
        self.inv_2q = 0.5 / k - 0.5 * k;
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Highpass,
    Lowpass,
    HighShelf,
    LowShelf,
}

/// `configure`: the textbook (Audio EQ Cookbook) coefficients for `kind`, rounded the way upstream rounds them.
#[allow(
    clippy::manual_midpoint,
    reason = "upstream's `(1 + cosW0) * 0.5` stays verbatim so the port can be diffed against the C++ line by line"
)]
fn configure(kind: Kind, c: &FreqSpec, sqrt_gain: f64) -> Coeffs {
    let alpha = c.sin_w0 * c.inv_2q;
    let a = sqrt_gain;
    let sqrt_a2alpha = 2.0 * libm::sqrt(a) * alpha;
    let cw = c.cos_w0;
    // Every arm rounds its f64 results to f32 first, as upstream's `Sample` members do; `a0` stays f64.
    let (b0, b1, b2, a0, a1, a2): (f32, f32, f32, f64, f32, f32) = match kind {
        Kind::Highpass => {
            let b1 = (-1.0 - cw) as f32;
            let b2 = ((1.0 + cw) * 0.5) as f32;
            (
                b2,
                b1,
                b2,
                1.0 + alpha,
                (-2.0 * cw) as f32,
                (1.0 - alpha) as f32,
            )
        }
        Kind::Lowpass => {
            let b1 = (1.0 - cw) as f32;
            let b2 = (f64::from(b1) * 0.5) as f32;
            (
                b2,
                b1,
                b2,
                1.0 + alpha,
                (-2.0 * cw) as f32,
                (1.0 - alpha) as f32,
            )
        }
        Kind::HighShelf => (
            (a * (((a + 1.0) + (a - 1.0) * cw) + sqrt_a2alpha)) as f32,
            ((-2.0 * a) * ((a - 1.0) + (a + 1.0) * cw)) as f32,
            (a * (((a + 1.0) + (a - 1.0) * cw) - sqrt_a2alpha)) as f32,
            ((a + 1.0) - (a - 1.0) * cw) + sqrt_a2alpha,
            (2.0 * ((a - 1.0) - (a + 1.0) * cw)) as f32,
            (((a + 1.0) - (a - 1.0) * cw) - sqrt_a2alpha) as f32,
        ),
        Kind::LowShelf => (
            (a * (((a + 1.0) - (a - 1.0) * cw) + sqrt_a2alpha)) as f32,
            ((2.0 * a) * ((a - 1.0) - (a + 1.0) * cw)) as f32,
            (a * (((a + 1.0) - (a - 1.0) * cw) - sqrt_a2alpha)) as f32,
            ((a + 1.0) + (a - 1.0) * cw) + sqrt_a2alpha,
            (-2.0 * ((a - 1.0) + (a + 1.0) * cw)) as f32,
            (((a + 1.0) + (a - 1.0) * cw) - sqrt_a2alpha) as f32,
        ),
    };
    let inv_a0 = 1.0 / a0;
    let norm = |v: f32| (f64::from(v) * inv_a0) as f32;
    let (b0, b1, b2, a1, a2) = (norm(b0), norm(b1), norm(b2), norm(a1), norm(a2));
    Coeffs { a1, a2, b0, b1, b2 }
}

/// `highpassQ(freq, q)`; `freq` is in cycles per sample.
pub(super) fn highpass_q(freq: f64, q: f64) -> Coeffs {
    configure(Kind::Highpass, &FreqSpec::with_q(freq, q), 0.0)
}

/// `lowpassQ(freq, q)`.
pub(super) fn lowpass_q(freq: f64, q: f64) -> Coeffs {
    configure(Kind::Lowpass, &FreqSpec::with_q(freq, q), 0.0)
}

/// `lowShelfQ(freq, gain, q)` with the one-sided bandwidth design; `gain` is linear.
pub(super) fn low_shelf_q(freq: f64, gain: f64, q: f64) -> Coeffs {
    let mut spec = FreqSpec::with_q(freq, q);
    spec.one_sided_comp_q();
    configure(Kind::LowShelf, &spec, libm::sqrt(gain))
}

/// `highShelf(freq, gain)` at the default bandwidth, one-sided design; `gain` is linear.
pub(super) fn high_shelf(freq: f64, gain: f64) -> Coeffs {
    let mut spec = FreqSpec::new(freq);
    spec.inv_2q = libm::sinh(libm::log(2.0) * 0.5 * DEFAULT_BANDWIDTH);
    spec.one_sided_comp_q();
    configure(Kind::HighShelf, &spec, libm::sqrt(gain))
}

/// One section: coefficients plus its four-sample history (`BiquadStatic::operator()`, direct form I).
#[derive(Clone, Copy)]
pub(super) struct Biquad {
    k: Coeffs,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    pub(super) const NEW: Self = Self {
        k: Coeffs::IDENTITY,
        x1: 0.0,
        x2: 0.0,
        y1: 0.0,
        y2: 0.0,
    };

    /// Clear the history; the coefficients stay.
    pub(super) fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    #[inline(always)]
    pub(super) fn set_coeffs(&mut self, k: Coeffs) {
        self.k = k;
    }

    /// Filter one sample. The output is flushed to zero below 1e-15 (see the module note).
    #[inline(always)]
    pub(super) fn tick(&mut self, x0: f32) -> f32 {
        let k = &self.k;
        let y0 =
            flush(x0 * k.b0 + self.x1 * k.b1 + self.x2 * k.b2 - self.y1 * k.a1 - self.y2 * k.a2);
        self.y2 = self.y1;
        self.y1 = y0;
        self.x2 = self.x1;
        self.x1 = x0;
        y0
    }

    /// The four history values (`x1`, `x2`, `y1`, `y2`).
    pub(super) fn state(&self) -> [f32; 4] {
        [self.x1, self.x2, self.y1, self.y2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Magnitude response of the section at `freq` (cycles per sample).
    fn magnitude(k: &Coeffs, freq: f64) -> f64 {
        let w = 2.0 * PI * freq;
        let (c1, s1) = (libm::cos(w), libm::sin(w));
        let (c2, s2) = (libm::cos(2.0 * w), libm::sin(2.0 * w));
        let num_re = f64::from(k.b0) + f64::from(k.b1) * c1 + f64::from(k.b2) * c2;
        let num_im = -(f64::from(k.b1) * s1 + f64::from(k.b2) * s2);
        let den_re = 1.0 + f64::from(k.a1) * c1 + f64::from(k.a2) * c2;
        let den_im = -(f64::from(k.a1) * s1 + f64::from(k.a2) * s2);
        libm::sqrt((num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im))
    }

    #[test]
    fn highpass_blocks_dc_and_passes_nyquist() {
        let k = highpass_q(80.0 / 48_000.0, 0.5);
        // Second order: falls as the square of the frequency below the corner (1.7e-3 cycles per sample here).
        assert!(magnitude(&k, 1e-5) < 1e-4);
        assert!((magnitude(&k, 0.45) - 1.0).abs() < 0.01);
        // Q = 0.5 is a critically damped pair: -6 dB at the corner.
        assert!((magnitude(&k, 80.0 / 48_000.0) - 0.5).abs() < 0.01);
    }

    #[test]
    fn lowpass_passes_dc_and_blocks_nyquist() {
        let k = lowpass_q(12_000.0 / 48_000.0, 0.5);
        assert!((magnitude(&k, 1e-5) - 1.0).abs() < 1e-4);
        assert!(magnitude(&k, 0.5) < 0.05);
        assert!((magnitude(&k, 0.25) - 0.5).abs() < 0.01);
    }

    #[test]
    fn shelves_apply_their_gain_far_from_the_corner() {
        let gain = 0.25; // -12 dB
        let low = low_shelf_q(180.0 / 48_000.0, gain, 0.5);
        assert!((magnitude(&low, 1e-5) - gain).abs() < 0.01 * gain);
        assert!((magnitude(&low, 0.4) - 1.0).abs() < 0.01);
        let high = high_shelf(6_000.0 / 48_000.0, gain);
        assert!((magnitude(&high, 0.49) - gain).abs() < 0.02);
        assert!((magnitude(&high, 1e-4) - 1.0).abs() < 0.01);
    }

    #[test]
    fn unity_shelves_are_neutral() {
        let low = low_shelf_q(0.01, 1.0, 0.5);
        let high = high_shelf(0.1, 1.0);
        for f in [0.001, 0.01, 0.1, 0.3, 0.49] {
            assert!((magnitude(&low, f) - 1.0).abs() < 1e-4);
            assert!((magnitude(&high, f) - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn tick_flushes_a_decaying_tail_to_exact_zero() {
        let mut f = Biquad::NEW;
        f.set_coeffs(lowpass_q(2_000.0 / 48_000.0, 0.5));
        f.tick(1.0);
        for _ in 0..20_000 {
            f.tick(0.0);
        }
        assert!(f.state().iter().all(|v| *v == 0.0 && !v.is_subnormal()));
    }

    #[test]
    fn reset_clears_history_and_keeps_coefficients() {
        let mut f = Biquad::NEW;
        let k = lowpass_q(0.1, 0.5);
        f.set_coeffs(k);
        f.tick(1.0);
        f.tick(0.5);
        f.reset();
        assert_eq!(f.state(), [0.0; 4]);
        assert!(f.k == k);
    }
}
