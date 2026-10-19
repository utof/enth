//! The multi-harmonic beating estimator, with tremolo detection.
//!
//! Two oscillators detuned by Δf make harmonic *h* beat at *h*·Δf. Demodulate each strong harmonic *h*·f0 to a
//! magnitude track (a boxcar of 4 periods of f0, which nulls the other harmonics, decimated to 500 Hz), divide out
//! the overall fade (a cubic fit of the log magnitude), and take the modulation spectrum `M_h(f)` as a relative
//! wobble amplitude (Hann, zero-padded ×8). The fundamental beat rate r maximises the harmonic sum
//! `S(r)` = weighted mean over usable h of `M_h(h·r)`, where harmonic h is usable for r when h·r has at least 2.5 cycles
//! in the stretch, h·r < f0/8 (inside the demodulator's passband) and h·r ≤ 60 Hz. Higher harmonics make slow beats
//! measurable (5 ct at C3 beats at 0.38 Hz; harmonic 8 beats at 3 Hz) and sharpen the estimate by a factor h.
//!
//! A tremolo (an amplitude LFO) moves every harmonic at the *same* rate, detune beating moves harmonic h at h·r.
//! Comparing the two readings on harmonics ≥ 2 tells them apart: a candidate where the same-rate reading is at least
//! as strong is not beating. A separate same-rate search then reports the tremolo itself.
//!
//! Report 03 §4.2 (P1a research, prototype `proto03/src/beat2.rs`); the tremolo report is new here. Measured
//! accuracy: `docs/dsp/analysis.md`.

use crate::fft::{FftPlanner, hann};
use crate::linalg::{median, polyfit_eval};
use realfft::num_complex::Complex32;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// Settings of the estimator. The defaults are report 03's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatCfg {
    /// Highest harmonic demodulated (also capped at 0.45·sr and 10 kHz).
    pub max_harmonics: usize,
    /// Search band for the beat rate (Hz).
    pub r_min: f64,
    pub r_max: f64,
    /// Smallest relative wobble of the best harmonic that counts as beating (0.35 ≈ 6 dB peak to trough).
    pub min_depth: f64,
    /// S(r) must be this many times the median of S over the band.
    pub min_prominence: f64,
    /// Smallest relative wobble that counts as a tremolo (0.1 ≈ 1.7 dB peak to trough).
    pub tremolo_min_depth: f64,
}

impl Default for BeatCfg {
    fn default() -> Self {
        Self {
            max_harmonics: 12,
            r_min: 0.15,
            r_max: 25.0,
            min_depth: 0.35,
            min_prominence: 6.0,
            tremolo_min_depth: 0.1,
        }
    }
}

/// Detune beating.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Beating {
    /// Beat rate of the fundamental (Hz).
    pub beating_hz: f64,
    /// Peak-to-trough depth of the deepest harmonic's wobble at its rate (dB).
    pub depth_db: f64,
    /// Number of harmonics that contributed at this rate.
    pub harmonics_used: usize,
    /// S(r) over the median of S across the band.
    pub prominence: f64,
}

/// An amplitude LFO: every harmonic moves at the same rate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tremolo {
    pub hz: f64,
    /// Peak-to-trough depth of the weighted mean wobble (dB).
    pub depth_db: f64,
    pub harmonics_used: usize,
}

/// What the estimator found in one stretch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Modulation {
    pub beating: Option<Beating>,
    pub tremolo: Option<Tremolo>,
}

struct Track {
    h: usize,
    /// Relative-wobble modulation spectrum, one value per bin of `f_res` Hz.
    spec: Vec<f64>,
    f_res: f64,
    weight: f64,
}

impl Track {
    /// Linear interpolation of the spectrum at `f` Hz (0 outside it).
    fn at(&self, f: f64) -> f64 {
        let k = f / self.f_res;
        let i = libm::floor(k) as usize;
        if i + 1 >= self.spec.len() {
            return 0.0;
        }
        let fr = k - i as f64;
        self.spec[i] * (1.0 - fr) + self.spec[i + 1] * fr
    }

    /// Frequency of this track's own spectral maximum within ±15 % of `f`, if it is a true interior peak.
    fn own_peak_near(&self, f: f64) -> Option<f64> {
        let lo = (f * 0.85 / self.f_res) as usize;
        let hi = ((f * 1.15 / self.f_res) as usize).min(self.spec.len() - 1);
        if lo >= hi {
            return None;
        }
        let k = (lo..=hi).max_by(|&a, &b| self.spec[a].total_cmp(&self.spec[b]))?;
        (k != lo && k != hi).then_some(k as f64 * self.f_res)
    }
}

