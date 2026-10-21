//! **Owner: lane FE** (the intake, freeze point F1s).
//!
//! The inferred performance, [`InferredPerf`] (docs/r3.md, "The intake"): what to play on every candidate so its render
//! lines up with the upload. One note:
//!
//! - **the key** from YIN ([`pitch`](crate::pitch)): the median f0 of the voiced frames (80 ms frames every 20 ms,
//!   27.5 Hz to 4.5 kHz, frames within 30 dB of the loudest and 10 dB above the noise floor), rounded to the nearest
//!   MIDI note. When YIN finds too few voiced frames but the held part's spectrum is tonal, the harmonic peaks name
//!   the pitch instead ([`tonal_pitch`](crate::pitch::tonal_pitch)); otherwise the clip is unpitched: the key is C3
//!   (the CLI's default) with an `Unpitched` warning;
//! - **the note-on** at the clip's onset (the pre-roll), and **the note-off** from
//!   [`infer_note_off`](crate::envelope::infer_note_off); a sound that only decays has no detectable note-off, and the
//!   note is then held until the signal falls into the floor;
//! - **the velocity** is not inferred: the metric normalises loudness, so an upload's level says nothing about it. It
//!   is the CLI's default, 100 (100/127).
//!
//! Three warnings, because one note cannot reproduce them and the CLI's `--perf` (CLC) overrides all of this:
//!
//! - **several onsets** ("this looks like a phrase"): new partials arriving with a rise in level (the super flux of
//!   Böck and Widmer, with a level check), at least 100 ms after the previous onset, within 30 dB of the loudest level;
//! - **several notes:** the pitch holds two different notes (not octaves) for at least 80 ms each;
//! - **a chord** (or an inharmonic sound), from the harmonic peaks of the held part's spectrum: either a second
//!   harmonic series that the first does not explain carries at least 10 % of the peaks' power at an unrelated
//!   pitch, or the best series is a virtual pitch, with its fundamental and second harmonic missing and gaps in its
//!   harmonics (a major triad in 12-TET reads as harmonics 4, 5, 6, 8, 10, 12 of a note two octaves below the root).
//!   For a chord the key is its lowest tone.

use serde::{Deserialize, Serialize};

use super::trim::{SEGMENT_FRAMES, prefix_squares, rms_db};
use super::welch;
use crate::envelope::infer_note_off;
use crate::fft::{FftPlanner, hann};
use crate::linalg::median;
use crate::pitch::{Peak, Yin, tonal_pitch};
use crate::units::hz_to_midi;
use realfft::num_complex::Complex32;

/// The key of an unpitched clip: C3, the CLI's default note.
pub const UNPITCHED_KEY: u8 = 48;
/// The velocity every candidate plays: the CLI's default, 100 of 127.
pub const VELOCITY: f32 = 100.0 / 127.0;

const YIN_FRAME_S: f64 = 0.08;
const YIN_HOP_S: f64 = 0.02;
const YIN_MIN_HZ: f64 = 27.5;
const YIN_MAX_HZ: f64 = 4500.0;
/// Frames that count for the pitch: within this of the loudest frame, and this far above the floor.
const LOUD_DB: f64 = 30.0;
const ABOVE_FLOOR_DB: f64 = 10.0;
/// A clip is pitched when at least this share of its loud frames is voiced.
const MIN_VOICED: f64 = 0.4;
/// Onsets (the super flux): frames of at least 30 ms and four periods of the note, rounded up to a power of two, every
/// 10 ms, the rise over two frames back, bins to 8 kHz, the log's
/// gain, the rise above the local median that makes an onset (natural-log units per bin), and the shortest gap.
const FLUX_FRAME_S: f64 = 0.03;
const FLUX_PERIODS: f64 = 4.0;
const FLUX_HOP_S: f64 = 0.01;
const FLUX_LAG: usize = 2;
const FLUX_TOP_HZ: f64 = 8000.0;
const LOG_GAIN: f32 = 1000.0;
const FLUX_RISE: f64 = 0.08;
/// ...and the level just after the peak at least this far above the level 30 ms before it.
const LEVEL_RISE_DB: f64 = 2.0;
const GAP_S: f64 = 0.1;
/// Notes: a run of voiced frames on one nearest note at least this long.
const NOTE_RUN_S: f64 = 0.08;
/// Chord spectrum: frames of about 170 ms over the held part, from 50 ms after the onset.
const CHORD_FRAME_S: f64 = 0.17;
const CHORD_SKIP_S: f64 = 0.05;
const PEAK_RANGE_HZ: (f64, f64) = (30.0, 5000.0);
const MAX_PEAKS: usize = 30;
const SERIES_RANGE_HZ: (f64, f64) = (25.0, 2000.0);
const MAX_HARMONIC: f64 = 16.0;
/// The peaks must hold at least this share of the band's power for a chord check: noise has peaks too.
const MIN_TONAL: f64 = 0.6;

