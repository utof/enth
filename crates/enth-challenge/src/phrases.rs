//! **Owner: lane RG** (the randomizer picks the phrase; CT's Challenge v1 refers to it by id). Pre-declared empty by
//! lane S0r.
//!
//! The phrase table (design.md §9, report 01 §2.5): the [`Phrase`]s a target is heard and scored through, by
//! [`PhraseId`] (CT's types, in [`crate::model`]). One held note for levels 1-4; at level 5, when structure is
//! randomized, the generator may pick two notes an octave apart at two velocities (keytrack and velocity amounts
//! become audible) or a legato pair (legato and glide become audible). The generator picks the phrase, so
//! observability is decided before a round starts.
//!
//! The generator reads only the data (keys, velocities, holds, tails, through [`PhraseFacts`]), never a
//! `Performance`: target generation works in parameter space and never reaches `enth-render` (RG's acceptance;
//! `tests/rg_render_free.rs`).
//!
//! **The generator's table.** [`TABLE`] holds the phrases generator v1 picks (ids 1-3); the lesson-only phrases (ids
//! 4 and up, short holds and chords) are lane CV2's [`crate::lesson_phrases`], with their own invariants (R2, Q8).
//! [`entry`] and [`phrase`] look in both.
//!
//! **Append-only.** A released phrase never changes (codes, golden challenges and duels depend on it); a new phrase
//! is a new id. Every phrase here shares two facts the v1 priors rely on: its lowest key is [`REFERENCE_KEY`] (C3),
//! and every note is held at least [`MIN_HOLD_MS`] with at least [`MIN_TAIL_MS`] after the last note-off. So the
//! envelope priors (attack up to the hold, release up to the tail) and the cutoff priors (relative to the reference
//! note) are the same for every phrase, and picking a phrase never moves a lower category's values.

use crate::model::{Phrase, PhraseId, PhraseNote};

/// The lowest key of every phrase in the table: C3 (MIDI 48, 130.81 Hz). The priors are relative to it.
pub const REFERENCE_KEY: u8 = 48;

/// Every note of every phrase in the table is held at least this long, so an attack up to it is heard to its peak.
pub const MIN_HOLD_MS: u32 = 1500;

/// Every phrase in the table ends at least this long after its last note-off, so a release up to it is heard to its
/// end.
pub const MIN_TAIL_MS: u32 = 1500;

/// One note, C3 at velocity 100, held 1.5 s in a 3 s render. Levels 1-4 always use it.
pub const HELD: PhraseId = PhraseId(1);
/// C3 at velocity 100, then C4 at 50: keytrack and velocity amounts become audible.
pub const OCTAVE_PAIR: PhraseId = PhraseId(2);
/// C3, then G3 overlapping it: legato and glide become audible.
pub const LEGATO_PAIR: PhraseId = PhraseId(3);

/// One table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its id, 1 and up, append-only.
    pub id: PhraseId,
    /// A stable name for logs and the CLI.
    pub name: &'static str,
    /// The notes and the render length.
    pub phrase: Phrase,
}

const fn note(key: u8, velocity: u8, start_ms: u32, hold_ms: u32) -> PhraseNote {
    PhraseNote {
        key,
        velocity,
        start_ms,
        hold_ms,
    }
}

/// The table, in id order.
pub const TABLE: [Entry; 3] = [
    Entry {
        id: HELD,
        name: "held",
        phrase: Phrase {
            notes: &[note(REFERENCE_KEY, 100, 0, 1500)],
            length_ms: 3000,
        },
    },
    Entry {
        id: OCTAVE_PAIR,
        name: "octave_pair",
        phrase: Phrase {
            notes: &[
                note(REFERENCE_KEY, 100, 0, 1500),
                note(REFERENCE_KEY + 12, 50, 1750, 1500),
            ],
            length_ms: 4750,
        },
    },
    Entry {
        id: LEGATO_PAIR,
        name: "legato_pair",
        phrase: Phrase {
            notes: &[
                note(REFERENCE_KEY, 100, 0, 1750),
                note(REFERENCE_KEY + 7, 100, 1500, 1750),
            ],
            length_ms: 4750,
        },
    },
];

/// The released phrase with this id: the generator's table first, then the lesson-only phrases
/// ([`crate::lesson_phrases`], ids 4 and up; lane CV2, R2). One id sequence spans both tables, so a lesson
/// challenge's phrase resolves wherever a generated one does.
pub fn entry(id: PhraseId) -> Option<&'static Entry> {
    TABLE
        .iter()
        .find(|e| e.id == id)
        .or_else(|| crate::lesson_phrases::entry(id))
}

/// The phrase with this id, from either table ([`entry`]).
pub fn phrase(id: PhraseId) -> Option<&'static Phrase> {
    entry(id).map(|e| &e.phrase)
}

