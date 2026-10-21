//! **Owner: lane FE** (the intake, freeze point F1s).
//!
//! The noise floor of an upload (docs/r3.md, "The intake" and the measured table of report 04 §2): noise at 20 dB SNR
//! took today's search from 99 % to 10 % against the clean target, and trimming the noisy tail alone raised it to 56 %.
//! The floor and the hum (50 or 60 Hz and harmonics) are reported as `Degradation`s, and lane OBJ's robust distance
//! compares only the levels above the floor. Pure and deterministic, on the scalar FFT path.
//!
//! **The floor** is what is left when the instrument is not playing, so it is read from two stretches of the level
//! track (20 ms frames every 10 ms), and the quieter one that passes the checks below wins:
//!
//! - the quietest stretch: the sounding frames within 6 dB of their 10th percentile, at least 100 ms of them, their
//!   power mean; skipped when the file holds 100 ms of digital silence (it is clean, and its quietest sounding
//!   frames are the instrument's own tail);
//! - the lead-in: the frames before the level first rises 6 dB over their running median, which precede the onset by
//!   construction, so a floor is found even when the file ends before the sound has decayed into it.
//!
//! A stretch is a floor only when it is a background and not the instrument:
//!
//! - within 70 dB of the loudest frame (lower is clean for every purpose here) and at least 6 dB below it (otherwise
//!   the clip has no quiet part to call a floor);
//! - its longest run (at least 50 ms) is level, not decaying or rising by more than 20 dB/s (the end of a release
//!   cut off by the file is not a floor); its spread is the median absolute deviation of its frames' levels;
//! - YIN finds no pitch in it other than the mains (a held note's quietest stretch is still the note; hum is a floor).
//!
//! A noise-like sound held through the whole file (a noise pad with no lead-in and no tail) cannot be told from a
//! noise floor this way; it is reported as a floor only when it is at least 6 dB below the loudest frame.
//!
//! **Hum** is looked for in that same run, when it holds at least 1/8 s per Hz of resolution (170 ms at 48 kHz): peaks
//! at least 10 dB above their neighbourhood on two or more of the first eight harmonics of 50 or 60 Hz.

use serde::{Deserialize, Serialize};

use super::trim::Levels;
use super::welch;
use crate::fft::FftPlanner;
use crate::linalg::{line_fit, median, quantile};
use crate::pitch::Yin;

/// The floor stretch: frames within this many dB of the [`QUANTILE`] of the level track.
const BAND_DB: f64 = 6.0;
/// Frames at or below this level (dBFS) are digital silence; [`MIN_FRAMES`] of them make a file clean.
const DIGITAL_SILENCE_DB: f64 = -140.0;
/// The lead-in ends where a frame rises this far over the median of the frames before it, and is searched over at
/// most this many frames (5 s).
const LEAD_RISE_DB: f64 = 6.0;
const MAX_LEAD_FRAMES: usize = 500;
const QUANTILE: f64 = 0.1;
/// At least this many frames (100 ms) in the floor stretch.
const MIN_FRAMES: usize = 10;
/// Its longest run must hold at least this many frames (50 ms).
const MIN_RUN_FRAMES: usize = 5;
/// A floor more than this far below the loudest frame is reported as none (clean).
const CLEAN_BELOW_PEAK_DB: f64 = 70.0;
/// A floor must be at least this far below the loudest frame.
const MIN_BELOW_PEAK_DB: f64 = 6.0;
/// The run may not trend by more than this (dB/s).
const MAX_TREND_DB_PER_S: f64 = 20.0;
/// YIN's frame on the run (s), its lowest frequency (Hz), and the voiced share above which the run is a note.
/// (YIN reads a 50 Hz hum 4–6 % sharp on short frames, hence the long frame and the wide mains tolerance.)
const YIN_FRAME_S: f64 = 0.1;
const YIN_MIN_HZ: f64 = 45.0;
const VOICED_NOTE: f64 = 0.5;
/// Mains frequencies, and how far a harmonic may sit from its nominal frequency (share of it).
const MAINS_HZ: [f64; 2] = [50.0, 60.0];
const MAINS_TOL: f64 = 0.08;
/// A hum harmonic stands this many dB above the median of its neighbourhood (10–30 Hz either side).
const HUM_PEAK_DB: f64 = 10.0;
const HUM_HARMONICS: usize = 8;

