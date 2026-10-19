//! Brightness sweeps (`modulation.filter_sweep`): a filter envelope (one-shot) or a filter LFO (cyclic), read from
//! the spectral centroid over time.
//!
//! A sweep moves the *balance* of the spectrum, so it shows in the centroid, which a level change does not move:
//! a pure tremolo leaves the centroid where it is, and a filter LFO moves it whether or not the level wobbles enough
//! to read as a tremolo. That is the gap this descriptor fills: before it, a filter wobble was visible only as a
//! tremolo, and only when the sweep was deep (`docs/p1a-g1.md`, eval finding 6).
//!
//! The input is the brightness STFT of [`crate::descriptors`] (Hann 2048, hop 512): one centroid and one power per
//! frame. The track is `log2` of the centroid (octaves), over frames within 40 dB of the loudest; quieter frames
//! are bridged by linear interpolation.
//!
//! - **LFO** (cyclic), over the held part (end of the attack to note-off): a cubic trend is fitted and removed, and
//!   the residual's spectrum (Hann, zero-padded ×8, scaled to a sinusoid's amplitude) is searched between
//!   max(0.1 Hz, 2 cycles in the stretch) and min(20 Hz, a quarter of the frame rate). The peak is accepted when its
//!   peak-to-trough swing is at least [`MIN_LFO_OCT`], it stands [`LFO_PROMINENCE`] times above the band's median,
//!   the stretch is at least 0.5 s long, and its rate is not within 3 % (or one cycle over the stretch, the track's
//!   resolution) of a multiple (1–4) of the detune beat rate: beating oscillators swing the centroid too, at the
//!   beat rate and its first multiples. The rate and the swing are then refined by fitting the cubic trend and a
//!   sinusoid together on a fine grid within one bin of the peak (the spectrum alone reads a 0.75 Hz LFO 4 % low
//!   when the stretch holds 2.5 cycles).
//! - **Env** (one-shot), over the audible part of the held note: the start level (mean over the first 5 frames,
//!   about 53 ms, or over one beat period when the oscillators beat) against the end level (median of the last
//!   quarter). A change of at least [`MIN_ENV_OCT`] is a sweep: `dir` is its sign, `depth_oct` its size and `ms` the
//!   time from the onset until 90 % of the change is done (on the same smoothing). A cyclic reading wins over a
//!   one-shot one.

use crate::fft::{FftPlanner, hann};
use crate::linalg::{ipow, lstsq, median, movavg, polyfit_eval};
use realfft::num_complex::Complex32;
use serde::{Deserialize, Serialize};

/// Smallest peak-to-trough brightness swing of a filter LFO, in octaves (about 15 % of the centroid). At 0.1 octave
/// the swing of slow, undetected detune beats passed for an LFO on 3 of the 378 detuned grid cells.
pub const MIN_LFO_OCT: f64 = 0.2;
/// Smallest one-way brightness change of a filter envelope, in octaves (about 41 % of the centroid).
pub const MIN_ENV_OCT: f64 = 0.5;
/// The LFO peak must stand this many times above the median of the band.
pub const LFO_PROMINENCE: f64 = 6.0;
/// Beat multiples (1..=this) that veto an LFO reading at the same rate.
const BEAT_MULTIPLES: u32 = 4;
/// Fewest LFO cycles in the held part.
pub const LFO_MIN_CYCLES: f64 = 2.0;
/// The LFO search band (Hz); the top is also capped at a quarter of the frame rate.
pub const LFO_BAND_HZ: (f64, f64) = (0.1, 20.0);
/// Shortest held part for an LFO reading (s).
const LFO_MIN_SECONDS: f64 = 0.5;
/// Frames this far below the loudest (power) are bridged, 40 dB (the "audible" level used elsewhere).
const FLOOR: f64 = 1e-4;
/// At least this share of the frames in a stretch must be loud enough.
const MIN_VALID: f64 = 0.7;
/// Fewest frames for a one-shot reading (about 85 ms).
const ENV_MIN_FRAMES: usize = 8;
/// Frames in the start mean of a one-shot sweep (about 53 ms).
const ENV_START_FRAMES: usize = 5;

/// One-shot or cyclic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepKind {
    /// A filter envelope: the brightness moves one way over the note.
    Env,
    /// A filter LFO: the brightness swings back and forth.
    Lfo,
}

impl SweepKind {
    /// `env` or `lfo`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SweepKind::Env => "env",
            SweepKind::Lfo => "lfo",
        }
    }
}

