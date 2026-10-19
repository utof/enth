//! Spectral features for the sound score and the distances: loudness normalisation (EBU R128), multi-resolution
//! log-mel, an amplitude envelope (plain and crest), an energy-weighted spectral-centroid track, MFCC and an optional
//! YIN f0 track.
//!
//! Ported from `spikes/spike-metric/src/features.rs` (in-repo, MIT; `docs/spikes/metric.md`), on this crate's
//! [`FftPlanner`]: the scalar path by default, so a score is the same on every CPU. Pure: the only state is scratch
//! buffers, so one [`FeatureExtractor`] per worker thread.

use crate::fft::{FftPath, FftPlanner, RealFft, hann};
use crate::loudness::LoudnessMeter;
use crate::pitch::Yin;
use realfft::num_complex::Complex32;
use std::f64::consts::PI;
use std::sync::Arc;

/// Mel bands per resolution.
pub const BANDS: usize = 64;
/// MFCC coefficients, c0 to c12.
pub const N_MFCC: usize = 13;
/// FFT sizes of the three log-mel resolutions.
pub const FFT_SIZES: [usize; 3] = [2048, 1024, 512];
/// Lowest and highest mel band edge (Hz).
pub const MEL_RANGE_HZ: (f64, f64) = (30.0, 16_000.0);
/// Index into [`FFT_SIZES`] that feeds the centroid track and the MFCCs.
const MID_RES: usize = 1;

/// Settings of a [`FeatureExtractor`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeatureCfg {
    pub sample_rate: f64,
    /// Hop is `n_fft / hop_div`. 2 is enough and halves the cost of 4 (`docs/spikes/metric.md`).
    pub hop_div: usize,
    /// Every clip is scaled to this integrated loudness first, so louder never scores closer.
    pub target_lufs: f64,
    /// Crest-envelope half-width in 10 ms frames.
    pub env_hold_frames: usize,
    pub with_f0: bool,
    pub fft: FftPath,
}

impl Default for FeatureCfg {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            hop_div: 2,
            target_lufs: -23.0,
            env_hold_frames: 5,
            with_f0: false,
            fft: FftPath::Scalar,
        }
    }
}

/// Features of one clip, after loudness normalisation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Features {
    /// Natural-log mel magnitudes per resolution, `frames × BANDS` each.
    pub logmel: Vec<Vec<f32>>,
    /// 10 ms RMS in dB (floored at −80), one value per 10 ms.
    pub env_db: Vec<f32>,
    /// The same, max-held over `±env_hold_frames` (a crest envelope).
    pub crest_db: Vec<f32>,
    /// log2 of the energy-weighted spectral centroid (Hz) per frame at the mid resolution.
    pub centroid: Vec<f32>,
    /// Each frame's share of the clip's energy (weights for the centroid distance).
    pub centroid_w: Vec<f32>,
    /// `frames × N_MFCC`, mid resolution.
    pub mfcc: Vec<f32>,
    /// YIN f0 in Hz per 1024-sample hop (0 = unvoiced); empty unless `with_f0`.
    pub f0_hz: Vec<f32>,
    /// Integrated loudness before normalisation (LUFS).
    pub lufs: f64,
    /// Below −70 LUFS: no other field is filled.
    pub silent: bool,
}

struct Res {
    n_fft: usize,
    hop: usize,
    fft: Arc<RealFft>,
    window: Vec<f32>,
    window_sum: f32,
    mel: Vec<(usize, Vec<f32>)>,
}

/// Extracts [`Features`]; holds the FFT plans, filter banks, loudness meter and scratch buffers.
pub struct FeatureExtractor {
    cfg: FeatureCfg,
    res: Vec<Res>,
    dct: Vec<f32>,
    meter: LoudnessMeter,
    yin: Option<Yin>,
    norm: Vec<f32>,
}

impl std::fmt::Debug for FeatureExtractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeatureExtractor")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

fn hz_to_mel(f: f64) -> f64 {
    2595.0 * libm::log10(1.0 + f / 700.0)
}

fn mel_to_hz(m: f64) -> f64 {
    700.0 * (libm::pow(10.0, m / 2595.0) - 1.0)
}

