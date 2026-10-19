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

/// The pitch of the steady tonal part of a sound, read from the peaks of its power spectrum. Unlike YIN, which needs
/// the waveform itself to repeat, it still finds a tone under strong noise (a snare's body under its crack).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tonal {
    /// Fundamental of the harmonic series that best explains the spectral peaks (Hz).
    pub f0_hz: f64,
    /// Nearest note name (A4 = 440 Hz).
    pub note: String,
    /// Offset of `f0_hz` from that note, in cents.
    pub cents_dev: f64,
    /// Share of the peaks' power that the harmonic series of `f0_hz` explains, 0–1.
    pub harmonicity: f64,
    /// Share of the band's power (20 Hz–16 kHz) that is in the peaks, 0–1: the tonal part's weight in the sound.
    pub share: f64,
}

/// The tonal part must hold at least this share of the band's power (−20 dB re the whole).
pub const TONAL_MIN_SHARE: f64 = 0.01;
/// The chosen series must explain at least this share of the peaks' power.
pub const TONAL_MIN_HARMONICITY: f64 = 0.5;
/// Fundamentals searched (Hz).
pub const TONAL_F0_RANGE: (f64, f64) = (25.0, 2000.0);
/// Highest harmonic number matched to a candidate fundamental.
const TONAL_MAX_HARMONIC: f64 = 16.0;
/// Peaks used: at most this many, the strongest, each within 40 dB of the strongest.
const TONAL_MAX_PEAKS: usize = 32;
/// Candidate fundamentals come from the strongest few peaks divided by 1..=16.
const TONAL_SOURCE_PEAKS: usize = 8;
/// A candidate within this share of the best score counts as tied; the highest tied candidate wins (so a series is
/// never read an octave low just because its subharmonic explains the same peaks).
const TONAL_TIE: f64 = 0.9;

/// One spectral peak: its interpolated frequency and its power (the peak bin and 3 bins either side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Peak {
    pub hz: f64,
    pub power: f64,
}

/// Matched power of the harmonic series on `f0`, and the least-squares refined fundamental of the matched peaks.
fn series_score(peaks: &[Peak], f0: f64, bin_hz: f64) -> (f64, f64) {
    let (mut matched, mut num, mut den) = (0.0, 0.0, 0.0);
    for p in peaks {
        let h = libm::round(p.hz / f0);
        if !(1.0..=TONAL_MAX_HARMONIC).contains(&h) {
            continue;
        }
        // 0.6 bin for the interpolation error, 1.2 % (about 20 ct) for detune and stretched partials; never so wide
        // that neighbouring harmonics of a low candidate overlap.
        let tol = (0.6 * bin_hz).max(0.012 * h * f0).min(0.25 * f0);
        if (p.hz - h * f0).abs() <= tol {
            matched += p.power;
            num += p.power * h * p.hz;
            den += p.power * h * h;
        }
    }
    let refined = if den > 0.0 { num / den } else { f0 };
    (matched, refined)
}