/// Mains hum in the floor: 50 or 60 Hz and its harmonics.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hum {
    /// 50 or 60.
    pub mains_hz: f64,
    /// The power of the hum's harmonics together, as an RMS level in dBFS.
    pub level_dbfs: f64,
}

/// The noise floor as the intake uses it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Floor {
    /// Power mean of the floor stretch, RMS dBFS.
    pub level_db: f64,
    /// Standard deviation of the floor frames' levels (dB): small for white noise, larger for brown noise or hum
    /// that wanders. The trim's thresholds sit above the floor by a margin that grows with it.
    pub spread_db: f64,
    /// The longest run of the floor stretches that passed, in samples (where hum is looked for).
    pub run: (usize, usize),
}

/// The noise floor of `x`, or `None` (clean, or no background to read; see the module docs).
pub(crate) fn estimate(
    levels: &Levels,
    x: &[f32],
    sr: f64,
    planner: &mut FftPlanner,
) -> Option<Floor> {
    let db = &levels.db;
    if db.len() < MIN_FRAMES {
        return None;
    }
    let peak = levels.peak_db();
    let mut found: Option<Floor> = None;
    // (1) The quietest stretch, unless the file holds digital silence: then it is clean, and its quietest sounding
    // frames are the instrument's own tail.
    let silent = db.iter().filter(|&&d| d <= DIGITAL_SILENCE_DB).count();
    if silent < MIN_FRAMES {
        let sounding: Vec<f64> = db
            .iter()
            .copied()
            .filter(|&d| d > DIGITAL_SILENCE_DB)
            .collect();
        let q = quantile(&sounding, QUANTILE);
        let member: Vec<bool> = db
            .iter()
            .map(|&d| d > DIGITAL_SILENCE_DB && d <= q + BAND_DB)
            .collect();
        if let Some(run) = longest_run(&member) {
            let picked: Vec<f64> = db
                .iter()
                .zip(&member)
                .filter(|(_, m)| **m)
                .map(|(d, _)| *d)
                .collect();
            found = check(&picked, run, peak, levels, x, sr, planner);
        }
    }
    // (2) The lead-in: the frames before the level first rises 6 dB over their running median. It is before the
    // onset by construction, so it holds a floor even when the file ends before the sound has decayed into it.
    if let Some(end) = lead_in(db)
        && let Some(f) = check(&db[..end], (0, end), peak, levels, x, sr, planner)
    {
        found = Some(match found {
            // The quieter level; the longer run, for the hum.
            Some(g) => Floor {
                run: if f.run.1 - f.run.0 > g.run.1 - g.run.0 {
                    f.run
                } else {
                    g.run
                },
                ..if f.level_db < g.level_db { f } else { g }
            },
            None => f,
        });
    }
    found
}

/// The longest run of `true` in `member` (frame indices `a..b`), when it holds at least [`MIN_RUN_FRAMES`] and the
/// whole set at least [`MIN_FRAMES`].
fn longest_run(member: &[bool]) -> Option<(usize, usize)> {
    if member.iter().filter(|m| **m).count() < MIN_FRAMES {
        return None;
    }
    let (mut best, mut cur) = ((0usize, 0usize), None::<usize>);
    for (i, &is) in member.iter().chain([false].iter()).enumerate() {
        match (is, cur) {
            (true, None) => cur = Some(i),
            (false, Some(s)) => {
                if i - s > best.1 - best.0 {
                    best = (s, i);
                }
                cur = None;
            }
            _ => {}
        }
    }
    (best.1 - best.0 >= MIN_RUN_FRAMES).then_some(best)
}

/// The lead-in's end (a frame index): the first frame more than [`LEAD_RISE_DB`] above the median of the frames
/// before it; `None` when that leaves fewer than [`MIN_RUN_FRAMES`].
fn lead_in(db: &[f64]) -> Option<usize> {
    let mut end = db.len().min(MAX_LEAD_FRAMES);
    for i in 1..end {
        if db[i] > median(&db[..i]) + LEAD_RISE_DB {
            end = i;
            break;
        }
    }
    (end >= MIN_RUN_FRAMES && end < db.len()).then_some(end)
}

