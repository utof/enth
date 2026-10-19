//! The beat-corrected amplitude envelope and the descriptors read from it: attack, decay, sustain, release,
//! duration and residual ripple.
//!
//! Detuned oscillators make the plain RMS envelope beat, and a raw envelope then misreads every stage: the first
//! beat peak sets the "peak", so attack reads about 1/(4 × beat rate) whatever the attack is, beat nulls look like
//! decay, and sustain moves by several dB with where note-off falls in the beat cycle (`docs/spikes/agent-probe.md`).
//! Smoothing over a beat period (boxcar or crest) was measured and rejected: it smears transients by a whole period.
//! Instead (report 03 §4.1):
//!
//! 1. `E` = RMS over 8 ms every 1 ms. The fit region runs from the first time `E ≥ 0.3·max E` (+20 ms) to note-off
//!    − 15 ms (or the end).
//! 2. With the beat rate r measured first ([`crate::beat`]), fit by linear least squares over that region (every
//!    2nd ms, ignoring points 30 dB below the peak):
//!    `ln E(t) ≈ Σ_{p≤deg} c_p x^p + Σ_{k≤K} (a_k cos 2πkrt + b_k sin 2πkrt)`, x in −1..1 over the region;
//!    deg = 3 with ≥ 3 periods, 2 with ≥ 1.5, else 1; K = 6 with ≥ 2 periods, else 2. Fewer than 1 period: no
//!    correction, and `beat_corrected` is false.
//! 3. Divide the periodic part `P(t)` out of the whole envelope (it is phase-locked to note-on, so it extends to the
//!    attack and release), at most 20 dB; where `P < −0.7` (a null deeper than 6 dB) drop the point and bridge the
//!    gap by linear interpolation of the log level.
//! 4. Read the stages on the corrected curve `C` (definitions on [`Envelope`]).

use crate::linalg::{ipow, line_fit, lstsq, median, movavg, quantile};
use crate::loudness::db20;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// Envelope hop (ms).
pub const HOP_MS: f64 = 1.0;
/// Envelope RMS window (ms).
pub const WIN_MS: f64 = 8.0;
/// "Audible": 40 dB below the peak (amplitude ratio).
const AUDIBLE: f64 = 0.01;
/// "At the peak": within 1 dB of it.
const AT_PEAK: f64 = 0.891_25;
/// The peak is the maximum of a moving average over this many ms (robust to one-sample spikes).
const PEAK_WINDOW_MS: usize = 5;
/// Shortest sustain window (ms).
const SUSTAIN_MIN_MS: usize = 70;
/// Fewest beat periods in the fit region for a correction.
pub const MIN_BEAT_PERIODS: f64 = 1.0;

/// Envelope descriptors. Times in ms, levels in dB relative to the peak.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// From the onset (first time within 40 dB of the peak) to the first time within 1 dB of the peak.
    pub attack_ms: f64,
    /// From the peak to the first time the level drops below `sustain + max(1.5 dB, 20 % of |sustain|)` (sustain
    /// floored at −40 dB) and stays below for max(30 ms, half a beat period); 0 when the sustain is within 1 dB of
    /// the peak. A threshold time in dB, not the ADSR parameter. `None` without a note-off.
    pub decay_ms: Option<f64>,
    /// A straight line through the dB level over max(70 ms, one beat period when ≥ 2 periods) ending 10 ms before
    /// note-off, read at its end; dB re the peak, floored at −80. `None` without a note-off, or when the note is
    /// released before the attack ends.
    pub sustain_db: Option<f64>,
    /// From note-off to the last time the level is above −40 dB re its level just before note-off; 0 when the sound
    /// had already gone. `None` without a note-off inside the clip.
    pub release_ms: Option<f64>,
    /// The clip ended before the release did: `release_ms` is extrapolated from a line in dB over the tail.
    pub release_extrapolated: bool,
    /// From the onset to the last time within 40 dB of the peak.
    pub duration_ms: f64,
    /// The sound is still audible at the end of the clip: `duration_ms` is a lower bound.
    pub duration_truncated: bool,
    /// Residual wobble of the corrected envelope over the held part: p95 − p5 of the level against its 200 ms
    /// moving average (dB). A measurement-noise proxy.
    pub ripple_db: f64,
    /// False when a beat was found but could not be removed (less than one period in the held part), so sustain
    /// may be off by a few dB. True otherwise, including when there is no beat.
    pub beat_corrected: bool,
    /// Beat periods in the fit region, when a beat rate was given.
    pub beat_periods: Option<f64>,
}

