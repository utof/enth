//! Hashing and the feature summary that explains a hash change.
//!
//! The SHA-256 is the contract (bit-exact on every platform). The summary only says *how much* a render changed when
//! the hash differs: RMS per 10 ms and the spectral centroid per 4,096-sample window, both rounded so that the last
//! bits of a platform-dependent FFT cannot show.

use std::fmt::Write as _;

use realfft::RealFftPlanner;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Window and hop of the centroid series, in samples.
pub const CENTROID_WINDOW: usize = 4096;

/// What a case file records next to the hash, compared with a tolerance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    /// RMS per 10 ms frame in dBFS, rounded to 0.001 dB (the last frame may be shorter).
    pub rms_db: Vec<f64>,
    /// Spectral centroid in Hz per Hann window of [`CENTROID_WINDOW`] samples (hop = window), rounded to 0.1 Hz;
    /// 0 for a silent window. Only whole windows count.
    pub centroid_hz: Vec<f64>,
}

/// Allowed absolute difference per summary series.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tolerance {
    pub rms_db: f64,
    pub centroid_hz: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            rms_db: 0.01,
            centroid_hz: 1.0,
        }
    }
}

/// SHA-256 over (sample rate, channels, frame count) and the little-endian bits of every sample, as lowercase hex.
#[must_use]
pub fn hash_samples(sample_rate: u32, channels: u16, samples: &[f32]) -> String {
    let mut h = Sha256::new();
    h.update(sample_rate.to_le_bytes());
    h.update(channels.to_le_bytes());
    h.update((samples.len() as u64).to_le_bytes());
    for s in samples {
        h.update(s.to_bits().to_le_bytes());
    }
    let mut out = String::with_capacity(64);
    for b in h.finalize() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn round_to(x: f64, step_inverse: f64) -> f64 {
    (x * step_inverse).round() / step_inverse
}

/// The summary of a mono render.
#[must_use]
pub fn summarise(sample_rate: u32, samples: &[f32]) -> Summary {
    Summary {
        rms_db: rms_series(sample_rate, samples),
        centroid_hz: centroid_series(sample_rate, samples),
    }
}

fn rms_series(sample_rate: u32, samples: &[f32]) -> Vec<f64> {
    let frame = ((sample_rate / 100) as usize).max(1);
    samples
        .chunks(frame)
        .map(|c| {
            let ms = c.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / c.len() as f64;
            round_to(10.0 * libm::log10(ms.max(1e-20)), 1000.0)
        })
        .collect()
}

fn centroid_series(sample_rate: u32, samples: &[f32]) -> Vec<f64> {
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(CENTROID_WINDOW);
    let window: Vec<f64> = (0..CENTROID_WINDOW)
        .map(|n| 0.5 - 0.5 * libm::cos(core::f64::consts::TAU * n as f64 / CENTROID_WINDOW as f64))
        .collect();
    let bin_hz = f64::from(sample_rate) / CENTROID_WINDOW as f64;
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    samples
        .as_chunks::<CENTROID_WINDOW>()
        .0
        .iter()
        .map(|chunk| {
            for ((x, &s), &w) in input.iter_mut().zip(chunk).zip(&window) {
                *x = f64::from(s) * w;
            }
            fft.process(&mut input, &mut spectrum)
                .expect("buffers come from the plan, so their lengths match");
            let (mut weighted, mut total) = (0.0, 0.0);
            for (k, c) in spectrum.iter().enumerate() {
                let magnitude = (c.re * c.re + c.im * c.im).sqrt();
                weighted += k as f64 * bin_hz * magnitude;
                total += magnitude;
            }
            if total > 1e-9 {
                round_to(weighted / total, 10.0)
            } else {
                0.0
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (0.5 * libm::sin(core::f64::consts::TAU * hz * n as f64 / 48_000.0)) as f32)
            .collect()
    }

    #[test]
    fn one_changed_bit_changes_the_hash() {
        let a = sine(440.0, 4800);
        let mut b = a.clone();
        b[100] = f32::from_bits(b[100].to_bits() ^ 1);
        assert_ne!(
            hash_samples(48_000, 1, &a),
            hash_samples(48_000, 1, &b),
            "the hash must see the last mantissa bit"
        );
        assert_eq!(hash_samples(48_000, 1, &a), hash_samples(48_000, 1, &a));
    }

    #[test]
    fn the_hash_depends_on_rate_channels_and_length() {
        let a = sine(440.0, 4800);
        let base = hash_samples(48_000, 1, &a);
        assert_ne!(base, hash_samples(44_100, 1, &a));
        assert_ne!(base, hash_samples(48_000, 2, &a));
        assert_ne!(base, hash_samples(48_000, 1, &a[..4799]));
    }

    #[test]
    fn a_half_scale_sine_reads_minus_9_db_rms() {
        // RMS of A*sin is A/sqrt(2): 0.5 -> 0.3536 -> -9.03 dBFS.
        let s = summarise(48_000, &sine(440.0, 48_000));
        assert_eq!(s.rms_db.len(), 100);
        for &db in &s.rms_db[1..99] {
            assert!((db - -9.03).abs() < 0.1, "got {db}");
        }
    }

    #[test]
    fn the_centroid_of_a_sine_is_near_its_frequency() {
        let s = summarise(48_000, &sine(1000.0, 48_000));
        assert_eq!(s.centroid_hz.len(), 48_000 / CENTROID_WINDOW);
        for &hz in &s.centroid_hz {
            assert!((hz - 1000.0).abs() < 30.0, "got {hz}");
        }
    }

    #[test]
    fn silence_has_a_floor_and_a_zero_centroid() {
        let s = summarise(48_000, &vec![0.0; 8192]);
        assert!(s.rms_db.iter().all(|&db| (db - -200.0).abs() < 1e-9));
        assert_eq!(s.centroid_hz, vec![0.0, 0.0]);
    }
}