/// What every candidate plays: one note. Times are clip time (seconds from the conditioned clip's first sample).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InferredPerf {
    /// The MIDI key: the nearest note to `f0_hz` (a chord's lowest tone; [`UNPITCHED_KEY`] when unpitched).
    pub key: u8,
    /// The median YIN f0 of the voiced frames (Hz); `None` when unpitched.
    pub f0_hz: Option<f64>,
    /// Share of the loud frames YIN found voiced, 0 to 1.
    pub voiced: f64,
    /// 0 to 1: always [`VELOCITY`], not inferred.
    pub velocity: f32,
    /// Note-on: the clip's onset, after the pre-roll (whole 32-frame engine segments).
    pub on_s: f64,
    /// The note-off `infer_note_off` found; `None` for a sound that only decays.
    pub off_s: Option<f64>,
    /// How long the note is held: `off_s − on_s`, or until the signal falls into the floor when there is no note-off.
    /// At least one engine segment.
    pub hold_s: f64,
    /// The clip's length: what every candidate renders.
    pub duration_s: f64,
    /// Every onset found, the first being `on_s`.
    pub onsets_s: Vec<f64>,
}

/// What [`infer`] found besides the performance, for the intake's warnings.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct Findings {
    pub unpitched: bool,
    /// Keys of the stable notes, in order, when there are two or more (not octaves of one another).
    pub notes: Vec<u8>,
    /// The chord's tones (Hz), ascending.
    pub chord: Vec<f64>,
}

/// Infer the performance of `clip`: the onset is at sample `on`, the signal ends at `sig_end`.
pub(crate) fn infer(
    clip: &[f32],
    sr: f64,
    on: usize,
    sig_end: usize,
    floor_db: Option<f64>,
    planner: &mut FftPlanner,
) -> (InferredPerf, Findings) {
    let mut found = Findings::default();
    let (f0, voiced, runs) = pitch(clip, sr, on, sig_end, floor_db, planner);
    let off = infer_note_off(&clip[on.min(clip.len())..], sr).map(|t| on as f64 / sr + t);
    let held_end = off.map_or(sig_end, |t| (t * sr) as usize);
    let held = held_part(clip, sr, on, held_end.max(on + 1), planner);
    found.chord = chord(&held);
    found.notes = notes(&runs);
    // YIN needs the waveform itself to repeat; when it finds no pitch in a tonal spectrum (a narrow pulse through a
    // resonant filter, say), the harmonic peaks still name it (`pitch::tonal_pitch`).
    let f0 = f0.or_else(|| {
        (held.tonal() && found.chord.is_empty())
            .then(|| tonal_pitch(&held.peaks, held.share(), held.bin).map(|t| t.f0_hz))
            .flatten()
    });
    let key = if let Some(&low) = found.chord.first() {
        to_key(low)
    } else if let Some(&first) = found.notes.first() {
        first
    } else if let Some(f) = f0 {
        to_key(f)
    } else {
        found.unpitched = true;
        UNPITCHED_KEY
    };
    let on_s = on as f64 / sr;
    let segment = SEGMENT_FRAMES as f64 / sr;
    let hold_s = off
        .map_or((sig_end.saturating_sub(on)) as f64 / sr, |t| t - on_s)
        .max(segment);
    let mut onsets_s = vec![on_s];
    onsets_s.extend(onsets(clip, sr, on, sig_end, floor_db, f0, planner));
    (
        InferredPerf {
            key,
            f0_hz: f0,
            voiced,
            velocity: VELOCITY,
            on_s,
            off_s: off,
            hold_s,
            duration_s: clip.len() as f64 / sr,
            onsets_s,
        },
        found,
    )
}

