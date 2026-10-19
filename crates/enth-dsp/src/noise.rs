//! White and pink noise (P1a lane D1). Docs, sources and the measured slope: `docs/dsp/noise.md`.
//!
//! - **White**: Marsaglia's xorshift32 (2003), one `u32` per sample, mapped to `[−1, 1)` with 24 random bits
//!   (`(x >> 8)·2⁻²³ − 1`, exact in `f32`). Flat to ±0.1 dB, RMS 0.577.
//! - **Pink**: the same white stream through J. O. Smith III's third-order pinking filter from *Spectral Audio Signal
//!   Processing* ("Example: Synthesis of 1/F Noise"), −10 dB per decade from 100 Hz to 10 kHz. The filter runs in
//!   `f64` (it has a pole near 0.999, where `f32` direct form loses the lowest octaves) and is scaled by
//!   [`PINK_GAIN`].
//!
//! Integer state and `+ − ×` only, so the streams are the same on every platform. A voice's stream starts from the
//! seed the engine derives from the render seed, the module id and the note index ([`White::from_seed`]), never from
//! a parameter, so every note has its own stream and moving a knob changes none of them.

use crate::util::flush::flush64;
use crate::util::rng::Rng;

/// Smith's coefficients: numerator `B`.
const PINK_B: [f64; 4] = [0.049922035, -0.095993537, 0.050612699, -0.004408786];
/// Smith's coefficients: denominator `A` (without the leading 1).
const PINK_A: [f64; 3] = [-2.494956002, 2.017265875, -0.522189400];

/// Output scaling of the pink filter. Smith's filter gives RMS 0.050 for white input of RMS 0.577; this gain brings
/// pink to RMS 0.20 (−9.2 dB below white's 0.577), which keeps its peaks (about 4.5 σ over a minute) near ±1, so
/// pink at level 100 % does not clip where white does not.
pub const PINK_GAIN: f64 = 4.0;

/// xorshift32 white noise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct White {
    state: u32,
}

impl White {
    /// A stream from a 64-bit seed: the high half of one `SplitMix64` step, forced non-zero (zero is the fixed point
    /// of xorshift).
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let s = Rng::new(seed).next_u32();
        Self {
            state: if s == 0 { 0x9E37_79B9 } else { s },
        }
    }

    /// The next 32 random bits (Marsaglia's 13, 17, 5 triple).
    #[inline(always)]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// The next sample in `[−1, 1)`, exact in `f32`.
    #[inline(always)]
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 8_388_608.0) - 1.0
    }
}

/// Smith's third-order pinking filter in direct form I, `f64` state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pink {
    x: [f64; 3],
    y: [f64; 3],
}

impl Pink {
    /// The next pink sample for white input `w`, before [`PINK_GAIN`].
    #[inline(always)]
    pub fn tick(&mut self, w: f64) -> f64 {
        let y =
            PINK_B[0] * w + PINK_B[1] * self.x[0] + PINK_B[2] * self.x[1] + PINK_B[3] * self.x[2]
                - PINK_A[0] * self.y[0]
                - PINK_A[1] * self.y[1]
                - PINK_A[2] * self.y[2];
        self.x = [w, self.x[0], self.x[1]];
        self.y = [y, self.y[0], self.y[1]];
        y
    }

    /// Flush tiny state to zero (call at least once per block).
    pub fn flush(&mut self) {
        for v in self.x.iter_mut().chain(self.y.iter_mut()) {
            *v = flush64(*v);
        }
    }

    /// Whether any state value is subnormal.
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.x.iter().chain(&self.y).any(|v| v.is_subnormal())
    }
}

/// One voice of noise: white, or white through the pink filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseVoice {
    white: White,
    pink: Pink,
}

