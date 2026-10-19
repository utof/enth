//! Audio features for the sound scorer: loudness normalisation (EBU R128), multi-resolution log-mel, amplitude envelope,
//! spectral-centroid track, MFCC and optional YIN f0.
//!
//! Pure: no I/O, no clocks, no global RNG. The only state is scratch buffers owned by [`Analyzer`] (one per worker thread).

use crate::yin::Yin;
use core::f32::consts::PI;
use ebur128::{EbuR128, Mode};
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub const BANDS: usize = 64;
pub const N_MFCC: usize = 13;
const FFT_SIZES: [usize; 3] = [2048, 1024, 512];
/// Index into [`FFT_SIZES`] that feeds the centroid track and the MFCCs.
const MID_RES: usize = 1;
/// Below this loudness (the BS.1770 absolute gate) a clip counts as silent.
const SILENCE_LUFS: f64 = -70.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnalysisCfg {
    pub sample_rate: f32,
    /// Hop is `n_fft / hop_div`. 2 is enough and halves the cost of 4 (04-metric-search-final, Q6).
    pub hop_div: usize,
    /// Both clips of every comparison are scaled to this integrated loudness first, so louder never scores closer.
    pub target_lufs: f64,
    /// Crest-envelope half-width in 10 ms frames: detuned oscillators make the raw RMS beat at the detune rate.
    pub env_hold_frames: usize,
    pub with_f0: bool,
}

impl Default for AnalysisCfg {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            hop_div: 2,
            target_lufs: -23.0,
            env_hold_frames: 5,
            with_f0: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Features {
    /// Log-mel per resolution, `frames * BANDS` each.
    pub logmel: Vec<Vec<f32>>,
    /// Crest envelope: 10 ms RMS in dB, max-held over `+-env_hold_frames`.
    pub env_db: Vec<f32>,
    /// log2 of the energy-weighted spectral centroid (Hz) per frame at the mid resolution.
    pub centroid: Vec<f32>,
    /// Each frame's share of the clip's energy (weights for the centroid distance).
    pub centroid_w: Vec<f32>,
    /// `frames * N_MFCC`, mid resolution.
    pub mfcc: Vec<f32>,
    /// YIN f0 in Hz per 1024-sample hop (0.0 = unvoiced); empty unless `with_f0`.
    pub f0_hz: Vec<f32>,
    /// Integrated loudness before normalisation.
    pub lufs: f64,
    pub silent: bool,
}

struct Res {
    n_fft: usize,
    hop: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    window_sum: f32,
    mel: Vec<(usize, Vec<f32>)>,
}

pub struct Analyzer {
    cfg: AnalysisCfg,
    res: Vec<Res>,
    dct: Vec<f32>,
    ebu: EbuR128,
    yin: Option<Yin>,
    norm: Vec<f32>,
}

fn hz_to_mel(f: f32) -> f32 {
    2595.0 * libm::log10f(1.0 + f / 700.0)
}

fn mel_to_hz(m: f32) -> f32 {
    700.0 * (libm::powf(10.0, m / 2595.0) - 1.0)
}

fn mel_bank(sample_rate: f32, n_fft: usize) -> Vec<(usize, Vec<f32>)> {
    let (lo, hi) = (hz_to_mel(30.0), hz_to_mel(16_000.0));
    let pts: Vec<f32> = (0..BANDS + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f32 / (BANDS + 1) as f32))
        .collect();
    let bin_hz = sample_rate / n_fft as f32;
    let nb = n_fft / 2 + 1;
    let mut out = Vec::new();
    for b in 0..BANDS {
        let (l, c, r) = (pts[b], pts[b + 1], pts[b + 2]);
        let mut start = usize::MAX;
        let mut w = Vec::new();
        for k in 0..nb {
            let f = k as f32 * bin_hz;
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
                w.push(v);
            } else if start != usize::MAX {
                break;
            }
        }
        if start == usize::MAX {
            // band narrower than one bin: use the nearest bin
            start = ((c / bin_hz).round() as usize).min(nb - 1);
            w.push(1.0);
        }
        out.push((start, w));
    }
    out
}

