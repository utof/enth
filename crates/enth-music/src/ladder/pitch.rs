//! **Owner: lane G1** (ladder file; level 1 is lane MUS's). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/ear.md, "The games".
//!
//! R4b's **note naming**, opt-in and never in the default workout (Wong 2025's ladder). Honest copy only: "Most adults
//! who train for about 20 hours learn to name a handful of notes; a few learn all twelve; most stop sooner." No
//! content is gated on it.
//!
//! **Level 1 (lane MUS): two pitch classes far apart.** One note, name which of two pitch classes at least a fourth
//! apart on the circle of semitones (5 or more), the octave roving over C3 to B5 and the timbre over four. Options are
//! the two pitch classes in ascending order (C = 0).
//!
//! **Draw order** (part of the frozen form): the answer's pitch class, the distractor among the pitch classes at least
//! `min_gap` away, the octave, the timbre.

use crate::ear::timbre;
use crate::ear::{Answer, OptionKind, Options, Question, below, clip_length_ms, note, pick};
use enth_dsp::util::rng::Rng;

/// One row of the ladder.
#[derive(Clone, Copy, Debug)]
pub struct Level {
    /// The pitch classes (0 = C) the answer and the distractors come from.
    pub pool: &'static [u8],
    /// How many options a question lists.
    pub options: usize,
    /// The least circular distance, in semitones, between any two options (6 is a tritone).
    pub min_gap: u8,
    /// The lowest and highest octave number (scientific pitch notation; C4 = 60), inclusive.
    pub octaves: (u8, u8),
    /// The timbres the question rotates over.
    pub timbres: &'static [u8],
    /// How long the note is held, in ms.
    pub hold_ms: u32,
    /// The velocity.
    pub velocity: u8,
}

/// The ladder, level `n` at index `n - 1`.
pub const LEVELS: [Level; 1] = [Level {
    pool: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    options: 2,
    min_gap: 5,
    octaves: (3, 5),
    timbres: &[
        timbre::ORGAN,
        timbre::PLUCK,
        timbre::SAW_LOW_PASS,
        timbre::SINE,
    ],
    hold_ms: 1500,
    velocity: 100,
}];

/// The number of levels.
pub const MAX_LEVEL: u8 = LEVELS.len() as u8;

/// The distance between two pitch classes around the octave, 0 to 6.
const fn circular(a: u8, b: u8) -> u8 {
    let d = a.abs_diff(b);
    if d > 6 { 12 - d } else { d }
}

/// A question at `level` (1 to [`MAX_LEVEL`]; the caller checks), drawing from `rng`.
pub fn question(level: u8, rng: &mut Rng) -> Question {
    let l = &LEVELS[usize::from(level.clamp(1, MAX_LEVEL)) - 1];
    let answer_pc = pick(rng, l.pool);
    let mut chosen = vec![answer_pc];
    while chosen.len() < l.options {
        let candidates: Vec<u8> = l
            .pool
            .iter()
            .copied()
            .filter(|pc| chosen.iter().all(|c| circular(*c, *pc) >= l.min_gap))
            .collect();
        if candidates.is_empty() {
            break;
        }
        chosen.push(candidates[below(rng, candidates.len())]);
    }
    let mut ids: Vec<u32> = chosen.iter().map(|pc| u32::from(*pc)).collect();
    ids.sort_unstable();
    let index = ids
        .iter()
        .position(|id| *id == u32::from(answer_pc))
        .unwrap_or(0);
    let octave = l.octaves.0 + rng.below(u64::from(l.octaves.1 - l.octaves.0) + 1) as u8;
    let timbre = pick(rng, l.timbres);
    let notes = vec![note(
        12 * (octave + 1) + answer_pc,
        l.velocity,
        0,
        l.hold_ms,
    )];
    Question {
        timbre,
        length_ms: clip_length_ms(&notes),
        notes,
        post: None,
        answer: Answer::Choice(index as u8),
        options: Options {
            kind: OptionKind::PitchClass,
            ids,
        },
    }
}