/// What a phrase can reveal: the facts the observability mask and the priors read. Parameter space only.
pub trait PhraseFacts {
    /// The lowest key played (the note the priors are relative to).
    fn lowest_key(&self) -> u8;
    /// The shortest hold of any note, in ms.
    fn min_hold_ms(&self) -> u32;
    /// The time from the last note-off to the end of the render, in ms.
    fn tail_ms(&self) -> u32;
    /// Whether two notes are an octave apart (keytrack is audible, and is not the same as moving the cutoff).
    fn has_octave_pair(&self) -> bool;
    /// Whether at least two velocities are played (velocity amounts are audible).
    fn has_two_velocities(&self) -> bool;
    /// Whether at least two keys are played.
    fn has_pitch_change(&self) -> bool;
    /// Whether a note starts on another key while an earlier one is still held (legato, mono retrigger and
    /// legato-mode glide are audible).
    fn has_overlapping_pitch_change(&self) -> bool;
    /// Whether a note starts on another key after every earlier key was released (`glide_mode` always and legato
    /// differ only then).
    fn has_detached_pitch_change(&self) -> bool;
    /// The most notes held at once.
    fn max_simultaneous(&self) -> usize;
}

const fn off(n: &PhraseNote) -> u32 {
    n.start_ms.saturating_add(n.hold_ms)
}

impl PhraseFacts for Phrase {
    fn lowest_key(&self) -> u8 {
        self.notes
            .iter()
            .map(|n| n.key)
            .min()
            .unwrap_or(REFERENCE_KEY)
    }

    fn min_hold_ms(&self) -> u32 {
        self.notes.iter().map(|n| n.hold_ms).min().unwrap_or(0)
    }

    fn tail_ms(&self) -> u32 {
        let last_off = self.notes.iter().map(off).max().unwrap_or(0);
        self.length_ms.saturating_sub(last_off)
    }

    fn has_octave_pair(&self) -> bool {
        self.notes
            .iter()
            .any(|a| self.notes.iter().any(|b| b.key == a.key.saturating_add(12)))
    }

    fn has_two_velocities(&self) -> bool {
        self.notes
            .iter()
            .any(|a| a.velocity != self.notes[0].velocity)
    }

    fn has_pitch_change(&self) -> bool {
        self.notes.iter().any(|a| a.key != self.notes[0].key)
    }

    fn has_overlapping_pitch_change(&self) -> bool {
        self.notes.iter().enumerate().any(|(i, a)| {
            self.notes[..i]
                .iter()
                .any(|b| b.key != a.key && b.start_ms <= a.start_ms && a.start_ms < off(b))
        })
    }

    fn has_detached_pitch_change(&self) -> bool {
        self.notes.iter().enumerate().any(|(i, a)| {
            i > 0
                && self.notes[..i].iter().all(|b| off(b) <= a.start_ms)
                && self.notes[..i].iter().any(|b| b.key != a.key)
        })
    }

    fn max_simultaneous(&self) -> usize {
        self.notes
            .iter()
            .map(|a| {
                self.notes
                    .iter()
                    .filter(|b| b.start_ms <= a.start_ms && a.start_ms < off(b))
                    .count()
            })
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_render::Note;

    #[test]
    fn ids_are_append_only_and_valid() {
        let ids: Vec<u16> = TABLE.iter().map(|e| e.id.0).collect();
        assert_eq!(ids, [1, 2, 3], "ids start at 1 and follow the table order");
        let names: Vec<&str> = TABLE.iter().map(|e| e.name).collect();
        assert_eq!(names, ["held", "octave_pair", "legato_pair"]);
        for e in &TABLE {
            assert_eq!(e.phrase.validate(), Ok(()), "{}", e.name);
            assert_eq!(phrase(e.id), Some(&e.phrase));
        }
        assert_eq!(phrase(PhraseId(0)), None);
        // Ids 4-7 are the lesson-only phrases (lane CV2, `lesson_phrases`): the lookup finds them too.
        assert_eq!(phrase(PhraseId(8)), None);
    }

    /// The facts the v1 priors rely on hold for every phrase: the same reference key, notes in time order, every
    /// hold and tail at least the caps.
    #[test]
    fn every_phrase_meets_the_shared_facts() {
        for e in &TABLE {
            let p = &e.phrase;
            assert_eq!(p.lowest_key(), REFERENCE_KEY, "{}", e.name);
            assert!(p.min_hold_ms() >= MIN_HOLD_MS, "{}", e.name);
            assert!(p.tail_ms() >= MIN_TAIL_MS, "{}", e.name);
            assert!(
                p.notes.windows(2).all(|w| w[0].start_ms <= w[1].start_ms),
                "{}",
                e.name
            );
        }
    }

    #[test]
    fn what_each_phrase_reveals() {
        let [held, octave, legato] = TABLE.map(|e| e.phrase);
        assert!(!held.has_octave_pair() && !held.has_two_velocities() && !held.has_pitch_change());
        assert_eq!(held.max_simultaneous(), 1);
        assert!(
            octave.has_octave_pair() && octave.has_two_velocities() && octave.has_pitch_change()
        );
        assert!(!octave.has_overlapping_pitch_change() && octave.has_detached_pitch_change());
        assert!(!legato.has_octave_pair() && !legato.has_two_velocities());
        assert!(legato.has_overlapping_pitch_change() && !legato.has_detached_pitch_change());
        assert_eq!(legato.max_simultaneous(), 2);
    }

    #[test]
    fn the_performance_is_exact() {
        let p = phrase(OCTAVE_PAIR).unwrap().performance();
        assert_eq!(p.duration_s, 4.75);
        assert_eq!(p.notes.len(), 2);
        assert_eq!(p.notes[0], Note::new(48, 100.0 / 127.0, 0.0, 1.5));
        assert_eq!(p.notes[1], Note::new(60, 50.0 / 127.0, 1.75, 3.25));
        assert!(p.automation.is_empty() && p.controls.is_empty());
    }
}