/// Which way a one-shot sweep goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepDir {
    /// It gets brighter (a filter opening).
    Up,
    /// It gets darker (a filter closing, as in a pluck).
    Down,
}

impl SweepDir {
    /// `up` or `down`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SweepDir::Up => "up",
            SweepDir::Down => "down",
        }
    }
}

/// A brightness sweep.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterSweep {
    pub kind: SweepKind,
    /// One-shot only: which way.
    pub dir: Option<SweepDir>,
    /// One-shot only: onset until 90 % of the change is done (ms).
    pub ms: Option<f64>,
    /// Cyclic only: the rate (Hz).
    pub hz: Option<f64>,
    /// Octaves: the peak-to-trough swing at the LFO rate, or the size of the one-way change.
    pub depth_oct: f64,
}

/// The brightness STFT: per frame, the centroid (Hz, 0 when the frame is silent) and the power.
#[derive(Clone, Copy, Debug)]
pub struct Frames<'a> {
    pub centroid_hz: &'a [f64],
    pub power: &'a [f64],
    /// Hop and frame length in samples.
    pub hop: usize,
    pub len: usize,
    pub sample_rate: f64,
}

impl Frames<'_> {
    fn centre(&self, i: usize) -> usize {
        i * self.hop + self.len / 2
    }

    fn frame_rate(&self) -> f64 {
        self.sample_rate / self.hop as f64
    }

    /// The `log2` centroid track of the frames centred in `[a, b)` samples, quiet frames bridged; with the index of
    /// its first frame. `None` when fewer than 70 % of them (or fewer than 3) are loud enough.
    fn track(&self, a: usize, b: usize) -> Option<(usize, Vec<f64>)> {
        let n = self.centroid_hz.len().min(self.power.len());
        let loudest = self.power[..n].iter().copied().fold(0.0, f64::max);
        if loudest <= 0.0 {
            return None;
        }
        let first = (0..n).find(|&i| self.centre(i) >= a)?;
        let last = (first..n).take_while(|&i| self.centre(i) < b).last()?;
        let raw: Vec<Option<f64>> = (first..=last)
            .map(|i| {
                (self.power[i] >= FLOOR * loudest && self.centroid_hz[i] > 0.0)
                    .then(|| libm::log2(self.centroid_hz[i]))
            })
            .collect();
        let valid = raw.iter().filter(|v| v.is_some()).count();
        if valid < 3 || (valid as f64) < MIN_VALID * raw.len() as f64 {
            return None;
        }
        Some((first, bridge(&raw)))
    }
}

/// Fill the gaps of a track by linear interpolation (the nearest value at the ends).
fn bridge(raw: &[Option<f64>]) -> Vec<f64> {
    let n = raw.len();
    let mut out = vec![0.0; n];
    let mut i = 0;
    while i < n {
        if let Some(v) = raw[i] {
            out[i] = v;
            i += 1;
            continue;
        }
        let a = i;
        while i < n && raw[i].is_none() {
            i += 1;
        }
        let left = a.checked_sub(1).and_then(|j| raw[j]);
        let right = raw.get(i).copied().flatten();
        for (j, slot) in out.iter_mut().enumerate().take(i).skip(a) {
            *slot = match (left, right) {
                (Some(l), Some(r)) => l + (r - l) * (j - a + 1) as f64 / (i - a + 1) as f64,
                (Some(l), None) => l,
                (None, Some(r)) => r,
                (None, None) => 0.0,
            };
        }
    }
    out
}

/// Detect a brightness sweep. `audible` and `held` are sample ranges: the audible span (onset to the last frame
/// within 40 dB of the peak, cut at note-off when there is one) for a one-shot sweep, the held part (end of the
/// attack to note-off) for an LFO. `beat_hz` is the detune beat rate, when one was found.
pub fn detect(
    planner: &mut FftPlanner,
    frames: &Frames<'_>,
    audible: (usize, usize),
    held: (usize, usize),
    beat_hz: Option<f64>,
) -> Option<FilterSweep> {
    lfo(planner, frames, held, beat_hz).or_else(|| env(frames, audible, beat_hz))
}