/// A floor candidate from the levels of its frames and its longest run (frame indices), if it passes the checks of
/// the module docs.
fn check(
    picked: &[f64],
    run: (usize, usize),
    peak: f64,
    levels: &Levels,
    x: &[f32],
    sr: f64,
    planner: &mut FftPlanner,
) -> Option<Floor> {
    let mean_pow = picked
        .iter()
        .map(|d| libm::pow(10.0, d / 10.0))
        .sum::<f64>()
        / picked.len().max(1) as f64;
    let level_db = 10.0 * libm::log10(mean_pow.max(1e-30));
    if level_db < peak - CLEAN_BELOW_PEAK_DB || level_db > peak - MIN_BELOW_PEAK_DB {
        return None;
    }
    // A robust spread (1.4826 × the median absolute deviation), so a few odd frames do not inflate it.
    let m = median(picked);
    let dev: Vec<f64> = picked.iter().map(|d| (d - m).abs()).collect();
    let spread_db = 1.4826 * median(&dev);
    let hop_s = levels.hop as f64 / sr;
    let (slope, _) = line_fit(&levels.db[run.0..run.1]);
    if (slope / hop_s).abs() > MAX_TREND_DB_PER_S {
        return None;
    }
    let samples = (
        levels.range(run.0, x.len()).0,
        levels.range(run.1 - 1, x.len()).1,
    );
    if is_a_note(&x[samples.0..samples.1], sr, planner) {
        return None;
    }
    Some(Floor {
        level_db,
        spread_db,
        run: samples,
    })
}

/// Whether YIN finds a pitch other than the mains in most of `x`.
fn is_a_note(x: &[f32], sr: f64, planner: &mut FftPlanner) -> bool {
    let frame = (libm::round(sr * YIN_FRAME_S) as usize) & !1;
    if x.len() < frame || frame < 4 {
        return false;
    }
    let mut yin = Yin::new(planner, sr, frame, YIN_MIN_HZ, 4000.0);
    let track = yin.track(x, frame / 2);
    let notes = track
        .iter()
        .filter(|f| f.is_some_and(|hz| !is_mains(hz)))
        .count();
    notes as f64 > VOICED_NOTE * track.len() as f64
}

/// Whether `hz` is a low harmonic (1 to 6) of 50 or 60 Hz.
fn is_mains(hz: f64) -> bool {
    MAINS_HZ.iter().any(|m| {
        (1..=6).any(|k| {
            let f = m * f64::from(k);
            (hz - f).abs() <= MAINS_TOL * f
        })
    })
}

