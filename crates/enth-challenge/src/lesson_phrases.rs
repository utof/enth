//! **Owner: lane CV2** (Challenge v2, freeze point F1l; the F1l judge reviews it). Pre-declared by lane S0l (R2).
//! The reference is docs/challenge.md, "Challenge v2" ("Phrases: the generator's and the lessons'").
//!
//! The lesson-only phrases: ids 4 and up, for lessons whose controls the generator's three phrases cannot reveal
//! (content/lessons/PLAN.md §8.1; the open question Q8, settled here):
//!
//! | Id | Name | Notes (key, velocity, start–end ms) | Length | Lessons |
//! | --- | --- | --- | --- | --- |
//! | 4 | `chord` | C3, E3, G3, B3 at 100, 0–1500 | 3000 ms | 6.1, 8.2; soft 7.3 |
//! | 5 | `long_hold` | C3 at 100, 0–8000 | 10000 ms | 5.3, 5.4; soft 9.3, 9.6 |
//! | 6 | `staccato` | C3 at 100, 0–150 | 6000 ms | 7.1, 7.2 |
//! | 7 | `staccato_run` | C3 at 100 four times, 120 ms each, one every 400 ms | 3320 ms | soft 7.3, 7.4 |
//!
//! **The table is split** (Q8): [`crate::phrases::TABLE`] keeps the generator's phrases (ids 1–3) and their shared
//! facts (every hold and tail at least 1.5 s, the lowest key C3), which generator v1's priors rely on, so nothing the
//! generator reads moves. These phrases break the hold fact on purpose (`staccato` holds for 150 ms) and are never
//! picked by the generator: its `PHRASE_WEIGHTS` name ids 1–3 only (tested). They have invariants of their own,
//! [`MIN_HOLD_MS`], [`MAX_LENGTH_MS`] and [`MAX_AT_ONCE`] below, plus the two facts every phrase shares: the lowest key
//! is C3 ([`REFERENCE_KEY`]) and the tail is at least [`MIN_TAIL_MS`].
//!
//! Ids are one sequence across both tables, append-only: a released phrase never changes, a new phrase is a new id
//! (here or, for a new generator version, in the generator's table), and a name is never reused.
//! [`crate::phrases::entry`] and [`crate::phrases::phrase`] look in both tables, so a lesson challenge's phrase
//! resolves wherever a generated one does (the session, the plugin's worker, the CLI).

use crate::model::{Phrase, PhraseId, PhraseNote};
use crate::phrases::{Entry, MIN_TAIL_MS, REFERENCE_KEY};

/// C3, E3, G3 and B3 together at velocity 100, held 1.5 s in a 3 s render: poly against mono, voice stealing, and
/// a chord that clips (lessons 6.1 and 8.2).
pub const CHORD: PhraseId = PhraseId(4);
/// C3 at velocity 100 held 8 s in a 10 s render: two or more cycles of an LFO at 0.3 to 0.7 Hz (lessons 5.3, 5.4).
pub const LONG_HOLD: PhraseId = PhraseId(5);
/// C3 at velocity 100 held 150 ms in a 6 s render: a reverb's tail on a decaying sound (lessons 7.1, 7.2).
pub const STACCATO: PhraseId = PhraseId(6);
/// C3 at velocity 100 four times, each held 120 ms, one every 400 ms, then a 2 s tail: transients for a compressor
/// (lessons 7.3, 7.4).
pub const STACCATO_RUN: PhraseId = PhraseId(7);

/// Every note of a lesson phrase is held at least this long: 4,800 frames at the scoring rate, so a note is far
/// more than the ~1 ms click ramp and the descriptors see a body. (`staccato_run`'s 120 ms is the shortest.)
pub const MIN_HOLD_MS: u32 = 100;

/// No lesson phrase renders longer than this: the sound scorer prepares about 19 renders of the phrase, so a 10 s
/// phrase costs about 3.3 times the held note (`long_hold` is the longest).
pub const MAX_LENGTH_MS: u32 = 10_000;

/// No lesson phrase holds more notes at once than this: the starter's polyphony, so a phrase never steals a voice
/// by itself under the default settings.
pub const MAX_AT_ONCE: usize = 8;

const fn note(key: u8, velocity: u8, start_ms: u32, hold_ms: u32) -> PhraseNote {
    PhraseNote {
        key,
        velocity,
        start_ms,
        hold_ms,
    }
}