impl NoiseVoice {
    /// A fresh stream from `seed` (see [`White::from_seed`]) with a cold pink filter.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            white: White::from_seed(seed),
            pink: Pink::default(),
        }
    }

    /// Fill `out` with white noise.
    #[inline]
    pub fn white<const N: usize>(&mut self, out: &mut [f32; N]) {
        for o in out.iter_mut() {
            *o = self.white.next_f32();
        }
    }

    /// Fill `out` with pink noise (scaled by [`PINK_GAIN`]) and flush the filter state.
    #[inline]
    pub fn pink<const N: usize>(&mut self, out: &mut [f32; N]) {
        for o in out.iter_mut() {
            let w = f64::from(self.white.next_f32());
            *o = (self.pink.tick(w) * PINK_GAIN) as f32;
        }
        self.pink.flush();
    }

    /// Whether any filter state value is subnormal.
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.pink.has_subnormal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wavetable::testkit::power_spectrum;

    #[test]
    fn xorshift32_matches_marsaglias_reference() {
        // Marsaglia (2003), 13/17/5, from state 2463534242 (his example seed): the first outputs.
        let mut w = White {
            state: 2_463_534_242,
        };
        assert_eq!(w.next_u32(), 723_471_715);
        assert_eq!(w.next_u32(), 2_497_366_906);
        assert_eq!(w.next_u32(), 2_064_144_800);
    }

    #[test]
    fn white_is_in_range_flat_and_has_the_right_rms() {
        let mut v = NoiseVoice::new(1);
        let mut x = vec![0.0f32; 1 << 18];
        for c in x.as_chunks_mut::<32>().0 {
            v.white(c);
        }
        assert!(x.iter().all(|s| (-1.0..1.0).contains(s)));
        let ms = x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len() as f64;
        assert!((ms.sqrt() - 0.57735).abs() < 0.003, "rms {}", ms.sqrt());
        let mean = x.iter().map(|&s| f64::from(s)).sum::<f64>() / x.len() as f64;
        assert!(mean.abs() < 0.005, "mean {mean}");
        // Flat: eight octave-ish bands of an averaged spectrum agree within 0.5 dB.
        let psd = welch(&x, 4096);
        let band = |lo: usize, hi: usize| {
            10.0 * crate::util::math::log10(psd[lo..hi].iter().sum::<f64>() / (hi - lo) as f64)
        };
        let (a, b, c) = (band(40, 200), band(200, 800), band(800, 2000));
        assert!((a - b).abs() < 0.5 && (b - c).abs() < 0.5, "{a} {b} {c}");
    }

    /// Averaged periodogram (Welch, 50 % overlap) of `x` in `n`-point segments.
    fn welch(x: &[f32], n: usize) -> Vec<f64> {
        let mut acc = vec![0.0; n / 2 + 1];
        let mut count = 0.0;
        let mut start = 0;
        while start + n <= x.len() {
            for (a, p) in acc.iter_mut().zip(power_spectrum(&x[start..start + n])) {
                *a += p;
            }
            count += 1.0;
            start += n / 2;
        }
        acc.iter().map(|a| a / count).collect()
    }

    #[test]
    fn pink_falls_ten_db_per_decade() {
        let mut v = NoiseVoice::new(7);
        let mut x = vec![0.0f32; 1 << 20];
        for c in x.as_chunks_mut::<32>().0 {
            v.pink(c);
        }
        let n = 8192;
        let psd = welch(&x, n);
        // Least-squares slope of 10·log10(PSD) against log10(f) over 100 Hz - 10 kHz at 48 kHz.
        let bin = 48_000.0 / n as f64;
        let (lo, hi) = ((100.0 / bin).ceil() as usize, (10_000.0 / bin) as usize);
        // Average in log-spaced groups first so every decade weighs the same.
        let mut pts = Vec::new();
        let mut k = lo;
        while k < hi {
            let k2 = ((k as f64 * 1.05) as usize).max(k + 1).min(hi);
            let p = psd[k..k2].iter().sum::<f64>() / (k2 - k) as f64;
            let f = (k + k2 - 1) as f64 / 2.0 * bin;
            pts.push((
                crate::util::math::log10(f),
                10.0 * crate::util::math::log10(p),
            ));
            k = k2;
        }
        let m = pts.len() as f64;
        let (sx, sy) = (
            pts.iter().map(|p| p.0).sum::<f64>(),
            pts.iter().map(|p| p.1).sum::<f64>(),
        );
        let sxx = pts.iter().map(|p| p.0 * p.0).sum::<f64>();
        let sxy = pts.iter().map(|p| p.0 * p.1).sum::<f64>();
        let slope = (m * sxy - sx * sy) / (m * sxx - sx * sx);
        eprintln!("pink slope {slope:.3} dB/decade");
        assert!((slope + 10.0).abs() < 0.3, "slope {slope}");
        // Level: RMS 0.20 and peaks near unit.
        let rms =
            (x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len() as f64).sqrt();
        let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        eprintln!("pink rms {rms:.4} peak {peak:.3}");
        assert!((rms - 0.20).abs() < 0.01, "rms {rms}");
        assert!(peak < 1.3, "peak {peak}");
    }

    #[test]
    fn streams_depend_on_the_seed_only_and_never_on_zero() {
        let draw = |seed| {
            let mut v = NoiseVoice::new(seed);
            let mut x = [0.0f32; 32];
            v.white(&mut x);
            x
        };
        assert_eq!(draw(5), draw(5));
        assert_ne!(draw(5), draw(6));
        assert_ne!(White::from_seed(0).state, 0);
        // Whatever the seed, the state is never the xorshift fixed point.
        for s in 0..1000u64 {
            assert_ne!(White::from_seed(s).state, 0);
        }
    }

    #[test]
    fn the_pink_filter_flushes_and_stays_finite() {
        let mut p = Pink {
            x: [1e-300, 0.0, 1e-310],
            y: [1e-40, 0.0, 0.0],
        };
        p.flush();
        assert!(!p.has_subnormal());
        assert_eq!(p, Pink::default());
        let mut v = NoiseVoice::new(3);
        let mut x = [0.0f32; 32];
        for _ in 0..10_000 {
            v.pink(&mut x);
            assert!(x.iter().all(|s| s.is_finite()));
        }
    }
}
