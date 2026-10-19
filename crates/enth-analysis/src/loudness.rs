//! EBU R128 loudness, `peak_dbfs`, `true_peak_dbtp` and `clipped_pct` on the float render.
//!
//! Levels are taken on the engine's float output, never on a clamped or quantised file: a patch that clips reads
//! `peak_dbfs` above 0, and the share of samples at or beyond full scale is reported, because clipping bends the
//! other descriptors too (a centroid read 24 % high in one P1a research case and 116 % in another, report 03 §F4).
//!
//! Loudness and true peak come from `ebur128` (BS.1770-4; true peak by 4× polyphase oversampling below 96 kHz).
//! That crate uses `std` maths, so these two numbers agree across platforms within a tolerance, not bit for bit.

use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

/// Below this integrated loudness (the BS.1770 absolute gate) a clip counts as silent.
pub const SILENCE_LUFS: f64 = -70.0;
/// Level reported for digital silence instead of minus infinity (dBFS, dBTP).
pub const FLOOR_DB: f64 = -150.0;

/// Level descriptors of one signal.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Level {
    /// Integrated loudness (LUFS); `None` when silent (below −70 LUFS).
    pub lufs_i: Option<f64>,
    /// Highest absolute sample, dBFS; above 0 when the float signal exceeds full scale. −150 for silence.
    pub peak_dbfs: f64,
    /// Highest inter-sample peak (4× oversampled), dBTP; at least `peak_dbfs`. −150 for silence.
    pub true_peak_dbtp: f64,
    /// Share of samples with |x| ≥ 1.0, in percent.
    pub clipped_pct: f64,
    /// `clipped_pct > 0`.
    pub clipping: bool,
}

/// Amplitude to dB, floored at [`FLOOR_DB`].
#[must_use]
pub fn db20(x: f64) -> f64 {
    if x > 0.0 {
        (20.0 * libm::log10(x)).max(FLOOR_DB)
    } else {
        FLOOR_DB
    }
}

/// A reusable loudness meter for mono signals at one sample rate.
pub struct LoudnessMeter {
    sample_rate: u32,
    ebu: EbuR128,
}

impl std::fmt::Debug for LoudnessMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoudnessMeter")
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}

impl LoudnessMeter {
    /// # Panics
    /// If `ebur128` rejects the rate (it accepts 1 Hz to several MHz; true peak needs below 192 kHz).
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let ebu = EbuR128::new(
            1,
            sample_rate,
            Mode::I | Mode::TRUE_PEAK | Mode::SAMPLE_PEAK,
        )
        .expect("ebur128 accepts mono at this sample rate");
        Self { sample_rate, ebu }
    }

    fn feed(&mut self, x: &[f32]) -> bool {
        self.ebu.reset();
        self.ebu.add_frames_f32(x).is_ok()
    }

    fn integrated_after_feed(&self, n: usize) -> Option<f64> {
        let gated = self.ebu.loudness_global().ok().filter(|l| l.is_finite());
        let l = gated.or_else(|| {
            // Very short sounds leave nothing after the gates: fall back to the ungated whole-clip window.
            let ms = (n as f64 / f64::from(self.sample_rate) * 1000.0) as u32;
            self.ebu
                .loudness_window(ms.clamp(100, 3000))
                .ok()
                .filter(|l| l.is_finite())
        })?;
        (l > SILENCE_LUFS).then_some(l)
    }

    /// Integrated loudness in LUFS, with an ungated fallback for very short sounds; `None` when silent.
    pub fn integrated(&mut self, x: &[f32]) -> Option<f64> {
        if !self.feed(x) {
            return None;
        }
        self.integrated_after_feed(x.len())
    }

    /// Loudness, sample peak, true peak and clipping of `x`.
    pub fn level(&mut self, x: &[f32]) -> Level {
        let fed = self.feed(x);
        let lufs_i = if fed {
            self.integrated_after_feed(x.len())
        } else {
            None
        };
        let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let tp = if fed {
            self.ebu.true_peak(0).unwrap_or(0.0)
        } else {
            0.0
        };
        let over = x.iter().filter(|s| s.abs() >= 1.0).count();
        let clipped_pct = if x.is_empty() {
            0.0
        } else {
            100.0 * over as f64 / x.len() as f64
        };
        Level {
            lufs_i,
            peak_dbfs: db20(f64::from(peak)),
            true_peak_dbtp: db20(tp.max(f64::from(peak))),
            clipped_pct,
            clipping: over > 0,
        }
    }

    /// Linear gain that brings `x` to `target_lufs`; `None` if `x` is silent.
    pub fn gain_to(&mut self, x: &[f32], target_lufs: f64) -> Option<f64> {
        let l = self.integrated(x)?;
        Some(libm::pow(10.0, (target_lufs - l) / 20.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{SR, sine, sine_phase};

    #[test]
    fn a_997hz_sine_reads_bs1770() {
        // A full-scale sine reads -3.01 LUFS, so amplitude 0.1 reads -23.01.
        let mut m = LoudnessMeter::new(48_000);
        let l = m.integrated(&sine(SR, 997.0, 0.1, 3.0)).unwrap();
        assert!((l + 23.01).abs() < 0.05, "{l}");
        assert!(m.integrated(&vec![0.0; 48_000]).is_none());
        // a 50 ms blip still has a loudness (ungated fallback)
        assert!(m.integrated(&sine(SR, 997.0, 0.5, 0.05)).is_some());
    }

    #[test]
    fn peaks_and_clipping_on_the_float_signal() {
        let mut m = LoudnessMeter::new(48_000);
        let lv = m.level(&sine(SR, 440.0, 0.5, 1.0));
        assert!((lv.peak_dbfs + 6.02).abs() < 0.01, "{lv:?}");
        assert!(!lv.clipping && lv.clipped_pct == 0.0);
        // +6 dB over full scale: peak above 0, a known share of samples clipped
        let x = sine(SR, 997.0, 2.0, 1.0);
        let lv = m.level(&x);
        assert!((lv.peak_dbfs - 6.02).abs() < 0.01, "{lv:?}");
        let want = 100.0 * x.iter().filter(|s| s.abs() >= 1.0).count() as f64 / x.len() as f64;
        assert!((lv.clipped_pct - want).abs() < 1e-12 && lv.clipping);
        // |sin| >= 0.5 for two thirds of the time
        assert!((lv.clipped_pct - 66.67).abs() < 0.5, "{}", lv.clipped_pct);
        let silent = m.level(&vec![0.0; 4800]);
        assert_eq!(
            (silent.peak_dbfs, silent.true_peak_dbtp, silent.lufs_i),
            (FLOOR_DB, FLOOR_DB, None)
        );
    }

    #[test]
    fn true_peak_sees_between_samples() {
        // A sine at fs/4 sampled at 45 degrees: every sample is 0.707 A, the waveform peaks at A (+3.01 dB).
        let mut m = LoudnessMeter::new(48_000);
        let x = sine_phase(SR, 12_000.0, 0.5, 1.0, std::f64::consts::FRAC_PI_4);
        let lv = m.level(&x);
        assert!((lv.peak_dbfs - (-6.02 - 3.01)).abs() < 0.02, "{lv:?}");
        assert!(
            (lv.true_peak_dbtp - lv.peak_dbfs - 3.01).abs() < 0.3,
            "{lv:?}"
        );
    }
}
