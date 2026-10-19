//! The 8-channel mixing matrices of the feedback delay network: Hadamard, Householder and the stereo <-> 8-channel
//! mixer.
//!
//! Derived from Signalsmith DSP `mix.h` (MIT, Copyright (c) 2021 Geraint Luff / Signalsmith Audio Ltd.:
//! `Hadamard<8>::unscaledInPlace`, `Householder<8>::inPlace`, `StereoMultiMixer<8>`). See `THIRD_PARTY_NOTICES.md`.
//! The butterfly order and the order of additions are upstream's; changing either changes the last bits.

use core::f64::consts::PI;

use super::delay::{Array, CHANNELS};

/// Unscaled recursive Hadamard transform: the butterflies of `Hadamard<Sample, 8>::unscaledInPlace`, in order.
/// Applied four times along the signal path; the missing `1/sqrt(8)` factors live in the input gain.
#[inline(always)]
pub(super) fn hadamard(d: &mut Array) {
    const BUTTERFLIES: [(usize, usize); 12] = [
        (0, 1),
        (2, 3),
        (0, 2),
        (1, 3),
        (4, 5),
        (6, 7),
        (4, 6),
        (5, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (a, b) in BUTTERFLIES {
        let (x, y) = (d[a], d[b]);
        d[a] = x + y;
        d[b] = x - y;
    }
}

/// `Householder<Sample, 8>::inPlace`: `d -= (2/8) * sum(d)`.
#[inline(always)]
pub(super) fn householder(d: &mut Array) {
    let factor = -2.0f32 / CHANNELS as f32;
    let mut sum = d[0];
    for &v in &d[1..] {
        sum += v;
    }
    sum *= factor;
    for v in d.iter_mut() {
        *v += sum;
    }
}

/// `StereoMultiMixer<Sample, 8>`: spreads a stereo pair over four rotated stereo pairs and folds them back.
pub(super) struct StereoMixer {
    /// `[cos, sin]` of `pi * k / 8` for the pairs `k = 1..4`, after a unit pair for `k = 0`.
    coeffs: [f32; CHANNELS],
}

impl StereoMixer {
    /// Upstream's `scalingFactor2()` times its `0.015625` (`1/64`, "4 Hadamard mixes"): `sqrt(2/8) / 64`, exact in
    /// binary.
    pub(super) const SCALING: f32 = 0.5 * 0.015_625;

    pub(super) fn new() -> Self {
        let mut coeffs = [0.0f32; CHANNELS];
        coeffs[0] = 1.0;
        coeffs[1] = 0.0;
        for i in 1..CHANNELS / 2 {
            let phase = PI * f64::from(i as u32) / 8.0;
            coeffs[2 * i] = libm::cos(phase) as f32;
            coeffs[2 * i + 1] = libm::sin(phase) as f32;
        }
        Self { coeffs }
    }

    /// `stereoToMulti`.
    #[inline(always)]
    pub(super) fn to_multi(&self, s: [f32; 2]) -> Array {
        let c = &self.coeffs;
        let mut out = [0.0f32; CHANNELS];
        out[0] = s[0];
        out[1] = s[1];
        for k in (2..CHANNELS).step_by(2) {
            out[k] = s[0] * c[k] + s[1] * c[k + 1];
            out[k + 1] = s[1] * c[k] - s[0] * c[k + 1];
        }
        out
    }

    /// `multiToStereo`.
    #[inline(always)]
    pub(super) fn to_stereo(&self, m: &Array) -> [f32; 2] {
        let c = &self.coeffs;
        let mut out = [m[0], m[1]];
        for k in (2..CHANNELS).step_by(2) {
            out[0] += m[k] * c[k] - m[k + 1] * c[k + 1];
            out[1] += m[k + 1] * c[k] + m[k] * c[k + 1];
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hadamard_applied_twice_scales_by_eight() {
        let mut d: Array = [1.0, -2.0, 3.0, 0.5, -0.25, 4.0, -8.0, 0.125];
        let orig = d;
        hadamard(&mut d);
        hadamard(&mut d);
        for (a, b) in d.iter().zip(orig) {
            assert_eq!(*a, b * 8.0); // exact: the values are dyadic
        }
    }

    #[test]
    fn hadamard_matches_the_sylvester_matrix() {
        for col in 0..CHANNELS {
            let mut d: Array = [0.0; CHANNELS];
            d[col] = 1.0;
            hadamard(&mut d);
            for (row, v) in d.iter().enumerate() {
                let sign = if (row & col).count_ones() % 2 == 0 {
                    1.0
                } else {
                    -1.0
                };
                assert_eq!(*v, sign, "row {row}, column {col}");
            }
        }
    }

    #[test]
    fn householder_is_an_involution_that_preserves_energy() {
        let mut d: Array = [0.5, -0.25, 1.0, 0.0, -0.75, 0.125, 0.375, -1.0];
        let orig = d;
        let e0: f32 = d.iter().map(|x| x * x).sum();
        householder(&mut d);
        let e1: f32 = d.iter().map(|x| x * x).sum();
        assert!((e0 - e1).abs() < 1e-6);
        householder(&mut d);
        for (a, b) in d.iter().zip(orig) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn stereo_mixer_round_trip_gain() {
        // multiToStereo(stereoToMulti(x)) = (1 + 3 * (cos^2 + sin^2)) / ... : four unit rotations add up to 4x.
        let m = StereoMixer::new();
        let out = m.to_stereo(&m.to_multi([0.5, -0.25]));
        assert!((out[0] - 2.0).abs() < 1e-6, "{out:?}");
        assert!((out[1] + 1.0).abs() < 1e-6, "{out:?}");
    }
}