/// Magnitude track of the harmonic at `fh` Hz over `seg`: boxcar of `m_len` samples, one value every `dec` samples.
/// A phasor recurrence, renormalised every 1,024 samples, so there is no per-sample `sin`.
fn demod(seg: &[f32], fh: f64, sr: f64, m_len: usize, dec: usize) -> Vec<f64> {
    let w = 2.0 * PI * fh / sr;
    let (rot_re, rot_im) = (libm::cos(w), -libm::sin(w));
    let (mut pr, mut pi) = (1.0f64, 0.0f64);
    let mut pre_re = Vec::with_capacity(seg.len() + 1);
    let mut pre_im = Vec::with_capacity(seg.len() + 1);
    let (mut re, mut im) = (0.0f64, 0.0f64);
    pre_re.push(0.0);
    pre_im.push(0.0);
    for (n, &s) in seg.iter().enumerate() {
        re += f64::from(s) * pr;
        im += f64::from(s) * pi;
        pre_re.push(re);
        pre_im.push(im);
        let npr = pr * rot_re - pi * rot_im;
        let npi = pr * rot_im + pi * rot_re;
        pr = npr;
        pi = npi;
        if n % 1024 == 1023 {
            let g = (pr * pr + pi * pi).sqrt();
            pr /= g;
            pi /= g;
        }
    }
    let mut m = Vec::with_capacity(seg.len() / dec + 1);
    let mut n0 = 0;
    while n0 + m_len <= seg.len() {
        let (a, b) = (
            pre_re[n0 + m_len] - pre_re[n0],
            pre_im[n0 + m_len] - pre_im[n0],
        );
        m.push((a * a + b * b).sqrt() / m_len as f64);
        n0 += dec;
    }
    m
}

/// Relative depth `a` of a wobble `(1 ± a)` as peak to trough in dB.
fn depth_db(a: f64) -> f64 {
    let a = a.min(0.95);
    20.0 * libm::log10((1.0 + a) / (1.0 - a))
}

/// Estimate detune beating and tremolo in `x[start..end]`, given the fundamental `f0`. The stretch should be the
/// held part of the note (at least 0.5 s; shorter returns nothing).
pub fn estimate(
    planner: &mut FftPlanner,
    x: &[f32],
    sr: f64,
    f0: f64,
    start: usize,
    end: usize,
    cfg: &BeatCfg,
) -> Modulation {
    let none = Modulation::default();
    let end = end.min(x.len());
    if f0.is_nan() || f0 <= 0.0 || end <= start || end - start < (0.5 * sr) as usize {
        return none;
    }
    let seg = &x[start..end];
    let m_len = (libm::round(4.0 * sr / f0) as usize).max(8);
    if m_len >= seg.len() {
        return none;
    }
    let dec = (libm::round(sr / 500.0) as usize).max(1);
    let rate = sr / dec as f64;
    let seconds = (seg.len() - m_len) as f64 / sr;
    let mut raw = Vec::new();
    for h in 1..=cfg.max_harmonics {
        let fh = f0 * h as f64;
        if fh > 0.45 * sr || fh > 10_000.0 {
            break;
        }
        let m = demod(seg, fh, sr, m_len, dec);
        if m.len() < 64 {
            return none;
        }
        let e = m.iter().map(|v| v * v).sum::<f64>() / m.len() as f64;
        raw.push((h, e, m));
    }
    let emax = raw.iter().map(|t| t.1).fold(0.0, f64::max);
    if emax <= 0.0 {
        return none;
    }
    let mut tracks = Vec::new();
    for (h, e, m) in raw {
        if e < emax * 1e-3 {
            continue; // 30 dB below the strongest harmonic: too weak to trust
        }
        let floor = m.iter().copied().fold(0.0, f64::max) * 1e-3;
        let logs: Vec<f64> = m.iter().map(|v| libm::log(v.max(floor))).collect();
        let trend = polyfit_eval(&logs, 3);
        let ratio: Vec<f64> = m
            .iter()
            .zip(&trend)
            .map(|(v, t)| v.max(floor) / libm::exp(*t))
            .collect();
        let mean_r = ratio.iter().sum::<f64>() / ratio.len() as f64;
        let n = ratio.len();
        let win = hann(n);
        let win_sum: f64 = win.iter().map(|w| f64::from(*w)).sum();
        let nfft = (n * 8).next_power_of_two();
        let fft = planner.forward(nfft);
        let mut input = vec![0f32; nfft];
        for (o, (r, w)) in input.iter_mut().zip(ratio.iter().zip(&win)) {
            *o = ((r / mean_r - 1.0) * f64::from(*w)) as f32;
        }
        let mut out = vec![Complex32::new(0.0, 0.0); fft.bins()];
        let mut scratch = fft.make_scratch();
        fft.forward(&mut input, &mut out, &mut scratch);
        let spec: Vec<f64> = out
            .iter()
            .map(|c| 2.0 * f64::from(c.norm_sqr()).sqrt() / win_sum)
            .collect();
        tracks.push(Track {
            h,
            spec,
            f_res: rate / nfft as f64,
            weight: e.sqrt(),
        });
    }
    if tracks.is_empty() {
        return none;
    }
    let usable = |fh: f64| fh * seconds >= 2.5 && fh < f0 / 8.0 && fh <= 60.0;
    let mut grid = Vec::new();
    let mut r = cfg.r_min;
    while r <= cfg.r_max {
        grid.push(r);
        r *= 1.002;
    }
    let beating = beat_search(&tracks, &grid, seconds, f0, cfg, &usable);
    let tremolo = if beating.is_some() {
        None
    } else {
        tremolo_search(&tracks, &grid, f0, cfg, &usable)
    };
    Modulation { beating, tremolo }
}