fn to_key(hz: f64) -> u8 {
    libm::round(hz_to_midi(hz)).clamp(0.0, 127.0) as u8
}

/// The median f0 of the voiced loud frames, the voiced share, and the per-frame track of the loud frames (`None`
/// where unvoiced) for the notes check.
#[allow(
    clippy::type_complexity,
    reason = "a private triple, named at its one call site"
)]
fn pitch(
    clip: &[f32],
    sr: f64,
    on: usize,
    sig_end: usize,
    floor_db: Option<f64>,
    planner: &mut FftPlanner,
) -> (Option<f64>, f64, Vec<Option<f64>>) {
    let frame = ((libm::round(sr * YIN_FRAME_S) as usize) & !1).max(64);
    let hop = (libm::round(sr * YIN_HOP_S) as usize).max(1);
    let mut starts = Vec::new();
    let mut p = on;
    while p < sig_end.max(on + 1) {
        starts.push(p);
        p += hop;
    }
    let mut x = clip.to_vec();
    let need = starts.last().map_or(0, |s| s + frame);
    if x.len() < need {
        x.resize(need, 0.0);
    }
    let pre = prefix_squares(&x);
    let lv: Vec<f64> = starts.iter().map(|&s| rms_db(&pre, s, s + frame)).collect();
    let peak = lv.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let gate = (peak - LOUD_DB).max(floor_db.map_or(f64::NEG_INFINITY, |f| f + ABOVE_FLOOR_DB));
    let mut yin = Yin::new(planner, sr, frame, YIN_MIN_HZ, YIN_MAX_HZ);
    let track: Vec<Option<f64>> = starts
        .iter()
        .zip(&lv)
        .filter(|(_, l)| **l >= gate)
        .map(|(&s, _)| yin.estimate(&x[s..s + frame]))
        .collect();
    let v: Vec<f64> = track.iter().filter_map(|f| *f).collect();
    let share = v.len() as f64 / track.len().max(1) as f64;
    let pitched = if track.len() <= 2 {
        !v.is_empty()
    } else {
        v.len() >= 2 && share >= MIN_VOICED
    };
    (pitched.then(|| median(&v)), share, track)
}

/// Keys of the stable notes (runs of at least 80 ms on one nearest note), when two of them differ by other than
/// whole octaves.
fn notes(track: &[Option<f64>]) -> Vec<u8> {
    let need = libm::round(NOTE_RUN_S / YIN_HOP_S) as usize;
    let mut keys: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < track.len() {
        let Some(f) = track[i] else {
            i += 1;
            continue;
        };
        let k = to_key(f);
        let s = i;
        while i < track.len() && track[i].is_some_and(|g| to_key(g) == k) {
            i += 1;
        }
        if i - s >= need && keys.last() != Some(&k) {
            keys.push(k);
        }
    }
    let distinct = keys.iter().any(|a| {
        keys.iter()
            .any(|b| (i32::from(*a) - i32::from(*b)).rem_euclid(12) != 0)
    });
    if distinct { keys } else { Vec::new() }
}

