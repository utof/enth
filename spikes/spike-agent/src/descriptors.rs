//! Perceptual descriptors, measured from audio samples (never read from the patch).
//!
//! These numbers are the oracle of the agent loop, so each is a plain, documented measurement:
//!
//! | descriptor | how it is measured |
//! | --- | --- |
//! | envelope | RMS over 8 ms windows every 1 ms (`rms_envelope`); `peak` is its maximum |
//! | `attack_ms` | first time the envelope is within 1 dB of its peak, minus the first time it is within 40 dB of it |
//! | `sustain_db` | median envelope over the 70 ms before note-off, in dB re the peak |
//! | `decay_ms` | peak to the time the envelope has fallen 90 % of the way (in amplitude) to the sustain level; 0 if sustain is within 1 dB of the peak |
//! | `release_ms` | note-off to the time the envelope is 40 dB below its note-off level; 0 if it was already 40 dB below the peak |
//! | `duration_ms` | onset (40 dB below peak) to the last time the envelope is within 40 dB of the peak |
//! | `centroid_hz` | magnitude-weighted mean frequency of a 2048-point Hann STFT, over frames within 50 dB of the loudest |
//! | `f0_hz` | median YIN estimate over frames within 40 dB of the peak |
//! | `noise_ratio` | share of 40 Hz to 16 kHz power (8192-point Welch, held part of the note) outside spectral peaks |
//! | `beating_hz` | dominant rate (1 to 25 Hz, at least 2.5 cycles in the stretch) of the 9.5 dB deep or deeper amplitude wobble of the fundamental, found by quadrature demodulation at `f0` |
//! | `lufs_i` | EBU R128 integrated loudness (`ebur128`, via `spike_metric`) |
//!
//! Limits are in `docs/spikes/agent-probe.md`. Pure functions of the samples: no I/O, clocks or RNG.

use crate::units::nearest_note;
use realfft::RealFftPlanner;
use spike_metric::features::{AnalysisCfg, Analyzer};
use spike_metric::yin::Yin;

const ENV_HOP_MS: f64 = 1.0;
const ENV_WIN_MS: f64 = 8.0;
const STFT_N: usize = 2048;
const STFT_HOP: usize = 512;
const WELCH_N: usize = 8192;
const YIN_FRAME: usize = 2048;
const YIN_HOP: usize = 512;
/// Envelope level that counts as "audible" relative to the peak (amplitude), 40 dB.
const AUDIBLE: f64 = 0.01;
/// Envelope level that counts as "at the peak" (amplitude), -1 dB.
const AT_PEAK: f64 = 0.891_25;
/// Frames this far below the loudest are ignored when averaging the spectrum (power), 50 dB.
const SPECTRUM_FLOOR: f64 = 1e-5;
/// Frames this far below the peak are not used for the beating search (amplitude), 30 dB.
const BEAT_FLOOR: f64 = 0.031_62;
/// Silence gate: peak below -80 dBFS.
const SILENT_PEAK: f32 = 1e-4;
/// Smallest modulation of the fundamental that counts as beating (relative amplitude of the wobble), 9.5 dB deep.
/// Two equal oscillators beat 13 to 16 dB deep; envelope shapes and slow beats the detrend only partly removes stay
/// below this (the audit in docs/spikes/agent-probe.md shows both).
const BEAT_MIN_DEPTH: f64 = 0.5;
const BEAT_MIN_SECONDS: f64 = 0.5;
/// How many times the median of the wobble spectrum the strongest line must be.
const BEAT_PROMINENCE: f64 = 6.0;

/// Which groups of descriptors to compute (the envelope is always computed). Saves time in the hint search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want(u8);

impl Want {
    pub const NONE: Want = Want(0);
    pub const PITCH: Want = Want(1);
    pub const SPECTRUM: Want = Want(2);
    pub const NOISE: Want = Want(4);
    /// Beating needs the pitch, so it includes it.
    pub const BEATING: Want = Want(8 | 1);
    pub const LOUDNESS: Want = Want(16);
    pub const ALL: Want = Want(31);

