//! Harmonic content (lane SS, R1/S1): how the harmonic energy above the fundamental splits between odd and even
//! harmonics. The sound score's `harmonics` row ("the waveform"): level 1 of the Recreate game is waveform plus
//! octave, and brightness alone cannot tell a square from a saw through the same filter.
//!
//! **Definition.** A Welch power spectrum (Hann, 8192 points, or 4096 or 2048 when the span is shorter; hop n/2) of
//! a span of the clip, normally the held part. Each harmonic h = 1…[`MAX_HARMONIC`] of the given f0, below
//! min(0.45 · sample rate, 16 kHz), takes the power of the bins within max(3 bins, 2 % of h·f0) of h·f0, never
//! reaching halfway to a neighbour. Then:
//!
//! - `odd_share` = power of the odd harmonics from 3 / power of all harmonics from 2: about 1 for a square or a
//!   triangle (odd harmonics only), 0.35 for a saw (1/h² energies to h = 16), lower for a narrow pulse;
//! - `above_db` = power of harmonics 2… over the fundamental's, in dB.
//!
//! `None` when the harmonics above the fundamental are more than 40 dB below it (a sine: "no harmonics"), when the
//! span is shorter than 2048 samples, or when f0 is not a usable frequency. On the scalar FFT path by default, so
//! the same on every platform.

use crate::fft::{FftPlanner, hann};
use realfft::num_complex::Complex32;

/// The highest harmonic counted.
pub const MAX_HARMONIC: usize = 16;
/// Below this ratio of harmonic power (2… over 1) a sound has no harmonics to speak of: −40 dB.
const NONE_BELOW: f64 = 1e-4;

/// The harmonic content of a span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Harmonics {
    /// Odd harmonics from 3 over all harmonics from 2, by power: 0–1.
    pub odd_share: f64,
    /// Harmonics from 2 over the fundamental, by power, in dB.
    pub above_db: f64,
}

/// Measure the harmonic content of `x[span.0..span.1]` with fundamental `f0_hz` (see the [module docs](self)).
pub fn measure(
    planner: &mut FftPlanner,
    x: &[f32],
    sample_rate: f64,
    f0_hz: f64,
    span: (usize, usize),
) -> Option<Harmonics> {
    let end = span.1.min(x.len());
    let start = span.0.min(end);
    let len = end - start;
    let n = [8192usize, 4096, 2048].into_iter().find(|n| *n <= len)?;
    if !(f0_hz.is_finite() && f0_hz > 0.0 && sample_rate > 0.0) {
        return None;
    }
    let fft = planner.forward(n);
    let window = hann(n);
    let mut input = vec![0f32; n];
    let mut out = vec![Complex32::new(0.0, 0.0); fft.bins()];
    let mut scratch = fft.make_scratch();
    let mut pw = vec![0.0f64; n / 2 + 1];
    let mut pos = start;
    while pos + n <= end {
        for ((o, s), w) in input.iter_mut().zip(&x[pos..pos + n]).zip(&window) {
            *o = s * w;
        }
        fft.forward(&mut input, &mut out, &mut scratch);
        for (p, c) in pw.iter_mut().zip(&out) {
            *p += f64::from(c.norm_sqr());
        }
        pos += n / 2;
    }
    let bin_hz = sample_rate / n as f64;
    let top_hz = (0.45 * sample_rate).min(16_000.0);
    let spacing = f0_hz / bin_hz;
    let (mut first, mut above, mut odd) = (0.0f64, 0.0f64, 0.0f64);
    for h in 1..=MAX_HARMONIC {
        let f = h as f64 * f0_hz;
        if f >= top_hz {
            break;
        }
        let centre = libm::round(f / bin_hz) as usize;
        let reach = libm::round(0.02 * f / bin_hz).max(3.0);
        let half = reach.min(libm::floor(spacing / 2.0 - 0.5)).max(0.0) as usize;
        let lo = centre.saturating_sub(half);
        let hi = (centre + half).min(pw.len() - 1);
        if lo > hi {
            break;
        }
        let mut e = 0.0f64;
        for p in &pw[lo..=hi] {
            e += *p;
        }
        match h {
            1 => first = e,
            _ if h % 2 == 1 => {
                above += e;
                odd += e;
            }
            _ => above += e,
        }
    }
    if !(above > NONE_BELOW * first && above > 0.0) {
        return None;
    }
    Some(Harmonics {
        odd_share: odd / above,
        above_db: if first > 0.0 {
            10.0 * libm::log10(above / first)
        } else {
            f64::INFINITY
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::FftPath;
    use crate::test_signals::{SR, Wave, oscillator, sine};

    fn odd(x: &[f32], f0: f64) -> Option<f64> {
        let mut planner = FftPlanner::new(FftPath::Scalar);
        measure(&mut planner, x, SR, f0, (0, x.len())).map(|h| h.odd_share)
    }

    #[test]
    fn waveforms_read_their_odd_share() {
        let saw = odd(&oscillator(SR, Wave::Saw, 220.0, 1.0, 0.0), 220.0).unwrap();
        assert!((saw - 0.347).abs() < 0.03, "saw {saw}");
        let square = odd(&oscillator(SR, Wave::Square, 220.0, 1.0, 0.0), 220.0).unwrap();
        assert!(square > 0.97, "square {square}");
        let tri = odd(&oscillator(SR, Wave::Triangle, 220.0, 1.0, 0.0), 220.0).unwrap();
        assert!(tri > 0.97, "triangle {tri}");
        assert_eq!(
            odd(&sine(SR, 220.0, 0.5, 1.0), 220.0),
            None,
            "a sine has none"
        );
        // low notes, short spans and bad input
        let c1 = odd(&oscillator(SR, Wave::Saw, 32.7, 1.0, 0.0), 32.7).unwrap();
        assert!((c1 - 0.347).abs() < 0.06, "C1 saw {c1}");
        let x = oscillator(SR, Wave::Saw, 220.0, 0.03, 0.0);
        assert_eq!(odd(&x, 220.0), None, "shorter than 2048 samples");
        let x = oscillator(SR, Wave::Saw, 220.0, 0.5, 0.0);
        assert_eq!(odd(&x, f64::NAN), None);
        assert_eq!(odd(&x, 0.0), None);
    }
}