/// Onsets after the first (clip time), by the super flux of Böck and Widmer ("Maximum filter vibrato suppression
/// for onset detection", 2013), on frames of at least four periods of the note, so that a low note's neighbouring
/// harmonics are resolved and do not flicker as the frame slides over its period: each bin's
/// log-magnitude rise over the largest of its three neighbours two frames back,
/// averaged over the bins from 30 Hz to 8 kHz. A new note over a held one adds partials where there were none, so it
/// shows here where the level barely moves; beating and vibrato barely show. A peak is an onset when it is the
/// largest within ±15 ms, stands [`FLUX_RISE`] above the median of the surrounding 200 ms, the level rises at least
/// [`LEVEL_RISE_DB`] across it (a note-off, a reopening filter or vibrato add partials but no energy), its frame is
/// loud (within 30 dB of the loudest and 10 dB above the floor), and it comes at least 100 ms after the previous
/// onset.
fn onsets(
    clip: &[f32],
    sr: f64,
    on: usize,
    sig_end: usize,
    floor_db: Option<f64>,
    f0: Option<f64>,
    planner: &mut FftPlanner,
) -> Vec<f64> {
    let frame_s = f0.map_or(FLUX_FRAME_S, |f| FLUX_FRAME_S.max(FLUX_PERIODS / f));
    let n = ((sr * frame_s) as usize).next_power_of_two().max(64);
    let hop = (libm::round(sr * FLUX_HOP_S) as usize).max(1);
    let peak = clip.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak <= 0.0 {
        return Vec::new();
    }
    let fft = planner.forward(n);
    let win = hann(n);
    let bin = sr / n as f64;
    let k0 = (30.0 / bin).ceil().max(1.0) as usize;
    let k1 = ((FLUX_TOP_HZ / bin) as usize).min(n / 2 - 1);
    if k1 <= k0 + 2 {
        return Vec::new();
    }
    // A full-scale sine reads 1 before the log: |X| = A·n/4 with a Hann window.
    let scale = f64::from(LOG_GAIN) * 4.0 / (n as f64 * f64::from(peak));
    let mut buf = vec![0.0f32; n];
    let mut spec = vec![Complex32::new(0.0, 0.0); fft.bins()];
    let mut scratch = fft.make_scratch();
    let pre = prefix_squares(clip);
    let mut rows: Vec<Vec<f64>> = Vec::new();
    let mut lv: Vec<f64> = Vec::new();
    let mut t0 = 0usize;
    while t0 < sig_end.max(on + 1) {
        buf.fill(0.0);
        let e = (t0 + n).min(clip.len());
        for (i, v) in clip[t0.min(e)..e].iter().enumerate() {
            buf[i] = v * win[i];
        }
        fft.forward(&mut buf, &mut spec, &mut scratch);
        rows.push(
            spec[k0 - 1..=k1 + 1]
                .iter()
                .map(|c| {
                    let m = libm::sqrt(
                        f64::from(c.re) * f64::from(c.re) + f64::from(c.im) * f64::from(c.im),
                    );
                    libm::log(1.0 + scale * m)
                })
                .collect(),
        );
        lv.push(rms_db(&pre, t0.min(clip.len()), e));
        t0 += hop;
    }
    let flux = superflux(&rows, k1 - k0 + 1, FLUX_LAG);
    let top = lv.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let gate = (top - LOUD_DB).max(floor_db.map_or(f64::NEG_INFINITY, |f| f + ABOVE_FLOOR_DB));
    let hop_s = hop as f64 / sr;
    let near = (libm::round(0.015 / hop_s) as usize).max(1);
    let ctx = libm::round(0.1 / hop_s) as usize;
    let gap = libm::round(GAP_S / hop_s) as usize;
    let first = on / hop;
    let mut out = Vec::new();
    let mut prev = first;
    for t in first + gap..flux.len() {
        if t < prev + gap || lv[t] < gate {
            continue;
        }
        let (a, b) = (t.saturating_sub(near), (t + near).min(flux.len() - 1));
        if flux[a..=b].iter().any(|&f| f > flux[t]) {
            continue;
        }
        let (c, d) = (t.saturating_sub(ctx), (t + ctx).min(flux.len() - 1));
        // A new note adds energy: the level just after is above the level just before. A note-off, a filter that
        // reopens or vibrato add partials without that.
        let after = lv[t..=(t + 2).min(lv.len() - 1)]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let before = lv[t.saturating_sub(3)];
        if flux[t] >= median(&flux[c..=d]) + FLUX_RISE && after - before >= LEVEL_RISE_DB {
            out.push((t * hop + n / 2) as f64 / sr);
            prev = t;
        }
    }
    out
}

/// Per frame, the mean over `bins` of each bin's rise over the largest of its neighbours `lag` frames back (each row
/// holds the bins with one neighbour either side).
fn superflux(rows: &[Vec<f64>], bins: usize, lag: usize) -> Vec<f64> {
    (0..rows.len())
        .map(|t| {
            if t < lag {
                return 0.0;
            }
            let (cur, old) = (&rows[t], &rows[t - lag]);
            (1..=bins)
                .map(|j| (cur[j] - old[j - 1].max(old[j]).max(old[j + 1])).max(0.0))
                .sum::<f64>()
                / bins as f64
        })
        .collect()
}

/// A harmonic series fitted to spectral peaks: its fundamental and which peaks it explains.
struct Series {
    f0: f64,
    power: f64,
    matched: Vec<bool>,
}