impl Analyzer {
    #[must_use]
    pub fn new(cfg: AnalysisCfg) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let res = FFT_SIZES
            .iter()
            .map(|&n_fft| {
                let window: Vec<f32> = (0..n_fft)
                    .map(|i| 0.5 - 0.5 * libm::cosf(2.0 * PI * i as f32 / n_fft as f32))
                    .collect();
                Res {
                    n_fft,
                    hop: (n_fft / cfg.hop_div).max(1),
                    fft: planner.plan_fft_forward(n_fft),
                    window_sum: window.iter().sum(),
                    window,
                    mel: mel_bank(cfg.sample_rate, n_fft),
                }
            })
            .collect();
        let mut dct = vec![0f32; N_MFCC * BANDS];
        for k in 0..N_MFCC {
            for b in 0..BANDS {
                dct[k * BANDS + b] = libm::cosf(PI * k as f32 * (b as f32 + 0.5) / BANDS as f32)
                    * libm::sqrtf(2.0 / BANDS as f32);
            }
        }
        let ebu = EbuR128::new(1, cfg.sample_rate as u32, Mode::I | Mode::S)
            .expect("valid ebur128 settings");
        let yin = cfg
            .with_f0
            .then(|| Yin::new(cfg.sample_rate, 2048, 50.0, 2000.0));
        Self {
            cfg,
            res,
            dct,
            ebu,
            yin,
            norm: Vec::new(),
        }
    }

    #[must_use]
    pub fn cfg(&self) -> &AnalysisCfg {
        &self.cfg
    }

    /// Integrated loudness in LUFS. Falls back to the ungated whole-clip window when the gate leaves nothing
    /// (very short sounds). `None` = silent (below the BS.1770 absolute gate).
    pub fn loudness(&mut self, audio: &[f32]) -> Option<f64> {
        self.ebu.reset();
        self.ebu.add_frames_f32(audio).ok()?;
        let gated = self.ebu.loudness_global().ok().filter(|l| l.is_finite());
        let l = gated.or_else(|| {
            let ms = (audio.len() as f64 / f64::from(self.cfg.sample_rate) * 1000.0) as u32;
            self.ebu
                .loudness_window(ms.clamp(100, 3000))
                .ok()
                .filter(|l| l.is_finite())
        })?;
        (l > SILENCE_LUFS).then_some(l)
    }

    /// Gain (linear) that brings `audio` to the target loudness; `None` if silent.
    pub fn gain_to_target(&mut self, audio: &[f32]) -> Option<f32> {
        let l = self.loudness(audio)?;
        Some(libm::pow(10.0, (self.cfg.target_lufs - l) / 20.0) as f32)
    }

    /// Normalise loudness, then extract every feature.
    pub fn features(&mut self, audio: &[f32]) -> Features {
        let mut f = Features::default();
        let Some(lufs) = self.loudness(audio) else {
            f.silent = true;
            return f;
        };
        f.lufs = lufs;
        let gain = libm::pow(10.0, (self.cfg.target_lufs - lufs) / 20.0) as f32;
        let mut x = std::mem::take(&mut self.norm);
        x.clear();
        x.extend(audio.iter().map(|v| v * gain));
        self.extract(&x, &mut f);
        self.norm = x;
        f
    }

    fn extract(&mut self, x: &[f32], f: &mut Features) {
        let sr = self.cfg.sample_rate;
        // crest envelope: 10 ms RMS in dB, max-held
        let fl = (sr * 0.01) as usize;
        let raw: Vec<f32> = x
            .chunks(fl)
            .map(|c| {
                let e = libm::sqrtf(c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32);
                (20.0 * libm::log10f(e + 1e-9)).max(-80.0)
            })
            .collect();
        let h = self.cfg.env_hold_frames;
        f.env_db = (0..raw.len())
            .map(|i| {
                raw[i.saturating_sub(h)..(i + h + 1).min(raw.len())]
                    .iter()
                    .copied()
                    .fold(-80.0, f32::max)
            })
            .collect();
        for (ri, r) in self.res.iter().enumerate() {
            let mut inbuf = r.fft.make_input_vec();
            let mut spec = r.fft.make_output_vec();
            let mut scratch = r.fft.make_scratch_vec();
            let nb = r.n_fft / 2 + 1;
            let mut mag = vec![0f32; nb];
            let mut lm = Vec::new();
            let mut frame_e = Vec::new();
            let mut pos = 0usize;
            while pos + r.n_fft <= x.len() {
                for i in 0..r.n_fft {
                    inbuf[i] = x[pos + i] * r.window[i];
                }
                r.fft
                    .process_with_scratch(&mut inbuf, &mut spec, &mut scratch)
                    .expect("fft sizes match the plan");
                for k in 0..nb {
                    mag[k] = libm::sqrtf(spec[k].norm_sqr()) / r.window_sum;
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
                    let (mut num, mut den) = (0f32, 0f32);
                    for k in 1..nb {
                        let p = mag[k] * mag[k];
                        num += p * k as f32;
                        den += p;
                    }
                    let c_hz = if den > 0.0 {
                        num / den * sr / r.n_fft as f32
                    } else {
                        100.0
                    };
                    f.centroid.push(libm::log2f(c_hz.max(20.0)));
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
                let total: f32 = frame_e.iter().sum();
                f.centroid_w = frame_e.iter().map(|e| e / total.max(1e-12)).collect();
            }
            f.logmel.push(lm);
        }
        if let Some(yin) = &mut self.yin {
            f.f0_hz = yin.track(x, 1024);
        }
    }
}