fn lfo(
    planner: &mut FftPlanner,
    frames: &Frames<'_>,
    held: (usize, usize),
    beat_hz: Option<f64>,
) -> Option<FilterSweep> {
    let (_, l) = frames.track(held.0, held.1)?;
    let n = l.len();
    let rate = frames.frame_rate();
    let seconds = n as f64 / rate;
    if seconds < LFO_MIN_SECONDS || n < 16 {
        return None;
    }
    let trend = polyfit_eval(&l, 3);
    let win = hann(n);
    let win_sum: f64 = win.iter().map(|w| f64::from(*w)).sum();
    let nfft = (n * 8).next_power_of_two();
    let fft = planner.forward(nfft);
    let mut input = vec![0f32; nfft];
    for (o, ((v, t), w)) in input.iter_mut().zip(l.iter().zip(&trend).zip(&win)) {
        *o = ((v - t) * f64::from(*w)) as f32;
    }
    let mut out = vec![Complex32::new(0.0, 0.0); fft.bins()];
    let mut scratch = fft.make_scratch();
    fft.forward(&mut input, &mut out, &mut scratch);
    let amp: Vec<f64> = out
        .iter()
        .map(|c| 2.0 * f64::from(c.norm_sqr()).sqrt() / win_sum)
        .collect();
    let f_res = rate / nfft as f64;
    let lo = LFO_BAND_HZ.0.max(LFO_MIN_CYCLES / seconds);
    let hi = LFO_BAND_HZ.1.min(rate / 4.0);
    let k_lo = libm::ceil(lo / f_res) as usize;
    let k_hi = (libm::floor(hi / f_res) as usize).min(amp.len() - 1);
    if k_lo + 2 > k_hi {
        return None;
    }
    let band = &amp[k_lo..=k_hi];
    let (j, &peak) = band.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    if j == 0 || j + 1 == band.len() {
        return None; // the slope of something slower or faster than the band
    }
    let prominence = peak / median(band).max(1e-12);
    if prominence < LFO_PROMINENCE {
        return None;
    }
    // The spectrum's peak is biased when the stretch holds only a few cycles (the cubic detrend and the window take
    // part of the swing): refine rate and swing by fitting the trend and a sinusoid together, on a fine grid of
    // rates within one bin of the peak.
    let peak_hz = (k_lo + j) as f64 * f_res;
    let (hz, amp) = (0..=40)
        .map(|i| peak_hz + f_res * (f64::from(i) / 20.0 - 1.0))
        .filter(|f| *f > 0.0)
        .filter_map(|f| sinusoid_fit(&l, rate, f).map(|a| (f, a)))
        .fold(
            (peak_hz, 0.0f64),
            |best, c| if c.1 > best.1 { c } else { best },
        );
    let depth_oct = 2.0 * amp;
    if depth_oct < MIN_LFO_OCT {
        return None;
    }
    // Beating oscillators swing the centroid at the beat rate and its first multiples; two rates closer than the
    // track's resolution (one cycle over the stretch) cannot be told apart.
    if let Some(b) = beat_hz.filter(|b| *b > 0.0)
        && (1..=BEAT_MULTIPLES).any(|k| {
            let kb = b * f64::from(k);
            (hz - kb).abs() <= (0.03 * kb).max(1.0 / seconds)
        })
    {
        return None;
    }
    Some(FilterSweep {
        kind: SweepKind::Lfo,
        dir: None,
        ms: None,
        hz: Some(hz),
        depth_oct,
    })
}

/// Amplitude (octaves) of the sinusoid at `hz` in a joint least-squares fit of a cubic trend and that sinusoid to
/// the track `l` (sampled at `rate` per second).
fn sinusoid_fit(l: &[f64], rate: f64, hz: f64) -> Option<f64> {
    let n = l.len();
    let w = 2.0 * core::f64::consts::PI * hz / rate;
    let rows: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            let x = 2.0 * i as f64 / (n - 1).max(1) as f64 - 1.0;
            let ph = w * i as f64;
            vec![1.0, x, ipow(x, 2), ipow(x, 3), libm::cos(ph), libm::sin(ph)]
        })
        .collect();
    let c = lstsq(&rows, l)?;
    Some((c[4] * c[4] + c[5] * c[5]).sqrt())
}