/// Peaks within tolerance of harmonic `h` (1 to 16) of `f0`: 0.6 bin, or 1.2 % (about 20 ct: detune and stretched
/// partials), never so wide that neighbouring harmonics overlap (the rule of `pitch::tonal_pitch`).
fn harmonic_of(p: f64, f0: f64, bin: f64) -> Option<f64> {
    let h = libm::round(p / f0);
    if !(1.0..=MAX_HARMONIC).contains(&h) {
        return None;
    }
    let tol = (0.6 * bin).max(0.012 * h * f0).min(0.25 * f0);
    ((p - h * f0).abs() <= tol).then_some(h)
}

fn score(peaks: &[Peak], f0: f64, bin: f64) -> Series {
    let (mut power, mut num, mut den) = (0.0, 0.0, 0.0);
    let matched: Vec<bool> = peaks
        .iter()
        .map(|p| match harmonic_of(p.hz, f0, bin) {
            Some(h) => {
                power += p.power;
                num += p.power * h * p.hz;
                den += p.power * h * h;
                true
            }
            None => false,
        })
        .collect();
    Series {
        f0: if den > 0.0 { num / den } else { f0 },
        power,
        matched,
    }
}

/// The series that explains the most peak power; among those within 90 % of the best, the highest fundamental
/// (so a series is never read an octave or more low because a subharmonic explains the same peaks).
fn best_series(peaks: &[Peak], bin: f64) -> Option<Series> {
    let mut idx: Vec<usize> = (0..peaks.len()).collect();
    idx.sort_by(|&a, &b| peaks[b].power.total_cmp(&peaks[a].power).then(a.cmp(&b)));
    let mut cands: Vec<f64> = Vec::new();
    for &i in idx.iter().take(8) {
        for h in 1..=MAX_HARMONIC as usize {
            let f = peaks[i].hz / h as f64;
            if (SERIES_RANGE_HZ.0..=SERIES_RANGE_HZ.1).contains(&f) {
                cands.push(f);
            }
        }
    }
    let scored: Vec<(f64, f64)> = cands
        .iter()
        .map(|&f| (f, score(peaks, f, bin).power))
        .collect();
    let best = scored.iter().map(|s| s.1).fold(0.0, f64::max);
    if best <= 0.0 {
        return None;
    }
    let f = scored
        .iter()
        .filter(|s| s.1 >= 0.9 * best)
        .map(|s| s.0)
        .fold(0.0, f64::max);
    let first = score(peaks, f, bin);
    // Re-score at the refined fundamental, so `matched` agrees with it.
    Some(score(peaks, first.f0, bin))
}

/// Whether `a` and `b` are within 3 % of a ratio of 1 to 8 (either way).
fn related(a: f64, b: f64) -> bool {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let r = hi / lo;
    let k = libm::round(r);
    (1.0..=8.0).contains(&k) && (r - k).abs() <= 0.03 * k
}