/// Score of rate `r` for detune beating: `(S(r), harmonics used, best single-harmonic wobble)`.
fn beat_score(tracks: &[Track], r: f64, usable: &dyn Fn(f64) -> bool) -> (f64, usize, f64) {
    let (mut s, mut wsum, mut used, mut best) = (0.0, 0.0, 0usize, 0.0f64);
    for t in tracks {
        let fh = t.h as f64 * r;
        if !usable(fh) {
            continue;
        }
        let v = t.at(fh);
        s += t.weight * v;
        wsum += t.weight;
        used += 1;
        best = best.max(v);
    }
    if used == 0 {
        (0.0, 0, 0.0)
    } else {
        (s / wsum, used, best)
    }
}

fn beat_search(
    tracks: &[Track],
    grid: &[f64],
    seconds: f64,
    f0: f64,
    cfg: &BeatCfg,
    usable: &dyn Fn(f64) -> bool,
) -> Option<Beating> {
    let scores: Vec<(f64, usize, f64)> = grid
        .iter()
        .map(|&r| beat_score(tracks, r, usable))
        .collect();
    let (ibest, _) = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.0.total_cmp(&b.1.0))?;
    if ibest == 0 || ibest + 1 == grid.len() {
        return None; // the slope of something slower or faster than the band
    }
    // Guard against picking r/2 or r/3: prefer a multiple that scores nearly as well.
    let (mut rb, mut sb) = (grid[ibest], scores[ibest]);
    for mult in [2.0, 3.0] {
        let r2 = rb * mult;
        if r2 < cfg.r_max {
            let s2 = beat_score(tracks, r2, usable);
            if s2.0 >= 0.85 * sb.0 && s2.1 > 0 {
                rb = r2;
                sb = s2;
            }
        }
    }
    // Refine on a fine linear grid around rb.
    let (mut best_r, mut best_s) = (rb, sb);
    for i in 0..=80 {
        let f = rb * (0.98 + 0.0005 * f64::from(i));
        let s = beat_score(tracks, f, usable);
        if s.0 > best_s.0 {
            best_r = f;
            best_s = s;
        }
    }
    let (rb, sb) = (best_r, best_s);
    // Every used harmonic's own peak near h·r must imply the same r within 3 %; a rate with less than one cycle at
    // the fundamental in the stretch needs two agreeing harmonics.
    let agree = tracks
        .iter()
        .filter(|t| usable(t.h as f64 * rb))
        .filter_map(|t| {
            t.own_peak_near(t.h as f64 * rb)
                .map(|f| f / t.h as f64 / rb)
        })
        .filter(|q| (q - 1.0).abs() <= 0.03)
        .count();
    // Tremolo test: on harmonics >= 2, the reading at r itself must be weaker than at h·r.
    let (mut same, mut scaled, mut ws) = (0.0, 0.0, 0.0);
    for t in tracks.iter().filter(|t| t.h >= 2) {
        let fh = t.h as f64 * rb;
        if fh > f0 / 8.0 || fh > 60.0 {
            continue;
        }
        same += t.weight * t.at(rb);
        scaled += t.weight * t.at(fh);
        ws += t.weight;
    }
    if ws > 0.0 && same >= scaled {
        return None;
    }
    if agree == 0 || (rb * seconds < 1.0 && agree < 2) {
        return None;
    }
    let band: Vec<f64> = scores.iter().map(|s| s.0).filter(|v| *v > 0.0).collect();
    let prominence = sb.0 / median(&band).max(1e-9);
    if sb.2 < cfg.min_depth || prominence < cfg.min_prominence {
        return None;
    }
    Some(Beating {
        beating_hz: rb,
        depth_db: depth_db(sb.2),
        harmonics_used: sb.1,
        prominence,
    })
}

