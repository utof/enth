//! Level matching: EBU R128 (ITU BS.1770-4) integrated loudness, the measure the scorer uses.
//!
//! Mono clips (the ABX stimuli) go through `enth-analysis`' [`LoudnessMeter`], exactly the scorer's number.
//! `enth-analysis` has no stereo meter, and a stereo reverb must not be matched on a mid signal (a wider, more
//! decorrelated tail reads quieter on `(L + R) / 2`, so matching on it would make the wider reverb louder), so
//! stereo clips use [`StereoMeter`]: the same `ebur128` crate at the same pin, two channels, with the same ungated
//! fallback for very short sounds. For `L = x, R = 0` it reads what the mono meter reads for `x` (a test).

use ebur128::{EbuR128, Mode};
use enth_analysis::loudness::{LoudnessMeter, SILENCE_LUFS};

/// Every stimulus is brought to this integrated loudness (the scorer's, ADR-0007), minus any shared peak margin.
pub const TARGET_LUFS: f64 = -23.0;
/// Highest sample after matching. When one clip of a group would exceed it, the whole group is scaled down together.
pub const PEAK_CEILING: f32 = 0.98;

/// Linear gain from decibels.
#[must_use]
pub fn db_to_gain(db: f64) -> f64 {
    libm::pow(10.0, db / 20.0)
}

/// Decibels from a linear gain (-150 for 0).
#[must_use]
pub fn gain_to_db(g: f64) -> f64 {
    if g > 0.0 {
        20.0 * libm::log10(g)
    } else {
        -150.0
    }
}

/// Integrated loudness of a mono clip through `enth-analysis`; `None` when silent (below -70 LUFS).
pub fn mono_lufs(meter: &mut LoudnessMeter, x: &[f32]) -> Option<f64> {
    meter.integrated(x)
}

/// A two-channel BS.1770 meter.
pub struct StereoMeter {
    sample_rate: u32,
    ebu: EbuR128,
}

impl std::fmt::Debug for StereoMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StereoMeter")
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}

impl StereoMeter {
    /// # Panics
    /// If `ebur128` rejects the sample rate.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let ebu = EbuR128::new(2, sample_rate, Mode::I | Mode::SAMPLE_PEAK)
            .expect("ebur128 accepts stereo at this sample rate");
        Self { sample_rate, ebu }
    }

    /// Integrated loudness of `left`/`right` in LUFS, with the ungated whole-clip fallback; `None` when silent.
    pub fn integrated(&mut self, left: &[f32], right: &[f32]) -> Option<f64> {
        self.ebu.reset();
        self.ebu.add_frames_planar_f32(&[left, right]).ok()?;
        let gated = self.ebu.loudness_global().ok().filter(|l| l.is_finite());
        let l = gated.or_else(|| {
            let ms = (left.len() as f64 / f64::from(self.sample_rate) * 1000.0) as u32;
            self.ebu
                .loudness_window(ms.clamp(100, 3000))
                .ok()
                .filter(|l| l.is_finite())
        })?;
        (l > SILENCE_LUFS).then_some(l)
    }
}

impl StereoMeter {
    /// Integrated loudness of a signal that may sit far below -70 LUFS (a raw reverb tail before it is mixed in),
    /// where the absolute gate would call it silent. The signal is scaled to a peak of 0.5, measured, and the scale
    /// taken off again; only the absolute gate depends on level (the relative gate and the K-weighting do not), so
    /// above -70 LUFS this reads what [`StereoMeter::integrated`] reads. `None` only for exact silence.
    pub fn integrated_any_level(&mut self, left: &[f32], right: &[f32]) -> Option<f64> {
        let peak = left.iter().chain(right).fold(0.0f32, |m, v| m.max(v.abs()));
        if !(peak.is_finite() && peak > 0.0) {
            return None;
        }
        let k = 0.5 / peak;
        let l: Vec<f32> = left.iter().map(|v| v * k).collect();
        let r: Vec<f32> = right.iter().map(|v| v * k).collect();
        let scaled = self.integrated(&l, &r)?;
        Some(scaled - gain_to_db(f64::from(k)))
    }
}

