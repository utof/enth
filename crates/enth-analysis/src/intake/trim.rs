//! **Owner: lane FE** (the intake, freeze point F1s).
//!
//! Mono and trim (docs/r3.md, "The intake"): mono is the mean of the channels; leading silence (about 60 dB below the
//! peak, or just above the noise floor when there is one) is stripped with a few ms of pre-roll; the tail is cut a
//! little after the signal falls into the floor; the length is capped (default 5 s, the 4 to 6 s of docs/r3.md),
//! because evaluation cost grows with it. The reported onset lands within 5 ms of the true one (FE's acceptance).
//! **A short sound is kept, never rejected**: nothing here filters by duration, and a clip shorter than
//! [`IntakeCfg::min_len_s`](super::IntakeCfg) is padded with silence instead.
//!
//! The onset is found in two steps: the first 20 ms frame of the level track at or above the threshold, then the
//! first 1 ms window inside it at or above the same level, then the first sample there whose magnitude reaches it.
//! The pre-roll is a whole number of the engine's 32-frame render segments, so a note played at the clip's onset
//! starts exactly on the render grid (docs/design.md §5).

use serde::{Deserialize, Serialize};

use crate::linalg::line_fit;
use crate::loudness::FLOOR_DB;

/// Length of one frame of the level track (s).
pub const FRAME_S: f64 = 0.020;
/// Hop of the level track (s).
pub const HOP_S: f64 = 0.010;
/// Window of the fine envelope that places the onset inside its frame (s).
const FINE_S: f64 = 0.001;
/// The engine's render segment in frames (docs/design.md §5): the pre-roll is a whole number of them.
pub const SEGMENT_FRAMES: usize = 32;

/// Where the conditioned clip came from in the decoded file. Times are seconds in the **source** (the decoded file,
/// at its own rate); every other time in the intake report is clip time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Span {
    /// The detected onset in the source.
    pub onset_s: f64,
    /// The first source sample copied into the clip (the onset minus the pre-roll, at least 0).
    pub start_s: f64,
    /// Just past the last source sample copied into the clip.
    pub end_s: f64,
    /// Where the signal falls into the noise floor (or 60 dB below its peak) for good, in the source.
    pub signal_end_s: f64,
    /// Silence prepended because the onset was nearer the start of the file than the pre-roll.
    pub lead_pad_s: f64,
    /// Silence appended because the clip was shorter than the minimum length. Short sounds are kept, never rejected.
    pub tail_pad_s: f64,
    /// The signal was still above the floor at the length cap: the clip is the sound's first part.
    pub capped: bool,
}

/// The mean of the channels, frame by frame (the CLI's WAV reader computes it the same way: the `f32` sum of the
/// frame divided by the channel count). `channels` of 0 is treated as 1; a trailing partial frame is dropped.
#[must_use]
pub fn mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let n = usize::from(channels.max(1));
    if n == 1 {
        return interleaved.to_vec();
    }
    let c = f32::from(channels);
    interleaved
        .chunks_exact(n)
        .map(|frame| frame.iter().sum::<f32>() / c)
        .collect()
}

/// Power lost by the mono mix against the mean power of the channels (dB, at least 0): about 0 for identical
/// channels, 3 for uncorrelated ones, large when they cancel (a polarity-flipped channel). `None` for one channel or
/// silence.
#[must_use]
pub fn mono_loss_db(interleaved: &[f32], channels: u16, mono: &[f32]) -> Option<f64> {
    let n = usize::from(channels);
    if n < 2 {
        return None;
    }
    let chans: f64 = interleaved
        .chunks_exact(n)
        .map(|f| f.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / n as f64)
        .sum();
    let m: f64 = mono.iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
    if chans <= 0.0 {
        return None;
    }
    Some((10.0 * libm::log10(chans / m.max(chans * 1e-15))).max(0.0))
}

/// The level track: RMS over [`FRAME_S`] every [`HOP_S`], in dBFS (floored at −150). Frame `i` covers samples
/// `i·hop .. i·hop + win`, cut at the end of the signal.
#[derive(Clone, Debug)]
pub(crate) struct Levels {
    pub db: Vec<f64>,
    pub hop: usize,
    pub win: usize,
}

impl Levels {
    pub fn new(x: &[f32], sr: f64, frame_s: f64, hop_s: f64) -> Levels {
        let hop = (libm::round(sr * hop_s) as usize).max(1);
        let win = (libm::round(sr * frame_s) as usize).max(hop);
        let pre = prefix_squares(x);
        let frames = x.len().div_ceil(hop).max(1);
        let db = (0..frames)
            .map(|i| {
                let a = (i * hop).min(x.len());
                let b = (a + win).min(x.len());
                rms_db(&pre, a, b)
            })
            .collect();
        Levels { db, hop, win }
    }