/// The lesson-only table, in id order; ids follow the generator's table without a gap.
pub const TABLE: [Entry; 4] = [
    Entry {
        id: CHORD,
        name: "chord",
        phrase: Phrase {
            notes: &[
                note(REFERENCE_KEY, 100, 0, 1500),
                note(REFERENCE_KEY + 4, 100, 0, 1500),
                note(REFERENCE_KEY + 7, 100, 0, 1500),
                note(REFERENCE_KEY + 11, 100, 0, 1500),
            ],
            length_ms: 3000,
        },
    },
    Entry {
        id: LONG_HOLD,
        name: "long_hold",
        phrase: Phrase {
            notes: &[note(REFERENCE_KEY, 100, 0, 8000)],
            length_ms: 10_000,
        },
    },
    Entry {
        id: STACCATO,
        name: "staccato",
        phrase: Phrase {
            notes: &[note(REFERENCE_KEY, 100, 0, 150)],
            length_ms: 6000,
        },
    },
    Entry {
        id: STACCATO_RUN,
        name: "staccato_run",
        phrase: Phrase {
            notes: &[
                note(REFERENCE_KEY, 100, 0, 120),
                note(REFERENCE_KEY, 100, 400, 120),
                note(REFERENCE_KEY, 100, 800, 120),
                note(REFERENCE_KEY, 100, 1200, 120),
            ],
            length_ms: 3320,
        },
    },
];

/// The lesson table's entry with this id (the generator's ids are not here: [`crate::phrases::entry`] looks in both).
pub fn entry(id: PhraseId) -> Option<&'static Entry> {
    TABLE.iter().find(|e| e.id == id)
}

/// Whether `id` is a lesson-only phrase: one generator v1 never picks.
pub fn is_lesson_phrase(id: PhraseId) -> bool {
    entry(id).is_some()
}

/// Why a lesson phrase breaks its invariants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LessonPhraseError {
    /// It fails [`Phrase::validate`].
    Invalid(crate::model::PhraseError),
    /// Its lowest key is not C3.
    LowestKey,
    /// Its notes are not in start order.
    Unordered,
    /// A note is held less than [`MIN_HOLD_MS`].
    HoldTooShort {
        /// The note.
        index: usize,
    },
    /// The tail after the last note-off is under [`MIN_TAIL_MS`].
    TailTooShort,
    /// The render is longer than [`MAX_LENGTH_MS`].
    TooLong,
    /// More than [`MAX_AT_ONCE`] notes sound together.
    TooManyAtOnce,
}