/// Triangular mel filters on the bins of an `n_fft` transform: `(first bin, weights)` per band.
fn mel_bank(sample_rate: f64, n_fft: usize) -> Vec<(usize, Vec<f32>)> {
    let (lo, hi) = (hz_to_mel(MEL_RANGE_HZ.0), hz_to_mel(MEL_RANGE_HZ.1));
    let pts: Vec<f64> = (0..BANDS + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (BANDS + 1) as f64))
        .collect();
    let bin_hz = sample_rate / n_fft as f64;
    let nb = n_fft / 2 + 1;
    let mut out = Vec::with_capacity(BANDS);
    for b in 0..BANDS {
        let (l, c, r) = (pts[b], pts[b + 1], pts[b + 2]);
        let mut start = usize::MAX;
        let mut w = Vec::new();
        for k in 0..nb {
            let f = k as f64 * bin_hz;
            let v = if f > l && f < c {
                (f - l) / (c - l)
            } else if f >= c && f < r {
                (r - f) / (r - c)
            } else {
                0.0
            };
            if v > 0.0 {
                if start == usize::MAX {
                    start = k;
                }
                w.push(v as f32);
            } else if start != usize::MAX {
                break;
            }
        }
        if start == usize::MAX {
            // band narrower than one bin: use the nearest bin
            start = (libm::round(c / bin_hz) as usize).min(nb - 1);
            w.push(1.0);
        }
        out.push((start, w));
    }
    out
}

impl FeatureExtractor {
    /// # Panics
    /// If the sample rate is not a positive integer number of Hz that `ebur128` accepts.
    #[must_use]
    pub fn new(cfg: FeatureCfg) -> Self {
        let mut planner = FftPlanner::new(cfg.fft);
        let res = FFT_SIZES
            .iter()
            .map(|&n_fft| {
                let window = hann(n_fft);
                Res {
                    n_fft,
                    hop: (n_fft / cfg.hop_div.max(1)).max(1),
                    fft: planner.forward(n_fft),
                    window_sum: window.iter().sum(),
                    window,
                    mel: mel_bank(cfg.sample_rate, n_fft),
                }
            })
            .collect();
        let mut dct = vec![0f32; N_MFCC * BANDS];
        for k in 0..N_MFCC {
            for b in 0..BANDS {
                dct[k * BANDS + b] = (libm::cos(PI * k as f64 * (b as f64 + 0.5) / BANDS as f64)
                    * libm::sqrt(2.0 / BANDS as f64)) as f32;
            }
        }
        let yin = cfg
            .with_f0
            .then(|| Yin::new(&mut planner, cfg.sample_rate, 2048, 50.0, 2000.0));
        Self {
            meter: LoudnessMeter::new(cfg.sample_rate as u32),
            cfg,
            res,
            dct,
            yin,
            norm: Vec::new(),
        }
    }

    #[must_use]
    pub fn cfg(&self) -> &FeatureCfg {
        &self.cfg
    }

    /// The loudness meter, for callers that need LUFS without the features.
    pub fn meter(&mut self) -> &mut LoudnessMeter {
        &mut self.meter
    }

    /// Normalise loudness to `target_lufs`, then extract every feature.
    pub fn extract(&mut self, audio: &[f32]) -> Features {
        let mut f = Features::default();
        let Some(lufs) = self.meter.integrated(audio) else {
            f.silent = true;
            return f;
        };
        f.lufs = lufs;
        let gain = libm::pow(10.0, (self.cfg.target_lufs - lufs) / 20.0) as f32;
        let mut x = std::mem::take(&mut self.norm);
        x.clear();
        x.extend(audio.iter().map(|v| v * gain));
        self.extract_normalised(&x, &mut f);
        self.norm = x;
        f
    }

    /// Every feature of `audio` as it is, without loudness normalisation (`lufs` is NaN). `ebur128` uses `std`
    /// maths, so the normalising gain can differ in its last bits between platforms; this path depends only on
    /// this crate's code and the FFT path, and is what the cross-platform determinism test pins.
    pub fn extract_unnormalised(&mut self, audio: &[f32]) -> Features {
        let mut f = Features {
            lufs: f64::NAN,
            ..Features::default()
        };
        if audio.iter().all(|v| *v == 0.0) {
            f.silent = true;
            return f;
        }
        self.extract_normalised(audio, &mut f);
        f
    }

