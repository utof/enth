//! Events the host passes to [`Engine::process`](crate::engine::Engine::process): notes and parameter values,
//! timed by a frame offset within the host block.
//!
//! An event takes effect at the first segment boundary at or after its frame, `ceil(frame / SEG) · SEG` on the
//! engine's free-running grid; events at the same boundary apply in arrival order (design.md §5). Blocks are never
//! split at event times, so the host's block size cannot change a render.

use crate::params::ParamId;

/// A note's identity, CLAP's (port, channel, key, note id) tuple, so P1b's per-note modulation and `NOTE_END`
/// need no rework. In a note-off, `-1` in any field is a wildcard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteKey {
    /// Note port index, or −1.
    pub port: i16,
    /// MIDI channel 0–15, or −1.
    pub channel: i16,
    /// MIDI key 0–127, or −1.
    pub key: i16,
    /// Host note id, or −1.
    pub note_id: i32,
}

impl NoteKey {
    /// Key `key` on port 0, channel 0, with a note id (the offline renderer uses the note's index in the
    /// performance, so overlapping notes on one key stay distinct).
    #[must_use]
    pub const fn new(key: u8, note_id: i32) -> Self {
        Self {
            port: 0,
            channel: 0,
            key: key as i16,
            note_id,
        }
    }

    /// Whether this key, used as a note-off query, matches a playing note: every field equal or a wildcard (−1).
    #[must_use]
    pub const fn matches(&self, playing: &NoteKey) -> bool {
        (self.port == -1 || self.port == playing.port)
            && (self.channel == -1 || self.channel == playing.channel)
            && (self.key == -1 || self.key == playing.key)
            && (self.note_id == -1 || self.note_id == playing.note_id)
    }
}

/// One timed event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    /// Frame offset from the start of the host block passed with it.
    pub offset: u32,
    /// What happens.
    pub kind: EventKind,
}

/// What an event does. New kinds are added by later lanes (choke, macros, mod wheel, per-note expressions), so the
/// enum is non-exhaustive.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum EventKind {
    /// Start a note. `velocity` is 0–1 (clamped).
    NoteOn {
        /// The note's identity; its `key` sets the pitch.
        key: NoteKey,
        /// 0–1.
        velocity: f32,
    },
    /// Release every held note matching `key` (wildcards allowed).
    NoteOff {
        /// The note to release.
        key: NoteKey,
        /// Release velocity, 0–1 (unused in P1).
        velocity: f32,
    },
    /// Set a parameter's base value, in plain units (clamped to its spec; NaN is ignored). This is host automation
    /// and the offline renderer's automation; UI edits arrive through the command ring with a plan epoch (lane S2b).
    Param {
        /// The parameter, from the plan's [`PlanMap`](crate::plan::PlanMap).
        id: ParamId,
        /// Plain value.
        value: f32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_any_field() {
        let playing = NoteKey::new(60, 7);
        assert!(NoteKey::new(60, 7).matches(&playing));
        assert!(!NoteKey::new(60, 8).matches(&playing));
        let by_key = NoteKey {
            note_id: -1,
            ..NoteKey::new(60, 0)
        };
        assert!(by_key.matches(&playing));
        let all = NoteKey {
            port: -1,
            channel: -1,
            key: -1,
            note_id: -1,
        };
        assert!(all.matches(&playing));
        let other_key = NoteKey {
            note_id: -1,
            ..NoteKey::new(61, 0)
        };
        assert!(!other_key.matches(&playing));
    }
}