    /// The level track of the intake: 20 ms frames every 10 ms.
    pub fn standard(x: &[f32], sr: f64) -> Levels {
        Levels::new(x, sr, FRAME_S, HOP_S)
    }

    pub fn peak_db(&self) -> f64 {
        self.db.iter().copied().fold(FLOOR_DB, f64::max)
    }

    /// Sample range of frame `i`.
    pub fn range(&self, i: usize, len: usize) -> (usize, usize) {
        let a = (i * self.hop).min(len);
        (a, (a + self.win).min(len))
    }
}

/// Running sums of squares: `pre[i]` is the energy of `x[..i]`.
pub(crate) fn prefix_squares(x: &[f32]) -> Vec<f64> {
    let mut pre = Vec::with_capacity(x.len() + 1);
    let mut acc = 0.0f64;
    pre.push(0.0);
    for &s in x {
        acc += f64::from(s) * f64::from(s);
        pre.push(acc);
    }
    pre
}

/// RMS of `x[a..b]` in dBFS from its prefix sums, floored at −150.
pub(crate) fn rms_db(pre: &[f64], a: usize, b: usize) -> f64 {
    if b <= a {
        return FLOOR_DB;
    }
    let ms = ((pre[b] - pre[a]) / (b - a) as f64).max(0.0);
    if ms > 0.0 {
        (10.0 * libm::log10(ms)).max(FLOOR_DB)
    } else {
        FLOOR_DB
    }
}

/// dB to amplitude.
pub(crate) fn amp(db: f64) -> f64 {
    libm::pow(10.0, db / 20.0)
}

/// The onset: the first sample at or above `thr_db` (an RMS level in dBFS), placed by the level track, then a 1 ms
/// window, then the sample. 0 when nothing reaches the threshold (cannot happen for a threshold below the peak).
pub(crate) fn onset(x: &[f32], sr: f64, levels: &Levels, thr_db: f64) -> usize {
    let Some(i) = levels.db.iter().position(|&d| d >= thr_db) else {
        return 0;
    };
    let (a, b) = levels.range(i, x.len());
    // The previous frame was below the threshold, but it overlaps this one: search from one hop before.
    let from = a.saturating_sub(levels.hop);
    let w = (libm::round(sr * FINE_S) as usize).max(8);
    let step = (w / 4).max(1);
    let pre = prefix_squares(x);
    let mut j = from;
    while j < b {
        let e = (j + w).min(x.len());
        if rms_db(&pre, j, e) >= thr_db {
            let thr = amp(thr_db) as f32;
            return (j..e).find(|&s| x[s].abs() >= thr).unwrap_or(j);
        }
        j += step;
    }
    a
}

/// Just past the last sample of the signal: the end of the last pair of consecutive frames at or above `thr_db`
/// (a single frame when no pair qualifies, for the shortest sounds), never before `onset + 1`.
pub(crate) fn signal_end(levels: &Levels, len: usize, thr_db: f64, onset: usize) -> usize {
    let db = &levels.db;
    let pair = (1..db.len())
        .rev()
        .find(|&i| db[i] >= thr_db && db[i - 1] >= thr_db);
    let last = pair.or_else(|| db.iter().rposition(|&d| d >= thr_db));
    let end = last.map_or(len, |i| levels.range(i, len).1);
    end.clamp((onset + 1).min(len), len)
}

/// What [`cut`] needs from the configuration, in frames.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CutFrames {
    pub preroll: usize,
    pub tail: usize,
    pub max_len: usize,
    pub min_len: usize,
}

impl CutFrames {
    pub fn new(
        sr: f64,
        preroll_ms: f64,
        tail_ms: f64,
        max_len_s: f64,
        min_len_s: f64,
    ) -> CutFrames {
        let raw = libm::ceil(sr * preroll_ms / 1000.0) as usize;
        let preroll = raw.div_ceil(SEGMENT_FRAMES) * SEGMENT_FRAMES;
        CutFrames {
            preroll,
            tail: libm::round(sr * tail_ms / 1000.0) as usize,
            max_len: (libm::round(sr * max_len_s) as usize).max(preroll + 1),
            min_len: libm::round(sr * min_len_s) as usize,
        }
    }
}