    fn extract_normalised(&mut self, x: &[f32], f: &mut Features) {
        let sr = self.cfg.sample_rate;
        let fl = ((sr * 0.01) as usize).max(1);
        f.env_db = x
            .chunks(fl)
            .map(|c| {
                let e = libm::sqrt(
                    c.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / c.len() as f64,
                );
                (20.0 * libm::log10(e + 1e-9)).max(-80.0) as f32
            })
            .collect();
        let h = self.cfg.env_hold_frames;
        let raw = &f.env_db;
        f.crest_db = (0..raw.len())
            .map(|i| {
                raw[i.saturating_sub(h)..(i + h + 1).min(raw.len())]
                    .iter()
                    .copied()
                    .fold(-80.0, f32::max)
            })
            .collect();
        for (ri, r) in self.res.iter().enumerate() {
            let mut inbuf = vec![0f32; r.n_fft];
            let mut spec = vec![Complex32::new(0.0, 0.0); r.fft.bins()];
            let mut scratch = r.fft.make_scratch();
            let nb = r.n_fft / 2 + 1;
            let mut mag = vec![0f32; nb];
            let mut lm = Vec::new();
            let mut frame_e = Vec::new();
            let mut pos = 0usize;
            while pos + r.n_fft <= x.len() {
                for i in 0..r.n_fft {
                    inbuf[i] = x[pos + i] * r.window[i];
                }
                r.fft.forward(&mut inbuf, &mut spec, &mut scratch);
                for k in 0..nb {
                    mag[k] = spec[k].norm_sqr().sqrt() / r.window_sum;
                }
                let start = lm.len();
                for (s0, w) in &r.mel {
                    let mut acc = 0f32;
                    for (j, wj) in w.iter().enumerate() {
                        acc += mag[s0 + j] * wj;
                    }
                    lm.push(libm::logf(acc + 1e-5));
                }
                if ri == MID_RES {
                    let (mut num, mut den) = (0f64, 0f64);
                    for (k, m) in mag.iter().enumerate().skip(1) {
                        let p = f64::from(*m) * f64::from(*m);
                        num += p * k as f64;
                        den += p;
                    }
                    let c_hz = if den > 0.0 {
                        num / den * sr / r.n_fft as f64
                    } else {
                        100.0
                    };
                    f.centroid.push(libm::log2(c_hz.max(20.0)) as f32);
                    frame_e.push(den);
                    let frame = &lm[start..start + BANDS];
                    for k in 0..N_MFCC {
                        let row = &self.dct[k * BANDS..(k + 1) * BANDS];
                        f.mfcc.push(frame.iter().zip(row).map(|(v, c)| v * c).sum());
                    }
                }
                pos += r.hop;
            }
            if ri == MID_RES {
                let total: f64 = frame_e.iter().sum();
                f.centroid_w = frame_e
                    .iter()
                    .map(|e| (e / total.max(1e-24)) as f32)
                    .collect();
            }
            f.logmel.push(lm);
        }
        if let Some(yin) = &mut self.yin {
            f.f0_hz = yin
                .track(x, 1024)
                .into_iter()
                .map(|v| v.unwrap_or(0.0) as f32)
                .collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{SR, sine, white_noise};

    #[test]
    fn mel_bank_covers_the_range() {
        let bank = mel_bank(SR, 2048);
        assert_eq!(bank.len(), BANDS);
        // bands are ordered and the last one ends below 16 kHz
        assert!(bank.windows(2).all(|w| w[0].0 <= w[1].0));
        let (s, w) = bank.last().unwrap();
        assert!(((s + w.len()) as f64 * SR / 2048.0) <= 16_100.0);
    }

    #[test]
    fn features_are_loudness_normalised_and_shaped() {
        let mut fx = FeatureExtractor::new(FeatureCfg::default());
        let x = sine(SR, 1000.0, 0.3, 1.0);
        let a = fx.extract(&x);
        let y: Vec<f32> = x.iter().map(|v| v * 0.25).collect();
        let b = fx.extract(&y);
        assert!(!a.silent && (a.lufs - b.lufs - 12.04).abs() < 0.05);
        for (r, n) in FFT_SIZES.iter().enumerate() {
            let frames = (x.len() - n) / (n / 2) + 1;
            assert_eq!(a.logmel[r].len(), frames * BANDS);
            let d: f32 = a.logmel[r]
                .iter()
                .zip(&b.logmel[r])
                .map(|(p, q)| (p - q).abs())
                .fold(0.0, f32::max);
            assert!(d < 1e-3, "normalised features match: {d}");
        }
        // a 1 kHz sine's centroid track sits at 1 kHz
        let c = libm::exp2(f64::from(a.centroid[a.centroid.len() / 2]));
        assert!((c - 1000.0).abs() < 30.0, "{c}");
        assert_eq!(a.mfcc.len(), a.centroid.len() * N_MFCC);
        assert!(fx.extract(&vec![0.0; 48_000]).silent);
    }

    #[test]
    fn scalar_and_dispatched_agree_within_tolerance() {
        let x = white_noise(SR, 0.3, 0.5, 7);
        let a = FeatureExtractor::new(FeatureCfg::default()).extract(&x);
        let b = FeatureExtractor::new(FeatureCfg {
            fft: FftPath::Dispatched,
            ..FeatureCfg::default()
        })
        .extract(&x);
        for (p, q) in a.logmel.iter().zip(&b.logmel) {
            let d = p
                .iter()
                .zip(q)
                .map(|(u, v)| (u - v).abs())
                .fold(0.0, f32::max);
            assert!(d < 1e-3, "{d}");
        }
    }
}