    #[must_use]
    pub fn has(self, other: Want) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub fn or(self, other: Want) -> Want {
        Want(self.0 | other.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a plain record of measurements and their truncation flags"
)]
pub struct Descriptors {
    pub dur_s: f64,
    pub silent: bool,
    pub peak_dbfs: f64,
    pub clipping: bool,
    pub lufs_i: Option<f64>,
    pub f0_hz: Option<f64>,
    pub note: Option<String>,
    pub cents_dev: Option<f64>,
    pub stability: Option<f64>,
    pub attack_ms: Option<f64>,
    pub decay_ms: Option<f64>,
    pub sustain_db: Option<f64>,
    pub release_ms: Option<f64>,
    /// The clip ended before the tail did: `release_ms` is a lower bound.
    pub release_truncated: bool,
    pub duration_ms: Option<f64>,
    /// The sound is still audible at the end of the clip: `duration_ms` is a lower bound.
    pub duration_truncated: bool,
    pub centroid_hz: Option<f64>,
    pub centroid_track_hz: Option<[f64; 3]>,
    pub noise_ratio: Option<f64>,
    pub beating_hz: Option<f64>,
    pub beat_depth_db: Option<f64>,
}

fn db20(x: f64) -> f64 {
    20.0 * x.max(1e-6).log10()
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// RMS envelope: window of 8 ms centred on every 1 ms; returns the envelope and the hop in samples.
fn rms_envelope(x: &[f32], sr: f64) -> (Vec<f64>, usize) {
    let hop = ((sr * ENV_HOP_MS / 1000.0).round() as usize).max(1);
    let win = (sr * ENV_WIN_MS / 1000.0).round() as usize;
    let mut pre = Vec::with_capacity(x.len() + 1);
    let mut acc = 0.0f64;
    pre.push(0.0);
    for &s in x {
        acc += f64::from(s) * f64::from(s);
        pre.push(acc);
    }
    let env = (0..x.len() / hop)
        .map(|i| {
            let c = i * hop;
            let a = c.saturating_sub(win / 2);
            let b = (c + win / 2).min(x.len());
            ((pre[b] - pre[a]) / (b - a).max(1) as f64).max(0.0).sqrt()
        })
        .collect();
    (env, hop)
}

/// Least-squares polynomial of degree `deg` through `y` (sampled at equal steps), evaluated at the same points.
/// Works on x in -1..1 so the normal equations stay well conditioned.
fn polyfit(y: &[f64], deg: usize) -> Vec<f64> {
    let n = y.len();
    let xs: Vec<f64> = (0..n)
        .map(|i| 2.0 * i as f64 / (n - 1).max(1) as f64 - 1.0)
        .collect();
    let k = deg + 1;
    let mut a = vec![vec![0.0f64; k + 1]; k];
    for (x, v) in xs.iter().zip(y) {
        let pows: Vec<f64> = (0..k).map(|p| x.powi(p as i32)).collect();
        for r in 0..k {
            for c in 0..k {
                a[r][c] += pows[r] * pows[c];
            }
            a[r][k] += pows[r] * v;
        }
    }
    // Gaussian elimination with partial pivoting
    for col in 0..k {
        let pivot = (col..k)
            .max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))
            .unwrap_or(col);
        a.swap(col, pivot);
        let d = a[col][col];
        if d.abs() < 1e-12 {
            continue;
        }
        for r in 0..k {
            if r != col {
                let f = a[r][col] / d;
                for c in col..=k {
                    a[r][c] -= f * a[col][c];
                }
            }
        }
    }
    let coef: Vec<f64> = (0..k)
        .map(|i| {
            if a[i][i].abs() < 1e-12 {
                0.0
            } else {
                a[i][k] / a[i][i]
            }
        })
        .collect();
    xs.iter()
        .map(|x| {
            coef.iter()
                .enumerate()
                .map(|(p, c)| c * x.powi(p as i32))
                .sum()
        })
        .collect()
}

fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * ((2.0 * std::f64::consts::PI * i as f64 / n as f64).cos() as f32))
        .collect()
}

