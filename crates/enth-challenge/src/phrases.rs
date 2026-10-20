//! **Owner: lane RG** (the randomizer picks the phrase; CT's Challenge v1 refers to it by id). Pre-declared empty by
//! lane S0r.
//!
//! The versioned phrase table (design.md §9, report 01 §2.5): what a challenge plays, so what the target is heard and
//! scored through. One held note for levels 1-4, two notes an octave apart at two velocities when keytrack or
//! velocity is in play, a legato pair for mono or glide. The generator picks the phrase (`random`), so observability
//! is decided before a round starts.
//!
//! A phrase is plain data here (keys, velocities and times as integers), and [`Phrase::performance`] turns it into
//! the `enth_render::Performance` that the scorer renders and Target ▶ / Mine ▶ play. The generator reads only the
//! data (notes, holds, tails), never the `Performance`: target generation works in parameter space and never reaches
//! `enth-render` (RG's acceptance; `tests/rg_render_free.rs` checks it).
//!
//! **Frozen.** The table is [`PHRASES_VERSION`] 1. A phrase's notes never change once shipped (codes, golden
//! challenges and duels depend on them); a new phrase is a new id, and a change to an existing one is a new table
//! version together with a new generator version. Every v1 phrase shares two facts the priors rely on: its lowest
//! key is [`REFERENCE_KEY`] (C3), and every note is held at least [`MIN_HOLD_MS`] with at least [`MIN_TAIL_MS`] of
//! tail after the last note-off, so the envelope priors (attack up to the hold, release up to the tail) and the
//! cutoff priors (relative to the reference note) are the same for every phrase. Choosing a phrase therefore never
//! changes the values of the lower difficulty categories.

use enth_render::{Note, Performance};

/// The phrase table's version. A change to an existing phrase is a new version (and a new generator version).
pub const PHRASES_VERSION: u16 = 1;

/// The lowest key of every v1 phrase: C3 (MIDI 48, 130.81 Hz). The priors are relative to it.
pub const REFERENCE_KEY: u8 = 48;

/// Every note of every v1 phrase is held at least this long, so an attack up to it is heard to its peak.
pub const MIN_HOLD_MS: u32 = 1500;

/// Every v1 phrase ends at least this long after its last note-off, so a release up to it is heard to its end.
pub const MIN_TAIL_MS: u32 = 1500;

/// A phrase's id: what Challenge v1 and the code store. Stable: a new phrase gets the next number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum PhraseId {
    /// One note, C3 at velocity 80 %, held 1.5 s in a 3 s clip (levels 1-4 always use it).
    Held = 0,
    /// C3 at velocity 80 %, then C4 at 40 %: keytrack and velocity amounts become audible.
    OctavePair = 1,
    /// C3, then G3 overlapping it: legato and glide become audible.
    LegatoPair = 2,
}

impl PhraseId {
    /// Every phrase id, in table order.
    pub const ALL: [PhraseId; 3] = [PhraseId::Held, PhraseId::OctavePair, PhraseId::LegatoPair];

    /// The id stored in a challenge or a code.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// The phrase with this stored id, if the table has it.
    pub const fn from_u8(id: u8) -> Option<PhraseId> {
        match id {
            0 => Some(PhraseId::Held),
            1 => Some(PhraseId::OctavePair),
            2 => Some(PhraseId::LegatoPair),
            _ => None,
        }
    }

    /// The phrase itself.
    pub const fn phrase(self) -> &'static Phrase {
        &PHRASES[self as usize]
    }
}

/// One note of a phrase, in integers so the table is exact: key, velocity in percent, on and off in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhraseNote {
    /// MIDI key (A4 = 69).
    pub key: u8,
    /// Velocity, 1-100 %.
    pub velocity: u8,
    /// Note-on, ms from the start.
    pub on_ms: u32,
    /// Note-off, ms from the start (after `on_ms`).
    pub off_ms: u32,
}

impl PhraseNote {
    const fn new(key: u8, velocity: u8, on_ms: u32, off_ms: u32) -> Self {
        PhraseNote {
            key,
            velocity,
            on_ms,
            off_ms,
        }
    }

    /// How long the key is held, in ms.
    pub const fn hold_ms(&self) -> u32 {
        self.off_ms - self.on_ms
    }
}

/// A phrase: the notes a challenge plays and the length of the clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phrase {
    /// Its id.
    pub id: PhraseId,
    /// A stable name for logs and the CLI (`held`, `octave_pair`, `legato_pair`).
    pub name: &'static str,
    /// The notes, in time order.
    pub notes: &'static [PhraseNote],
    /// Length of the clip in ms (the loop length of Target ▶).
    pub duration_ms: u32,
}

/// The v1 table, indexed by [`PhraseId`].
pub const PHRASES: [Phrase; 3] = [
    Phrase {
        id: PhraseId::Held,
        name: "held",
        notes: &[PhraseNote::new(REFERENCE_KEY, 80, 0, 1500)],
        duration_ms: 3000,
    },
    Phrase {
        id: PhraseId::OctavePair,
        name: "octave_pair",
        notes: &[
            PhraseNote::new(REFERENCE_KEY, 80, 0, 1500),
            PhraseNote::new(REFERENCE_KEY + 12, 40, 1750, 3250),
        ],
        duration_ms: 4750,
    },
    Phrase {
        id: PhraseId::LegatoPair,
        name: "legato_pair",
        notes: &[
            PhraseNote::new(REFERENCE_KEY, 80, 0, 1750),
            PhraseNote::new(REFERENCE_KEY + 7, 80, 1500, 3250),
        ],
        duration_ms: 4750,
    },
];

