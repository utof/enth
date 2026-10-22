//! **Owner: lane G1** (ladder file; level 1 is lane MUS's). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/ear.md, "The games".
//!
//! The ladder of **Interval stacks**: a two-oscillator stack, then a two-note phrase, naming the interval. P1 against
//! P8, then one interval at a time (P5, P4, M3, m3, and on); harmonic, then melodic with a roving root; timbre roving.
//! Accuracy per interval feeds the `ear.interval.*` skills.
//!
//! **Level 1 (lane MUS): P1 against P8.** Harmonic, two options far apart, a root anywhere in C3 to C5, a timbre that
//! rotates over a saw, an organ and a pad. A unison is *one* note (a stack of two oscillators on one key), an octave
//! is the root and the key 12 above it, both struck together; the options are listed P1 then P8.
//!
//! **Draw order** (part of the frozen form, docs/ear.md): `pick_options` (the answer, then the distractors), the root,
//! the timbre. Add a level by adding a row to [`LEVELS`] and bumping nothing else; the dispatcher in `ear.rs` reads
//! [`MAX_LEVEL`].

use crate::ear::{
    Answer, OptionKind, Options, Question, clip_length_ms, note, pick, pick_options, timbre,
};
use enth_dsp::util::rng::Rng;

/// One row of the ladder.
#[derive(Clone, Copy, Debug)]
pub struct Level {
    /// The interval sizes in semitones the answer and the distractors come from.
    pub pool: &'static [u32],
    /// How many options a question lists.
    pub options: usize,
    /// The lowest and highest root key (inclusive); the top note must stay at or below C7.
    pub root: (u8, u8),
    /// The timbres the question rotates over.
    pub timbres: &'static [u8],
    /// How long the notes are held, in ms.
    pub hold_ms: u32,
    /// The velocity.
    pub velocity: u8,
}

/// The ladder, level `n` at index `n - 1`.
pub const LEVELS: [Level; 1] = [Level {
    pool: &[0, 12],
    options: 2,
    root: (48, 72),
    timbres: &[timbre::SAW_LOW_PASS, timbre::ORGAN, timbre::PAD],
    hold_ms: 2000,
    velocity: 100,
}];

/// The number of levels.
pub const MAX_LEVEL: u8 = LEVELS.len() as u8;

/// A question at `level` (1 to [`MAX_LEVEL`]; the caller checks), drawing from `rng`.
pub fn question(level: u8, rng: &mut Rng) -> Question {
    let l = &LEVELS[usize::from(level.clamp(1, MAX_LEVEL)) - 1];
    let (ids, answer) = pick_options(rng, l.pool, l.options);
    let size = ids[usize::from(answer)];
    let span = u64::from(l.root.1 - l.root.0) + 1;
    let root = l.root.0 + rng.below(span) as u8;
    let timbre = pick(rng, l.timbres);
    let mut notes = vec![note(root, l.velocity, 0, l.hold_ms)];
    if size > 0 {
        notes.push(note(root + size as u8, l.velocity, 0, l.hold_ms));
    }
    Question {
        timbre,
        length_ms: clip_length_ms(&notes),
        notes,
        post: None,
        answer: Answer::Choice(answer),
        options: Options {
            kind: OptionKind::Interval,
            ids,
        },
    }
}
