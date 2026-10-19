//! YIN pitch tracking (de Cheveigné and Kawahara, "YIN, a fundamental frequency estimator for speech and music",
//! JASA 111(4), 2002), with the difference function computed by FFT.
//!
//! Ported from `spikes/spike-metric/src/yin.rs` (in-repo, MIT), on this crate's [`FftPlanner`] so the scalar path
//! gives the same track on every platform.

use crate::fft::{FftPath, FftPlanner, RealFft};
use crate::units::{cents, nearest_note};
use realfft::num_complex::Complex32;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// YIN's absolute threshold on the cumulative-mean-normalised difference.
pub const THRESHOLD: f32 = 0.15;

/// A YIN tracker for one frame size.
pub struct Yin {
    sample_rate: f64,
    frame: usize,
    window: usize,
    tau_min: usize,
    tau_max: usize,
    fwd: Arc<RealFft>,
    inv: Arc<RealFft>,
    buf_a: Vec<f32>,
    buf_b: Vec<f32>,
    spec_a: Vec<Complex32>,
    spec_b: Vec<Complex32>,
    scratch_f: Vec<Complex32>,
    scratch_i: Vec<Complex32>,
    pre: Vec<f64>,
    cmnd: Vec<f32>,
}

impl std::fmt::Debug for Yin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Yin")
            .field("frame", &self.frame)
            .field("tau", &(self.tau_min..=self.tau_max))
            .finish_non_exhaustive()
    }
}

impl Yin {
    /// `frame` must be even; the integration window is `frame − tau_max`, with `tau_max = sr / f_min` capped at
    /// half the frame.
    #[must_use]
    pub fn new(
        planner: &mut FftPlanner,
        sample_rate: f64,
        frame: usize,
        f_min: f64,
        f_max: f64,
    ) -> Self {
        let tau_max = ((sample_rate / f_min) as usize).min(frame / 2);
        let tau_min = ((sample_rate / f_max) as usize).max(2);
        let fwd = planner.forward(frame);
        let inv = planner.inverse(frame);
        Self {
            sample_rate,
            frame,
            window: frame - tau_max,
            tau_min,
            tau_max,
            buf_a: vec![0.0; frame],
            buf_b: vec![0.0; frame],
            spec_a: vec![Complex32::new(0.0, 0.0); fwd.bins()],
            spec_b: vec![Complex32::new(0.0, 0.0); fwd.bins()],
            scratch_f: fwd.make_scratch(),
            scratch_i: inv.make_scratch(),
            pre: vec![0.0; frame + 1],
            cmnd: vec![0.0; tau_max + 1],
            fwd,
            inv,
        }
    }

    #[must_use]
    pub fn frame(&self) -> usize {
        self.frame
    }

    #[must_use]
    pub fn path(&self) -> FftPath {
        self.fwd.path()
    }

    /// f0 in Hz for one `frame`-sample block; `None` when unvoiced or silent.
    ///
    /// # Panics
    /// If `x.len()` differs from the frame size.
    pub fn estimate(&mut self, x: &[f32]) -> Option<f64> {
        assert_eq!(x.len(), self.frame, "YIN frame length");
        let (w, tau_max) = (self.window, self.tau_max);
        self.buf_a.fill(0.0);
        self.buf_a[..w].copy_from_slice(&x[..w]);
        self.buf_b.copy_from_slice(x);
        self.fwd
            .forward(&mut self.buf_a, &mut self.spec_a, &mut self.scratch_f);
        self.fwd
            .forward(&mut self.buf_b, &mut self.spec_b, &mut self.scratch_f);
        for (a, b) in self.spec_a.iter_mut().zip(&self.spec_b) {
            *a = a.conj() * b;
        }
        // The end bins are real for real inputs; make that exact for the inverse.
        let last = self.spec_a.len() - 1;
        self.spec_a[0].im = 0.0;
        self.spec_a[last].im = 0.0;
        self.inv
            .inverse(&mut self.spec_a, &mut self.buf_a, &mut self.scratch_i);
        let norm = 1.0 / self.frame as f64;
        for (i, v) in x.iter().enumerate() {
            self.pre[i + 1] = self.pre[i] + f64::from(*v) * f64::from(*v);
        }
        let e0 = self.pre[w];
        if e0 < 1e-9 {
            return None;
        }
        // d(tau) = e(0..w) + e(tau..tau+w) - 2 acf(tau), then cumulative-mean-normalised.
        self.cmnd[0] = 1.0;
        let mut running = 0.0f64;
        for tau in 1..=tau_max {
            let acf = f64::from(self.buf_a[tau]) * norm;
            let d = (e0 + (self.pre[tau + w] - self.pre[tau]) - 2.0 * acf).max(0.0);
            running += d;
            self.cmnd[tau] = if running > 0.0 {
                (d * tau as f64 / running) as f32
            } else {
                1.0
            };
        }
        // First dip under the threshold, walked down to its local minimum.
        let mut tau = self.tau_min;
        let mut found = None;
        while tau < tau_max {
            if self.cmnd[tau] < THRESHOLD {
                while tau + 1 < tau_max && self.cmnd[tau + 1] < self.cmnd[tau] {
                    tau += 1;
                }
                found = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = found?;
        // Parabolic interpolation around the minimum.
        let (y0, y1, y2) = (
            self.cmnd[tau - 1],
            self.cmnd[tau],
            self.cmnd[(tau + 1).min(tau_max)],
        );
        let denom = y0 - 2.0 * y1 + y2;
        let shift = if denom.abs() > 1e-12 {
            0.5 * (y0 - y2) / denom
        } else {
            0.0
        };
        Some(self.sample_rate / (tau as f64 + f64::from(shift.clamp(-1.0, 1.0))))
    }

    /// f0 per `hop` (`None` where unvoiced); frame `i` covers `i·hop .. i·hop + frame`.
    pub fn track(&mut self, x: &[f32], hop: usize) -> Vec<Option<f64>> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos + self.frame <= x.len() {
            out.push(self.estimate(&x[pos..pos + self.frame]));
            pos += hop;
        }
        out
    }
}

/// Pitch descriptors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pitch {
    /// Median YIN estimate over the voiced frames within 40 dB of the envelope peak (Hz).
    pub f0_hz: f64,
    /// Nearest note name (A4 = 440 Hz).
    pub note: String,
    /// Offset of `f0_hz` from that note, in cents.
    pub cents_dev: f64,
    /// 1 − (standard deviation of the voiced frames in cents) / 50, clamped to 0..1. 1 is a steady pitch.
    pub stability: f64,
    /// Share of the loud frames that YIN found voiced.
    pub voiced: f64,
}