/// The tonal pitch of a set of spectral peaks (`share` is their share of the band's power), by harmonic matching:
/// every strong peak divided by 1..=16 is a candidate fundamental in 25–2000 Hz; a candidate scores the power of
/// the peaks within tolerance of its harmonics 1..=16; the highest candidate within 90 % of the best score wins,
/// refined by a power-weighted least-squares fit through the matched peaks. `None` when the tonal part is too weak
/// (`share` below [`TONAL_MIN_SHARE`]) or no series explains half the peaks' power. A missing fundamental is
/// restored (harmonics 2, 3 and 4 of 100 Hz read 100 Hz), as the ear does.
#[must_use]
pub fn tonal_pitch(peaks: &[Peak], share: f64, bin_hz: f64) -> Option<Tonal> {
    if peaks.is_empty() || share.is_nan() || share < TONAL_MIN_SHARE {
        return None;
    }
    let mut ps: Vec<Peak> = peaks.to_vec();
    // Strongest first; ties broken by frequency so the order never depends on the input's.
    ps.sort_by(|a, b| b.power.total_cmp(&a.power).then(a.hz.total_cmp(&b.hz)));
    let floor = ps[0].power * 1e-4;
    ps.retain(|p| p.power >= floor);
    ps.truncate(TONAL_MAX_PEAKS);
    let total: f64 = ps.iter().map(|p| p.power).sum();
    if total <= 0.0 {
        return None;
    }
    let mut cands = Vec::new();
    for p in ps.iter().take(TONAL_SOURCE_PEAKS) {
        for h in 1..=TONAL_MAX_HARMONIC as usize {
            let f = p.hz / h as f64;
            if (TONAL_F0_RANGE.0..=TONAL_F0_RANGE.1).contains(&f) {
                cands.push(f);
            }
        }
    }
    let scored: Vec<(f64, f64, f64)> = cands
        .iter()
        .map(|&f| {
            let (m, r) = series_score(&ps, f, bin_hz);
            (f, m, r)
        })
        .collect();
    let best = scored.iter().map(|s| s.1).fold(0.0, f64::max);
    if best <= 0.0 {
        return None;
    }
    let (_, _, refined) = scored.iter().filter(|s| s.1 >= TONAL_TIE * best).fold(
        None,
        |acc: Option<(f64, f64, f64)>, s| match acc {
            Some(a) if a.0 >= s.0 => Some(a),
            _ => Some(*s),
        },
    )?;
    let (matched, _) = series_score(&ps, refined, bin_hz);
    let harmonicity = (matched / total).clamp(0.0, 1.0);
    if harmonicity < TONAL_MIN_HARMONICITY || !refined.is_finite() || refined <= 0.0 {
        return None;
    }
    let (note, cents_dev) = nearest_note(refined);
    Some(Tonal {
        f0_hz: refined,
        note,
        cents_dev,
        harmonicity,
        share: share.clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{SR, Wave, oscillator, sine};

    fn series(f0: f64, amps: &[(usize, f64)]) -> Vec<Peak> {
        amps.iter()
            .map(|&(h, a)| Peak {
                hz: f0 * h as f64,
                power: a * a,
            })
            .collect()
    }

    #[test]
    fn tonal_pitch_reads_the_series_not_an_octave() {
        let bin = SR / 8192.0;
        // a saw-like series, a square-like (odd) series, a lone sine, a missing fundamental
        let saw: Vec<(usize, f64)> = (1..=20).map(|h| (h, 1.0 / h as f64)).collect();
        let odd: Vec<(usize, f64)> = (0..10)
            .map(|k| (2 * k + 1, 1.0 / (2 * k + 1) as f64))
            .collect();
        for (peaks, want) in [
            (series(220.0, &saw), 220.0),
            (series(220.0, &odd), 220.0),
            (series(440.0, &[(1, 1.0)]), 440.0),
            (series(100.0, &[(2, 1.0), (3, 0.8), (4, 0.6)]), 100.0),
            (
                series(32.7, &[(1, 1.0), (2, 0.5), (4, 0.3), (6, 0.2)]),
                32.7,
            ),
        ] {
            let t = tonal_pitch(&peaks, 0.5, bin).unwrap();
            assert!(cents(t.f0_hz, want).abs() < 1.0, "{want}: {t:?}");
            assert!(t.harmonicity > 0.9, "{t:?}");
        }
        // too weak, or nothing harmonic
        assert!(tonal_pitch(&series(220.0, &saw), 0.005, bin).is_none());
        assert!(tonal_pitch(&[], 0.5, bin).is_none());
    }

    #[test]
    fn a_strong_sub_octave_is_the_fundamental_and_a_weak_one_is_not() {
        let bin = SR / 8192.0;
        let mut strong = series(65.4, &[(1, 0.45), (2, 0.22), (3, 0.15)]);
        strong.push(Peak {
            hz: 32.7,
            power: 1.0,
        });
        assert!(cents(tonal_pitch(&strong, 0.9, bin).unwrap().f0_hz, 32.7).abs() < 1.0);
        let mut weak = series(65.4, &[(1, 1.0), (2, 0.5), (3, 0.33)]);
        weak.push(Peak {
            hz: 32.7,
            power: 0.01,
        });
        assert!(cents(tonal_pitch(&weak, 0.9, bin).unwrap().f0_hz, 65.4).abs() < 1.0);
    }

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