/// Score of rate `r` for a tremolo: weighted mean of every usable harmonic's wobble at `r` itself.
fn trem_score(tracks: &[Track], r: f64, usable: &dyn Fn(f64) -> bool) -> (f64, usize) {
    if !usable(r) {
        return (0.0, 0);
    }
    let (mut s, mut wsum) = (0.0, 0.0);
    for t in tracks {
        s += t.weight * t.at(r);
        wsum += t.weight;
    }
    (s / wsum, tracks.len())
}

fn tremolo_search(
    tracks: &[Track],
    grid: &[f64],
    f0: f64,
    cfg: &BeatCfg,
    usable: &dyn Fn(f64) -> bool,
) -> Option<Tremolo> {
    let scores: Vec<(f64, usize)> = grid
        .iter()
        .map(|&r| trem_score(tracks, r, usable))
        .collect();
    let (ibest, _) = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.0.total_cmp(&b.1.0))?;
    if ibest == 0 || ibest + 1 == grid.len() || scores[ibest].1 == 0 {
        return None;
    }
    let (mut rb, mut sb) = (grid[ibest], scores[ibest]);
    let r0 = rb;
    for i in 0..=80 {
        let f = r0 * (0.98 + 0.0005 * f64::from(i));
        let s = trem_score(tracks, f, usable);
        if s.0 > sb.0 {
            rb = f;
            sb = s;
        }
    }
    let band: Vec<f64> = scores.iter().map(|s| s.0).filter(|v| *v > 0.0).collect();
    let prominence = sb.0 / median(&band).max(1e-9);
    if sb.0 < cfg.tremolo_min_depth || prominence < cfg.min_prominence {
        return None;
    }
    // Every harmonic must peak at the same rate (within 3 %): at least two of them, or the only one.
    let agree = tracks
        .iter()
        .filter_map(|t| t.own_peak_near(rb))
        .filter(|f| (f / rb - 1.0).abs() <= 0.03)
        .count();
    if agree < tracks.len().min(2) {
        return None;
    }
    // Not beating: on harmonics >= 2 the same-rate reading must be stronger than the h-scaled one.
    let (mut same, mut scaled) = (0.0, 0.0);
    for t in tracks.iter().filter(|t| t.h >= 2) {
        let fh = t.h as f64 * rb;
        same += t.weight * t.at(rb);
        scaled += t.weight
            * if fh < f0 / 8.0 && fh <= 60.0 {
                t.at(fh)
            } else {
                0.0
            };
    }
    if same <= scaled {
        return None;
    }
    Some(Tremolo {
        hz: rb,
        depth_db: depth_db(sb.0),
        harmonics_used: sb.1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::FftPath;
    use crate::test_signals::{SR, Wave, detuned_pair, oscillator, theory_beat, tremolo};

    fn run(x: &[f32], f0: f64) -> Modulation {
        let mut p = FftPlanner::new(FftPath::Scalar);
        estimate(&mut p, x, SR, f0, 0, x.len(), &BeatCfg::default())
    }

    #[test]
    fn detuned_saws_beat_at_theory() {
        for (f0, det) in [(220.0, 20.0), (440.0, 10.0), (130.81, 30.0)] {
            let x = detuned_pair(SR, Wave::Saw, f0, det, 1.5, (0.0, 0.37));
            let m = run(&x, f0);
            let b = m
                .beating
                .unwrap_or_else(|| panic!("no beat at {f0} Hz, {det} ct"));
            let th = theory_beat(f0, det);
            assert!(
                (b.beating_hz / th - 1.0).abs() < 0.03,
                "{f0} {det}: {} vs {th}",
                b.beating_hz
            );
            assert!(m.tremolo.is_none());
        }
    }

    #[test]
    fn a_steady_tone_neither_beats_nor_trembles() {
        let x = oscillator(SR, Wave::Saw, 220.0, 1.5, 0.0);
        assert_eq!(run(&x, 220.0), Modulation::default());
    }

    #[test]
    fn tremolo_is_not_beating() {
        let mut x = oscillator(SR, Wave::Saw, 220.0, 1.5, 0.0);
        tremolo(&mut x, SR, 5.0, 0.5);
        let m = run(&x, 220.0);
        assert!(m.beating.is_none(), "{m:?}");
        let t = m.tremolo.expect("a 5 Hz tremolo");
        assert!((t.hz / 5.0 - 1.0).abs() < 0.03, "{t:?}");
    }

    #[test]
    fn short_stretches_report_nothing() {
        let x = detuned_pair(SR, Wave::Saw, 220.0, 20.0, 0.4, (0.0, 0.37));
        assert_eq!(run(&x, 220.0), Modulation::default());
    }
}