/// Spectral peaks of `x[a..b]`: local maxima within 50 dB of the strongest, 30 Hz to 5 kHz, at most 30 strongest,
/// frequency by parabolic interpolation, power over the peak bin and 2 bins either side; the bin width, and the
/// power of the whole band.
fn peaks(
    x: &[f32],
    sr: f64,
    a: usize,
    b: usize,
    planner: &mut FftPlanner,
) -> (Vec<Peak>, f64, f64) {
    let n = ((sr * CHORD_FRAME_S) as usize).next_power_of_two().max(512);
    let (pw, _) = welch(planner, x, a, b, n);
    let bin = sr / n as f64;
    let lo = (PEAK_RANGE_HZ.0 / bin).ceil().max(2.0) as usize;
    let hi = ((PEAK_RANGE_HZ.1.min(sr / 2.0 - 2.0 * bin)) / bin) as usize;
    if hi <= lo + 2 || hi + 2 >= pw.len() {
        return (Vec::new(), bin, 0.0);
    }
    let band: f64 = pw[lo..=hi].iter().sum();
    let top = pw[lo..=hi].iter().copied().fold(0.0, f64::max);
    if top <= 0.0 {
        return (Vec::new(), bin, band);
    }
    let mut ps: Vec<Peak> = (lo..=hi)
        .filter(|&k| {
            pw[k] >= top * 1e-5
                && pw[k] > pw[k - 1]
                && pw[k] >= pw[k + 1]
                && pw[k] >= pw[k - 2]
                && pw[k] >= pw[k + 2]
        })
        .map(|k| {
            let (l, c, r) = (
                libm::log(pw[k - 1].max(1e-30)),
                libm::log(pw[k].max(1e-30)),
                libm::log(pw[k + 1].max(1e-30)),
            );
            let d = l - 2.0 * c + r;
            let shift = if d.abs() > 1e-12 {
                (0.5 * (l - r) / d).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            Peak {
                hz: (k as f64 + shift) * bin,
                power: pw[k - 2..=k + 2].iter().sum(),
            }
        })
        .collect();
    ps.sort_by(|p, q| q.power.total_cmp(&p.power).then(p.hz.total_cmp(&q.hz)));
    ps.truncate(MAX_PEAKS);
    (ps, bin, band)
}

/// The spectral peaks of the held part (from 50 ms after the onset, when it is long enough, to the note-off or the
/// signal end), the bin width, and the power of the whole 30 Hz–5 kHz band.
struct Held {
    peaks: Vec<Peak>,
    bin: f64,
    band: f64,
}

impl Held {
    /// The peaks' share of the band's power.
    fn share(&self) -> f64 {
        self.peaks.iter().map(|p| p.power).sum::<f64>() / self.band.max(1e-30)
    }

    /// Tonal enough to read pitches from: the peaks hold at least [`MIN_TONAL`] of the band (noise has peaks too).
    fn tonal(&self) -> bool {
        self.band > 0.0 && self.share() >= MIN_TONAL
    }
}

fn held_part(clip: &[f32], sr: f64, on: usize, end: usize, planner: &mut FftPlanner) -> Held {
    let skip = (sr * CHORD_SKIP_S) as usize;
    let end = end.min(clip.len());
    let a = if end > on + 4 * skip { on + skip } else { on };
    let (peaks, bin, band) = peaks(clip, sr, a, end, planner);
    Held { peaks, bin, band }
}

/// The chord's tones (Hz, ascending), or empty for a single pitch (or no harmonic content).
fn chord(held: &Held) -> Vec<f64> {
    let (ps, bin) = (&held.peaks[..], held.bin);
    if ps.len() < 2 || !held.tonal() {
        return Vec::new();
    }
    let total: f64 = ps.iter().map(|p| p.power).sum();
    let strongest = ps[0].power;
    let Some(s1) = best_series(ps, bin) else {
        return Vec::new();
    };
    // A second, unrelated series in what the first leaves.
    let rest: Vec<Peak> = ps
        .iter()
        .zip(&s1.matched)
        .filter(|(_, m)| !**m)
        .map(|(p, _)| *p)
        .collect();
    let rest_power: f64 = rest.iter().map(|p| p.power).sum();
    if rest_power >= 0.15 * total
        && let Some(s2) = best_series(&rest, bin)
        && s2.power >= 0.10 * total
        && !related(s1.f0, s2.f0)
    {
        let mut tones = vec![s1.f0, s2.f0];
        tones.sort_by(f64::total_cmp);
        return tones;
    }
    // A virtual pitch: no fundamental, no second harmonic, and gaps in the harmonics from the lowest present.
    let strong = |p: &Peak| p.power >= 0.01 * strongest;
    let present = |h: f64, min: f64| {
        ps.iter()
            .any(|p| p.power >= min && harmonic_of(p.hz, s1.f0, bin) == Some(h))
    };
    let Some(h_low) = (1..=MAX_HARMONIC as usize)
        .map(|h| h as f64)
        .find(|&h| present(h, 0.01 * strongest))
    else {
        return Vec::new();
    };
    if h_low < 3.0 {
        return Vec::new();
    }
    let window: Vec<f64> = (0..=8)
        .map(|k| h_low + f64::from(k))
        .filter(|h| h * s1.f0 <= PEAK_RANGE_HZ.1 && *h <= MAX_HARMONIC)
        .collect();
    let hit = window
        .iter()
        .filter(|&&h| present(h, 1e-4 * strongest))
        .count();
    if window.len() < 4 || hit as f64 >= 0.8 * window.len() as f64 {
        return Vec::new();
    }
    // The tones: the strong peaks, lowest first, without harmonics of a tone already kept.
    let mut lows: Vec<f64> = ps.iter().filter(|p| strong(p)).map(|p| p.hz).collect();
    lows.sort_by(f64::total_cmp);
    let mut tones: Vec<f64> = Vec::new();
    for f in lows {
        if !tones.iter().any(|&t| f > t * 1.5 && related(t, f))
            && !tones.iter().any(|&t| (f / t - 1.0).abs() < 0.03)
        {
            tones.push(f);
        }
        if tones.len() == 4 {
            break;
        }
    }
    if tones.len() < 2 { Vec::new() } else { tones }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::FftPath;
    use crate::test_signals::{SR, ToyPatch, ToyPerf, Wave, mix, render_toy};
    use crate::units::midi_to_hz;

    fn run(x: &[f32]) -> (InferredPerf, Findings) {
        let mut pl = FftPlanner::new(FftPath::Scalar);
        infer(x, SR, 0, x.len(), None, &mut pl)
    }

    fn toy(wave: Wave, midi: f64, detune: f64) -> Vec<f32> {
        let p = ToyPatch {
            wave,
            detune_ct: detune,
            cutoff_hz: 5000.0,
            sustain_db: -6.0,
            ..ToyPatch::default()
        };
        let perf = ToyPerf {
            note_hz: midi_to_hz(midi),
            dur_s: 1.5,
            hold_s: 1.0,
        };
        render_toy(SR, &p, &perf, (0.0, 0.37), 0.0)
    }

    #[test]
    fn single_notes_read_their_key_and_no_chord() {
        for wave in Wave::ALL {
            for midi in [28.0, 40.0, 52.0, 64.0, 76.0, 88.0] {
                for detune in [0.0, 15.0] {
                    let x = toy(wave, midi, detune);
                    let (p, f) = run(&x);
                    assert_eq!(p.key, midi as u8, "{wave:?} {midi} {detune}: {p:?}");
                    assert!(f.chord.is_empty(), "{wave:?} {midi} {detune}: {f:?}");
                    assert!(f.notes.is_empty() && !f.unpitched, "{wave:?} {midi}: {f:?}");
                    assert_eq!(p.onsets_s.len(), 1, "{wave:?} {midi}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn triads_and_clusters_raise_a_chord() {
        for wave in Wave::ALL {
            for (root, shape) in [
                (48.0, [0.0, 4.0, 7.0]),
                (57.0, [0.0, 3.0, 7.0]),
                (52.0, [0.0, 4.0, 10.0]),
                (60.0, [0.0, 1.0, 6.0]),
            ] {
                let mut x: Vec<f32> = Vec::new();
                for s in shape {
                    x = mix(&x, &toy(wave, root + s, 0.0));
                }
                let (_, f) = run(&x);
                assert!(f.chord.len() >= 2, "{wave:?} {root} {shape:?}: {f:?}");
            }
        }
    }

    #[test]
    fn a_second_note_is_a_phrase() {
        let a = toy(Wave::Saw, 48.0, 0.0);
        let b = toy(Wave::Saw, 55.0, 0.0);
        let mut x = a.clone();
        x.resize(a.len() + (0.6 * SR) as usize, 0.0);
        let x = mix(&x, &[vec![0.0; (0.6 * SR) as usize], b].concat());
        let (p, f) = run(&x);
        assert_eq!(p.onsets_s.len(), 2, "{p:?}");
        assert!((p.onsets_s[1] - 0.6).abs() < 0.015, "{p:?}");
        // the overlap reads as the fifth's common period (C2), which is what one note would have to play there
        assert!(
            f.notes.first() == Some(&48) && f.notes.last() == Some(&55),
            "{f:?}"
        );
        assert_eq!(p.key, 48);
    }

    #[test]
    fn noise_is_unpitched_and_the_note_off_is_found() {
        let n = crate::test_signals::white_noise(SR, 0.3, 1.0, 7);
        let (p, f) = run(&n);
        assert!(
            f.unpitched && p.f0_hz.is_none() && p.key == UNPITCHED_KEY,
            "{p:?}"
        );
        assert!(f.chord.is_empty() && p.onsets_s.len() == 1, "{p:?} {f:?}");
        let x = toy(Wave::Saw, 45.0, 0.0);
        let (p, _) = run(&x);
        let off = p.off_s.expect("a note-off");
        assert!((off - 1.0).abs() < 0.03, "{p:?}");
        assert!((p.hold_s - off).abs() < 1e-12 && (p.velocity - VELOCITY).abs() < 1e-9);
    }
}