/// Hum in `x[run]` (the floor's run), or `None`.
pub(crate) fn hum(
    x: &[f32],
    sr: f64,
    run: (usize, usize),
    planner: &mut FftPlanner,
) -> Option<Hum> {
    let len = run.1 - run.0;
    let mut n = 1usize;
    while n * 2 <= len.min(1 << 16) {
        n *= 2;
    }
    if (n as f64) < sr / 8.0 {
        return None;
    }
    let (pw, frames) = welch(planner, x, run.0, run.1, n);
    let bin = sr / n as f64;
    let at = |hz: f64| libm::round(hz / bin) as usize;
    let mut best: Option<(usize, f64, f64, f64)> = None; // (count, sum of ratios, power, mains)
    for &m in &MAINS_HZ {
        let (mut count, mut sum, mut power) = (0usize, 0.0f64, 0.0f64);
        for k in 1..=HUM_HARMONICS {
            let f = m * k as f64;
            if f + 40.0 >= sr / 2.0 {
                break;
            }
            let reach = (1.5f64).max(0.005 * f / bin) as usize;
            let c = at(f);
            let (lo, hi) = (c.saturating_sub(reach), (c + reach).min(pw.len() - 1));
            let (pk_i, pk) = (lo..=hi)
                .map(|i| (i, pw[i]))
                .fold((c, 0.0), |a, b| if b.1 > a.1 { b } else { a });
            let mut nb: Vec<f64> = (at(f - 30.0)..=at(f - 10.0))
                .chain(at(f + 10.0)..=at(f + 30.0))
                .filter(|&i| i < pw.len())
                .map(|i| pw[i])
                .collect();
            if nb.is_empty() {
                continue;
            }
            nb.sort_by(f64::total_cmp);
            let med = nb[nb.len() / 2].max(1e-30);
            let ratio = 10.0 * libm::log10(pk.max(1e-30) / med);
            if ratio >= HUM_PEAK_DB {
                count += 1;
                sum += ratio;
                let a = pk_i.saturating_sub(2);
                let b = (pk_i + 2).min(pw.len() - 1);
                power += pw[a..=b].iter().sum::<f64>();
            }
        }
        let better = match best {
            None => true,
            Some((c, s, _, _)) => count > c || (count == c && sum > s),
        };
        if count >= 2 && better {
            best = Some((count, sum, power, m));
        }
    }
    let (_, _, power, mains_hz) = best?;
    // Parseval for a Hann frame: mean square = 2·Σ|X|² / (n² · mean(w²)), mean(w²) = 3/8; averaged over the frames.
    let ms = 2.0 * power / ((n * n) as f64 * 0.375 * frames as f64);
    Some(Hum {
        mains_hz,
        level_dbfs: 10.0 * libm::log10(ms.max(1e-30)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::FftPath;
    use crate::test_signals::{Adsr, SR, Wave, mix, oscillator, sine, white_noise};

    fn floor_of(x: &[f32]) -> Option<Floor> {
        let lv = Levels::standard(x, SR);
        estimate(&lv, x, SR, &mut FftPlanner::new(FftPath::Scalar))
    }

    fn pluck() -> Vec<f32> {
        let mut x = oscillator(SR, Wave::Saw, 220.0, 3.0, 0.0);
        Adsr {
            attack_s: 0.005,
            decay_s: 0.6,
            sustain: 0.0,
            release_s: 0.3,
        }
        .apply(&mut x, SR, 1.0);
        x
    }

    #[test]
    fn a_clean_decay_has_no_floor_and_noise_has_one() {
        let x = pluck();
        assert!(floor_of(&x).is_none());
        for (amp, want_db) in [(0.01, -44.8), (0.001, -64.8)] {
            let n = white_noise(SR, amp, 3.0, 9);
            let y = mix(&x, &n);
            let f = floor_of(&y).unwrap();
            // uniform noise in ±a has RMS a/√3
            assert!((f.level_db - want_db).abs() < 1.0, "{amp}: {f:?}");
            assert!(f.spread_db < 1.0, "{f:?}");
        }
    }

    #[test]
    fn a_held_note_is_not_a_floor() {
        // a note decaying to a held sustain, cut off by the file: its quietest stretch is the note
        let mut x = oscillator(SR, Wave::Saw, 110.0, 3.0, 0.0);
        Adsr {
            attack_s: 0.005,
            decay_s: 0.4,
            sustain: 0.1,
            release_s: 0.3,
        }
        .apply(&mut x, SR, 10.0);
        assert!(floor_of(&x).is_none());
        // and a release cut off by the end of the file is a decay, not a floor
        let mut y = oscillator(SR, Wave::Saw, 110.0, 2.0, 0.0);
        Adsr {
            attack_s: 0.005,
            decay_s: 0.1,
            sustain: 1.0,
            release_s: 4.0,
        }
        .apply(&mut y, SR, 0.3);
        let n = white_noise(SR, 1e-5, 2.0, 3);
        assert!(floor_of(&mix(&y, &n)).is_none());
    }

    #[test]
    fn hum_is_found_and_named() {
        let x = pluck();
        for mains in [50.0, 60.0] {
            let mut h = sine(SR, mains, 0.004, 3.0);
            let h3 = sine(SR, 3.0 * mains, 0.002, 3.0);
            h = mix(&h, &h3);
            let n = white_noise(SR, 0.002, 3.0, 5);
            let y = mix(&mix(&x, &h), &n);
            let mut pl = FftPlanner::new(FftPath::Scalar);
            let f = estimate(&Levels::standard(&y, SR), &y, SR, &mut pl).expect("a floor");
            let got = hum(&y, SR, f.run, &mut pl).expect("hum");
            assert_eq!(got.mains_hz, mains);
            // 0.004 and 0.002 peak: RMS² = (0.004² + 0.002²) / 2 → −50.0 dBFS
            assert!((got.level_dbfs + 50.0).abs() < 1.5, "{got:?}");
        }
        // a floor of white noise alone has no hum
        let y = mix(&x, &white_noise(SR, 0.002, 3.0, 5));
        let mut pl = FftPlanner::new(FftPath::Scalar);
        let f = estimate(&Levels::standard(&y, SR), &y, SR, &mut pl).unwrap();
        assert!(hum(&y, SR, f.run, &mut pl).is_none());
    }
}