/// Summarise a YIN track. `loud[i]` says whether frame `i` is loud enough to count. `None` with fewer than two
/// voiced loud frames.
#[must_use]
pub fn summarise(track: &[Option<f64>], loud: &[bool]) -> Option<Pitch> {
    let n_loud = loud.iter().filter(|l| **l).count();
    let mut voiced: Vec<f64> = track
        .iter()
        .zip(loud)
        .filter(|(_, l)| **l)
        .filter_map(|(f, _)| *f)
        .collect();
    if voiced.len() < 2 {
        return None;
    }
    voiced.sort_by(f64::total_cmp);
    let f0 = voiced[voiced.len() / 2];
    let c: Vec<f64> = voiced.iter().map(|f| cents(*f, f0)).collect();
    let mean = c.iter().sum::<f64>() / c.len() as f64;
    let std = (c.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / c.len() as f64).sqrt();
    let (note, cents_dev) = nearest_note(f0);
    Some(Pitch {
        f0_hz: f0,
        note,
        cents_dev,
        stability: (1.0 - std / 50.0).clamp(0.0, 1.0),
        voiced: voiced.len() as f64 / n_loud.max(1) as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{SR, Wave, oscillator, sine};

    fn yin(path: FftPath) -> Yin {
        Yin::new(&mut FftPlanner::new(path), SR, 2048, 50.0, 2000.0)
    }

    #[test]
    fn sines_read_their_frequency() {
        for path in [FftPath::Scalar, FftPath::Dispatched] {
            let mut y = yin(path);
            for f in [65.406, 110.0, 440.0, 1046.5] {
                let x = sine(SR, f, 0.4, 0.1);
                let got = y.estimate(&x[..2048]).unwrap();
                assert!(cents(got, f).abs() < 3.0, "{path:?} {f}: {got}");
            }
            assert!(y.estimate(&vec![0.0; 2048]).is_none());
        }
    }

    #[test]
    fn saws_read_their_fundamental_not_an_octave() {
        let mut y = yin(FftPath::Scalar);
        for f in [65.41, 130.81, 523.25] {
            let x = oscillator(SR, Wave::Saw, f, 0.1, 0.0);
            let got = y.estimate(&x[..2048]).unwrap();
            assert!(cents(got, f).abs() < 5.0, "{f}: {got}");
        }
    }

    #[test]
    fn summary_median_and_stability() {
        let track = [Some(440.0), Some(441.0), None, Some(439.0), Some(880.0)];
        let p = summarise(&track, &[true, true, true, true, false]).unwrap();
        assert_eq!(p.f0_hz, 440.0);
        assert_eq!(p.note, "A4");
        assert!(p.stability > 0.9, "{p:?}");
        assert!((p.voiced - 0.75).abs() < 1e-12);
        assert!(summarise(&[Some(440.0)], &[true]).is_none());
    }
}