/// RMS envelope: an 8 ms window centred on every 1 ms hop. Returns the envelope and the hop in samples.
#[must_use]
pub fn rms_envelope(x: &[f32], sr: f64) -> (Vec<f64>, usize) {
    let hop = (libm::round(sr * HOP_MS / 1000.0) as usize).max(1);
    let win = libm::round(sr * WIN_MS / 1000.0) as usize;
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

/// Measure the envelope descriptors from a raw RMS envelope `raw` (one value per `hop_ms`). `on` and `off` are the
/// note-on and note-off positions in envelope frames; `beat_hz` the beat rate if one was found.
///
/// # Panics
/// If `raw` is empty.
#[must_use]
pub fn measure(
    raw: &[f64],
    hop_ms: f64,
    on: usize,
    off: Option<usize>,
    beat_hz: Option<f64>,
) -> Envelope {
    assert!(!raw.is_empty(), "empty envelope");
    let ms = |frames: f64| frames * hop_ms;
    let frames_per_ms = 1.0 / hop_ms;
    let corrected = beat_hz.and_then(|b| fit(raw, hop_ms, off, b).map(|c| (b, c)));
    let periods = beat_hz.map(|b| fit_periods(raw, hop_ms, off, b));
    let (curve, sustain_win) = match &corrected {
        Some((b, (c, periods))) => {
            let p = libm::round(1000.0 / b * frames_per_ms) as usize;
            let win = if *periods >= 2.0 {
                p
            } else {
                (SUSTAIN_MIN_MS as f64 * frames_per_ms) as usize
            };
            (c.as_slice(), win)
        }
        None => (raw, (SUSTAIN_MIN_MS as f64 * frames_per_ms) as usize),
    };
    let mut env = read_stages(curve, ms, frames_per_ms, on, off, sustain_win);
    env.ripple_db = ripple(curve, raw, frames_per_ms, off);
    env.beat_corrected = beat_hz.is_none() || corrected.is_some();
    env.beat_periods = periods;
    env
}

/// Beat periods in the fit region (report 03 §4.1 step 1).
fn fit_region(raw: &[f64], frames_per_ms: f64, off: Option<usize>) -> Option<(usize, usize)> {
    let peak = raw.iter().copied().fold(0.0, f64::max);
    let start = raw.iter().position(|&e| e >= 0.3 * peak)? + (20.0 * frames_per_ms) as usize;
    let end = off
        .unwrap_or(raw.len())
        .min(raw.len())
        .saturating_sub((15.0 * frames_per_ms) as usize);
    (end > start).then_some((start, end))
}

fn fit_periods(raw: &[f64], hop_ms: f64, off: Option<usize>, beat_hz: f64) -> f64 {
    fit_region(raw, 1.0 / hop_ms, off)
        .map_or(0.0, |(s, e)| (e - s) as f64 * hop_ms / 1000.0 * beat_hz)
}

/// Fit and divide out the beat pattern. Returns the corrected envelope and the number of periods fitted.
fn fit(raw: &[f64], hop_ms: f64, off: Option<usize>, beat_hz: f64) -> Option<(Vec<f64>, f64)> {
    let frames_per_ms = 1.0 / hop_ms;
    let peak = raw.iter().copied().fold(0.0, f64::max);
    let (start, end) = fit_region(raw, frames_per_ms, off)?;
    if end <= start + (50.0 * frames_per_ms) as usize {
        return None;
    }
    let periods = (end - start) as f64 * hop_ms / 1000.0 * beat_hz;
    if periods < MIN_BEAT_PERIODS {
        return None;
    }
    let deg = if periods >= 3.0 {
        3
    } else if periods >= 1.5 {
        2
    } else {
        1
    };
    let kmax = if periods >= 2.0 { 6 } else { 2 };
    let n = (end - start) as f64;
    let floor = peak * 1e-3;
    let omega = |i: usize| 2.0 * PI * beat_hz * i as f64 * hop_ms / 1000.0;
    let step = ((2.0 * frames_per_ms) as usize).max(1);
    let mut rows = Vec::new();
    let mut y = Vec::new();
    for i in (start..end).step_by(step) {
        if raw[i] < 0.03 * peak {
            continue;
        }
        let x = 2.0 * (i - start) as f64 / n - 1.0;
        let mut r: Vec<f64> = (0..=deg).map(|p| ipow(x, p)).collect();
        let w = omega(i);
        for k in 1..=kmax {
            r.push(libm::cos(k as f64 * w));
            r.push(libm::sin(k as f64 * w));
        }
        rows.push(r);
        y.push(libm::log(raw[i].max(floor)));
    }
    if rows.len() < 4 * (deg + 1 + 2 * kmax) {
        return None;
    }
    let c = lstsq(&rows, &y)?;
    let corr: Vec<f64> = raw
        .iter()
        .enumerate()
        .map(|(i, &e)| {
            let w = omega(i);
            let mut pat = 0.0;
            for k in 1..=kmax {
                let (a, b) = (c[deg + 1 + 2 * (k - 1)], c[deg + 2 + 2 * (k - 1)]);
                pat += a * libm::cos(k as f64 * w) + b * libm::sin(k as f64 * w);
            }
            if pat < -0.7 {
                f64::NAN
            } else {
                e * libm::exp(-pat.clamp(-2.3, 2.3))
            }
        })
        .collect();
    Some((bridge(corr), periods))
}

/// Fill NaN runs by linear interpolation of the log level between the neighbouring valid points.
fn bridge(mut v: Vec<f64>) -> Vec<f64> {
    let n = v.len();
    let ln = |x: f64| libm::log(x.max(1e-12));
    let mut i = 0;
    while i < n {
        if v[i].is_nan() {
            let a = i;
            while i < n && v[i].is_nan() {
                i += 1;
            }
            let left = (a > 0).then(|| ln(v[a - 1]));
            let right = (i < n).then(|| ln(v[i]));
            for (j, slot) in v.iter_mut().enumerate().take(i).skip(a) {
                let val = match (left, right) {
                    (Some(l), Some(r)) => l + (r - l) * (j - a + 1) as f64 / (i - a + 1) as f64,
                    (Some(l), None) => l,
                    (None, Some(r)) => r,
                    (None, None) => ln(0.0),
                };
                *slot = libm::exp(val);
            }
        } else {
            i += 1;
        }
    }
    v
}

/// Read attack, decay, sustain, release and duration from a (corrected) envelope `c`.
fn read_stages(
    c: &[f64],
    ms: impl Fn(f64) -> f64,
    frames_per_ms: f64,
    on: usize,
    off: Option<usize>,
    sustain_win: usize,
) -> Envelope {
    let f = |m: f64| (m * frames_per_ms) as usize;
    let peak = movavg(c, f(PEAK_WINDOW_MS as f64).max(1))
        .iter()
        .copied()
        .fold(0.0, f64::max);
    let t_peak = c.iter().position(|&e| e >= peak).unwrap_or(0);
    let on = on.min(c.len() - 1);
    let onset = on
        + c[on..]
            .iter()
            .position(|&e| e >= AUDIBLE * peak)
            .unwrap_or(0);
    let t_top = onset
        + c[onset..]
            .iter()
            .position(|&e| e >= AT_PEAK * peak)
            .unwrap_or(0);
    let last = c
        .iter()
        .rposition(|&e| e >= AUDIBLE * peak)
        .unwrap_or(onset)
        .max(onset);
    let mut env = Envelope {
        attack_ms: ms((t_top - onset) as f64),
        decay_ms: None,
        sustain_db: None,
        release_ms: None,
        release_extrapolated: false,
        duration_ms: ms((last + 1 - onset) as f64),
        duration_truncated: last + 2 >= c.len(),
        ripple_db: 0.0,
        beat_corrected: true,
        beat_periods: None,
    };
    let Some(off) = off else { return env };
    let off_in_clip = off < c.len();
    let off = off.min(c.len() - 1);
    let hi = off.saturating_sub(f(10.0));
    let lo = hi.saturating_sub(sustain_win.max(f(SUSTAIN_MIN_MS as f64)));
    if lo > t_top && hi > lo {
        let db: Vec<f64> = c[lo..hi].iter().map(|v| db20(*v)).collect();
        let (_, end_db) = line_fit(&db);
        let lvl = libm::pow(10.0, end_db / 20.0);
        let s_db = db20(lvl / peak).max(-80.0);
        env.sustain_db = Some(s_db);
        env.decay_ms = Some(if lvl >= AT_PEAK * peak {
            0.0
        } else {
            let s40 = s_db.max(-40.0);
            let settle = peak * libm::pow(10.0, (s40 + (0.2 * -s40).max(1.5)) / 20.0);
            let s = t_peak.min(off);
            // first crossing below `settle` that stays below for `hold` (short beat nulls do not count)
            let hold = f(30.0).max(sustain_win / 2);
            let mut j = off;
            let mut i = s;
            while i < off {
                if c[i] <= settle {
                    let e = (i + hold).min(off);
                    if let Some(up) = c[i..e].iter().position(|&v| v > settle) {
                        i += up + 1;
                        continue;
                    }
                    j = i;
                    break;
                }
                i += 1;
            }
            ms((j - s) as f64)
        });
    }
    // Release needs the note-off inside the clip, with a little tail after it.
    if !off_in_clip || off + f(5.0) >= c.len() {
        return env;
    }
    let a = off.saturating_sub(f(20.0));
    let b = off.saturating_sub(f(5.0)).max(a + 1);
    let at_off = median(&c[a..b.min(c.len())]);
    if at_off < AUDIBLE * peak {
        env.release_ms = Some(0.0);
        return env;
    }
    let target = at_off * AUDIBLE;
    let found = c[off..]
        .iter()
        .rposition(|&e| e > target)
        .map(|p| p + 1)
        .filter(|&p| off + p < c.len());
    if let Some(p) = found {
        env.release_ms = Some(ms(p as f64));
    } else {
        // The clip ends first: extrapolate the fall (a straight line in dB over the observed tail) to -40 dB, so
        // the value keeps moving with the sound instead of sticking at the clip length.
        env.release_extrapolated = true;
        let observed = (c.len() - off) as f64;
        let tail: Vec<f64> = c[off..]
            .iter()
            .skip(f(5.0))
            .map(|v| db20(*v / at_off))
            .collect();
        let est = if tail.len() >= f(20.0).max(2) {
            let (slope, end_db) = line_fit(&tail);
            // line value at tail index k: end_db + slope·(k − (len − 1)); solve for −40 dB
            (slope < -1e-4 * hop_ms_from(frames_per_ms)).then(|| {
                let k = (tail.len() - 1) as f64 + (-40.0 - end_db) / slope;
                (k + f(5.0) as f64).max(observed)
            })
        } else {
            None
        };
        env.release_ms = Some(ms(est.unwrap_or(observed)));
    }
    env
}

fn hop_ms_from(frames_per_ms: f64) -> f64 {
    1.0 / frames_per_ms
}

/// Residual ripple (p95 − p5 in dB) of `e` against its own 200 ms moving average over the held part. The average
/// runs on the dB curve, so an exponential decay (a straight line in dB) is not mistaken for ripple, and only where
/// the whole window fits.
fn ripple(e: &[f64], raw: &[f64], frames_per_ms: f64, off: Option<usize>) -> f64 {
    let f = |m: f64| (m * frames_per_ms) as usize;
    let peak = raw.iter().copied().fold(0.0, f64::max);
    let start = raw.iter().position(|&v| v >= 0.5 * peak).unwrap_or(0) + f(50.0);
    let end = off.unwrap_or(e.len()).min(e.len()).saturating_sub(f(20.0));
    let w = f(200.0).max(3);
    if end <= start + w + f(100.0) {
        return 0.0;
    }
    let db: Vec<f64> = e[start..end].iter().map(|v| db20(*v)).collect();
    let sm = movavg(&db, w);
    let d: Vec<f64> = (w / 2..db.len() - w / 2)
        .filter(|&i| e[start + i] > peak * AUDIBLE)
        .map(|i| db[i] - sm[i])
        .collect();
    if d.len() < 10 {
        return 0.0;
    }
    quantile(&d, 0.95) - quantile(&d, 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{
        Adsr, SR, Wave, detuned_pair, linear_attack, oscillator, sine, theory_beat,
    };

    fn env_of(x: &[f32], hold_s: Option<f64>, beat: Option<f64>) -> Envelope {
        let (raw, hop) = rms_envelope(x, SR);
        let hop_ms = hop as f64 / SR * 1000.0;
        measure(
            &raw,
            hop_ms,
            0,
            hold_s.map(|h| (h * 1000.0 / hop_ms) as usize),
            beat,
        )
    }

    #[test]
    fn attack_of_a_linear_ramp() {
        // from 1 % to -1 dB (89 %) of a linear ramp is 88 % of its length; the 8 ms window blurs short ramps
        for (ramp, lo, hi) in [(0.2, 170.0, 182.0), (0.1, 84.0, 92.0), (0.03, 24.0, 30.0)] {
            let mut x = sine(SR, 440.0, 0.5, 0.6);
            linear_attack(&mut x, SR, ramp);
            let a = env_of(&x, None, None).attack_ms;
            assert!((lo..hi).contains(&a), "ramp {ramp}s read as {a} ms");
        }
    }

    #[test]
    fn adsr_stages_read_back() {
        let e = Adsr {
            attack_s: 0.005,
            decay_s: 0.4,
            sustain: 0.25,
            release_s: 0.4,
        };
        let mut x = sine(SR, 440.0, 0.5, 2.0);
        e.apply(&mut x, SR, 1.2);
        let m = env_of(&x, Some(1.2), None);
        // the 8 ms RMS window blunts the peak a little, so the sustain reads ~0.3 dB above -12.04
        assert!((m.sustain_db.unwrap() + 12.04).abs() < 0.5, "{m:?}");
        // release: -40 dB of an exponential with tau = 100 ms is 4.6 tau
        assert!((m.release_ms.unwrap() - 460.0).abs() < 25.0, "{m:?}");
        assert!(!m.release_extrapolated && m.beat_corrected);
        let d = m.decay_ms.unwrap();
        assert!(d > 100.0 && d < 400.0, "{m:?}");
        assert!(m.ripple_db < 0.5, "{m:?}");
    }

    #[test]
    fn release_is_extrapolated_when_the_clip_ends_first() {
        let e = Adsr {
            attack_s: 0.005,
            decay_s: 0.1,
            sustain: 1.0,
            release_s: 2.0,
        };
        let mut x = sine(SR, 440.0, 0.5, 1.5);
        e.apply(&mut x, SR, 1.0);
        let m = env_of(&x, Some(1.0), None);
        assert!(m.release_extrapolated, "{m:?}");
        // tau 500 ms: -40 dB after 2.3 s
        assert!((m.release_ms.unwrap() - 2303.0).abs() < 150.0, "{m:?}");
        assert!(m.duration_truncated);
    }

    #[test]
    fn note_held_past_the_clip_has_no_release() {
        let x = sine(SR, 440.0, 0.5, 1.0);
        let m = env_of(&x, Some(2.0), None);
        assert_eq!(m.release_ms, None);
        assert!(m.sustain_db.unwrap().abs() < 0.2);
    }

    #[test]
    fn the_fit_removes_a_beat() {
        // 25 ct at A3 beats at 3.2 Hz; a 10 ms attack reads ~10 ms after correction, not ~1/(4 beat)
        let f0 = 220.0;
        let beat = theory_beat(f0, 25.0);
        let e = Adsr {
            attack_s: 0.01,
            decay_s: 0.3,
            sustain: 1.0,
            release_s: 0.2,
        };
        let mut x = detuned_pair(SR, Wave::Saw, f0, 25.0, 2.5, (0.0, 0.37));
        e.apply(&mut x, SR, 2.0);
        let raw = env_of(&x, Some(2.0), None);
        let fit = env_of(&x, Some(2.0), Some(beat));
        let mut y = oscillator(SR, Wave::Saw, f0, 2.5, 0.0);
        e.apply(&mut y, SR, 2.0);
        let twin = env_of(&y, Some(2.0), None);
        assert!(
            fit.beat_corrected && fit.beat_periods.unwrap() > 5.0,
            "{fit:?}"
        );
        assert!(
            (fit.attack_ms - twin.attack_ms).abs() <= 5.0,
            "fit {fit:?} twin {twin:?}"
        );
        assert!(
            (fit.sustain_db.unwrap() - twin.sustain_db.unwrap()).abs() < 1.5,
            "fit {fit:?} twin {twin:?}"
        );
        assert!(fit.ripple_db < raw.ripple_db, "fit {fit:?} raw {raw:?}");
    }

    #[test]
    fn too_slow_a_beat_is_flagged_not_corrected() {
        let mut x = detuned_pair(SR, Wave::Saw, 130.81, 5.0, 1.5, (0.0, 0.37));
        Adsr {
            attack_s: 0.01,
            decay_s: 0.3,
            sustain: 1.0,
            release_s: 0.2,
        }
        .apply(&mut x, SR, 1.2);
        let m = env_of(&x, Some(1.2), Some(theory_beat(130.81, 5.0)));
        assert!(!m.beat_corrected, "{m:?}");
        assert!(m.beat_periods.unwrap() < 1.0);
    }
}