/// Cut the clip: the pre-roll (zeros first when the file starts too soon), the source from the onset to a little
/// after the signal end, capped, then zeros up to the minimum length. Returns the clip and its [`Span`].
pub(crate) fn cut(
    x: &[f32],
    sr: f64,
    onset: usize,
    signal_end: usize,
    f: CutFrames,
) -> (Vec<f32>, Span) {
    let start = onset.saturating_sub(f.preroll);
    let lead = f.preroll - (onset - start);
    let room = f.max_len - lead;
    let end = (signal_end + f.tail)
        .min(x.len())
        .min(start + room)
        .max(start);
    let capped = signal_end > end;
    let mut clip = Vec::with_capacity((lead + end - start).max(f.min_len));
    clip.resize(lead, 0.0);
    clip.extend_from_slice(&x[start..end]);
    let tail_pad = f.min_len.saturating_sub(clip.len());
    clip.resize(clip.len() + tail_pad, 0.0);
    let s = |n: usize| n as f64 / sr;
    (
        clip,
        Span {
            onset_s: s(onset),
            start_s: s(start),
            end_s: s(end),
            signal_end_s: s(signal_end),
            lead_pad_s: s(lead),
            tail_pad_s: s(tail_pad),
            capped,
        },
    )
}

/// The T60 of the final decay (s): a straight line in dB through the level track over the last 25 dB above
/// `floor_db` before `end` (at least 50 ms of it), extended to 60 dB. A release and a room tail look alike here: this
/// is the decay of whatever is last, not a reverb detector. `None` without such a decay.
pub(crate) fn tail_t60(levels: &Levels, sr: f64, end_frame: usize, floor_db: f64) -> Option<f64> {
    let db = &levels.db;
    let end = end_frame.min(db.len());
    if end < 2 {
        return None;
    }
    let lo = floor_db + 5.0;
    let hi = floor_db + 30.0;
    // Walk back from the end over frames below `hi`; the decay starts at the last frame at or above it.
    let mut a = end;
    while a > 0 && db[a - 1] < hi {
        a -= 1;
    }
    if a == 0 {
        return None;
    }
    let seg: Vec<f64> = db[a..end].iter().copied().filter(|&d| d >= lo).collect();
    let hop_s = levels.hop as f64 / sr;
    if (seg.len() as f64) * hop_s < 0.05 {
        return None;
    }
    let (slope, _) = line_fit(&seg);
    let per_s = slope / hop_s;
    (per_s < -1.0).then(|| -60.0 / per_s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{SR, white_noise};

    #[test]
    fn mono_is_the_mean_of_the_channels() {
        let st = [1.0f32, 0.0, 0.5, 0.5, -1.0, 1.0, 9.0];
        assert_eq!(mono(&st, 2), vec![0.5, 0.5, 0.0]);
        assert_eq!(mono(&st, 1), st.to_vec());
        let loss = |l: &[f32], r: &[f32]| {
            let i: Vec<f32> = l.iter().zip(r).flat_map(|(a, b)| [*a, *b]).collect();
            mono_loss_db(&i, 2, &mono(&i, 2)).unwrap()
        };
        let a = white_noise(SR, 0.3, 0.2, 1);
        let b = white_noise(SR, 0.3, 0.2, 2);
        let neg: Vec<f32> = a.iter().map(|v| -v).collect();
        assert!(loss(&a, &a) < 0.01);
        assert!((loss(&a, &b) - 3.0).abs() < 0.3, "{}", loss(&a, &b));
        assert!(loss(&a, &neg) > 100.0);
    }

    #[test]
    fn the_preroll_is_whole_segments_and_short_clips_are_padded() {
        let f = CutFrames::new(48_000.0, 5.0, 100.0, 5.0, 0.1);
        assert_eq!(f.preroll, 256);
        let f8 = CutFrames::new(8_000.0, 5.0, 100.0, 5.0, 0.1);
        assert_eq!(f8.preroll, 64);
        // a 2 ms click at the very start of a 3 ms file: zeros before it, zeros after it, never rejected
        let mut x = vec![0.0f32; 144];
        for v in &mut x[..96] {
            *v = 0.5;
        }
        let (clip, span) = cut(&x, 48_000.0, 0, 96, f);
        assert_eq!(clip.len(), 4800);
        assert_eq!(&clip[256..352], &x[..96]);
        assert!((span.lead_pad_s - 256.0 / 48_000.0).abs() < 1e-12);
        assert!(span.tail_pad_s > 0.09 && !span.capped);
    }

    #[test]
    fn the_cap_counts_the_lead_pad_and_says_so() {
        let x = vec![0.1f32; 48_000 * 8];
        let f = CutFrames::new(48_000.0, 5.0, 100.0, 5.0, 0.1);
        let (clip, span) = cut(&x, 48_000.0, 100, x.len(), f);
        assert_eq!(clip.len(), 240_000);
        assert!(span.capped);
    }
}
