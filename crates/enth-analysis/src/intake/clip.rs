//! **Owner: lane FE** (the intake, freeze point F1s).
//!
//! Clipped spans of an upload (docs/r3.md, "The intake"): clipping after +12 dB took today's search to 64 % against
//! the clean target, with a cutoff of 51 Hz and a drive of 6.2 dB. The spans are reported as a `Degradation`, and lane
//! OBJ's robust distance leaves the clipped frames out. A clipped synth render is still a valid sound (the training
//! filters keep them); this file only says where.
//!
//! **Hard clipping** leaves plateaus: runs of at least [`MIN_RUN`] consecutive samples at the clip's own extreme value
//! (each sign on its own, within one part in a million, so a float file and an integer one read alike, and a file
//! normalised after clipping still shows them). A clean waveform never holds its extreme for that long except at very
//! low frequencies and exactly full scale (a 16-bit sine below about 20 Hz).
//!
//! **A square wave's flat tops are not clipping.** A synth's unfiltered square (or pulse) holds its extremes too, but
//! it jumps onto each plateau from the other extreme and off it again, within the two or three samples a band-limited
//! step takes. A clipped waveform reaches its ceiling gradually on at least one side (a sine's or a saw's ramp). A
//! plateau with a sample at least [`JUMP_SHARE`] of the clip's range away within [`JUMP`] samples on both sides is a
//! square's top (the clip's own edges count as jumps); when they are at least half of an extreme's plateaus, that
//! extreme is a square's and none of its plateaus count (the few its envelope's attack and release enter gradually
//! included); otherwise only the gradual ones count. The price: content above about 1.2 kHz clipped by 12 dB or more
//! at 48 kHz looks like a square and is not reported (lower rates move that limit down in proportion).
//!
//! Not detected, and so not reported: soft clipping (tanh) and limiting, which leave no plateau, and clipping
//! followed by a lossy codec, which rings over it.

use serde::{Deserialize, Serialize};

/// Fewest consecutive samples at the extreme that count as a plateau.
pub const MIN_RUN: usize = 5;
/// "At the extreme": within this share of it.
const TOL: f32 = 1e-6;
/// Plateaus closer than this (s) are one span.
const MERGE_S: f64 = 0.010;
/// A square's band-limited step: within this many samples of a plateau's edge...
const JUMP: usize = 3;
/// ...a sample at least this share of the clip's range away from the plateau (the other extreme).
const JUMP_SHARE: f32 = 0.9;

/// Hard clipping in the conditioned clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clipping {
    /// The plateau level: the larger of the two extremes that hold plateaus, dBFS.
    pub level_dbfs: f64,
    /// Share of the clip's samples that sit on a plateau, 0 to 1.
    pub share: f64,
    /// Where the plateaus are, in clip time, merged across gaps under 10 ms.
    pub spans: Vec<ClipSpan>,
}

/// One clipped stretch of the clip, in seconds of clip time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipSpan {
    pub start_s: f64,
    pub end_s: f64,
}