/// Root-mean-square level of all channels together, dBFS (-150 for silence). Information only.
#[must_use]
pub fn rms_dbfs(channels: &[&[f32]]) -> f64 {
    let n: usize = channels.iter().map(|c| c.len()).sum();
    if n == 0 {
        return -150.0;
    }
    let sum: f64 = channels
        .iter()
        .flat_map(|c| c.iter())
        .map(|v| f64::from(*v).powi(2))
        .sum();
    gain_to_db(libm::sqrt(sum / n as f64))
}

/// Sample peak over channels, dBFS (-150 for silence).
#[must_use]
pub fn peak_dbfs(channels: &[&[f32]]) -> f64 {
    let p = channels
        .iter()
        .flat_map(|c| c.iter())
        .fold(0.0f32, |m, v| m.max(v.abs()));
    gain_to_db(f64::from(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_analysis::test_signals::{SR, sine};

    #[test]
    fn stereo_meter_with_one_silent_channel_reads_the_mono_meter() {
        let x = sine(SR, 997.0, 0.1, 3.0);
        let zeros = vec![0.0f32; x.len()];
        let mono = mono_lufs(&mut LoudnessMeter::new(48_000), &x).unwrap();
        let st = StereoMeter::new(48_000).integrated(&x, &zeros).unwrap();
        assert!((mono - st).abs() < 0.01, "{mono} vs {st}");
        // the same signal on both channels reads 3.01 LU louder (channel powers add)
        let both = StereoMeter::new(48_000).integrated(&x, &x).unwrap();
        assert!((both - st - 3.0103).abs() < 0.01, "{both} vs {st}");
        assert!(
            StereoMeter::new(48_000)
                .integrated(&zeros, &zeros)
                .is_none()
        );
    }

    #[test]
    fn a_wide_signal_is_not_matched_on_its_mid() {
        // L = x, R = -x: the mid is silent but the loudness is not.
        let x = sine(SR, 997.0, 0.1, 2.0);
        let neg: Vec<f32> = x.iter().map(|v| -v).collect();
        assert!(StereoMeter::new(48_000).integrated(&x, &neg).is_some());
        let mid: Vec<f32> = x.iter().zip(&neg).map(|(a, b)| 0.5 * (a + b)).collect();
        assert!(mono_lufs(&mut LoudnessMeter::new(48_000), &mid).is_none());
    }

    #[test]
    fn the_unbounded_reading_matches_above_the_gate_and_works_far_below_it() {
        let x = sine(SR, 997.0, 0.1, 3.0);
        let mut m = StereoMeter::new(48_000);
        let plain = m.integrated(&x, &x).unwrap();
        let any = m.integrated_any_level(&x, &x).unwrap();
        assert!((plain - any).abs() < 0.01, "{plain} vs {any}");
        // 80 dB down: the plain meter calls it silent, the unbounded one reads it
        let quiet: Vec<f32> = x.iter().map(|v| v * 1e-4).collect();
        assert!(m.integrated(&quiet, &quiet).is_none());
        let q = m.integrated_any_level(&quiet, &quiet).unwrap();
        assert!((q - (plain - 80.0)).abs() < 0.01, "{q} vs {}", plain - 80.0);
        assert!(m.integrated_any_level(&[0.0; 100], &[0.0; 100]).is_none());
    }

    #[test]
    fn short_clips_fall_back_to_the_ungated_window() {
        let x = sine(SR, 997.0, 0.3, 0.06);
        assert!(StereoMeter::new(48_000).integrated(&x, &x).is_some());
    }

    #[test]
    fn db_helpers() {
        assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-4);
        assert!((gain_to_db(0.5) + 6.0206).abs() < 1e-4);
        assert_eq!(gain_to_db(0.0), -150.0);
        assert!((peak_dbfs(&[&[0.0, -0.5, 0.25]]) + 6.0206).abs() < 1e-4);
        assert!((rms_dbfs(&[&[1.0, -1.0]]) - 0.0).abs() < 1e-9);
    }
}
