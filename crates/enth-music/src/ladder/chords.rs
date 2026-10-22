//! **Owner: lane G2** (ladder file; level 1 is lane MUS's). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/ear.md, "The games".
//!
//! The ladder of **Chord quality**: a chord on a pad, naming its quality. Major and minor first (with a note that some
//! people plateau), then diminished and augmented, sus, sevenths and inversions. No content is ever gated on it.
//!
//! **Level 1 (lane MUS): major or minor.** A root-position triad on a pad, an organ or a saw, root anywhere in C3 to
//! C4, the three notes struck together. Options are listed major then minor.
//!
//! **Draw order** (part of the frozen form): `pick_options`, the root, the timbre.

use crate::ear::{
    Answer, OptionKind, Options, Question, clip_length_ms, note, pick, pick_options, timbre,
};
use crate::theory::{self, MAJOR, MINOR};
use enth_dsp::util::rng::Rng;

/// One row of the ladder.
#[derive(Clone, Copy, Debug)]
pub struct Level {
    /// The chord qualities (indices into [`theory::CHORDS`]) the answer and the distractors come from.
    pub pool: &'static [u32],
    /// How many options a question lists.
    pub options: usize,
    /// The lowest and highest root key (inclusive).
    pub root: (u8, u8),
    /// The timbres the question rotates over.
    pub timbres: &'static [u8],
    /// How long the chord is held, in ms.
    pub hold_ms: u32,
    /// The velocity.
    pub velocity: u8,
}

/// The ladder, level `n` at index `n - 1`.
pub const LEVELS: [Level; 1] = [Level {
    pool: &[MAJOR as u32, MINOR as u32],
    options: 2,
    root: (48, 60),
    timbres: &[timbre::PAD, timbre::ORGAN, timbre::SAW_LOW_PASS],
    hold_ms: 2500,
    velocity: 90,
}];

/// The number of levels.
pub const MAX_LEVEL: u8 = LEVELS.len() as u8;

/// A question at `level` (1 to [`MAX_LEVEL`]; the caller checks), drawing from `rng`.
pub fn question(level: u8, rng: &mut Rng) -> Question {
    let l = &LEVELS[usize::from(level.clamp(1, MAX_LEVEL)) - 1];
    let (ids, answer) = pick_options(rng, l.pool, l.options);
    let quality = ids[usize::from(answer)] as u8;
    let span = u64::from(l.root.1 - l.root.0) + 1;
    let root = l.root.0 + rng.below(span) as u8;
    let timbre = pick(rng, l.timbres);
    let keys = theory::chord_keys(root, quality, 0).unwrap_or_else(|| vec![root]);
    let notes: Vec<_> = keys
        .iter()
        .map(|k| note(*k, l.velocity, 0, l.hold_ms))
        .collect();
    Question {
        timbre,
        length_ms: clip_length_ms(&notes),
        notes,
        post: None,
        answer: Answer::Choice(answer),
        options: Options {
            kind: OptionKind::Chord,
            ids,
        },
    }
}