/// Hard clipping in `x`, or `None`.
pub(crate) fn detect(x: &[f32], sr: f64) -> Option<Clipping> {
    let hi = x.iter().copied().fold(0.0f32, f32::max);
    let lo = x.iter().copied().fold(0.0f32, f32::min);
    let range = hi - lo;
    // Whether `x[a..b]` holds a sample far from `ext` (the clip's edge counts as one).
    let jump = |a: usize, b: usize, ext: f32| {
        a >= b || x[a..b].iter().any(|v| (v - ext).abs() >= JUMP_SHARE * range)
    };
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut level = 0.0f32;
    for ext in [hi, lo] {
        if ext == 0.0 {
            continue;
        }
        let at = |v: f32| (v - ext).abs() <= TOL * ext.abs();
        let before = runs.len();
        let mut tops = 0usize;
        let mut i = 0;
        while i < x.len() {
            if at(x[i]) {
                let s = i;
                while i < x.len() && at(x[i]) {
                    i += 1;
                }
                if i - s >= MIN_RUN {
                    let square_top = jump(s.saturating_sub(JUMP), s, ext)
                        && jump(i, (i + JUMP).min(x.len()), ext);
                    if square_top {
                        tops += 1;
                    } else {
                        runs.push((s, i));
                    }
                }
            } else {
                i += 1;
            }
        }
        if tops > 0 && 2 * tops >= tops + runs.len() - before {
            // A square's extreme: its gradual plateaus are its envelope's edges.
            runs.truncate(before);
        }
        if runs.len() > before {
            level = level.max(ext.abs());
        }
    }
    if runs.is_empty() {
        return None;
    }
    let on: usize = runs.iter().map(|(a, b)| b - a).sum();
    runs.sort_unstable();
    let gap = (sr * MERGE_S) as usize;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (a, b) in runs {
        match spans.last_mut() {
            Some(last) if a <= last.1 + gap => last.1 = last.1.max(b),
            _ => spans.push((a, b)),
        }
    }
    Some(Clipping {
        level_dbfs: crate::loudness::db20(f64::from(level)),
        share: on as f64 / x.len() as f64,
        spans: spans
            .iter()
            .map(|&(a, b)| ClipSpan {
                start_s: a as f64 / sr,
                end_s: b as f64 / sr,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{SR, ToyPatch, ToyPerf, render_toy, sine};

    #[test]
    fn hard_clipping_is_found_where_it_is() {
        let p = ToyPatch::default();
        let mut x = render_toy(SR, &p, &ToyPerf::default(), (0.0, 0.37), 12.0);
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 1.0);
        assert!(
            detect(&x, SR).is_none(),
            "a float render over full scale is not clipped"
        );
        for v in &mut x {
            *v = v.clamp(-1.0, 1.0) * 0.5; // clipped, then normalised
        }
        let c = detect(&x, SR).expect("clipped");
        assert!((c.level_dbfs + 6.02).abs() < 0.01, "{c:?}");
        assert!(c.share > 0.05, "{c:?}");
        // the clipping is in the held part and the release, not after the sound has died away
        assert!(
            c.spans[0].start_s < 0.05 && c.spans.last().unwrap().end_s < 2.0,
            "{:?}",
            c.spans
        );
    }

    #[test]
    fn a_square_wave_is_not_clipped_but_a_clipped_one_with_ramps_is() {
        use crate::test_signals::{Wave, oscillator};
        // the toy oscillator's band-limited square: flat tops entered and left by a step
        for f in [55.0, 220.0, 1000.0] {
            let x = oscillator(SR, Wave::Square, f, 0.5, 0.1);
            assert!(detect(&x, SR).is_none(), "square at {f} Hz");
            let half: Vec<f32> = x.iter().map(|v| 0.5 * v).collect();
            assert!(detect(&half, SR).is_none(), "square at {f} Hz, -6 dB");
        }
        // a saw clipped at +12 dB: entered by its ramp, left by its step; a sine clipped likewise
        for (wave, f) in [(Wave::Saw, 110.0), (Wave::Saw, 440.0), (Wave::Triangle, 220.0)] {
            let x: Vec<f32> = oscillator(SR, wave, f, 0.5, 0.1)
                .iter()
                .map(|v| (4.0 * v).clamp(-1.0, 1.0))
                .collect();
            assert!(detect(&x, SR).is_some(), "{} at {f} Hz clipped", wave.name());
        }
        for f in [100.0, 1000.0] {
            let x: Vec<f32> = sine(SR, f, 4.0, 0.5)
                .iter()
                .map(|v| v.clamp(-1.0, 1.0))
                .collect();
            assert!(detect(&x, SR).is_some(), "sine at {f} Hz clipped");
        }
    }

    #[test]
    fn clean_waveforms_are_not_clipped() {
        for f in [30.0, 110.0, 1000.0] {
            let x = sine(SR, f, 0.99, 0.5);
            assert!(detect(&x, SR).is_none(), "{f}");
            // the same, quantised to 16 bits at full scale
            let q: Vec<f32> = x
                .iter()
                .map(|v| (f64::from(*v) * 32767.0).round() as f32 / 32767.0)
                .collect();
            assert!(detect(&q, SR).is_none(), "16-bit {f}");
        }
        let t = render_toy(
            SR,
            &ToyPatch::default(),
            &ToyPerf::default(),
            (0.0, 0.37),
            0.0,
        );
        assert!(detect(&t, SR).is_none());
    }
}
