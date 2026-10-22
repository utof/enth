//! **Owner: lane G2** (ladder file; level 1 is lane MUS's). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/ear.md, "The games".
//!
//! R4b's **progressions**: seeded phrases in Functional Ear Trainer's order, over the voice-leading tables of
//! `theory`.
//!
//! **Level 1 (lane MUS): two progressions that part at the second chord.** A four-chord loop, `I-IV-V-I` or
//! `I-V-vi-IV`, in any of the 12 major keys, voiced with small moves ([`theory::voice_progression`]: the root in the
//! bass, three upper voices), each chord 1.2 s on a pad or an organ. Options are listed in table order.
//!
//! **Draw order** (part of the frozen form): `pick_options`, the key's pitch class, the timbre.

use crate::ear::{
    Answer, OptionKind, Options, Question, below, clip_length_ms, note, pick, pick_options, timbre,
};
use crate::theory;
use enth_dsp::util::rng::Rng;

/// One row of the ladder.
#[derive(Clone, Copy, Debug)]
pub struct Level {
    /// The progressions (indices into [`theory::PROGRESSIONS`]) the answer and the distractors come from.
    pub pool: &'static [u32],
    /// How many options a question lists.
    pub options: usize,
    /// The timbres the question rotates over.
    pub timbres: &'static [u8],
    /// How long each chord sounds, in ms.
    pub chord_ms: u32,
    /// The velocity.
    pub velocity: u8,
}

/// The ladder, level `n` at index `n - 1`.
pub const LEVELS: [Level; 1] = [Level {
    pool: &[0, 1],
    options: 2,
    timbres: &[timbre::PAD, timbre::ORGAN],
    chord_ms: 1200,
    velocity: 90,
}];

/// The number of levels.
pub const MAX_LEVEL: u8 = LEVELS.len() as u8;

/// A question at `level` (1 to [`MAX_LEVEL`]; the caller checks), drawing from `rng`.
pub fn question(level: u8, rng: &mut Rng) -> Question {
    let l = &LEVELS[usize::from(level.clamp(1, MAX_LEVEL)) - 1];
    let (ids, answer) = pick_options(rng, l.pool, l.options);
    let tonic_pc = below(rng, 12) as u8;
    let timbre = pick(rng, l.timbres);
    let id = ids[usize::from(answer)];
    let progression = theory::progression(id as u8).unwrap_or(&theory::PROGRESSIONS[0]);
    let mut notes = Vec::new();
    for (i, chord) in theory::voice_progression(tonic_pc, progression)
        .iter()
        .enumerate()
    {
        for key in chord {
            notes.push(note(*key, l.velocity, i as u32 * l.chord_ms, l.chord_ms));
        }
    }
    Question {
        timbre,
        length_ms: clip_length_ms(&notes),
        notes,
        post: None,
        answer: Answer::Choice(answer),
        options: Options {
            kind: OptionKind::Progression,
            ids,
        },
    }
}