impl Phrase {
    /// The `Performance` the scorer renders and the plugin plays: velocities as 0-1, times in seconds.
    pub fn performance(&self) -> Performance {
        Performance {
            notes: self
                .notes
                .iter()
                .map(|n| {
                    Note::new(
                        n.key,
                        f32::from(n.velocity) / 100.0,
                        f64::from(n.on_ms) / 1000.0,
                        f64::from(n.off_ms) / 1000.0,
                    )
                })
                .collect(),
            duration_s: f64::from(self.duration_ms) / 1000.0,
            ..Performance::default()
        }
    }

    /// The lowest key played (the note the priors are relative to).
    pub fn lowest_key(&self) -> u8 {
        self.notes
            .iter()
            .map(|n| n.key)
            .min()
            .unwrap_or(REFERENCE_KEY)
    }

    /// The shortest hold of any note, in ms.
    pub fn min_hold_ms(&self) -> u32 {
        self.notes
            .iter()
            .map(PhraseNote::hold_ms)
            .min()
            .unwrap_or(0)
    }

    /// The time from the last note-off to the end of the clip, in ms.
    pub fn tail_ms(&self) -> u32 {
        let last_off = self.notes.iter().map(|n| n.off_ms).max().unwrap_or(0);
        self.duration_ms.saturating_sub(last_off)
    }

    /// Whether two notes are an octave apart (keytrack is audible, and is not the same as moving the cutoff).
    pub fn has_octave_pair(&self) -> bool {
        self.notes
            .iter()
            .any(|a| self.notes.iter().any(|b| b.key == a.key.saturating_add(12)))
    }

    /// Whether at least two velocities are played (velocity amounts are audible).
    pub fn has_two_velocities(&self) -> bool {
        self.notes
            .iter()
            .any(|a| a.velocity != self.notes[0].velocity)
    }

    /// Whether at least two keys are played.
    pub fn has_pitch_change(&self) -> bool {
        self.notes.iter().any(|a| a.key != self.notes[0].key)
    }

    /// Whether a note starts on another key while an earlier one is still held (legato, mono retrigger and
    /// legato-mode glide are audible).
    pub fn has_overlapping_pitch_change(&self) -> bool {
        self.notes.iter().enumerate().any(|(i, a)| {
            self.notes[..i]
                .iter()
                .any(|b| b.key != a.key && b.on_ms <= a.on_ms && a.on_ms < b.off_ms)
        })
    }

    /// Whether a note starts on another key after every earlier key was released (`glide_mode` always and legato
    /// differ only then).
    pub fn has_detached_pitch_change(&self) -> bool {
        self.notes.iter().enumerate().any(|(i, a)| {
            i > 0
                && self.notes[..i].iter().all(|b| b.off_ms <= a.on_ms)
                && self.notes[..i].iter().any(|b| b.key != a.key)
        })
    }

    /// The most notes held at once.
    pub fn max_simultaneous(&self) -> usize {
        self.notes
            .iter()
            .map(|a| {
                self.notes
                    .iter()
                    .filter(|b| b.on_ms <= a.on_ms && a.on_ms < b.off_ms)
                    .count()
            })
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_index_the_table_and_round_trip() {
        for (i, id) in PhraseId::ALL.iter().enumerate() {
            assert_eq!(PHRASES[i].id, *id);
            assert_eq!(id.phrase().id, *id);
            assert_eq!(PhraseId::from_u8(id.as_u8()), Some(*id));
        }
        assert_eq!(PhraseId::from_u8(3), None);
        let names: Vec<&str> = PHRASES.iter().map(|p| p.name).collect();
        assert_eq!(names, ["held", "octave_pair", "legato_pair"]);
    }

    /// The facts the v1 priors rely on hold for every phrase: the same reference key, notes in time order, every
    /// hold and tail at least the caps.
    #[test]
    fn every_phrase_meets_the_shared_facts() {
        for p in &PHRASES {
            assert_eq!(p.lowest_key(), REFERENCE_KEY, "{}", p.name);
            assert!(p.min_hold_ms() >= MIN_HOLD_MS, "{}", p.name);
            assert!(p.tail_ms() >= MIN_TAIL_MS, "{}", p.name);
            assert!(
                p.notes.windows(2).all(|w| w[0].on_ms <= w[1].on_ms),
                "{}",
                p.name
            );
            for n in p.notes {
                assert!(
                    n.off_ms > n.on_ms && n.off_ms <= p.duration_ms,
                    "{}",
                    p.name
                );
                assert!((1..=100).contains(&n.velocity), "{}", p.name);
            }
        }
    }

    #[test]
    fn what_each_phrase_reveals() {
        let [held, octave, legato] = &PHRASES;
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
        let p = PhraseId::OctavePair.phrase().performance();
        assert_eq!(p.duration_s, 4.75);
        assert_eq!(p.notes.len(), 2);
        assert_eq!(p.notes[0], Note::new(48, 0.8, 0.0, 1.5));
        assert_eq!(p.notes[1], Note::new(60, 0.4, 1.75, 3.25));
        assert!(p.automation.is_empty() && p.controls.is_empty());
    }
}