fn env(frames: &Frames<'_>, audible: (usize, usize), beat_hz: Option<f64>) -> Option<FilterSweep> {
    let (first, l) = frames.track(audible.0, audible.1)?;
    let n = l.len();
    if n < ENV_MIN_FRAMES {
        return None;
    }
    // Beating oscillators swing the centroid at the beat rate: average over one beat period when that fits.
    let smooth = beat_hz
        .filter(|b| *b > 0.0)
        .map(|b| libm::round(frames.frame_rate() / b) as usize)
        .filter(|&w| w > ENV_START_FRAMES && w * 3 <= n)
        .unwrap_or(ENV_START_FRAMES);
    let s = movavg(&l, smooth);
    let start = l[..smooth.min(n)].iter().sum::<f64>() / smooth.min(n) as f64;
    let tail = n - (n / 4).max(2);
    let end = median(&l[tail..]);
    let change = end - start;
    if change.abs() < MIN_ENV_OCT {
        return None;
    }
    let target = start + 0.9 * change;
    let j = s
        .iter()
        .position(|&v| {
            if change > 0.0 {
                v >= target
            } else {
                v <= target
            }
        })
        .unwrap_or(n - 1);
    let ms =
        frames.centre(first + j).saturating_sub(audible.0) as f64 / frames.sample_rate * 1000.0;
    Some(FilterSweep {
        kind: SweepKind::Env,
        dir: Some(if change > 0.0 {
            SweepDir::Up
        } else {
            SweepDir::Down
        }),
        ms: Some(ms),
        hz: None,
        depth_oct: change.abs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::FftPath;

    const SR: f64 = 48_000.0;
    const HOP: usize = 512;
    const LEN: usize = 2048;

    /// Frames of a synthetic centroid track `f(t)` (Hz) at constant power.
    fn frames_of(secs: f64, f: impl Fn(f64) -> f64) -> (Vec<f64>, Vec<f64>) {
        let n = ((secs * SR) as usize - LEN) / HOP + 1;
        let c = (0..n).map(|i| f((i * HOP + LEN / 2) as f64 / SR)).collect();
        (c, vec![1.0; n])
    }

    fn run(c: &[f64], p: &[f64], beat: Option<f64>) -> Option<FilterSweep> {
        let fr = Frames {
            centroid_hz: c,
            power: p,
            hop: HOP,
            len: LEN,
            sample_rate: SR,
        };
        let end = c.len() * HOP + LEN;
        detect(
            &mut FftPlanner::new(FftPath::Scalar),
            &fr,
            (0, end),
            (0, end),
            beat,
        )
    }

    #[test]
    fn a_cyclic_sweep_reads_its_rate_and_swing() {
        // 1 kHz swinging +-0.5 octave at 2 Hz
        let (c, p) = frames_of(4.0, |t| {
            1000.0 * libm::exp2(0.5 * libm::sin(2.0 * core::f64::consts::PI * 2.0 * t))
        });
        let s = run(&c, &p, None).expect("an lfo");
        assert_eq!(s.kind, SweepKind::Lfo);
        assert!((s.hz.unwrap() / 2.0 - 1.0).abs() < 0.02, "{s:?}");
        assert!((s.depth_oct - 1.0).abs() < 0.1, "{s:?}");
        // the same rate as a detune beat is not a filter
        assert!(run(&c, &p, Some(1.0)).is_none_or(|s| s.kind != SweepKind::Lfo));
    }

    #[test]
    fn a_one_shot_sweep_reads_its_direction_and_time() {
        // 4 kHz closing to 1 kHz with a 100 ms time constant. The start is the mean of the first 5 frames (centred
        // 21 to 64 ms), 1.32 octaves up; 90 % of the way from there is 0.13 octave up, at 272 ms.
        let (c, p) = frames_of(1.5, |t| 1000.0 * libm::exp2(2.0 * libm::exp(-t / 0.1)));
        let s = run(&c, &p, None).expect("an env");
        assert_eq!((s.kind, s.dir), (SweepKind::Env, Some(SweepDir::Down)));
        assert!((s.depth_oct - 1.32).abs() < 0.05, "{s:?}");
        let ms = s.ms.unwrap();
        assert!((ms - 272.0).abs() < 25.0, "{s:?}");
        // and the other way
        let (c, p) = frames_of(1.5, |t| {
            1000.0 * libm::exp2(2.0 * (1.0 - libm::exp(-t / 0.1)))
        });
        let s = run(&c, &p, None).expect("an env");
        assert_eq!(s.dir, Some(SweepDir::Up));
    }

    #[test]
    fn a_steady_brightness_is_no_sweep() {
        let (c, p) = frames_of(3.0, |_| 1500.0);
        assert!(run(&c, &p, None).is_none());
        let (c, p) = frames_of(3.0, |t| 1500.0 * (1.0 + 0.01 * libm::sin(13.0 * t)));
        assert!(run(&c, &p, None).is_none());
    }
}
