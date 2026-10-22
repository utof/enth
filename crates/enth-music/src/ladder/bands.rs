//! **Owner: lane G3** (ladder file; level 1 is lane MUS's). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/ear.md, "The games".
//!
//! The ladder of **Frequency finder**: noise or a saw through an svf band-pass or a resonant low-pass peak, pick the
//! band, later place it on a log slider; then the EQ boost or cut on `enth-dsp`'s biquad (lane BQ). Bands go from 2 to
//! 10 (octaves, then thirds), Q and gain fall, and the source goes noise, saw, patch, chord. Nothing under 80 Hz below
//! level 5, for small monitors.
//!
//! **Level 1 (lane MUS): two bands three octaves apart or more, in the svf form.** Sustained pink noise
//! ([`timbre::NOISE`]) through the synth's band-pass ([`EqKind::BandPass`], Q 8) at one of 125, 250, 500, 2000, 4000 or 8000 Hz,
//! the question naming which of two bands (at least a ratio of 8 apart) it is. The clip is one note held 2.5 s, the key
//! only a placeholder for the renderer. Options are the two centres in ascending order.
//!
//! **Draw order** (part of the frozen form): the answer's band, the distractor among the bands at least `min_ratio`
//! away.

use crate::ear::{
    Answer, EqBand, EqKind, OptionKind, Options, Question, below, clip_length_ms, note, pick,
    timbre,
};
use enth_dsp::util::rng::Rng;

/// One row of the ladder.
#[derive(Clone, Copy, Debug)]
pub struct Level {
    /// The band centres in Hz the answer and the distractors come from.
    pub pool: &'static [u32],
    /// How many options a question lists.
    pub options: usize,
    /// The least ratio between any two options' centres (8 is three octaves).
    pub min_ratio: u32,
    /// The form of the band.
    pub kind: EqKind,
    /// The quality factor times 1000.
    pub q_milli: u16,
    /// The gain in hundredths of a dB (0 for the svf forms).
    pub gain_cdb: i16,
    /// The source.
    pub timbre: u8,
    /// How long the source is held, in ms.
    pub hold_ms: u32,
}

/// The ladder, level `n` at index `n - 1`.
pub const LEVELS: [Level; 1] = [Level {
    pool: &[125, 250, 500, 2000, 4000, 8000],
    options: 2,
    min_ratio: 8,
    kind: EqKind::BandPass,
    q_milli: 8000,
    gain_cdb: 0,
    timbre: timbre::NOISE,
    hold_ms: 2500,
}];

/// The number of levels.
pub const MAX_LEVEL: u8 = LEVELS.len() as u8;

/// A question at `level` (1 to [`MAX_LEVEL`]; the caller checks), drawing from `rng`.
pub fn question(level: u8, rng: &mut Rng) -> Question {
    let l = &LEVELS[usize::from(level.clamp(1, MAX_LEVEL)) - 1];
    let band = pick(rng, l.pool);
    let mut chosen = vec![band];
    while chosen.len() < l.options {
        let candidates: Vec<u32> = l
            .pool
            .iter()
            .copied()
            .filter(|f| {
                chosen
                    .iter()
                    .all(|c| (*c).max(*f) >= l.min_ratio * (*c).min(*f))
            })
            .collect();
        if candidates.is_empty() {
            break;
        }
        chosen.push(candidates[below(rng, candidates.len())]);
    }
    chosen.sort_unstable();
    let index = chosen.iter().position(|f| *f == band).unwrap_or(0);
    let notes = vec![note(60, 100, 0, l.hold_ms)];
    Question {
        timbre: l.timbre,
        length_ms: clip_length_ms(&notes),
        notes,
        post: Some(EqBand {
            kind: l.kind,
            freq_hz: band as u16,
            gain_cdb: l.gain_cdb,
            q_milli: l.q_milli,
        }),
        answer: Answer::Choice(index as u8),
        options: Options {
            kind: OptionKind::Band,
            ids: chosen,
        },
    }
}