/// Check a lesson phrase against its invariants (see the [module docs](self)).
pub fn check(phrase: &Phrase) -> Result<(), LessonPhraseError> {
    use crate::phrases::PhraseFacts;
    phrase.validate().map_err(LessonPhraseError::Invalid)?;
    if phrase.lowest_key() != REFERENCE_KEY {
        return Err(LessonPhraseError::LowestKey);
    }
    if !phrase
        .notes
        .windows(2)
        .all(|w| w[0].start_ms <= w[1].start_ms)
    {
        return Err(LessonPhraseError::Unordered);
    }
    if let Some(index) = phrase.notes.iter().position(|n| n.hold_ms < MIN_HOLD_MS) {
        return Err(LessonPhraseError::HoldTooShort { index });
    }
    if phrase.tail_ms() < MIN_TAIL_MS {
        return Err(LessonPhraseError::TailTooShort);
    }
    if phrase.length_ms > MAX_LENGTH_MS {
        return Err(LessonPhraseError::TooLong);
    }
    if phrase.max_simultaneous() > MAX_AT_ONCE {
        return Err(LessonPhraseError::TooManyAtOnce);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phrases::{self, PhraseFacts};

    #[test]
    fn ids_follow_the_generator_table_and_names_are_unique() {
        let ids: Vec<u16> = TABLE.iter().map(|e| e.id.0).collect();
        assert_eq!(ids, [4, 5, 6, 7], "append-only, after the generator's 1-3");
        let last_generator = phrases::TABLE.iter().map(|e| e.id.0).max().unwrap();
        assert_eq!(TABLE[0].id.0, last_generator + 1, "one sequence, no gap");
        let names: Vec<&str> = phrases::TABLE
            .iter()
            .chain(TABLE.iter())
            .map(|e| e.name)
            .collect();
        assert_eq!(
            names,
            [
                "held",
                "octave_pair",
                "legato_pair",
                "chord",
                "long_hold",
                "staccato",
                "staccato_run"
            ]
        );
        for e in &TABLE {
            assert_eq!(phrases::entry(e.id), Some(e), "{}", e.name);
            assert_eq!(phrases::phrase(e.id), Some(&e.phrase));
            assert!(is_lesson_phrase(e.id));
        }
        for e in &phrases::TABLE {
            assert!(!is_lesson_phrase(e.id), "{}", e.name);
        }
    }

    /// The lesson phrases' own invariants hold for every entry (Q8).
    #[test]
    fn every_lesson_phrase_meets_its_invariants() {
        for e in &TABLE {
            assert_eq!(check(&e.phrase), Ok(()), "{}", e.name);
        }
    }

    /// The generator never picks a lesson phrase: its weights name the generator's table only.
    #[test]
    fn the_generator_never_picks_a_lesson_phrase() {
        for (id, _) in crate::priors::v1::PHRASE_WEIGHTS {
            let id = PhraseId(id as u16);
            assert!(
                phrases::TABLE.iter().any(|e| e.id == id),
                "{id:?} is a generator phrase"
            );
            assert!(!is_lesson_phrase(id));
        }
    }

    #[test]
    fn what_each_lesson_phrase_reveals() {
        let [chord, long, staccato, run] = TABLE.map(|e| e.phrase);
        assert_eq!(chord.max_simultaneous(), 4);
        assert!(chord.has_pitch_change() && !chord.has_two_velocities());
        assert_eq!(long.min_hold_ms(), 8000);
        assert_eq!(long.tail_ms(), 2000);
        assert_eq!(staccato.min_hold_ms(), 150);
        assert_eq!(staccato.tail_ms(), 5850);
        assert_eq!(run.notes.len(), 4);
        assert!(!run.has_pitch_change());
        assert_eq!(run.max_simultaneous(), 1);
        assert_eq!(run.tail_ms(), 2000);
    }

    #[test]
    fn the_checks_catch_each_broken_invariant() {
        static SHORT: [PhraseNote; 1] = [note(48, 100, 0, 50)];
        static HELD: [PhraseNote; 1] = [note(48, 100, 0, 1500)];
        static HIGH: [PhraseNote; 1] = [note(60, 100, 0, 500)];
        static BACKWARDS: [PhraseNote; 2] = [note(48, 100, 500, 200), note(48, 100, 0, 200)];
        static NINE: [PhraseNote; 9] = [
            note(48, 100, 0, 500),
            note(49, 100, 0, 500),
            note(50, 100, 0, 500),
            note(51, 100, 0, 500),
            note(52, 100, 0, 500),
            note(53, 100, 0, 500),
            note(54, 100, 0, 500),
            note(55, 100, 0, 500),
            note(56, 100, 0, 500),
        ];
        let p = |notes: &'static [PhraseNote], length_ms| Phrase { notes, length_ms };
        assert_eq!(
            check(&p(&SHORT, 3000)),
            Err(LessonPhraseError::HoldTooShort { index: 0 })
        );
        assert_eq!(check(&p(&HIGH, 3000)), Err(LessonPhraseError::LowestKey));
        assert_eq!(
            check(&p(&BACKWARDS, 3000)),
            Err(LessonPhraseError::Unordered)
        );
        assert_eq!(
            check(&p(&NINE, 3000)),
            Err(LessonPhraseError::TooManyAtOnce)
        );
        assert_eq!(
            check(&p(&HIGH[..0], 3000)),
            Err(LessonPhraseError::Invalid(
                crate::model::PhraseError::NoNotes
            ))
        );
        assert_eq!(check(&p(&HELD, 2000)), Err(LessonPhraseError::TailTooShort));
        assert_eq!(check(&p(&HELD, 12_000)), Err(LessonPhraseError::TooLong));
    }

    #[test]
    fn the_performances_are_exact() {
        let run = phrases::phrase(STACCATO_RUN).unwrap().performance();
        assert_eq!(run.duration_s, 3.32);
        assert_eq!(run.notes.len(), 4);
        assert_eq!(run.notes[3].on_s, 1.2);
        assert_eq!(run.notes[3].off_s, 1.32);
        let chord = phrases::phrase(CHORD).unwrap().performance();
        let keys: Vec<u8> = chord.notes.iter().map(|n| n.key).collect();
        assert_eq!(keys, [48, 52, 55, 59]);
    }
}