pub struct Describer {
    sr: f32,
    analyzer: Analyzer,
    yin: Yin,
    planner: RealFftPlanner<f32>,
    win_stft: Vec<f32>,
    win_welch: Vec<f32>,
}

impl Describer {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            analyzer: Analyzer::new(AnalysisCfg {
                sample_rate,
                ..AnalysisCfg::default()
            }),
            yin: Yin::new(sample_rate, YIN_FRAME, 50.0, 2000.0),
            planner: RealFftPlanner::new(),
            win_stft: hann(STFT_N),
            win_welch: hann(WELCH_N),
        }
    }

    /// Measure `x`. `hold_s` is when the note ended (known for patches; for a bare WAV pass it explicitly or the
    /// descriptors that need it stay `None`).
    pub fn describe(&mut self, x: &[f32], hold_s: Option<f64>, want: Want) -> Descriptors {
        let sr = f64::from(self.sr);
        let mut d = Descriptors {
            dur_s: x.len() as f64 / sr,
            ..Descriptors::default()
        };
        let peak_sample = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        d.peak_dbfs = db20(f64::from(peak_sample));
        d.clipping = peak_sample >= 0.9999;
        let (env, hop) = rms_envelope(x, sr);
        if peak_sample < SILENT_PEAK || env.len() < 20 {
            d.silent = true;
            return d;
        }
        if want.has(Want::LOUDNESS) {
            d.lufs_i = self.analyzer.loudness(x);
        }
        let hop_ms = hop as f64 / sr * 1000.0;
        let peak = env.iter().copied().fold(0.0, f64::max);
        let t_peak = env.iter().position(|&e| e >= peak).unwrap_or(0);
        let onset = env.iter().position(|&e| e >= AUDIBLE * peak).unwrap_or(0);
        let t_top = onset
            + env[onset..]
                .iter()
                .position(|&e| e >= AT_PEAK * peak)
                .unwrap_or(0);
        d.attack_ms = Some((t_top - onset) as f64 * hop_ms);
        let last = env
            .iter()
            .rposition(|&e| e >= AUDIBLE * peak)
            .unwrap_or(onset);
        d.duration_ms = Some((last + 1 - onset) as f64 * hop_ms);
        d.duration_truncated = last + 2 >= env.len();

        let off = hold_s.map(|h| ((h * 1000.0 / hop_ms).round() as usize).min(env.len() - 1));
        if let Some(off) = off {
            Self::note_off_envelope(&mut d, &env, (peak, t_peak, t_top), off, hop_ms);
        }

        // The "held" part of the note: from the end of the attack to note-off (or to the end of the sound).
        let held_start = t_top * hop;
        let held_end = off.map_or((last + 1) * hop, |o| o * hop).min(x.len());

        if want.has(Want::PITCH) {
            self.pitch(&mut d, x, &env, peak, hop);
        }
        if want.has(Want::SPECTRUM) {
            let span = (onset * hop, ((last + 1) * hop).min(x.len()));
            self.spectrum(&mut d, x, span);
        }
        if want.has(Want::NOISE) {
            d.noise_ratio = self.noise_ratio(x, (held_start, held_end));
        }
        if want.has(Want::BEATING)
            && let Some(f0) = d.f0_hz
        {
            // End of the stretch within 30 dB of the peak; a 400 ms gap (not a single beat null) ends it.
            let floor = BEAT_FLOOR * peak;
            let mut quiet = t_top;
            for (i, &e) in env.iter().enumerate().skip(t_top) {
                if e >= floor {
                    quiet = i + 1;
                } else if i - quiet > 400 {
                    break;
                }
            }
            let end = held_end.min(quiet * hop);
            if let Some((hz, depth)) = self.beating(x, f0, held_start, end) {
                d.beating_hz = Some(hz);
                d.beat_depth_db = Some(depth);
            }
        }
        d
    }

    /// Sustain, decay and release: the descriptors that need to know when the note ended.
    fn note_off_envelope(
        d: &mut Descriptors,
        env: &[f64],
        (peak, t_peak, t_top): (f64, usize, usize),
        off: usize,
        hop_ms: f64,
    ) {
        let lo = off.saturating_sub(80);
        let hi = off.saturating_sub(10);
        if lo > t_top && hi > lo {
            let mut w = env[lo..hi].to_vec();
            let level = median(&mut w);
            d.sustain_db = Some(db20(level / peak).max(-80.0));
            if level >= AT_PEAK * peak {
                d.decay_ms = Some(0.0);
            } else {
                let settle = level + 0.1 * (peak - level);
                let j = env[t_peak.min(off)..off]
                    .iter()
                    .position(|&e| e <= settle)
                    .map_or(off, |p| t_peak.min(off) + p);
                d.decay_ms = Some((j - t_peak.min(off)) as f64 * hop_ms);
            }
        }
        let a = off.saturating_sub(20);
        let b = off.saturating_sub(5).max(a + 1);
        let mut w = env[a..b.min(env.len())].to_vec();
        let at_off = median(&mut w);
        if at_off < AUDIBLE * peak {
            d.release_ms = Some(0.0);
        } else {
            let target = at_off * AUDIBLE;
            if let Some(p) = env[off..].iter().position(|&e| e <= target) {
                d.release_ms = Some(p as f64 * hop_ms);
            } else {
                d.release_ms = Some((env.len() - off) as f64 * hop_ms);
                d.release_truncated = true;
            }
        }
    }

    fn pitch(&mut self, d: &mut Descriptors, x: &[f32], env: &[f64], peak: f64, hop: usize) {
        if x.len() < YIN_FRAME {
            return;
        }
        let track = self.yin.track(x, YIN_HOP);
        let mut voiced: Vec<f64> = track
            .iter()
            .enumerate()
            .filter(|&(i, &f)| {
                let centre = (i * YIN_HOP + YIN_HOP) / hop;
                f > 0.0 && env.get(centre).is_some_and(|&e| e >= AUDIBLE * peak)
            })
            .map(|(_, &f)| f64::from(f))
            .collect();
        if voiced.len() < 2 {
            return;
        }
        let f0 = median(&mut voiced);
        let cents: Vec<f64> = voiced.iter().map(|f| 1200.0 * (f / f0).log2()).collect();
        let mean = cents.iter().sum::<f64>() / cents.len() as f64;
        let std =
            (cents.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / cents.len() as f64).sqrt();
        let (name, dev) = nearest_note(f0);
        d.f0_hz = Some(f0);
        d.note = Some(name);
        d.cents_dev = Some(dev);
        d.stability = Some((1.0 - std / 50.0).clamp(0.0, 1.0));
    }

    fn spectrum(&mut self, d: &mut Descriptors, x: &[f32], span: (usize, usize)) {
        if x.len() < STFT_N {
            return;
        }
        let fft = self.planner.plan_fft_forward(STFT_N);
        let mut input = fft.make_input_vec();
        let mut out = fft.make_output_vec();
        let mut scratch = fft.make_scratch_vec();
        let frames = (x.len() - STFT_N) / STFT_HOP + 1;
        // per frame: sum of magnitudes, sum of bin * magnitude, power
        let mut stats = Vec::with_capacity(frames);
        for f in 0..frames {
            let seg = &x[f * STFT_HOP..f * STFT_HOP + STFT_N];
            for ((o, s), w) in input.iter_mut().zip(seg).zip(&self.win_stft) {
                *o = s * w;
            }
            fft.process_with_scratch(&mut input, &mut out, &mut scratch)
                .expect("fft sizes match the plan");
            let (mut sum, mut moment, mut power) = (0.0f64, 0.0f64, 0.0f64);
            for (k, c) in out.iter().enumerate().skip(1) {
                let m = f64::from(c.norm());
                sum += m;
                moment += k as f64 * m;
                power += m * m;
            }
            stats.push((sum, moment, power));
        }
        let loudest = stats.iter().map(|s| s.2).fold(0.0, f64::max);
        if loudest <= 0.0 {
            return;
        }
        let bin_hz = f64::from(self.sr) / STFT_N as f64;
        let centroid = |lo: usize, hi: usize| -> Option<f64> {
            let (mut sum, mut moment) = (0.0, 0.0);
            for (f, s) in stats.iter().enumerate() {
                let centre = f * STFT_HOP + STFT_N / 2;
                if centre >= lo && centre < hi && s.2 >= SPECTRUM_FLOOR * loudest {
                    sum += s.0;
                    moment += s.1;
                }
            }
            (sum > 0.0).then(|| moment / sum * bin_hz)
        };
        d.centroid_hz = centroid(0, usize::MAX);
        let third = (span.1.saturating_sub(span.0)) / 3;
        if third > STFT_HOP {
            let parts = [
                centroid(span.0, span.0 + third),
                centroid(span.0 + third, span.0 + 2 * third),
                centroid(span.0 + 2 * third, span.1),
            ];
            if let [Some(a), Some(b), Some(c)] = parts {
                d.centroid_track_hz = Some([a, b, c]);
            }
        }
    }

    /// Share of power outside spectral peaks. A peak is a local maximum 15 dB above the median of its ±64 bins;
    /// the peak and 3 bins either side count as tonal.
    fn noise_ratio(&mut self, x: &[f32], seg: (usize, usize)) -> Option<f64> {
        let end = seg.1.min(x.len());
        if end < WELCH_N {
            return None;
        }
        let start = seg.0.min(end - WELCH_N);
        let fft = self.planner.plan_fft_forward(WELCH_N);
        let mut input = fft.make_input_vec();
        let mut out = fft.make_output_vec();
        let mut scratch = fft.make_scratch_vec();
        let nb = WELCH_N / 2 + 1;
        let mut pw = vec![0.0f64; nb];
        let mut pos = start;
        while pos + WELCH_N <= end {
            for ((o, s), w) in input
                .iter_mut()
                .zip(&x[pos..pos + WELCH_N])
                .zip(&self.win_welch)
            {
                *o = s * w;
            }
            fft.process_with_scratch(&mut input, &mut out, &mut scratch)
                .ok()?;
            for (p, c) in pw.iter_mut().zip(&out) {
                *p += f64::from(c.norm_sqr());
            }
            pos += WELCH_N / 2;
        }
        let bin_hz = f64::from(self.sr) / WELCH_N as f64;
        let k_lo = ((40.0 / bin_hz).ceil() as usize).max(4);
        let k_hi = ((16_000.0 / bin_hz) as usize).min(nb - 5);
        let total: f64 = pw[k_lo..=k_hi].iter().sum();
        if total <= 0.0 {
            return None;
        }
        let pmax = pw[k_lo..=k_hi].iter().copied().fold(0.0, f64::max);
        let mut tonal = vec![false; nb];
        let mut scratch_med: Vec<f64> = Vec::with_capacity(129);
        for k in k_lo..=k_hi {
            if pw[k] > pw[k - 1] && pw[k] >= pw[k + 1] && pw[k] > 1e-9 * pmax {
                scratch_med.clear();
                scratch_med.extend_from_slice(&pw[k.saturating_sub(64)..=(k + 64).min(nb - 1)]);
                if pw[k] >= 31.6 * median(&mut scratch_med) {
                    for t in &mut tonal[k - 3..=k + 3] {
                        *t = true;
                    }
                }
            }
        }
        let noise: f64 = (k_lo..=k_hi).filter(|&k| !tonal[k]).map(|k| pw[k]).sum();
        Some((noise / total).clamp(0.0, 1.0))
    }

    /// Rate and depth of the slow amplitude wobble of the fundamental.
    ///
    /// Mix the held part down by `f0`, average over four periods (which removes the other harmonics), take the
    /// magnitude at 500 Hz, divide out the overall fade (a log-linear fit), and find the strongest component
    /// between 1 and 25 Hz. `None` when the stretch is shorter than 0.5 s or the wobble is under 9.5 dB deep.
    fn beating(&mut self, x: &[f32], f0: f64, start: usize, end: usize) -> Option<(f64, f64)> {
        let sr = f64::from(self.sr);
        let end = end.min(x.len());
        if end.saturating_sub(start) < (BEAT_MIN_SECONDS * sr) as usize {
            return None;
        }
        let seg = &x[start..end];
        let w = 2.0 * std::f64::consts::PI * f0 / sr;
        let mut pre_re = Vec::with_capacity(seg.len() + 1);
        let mut pre_im = Vec::with_capacity(seg.len() + 1);
        let (mut re, mut im) = (0.0f64, 0.0f64);
        pre_re.push(0.0);
        pre_im.push(0.0);
        for (n, &s) in seg.iter().enumerate() {
            let (sn, cs) = (w * n as f64).sin_cos();
            re += f64::from(s) * cs;
            im -= f64::from(s) * sn;
            pre_re.push(re);
            pre_im.push(im);
        }
        let m_len = ((4.0 * sr / f0).round() as usize).max(8);
        let dec = ((sr / 500.0).round() as usize).max(1);
        let mut m = Vec::new();
        let mut n0 = 0;
        while n0 + m_len <= seg.len() {
            let (a, b) = (
                pre_re[n0 + m_len] - pre_re[n0],
                pre_im[n0 + m_len] - pre_im[n0],
            );
            m.push(a.hypot(b) / m_len as f64);
            n0 += dec;
        }
        if m.len() < 64 {
            return None;
        }
        let rate = sr / dec as f64;
        let floor = m.iter().copied().fold(0.0, f64::max) * 1e-3;
        if floor <= 0.0 {
            return None;
        }
        // Divide out the overall fade: a cubic fit of the log magnitude (an exponential decay is a straight line
        // there; a decay into a sustain level is gently curved), so only faster wobble remains.
        let logs: Vec<f64> = m.iter().map(|v| v.max(floor).ln()).collect();
        let trend = polyfit(&logs, 3);
        let ratio: Vec<f64> = m
            .iter()
            .zip(&trend)
            .map(|(v, t)| v.max(floor) / t.exp())
            .collect();
        let mean_r = ratio.iter().sum::<f64>() / ratio.len() as f64;
        let win = hann(ratio.len());
        let win_sum: f64 = win.iter().map(|&w| f64::from(w)).sum();
        let nfft = (ratio.len() * 8).next_power_of_two();
        let fft = self.planner.plan_fft_forward(nfft);
        let mut input = fft.make_input_vec();
        for (o, (r, w)) in input.iter_mut().zip(ratio.iter().zip(&win)) {
            *o = ((r / mean_r - 1.0) as f32) * w;
        }
        let mut out = fft.make_output_vec();
        fft.process(&mut input, &mut out).ok()?;
        let mag: Vec<f64> = out.iter().map(|c| f64::from(c.norm())).collect();
        let f_res = rate / nfft as f64;
        let seconds = ratio.len() as f64 / rate;
        let k_lo = ((1.0f64.max(2.5 / seconds) / f_res).ceil() as usize).max(1);
        let k_hi = ((25.0 / f_res) as usize).min(mag.len() - 2);
        if k_lo >= k_hi {
            return None;
        }
        let k_pk = (k_lo..=k_hi).max_by(|&a, &b| mag[a].total_cmp(&mag[b]))?;
        // A peak on the edge of the band is the slope of something slower (or faster) than can be measured here.
        if k_pk == k_lo || k_pk == k_hi {
            return None;
        }
        let (l, c, r) = (
            mag[k_pk - 1].max(1e-12).ln(),
            mag[k_pk].max(1e-12).ln(),
            mag[k_pk + 1].max(1e-12).ln(),
        );
        let denom = l - 2.0 * c + r;
        let delta = if denom.abs() > 1e-12 {
            (0.5 * (l - r) / denom).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let amp = 2.0 * mag[k_pk] / win_sum;
        // A real beat is a clear line in the wobble's spectrum, not a bump above its floor.
        let mut band: Vec<f64> = mag[k_lo..=k_hi].to_vec();
        if amp < BEAT_MIN_DEPTH || mag[k_pk] < BEAT_PROMINENCE * median(&mut band) {
            return None;
        }
        let a = amp.min(0.95);
        Some((
            (k_pk as f64 + delta) * f_res,
            20.0 * ((1.0 + a) / (1.0 - a)).log10(),
        ))
    }
}
