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

    /// A query for one host note id, every other field a wildcard: how CLAP polyphonic modulation addresses a voice
    /// ([`EventKind::MacroVoice`]; nice-plug forwards only the note id).
    #[must_use]
    pub const fn note_id(note_id: i32) -> Self {
        Self {
            port: -1,
            channel: -1,
            key: -1,
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
    /// Set a parameter's base value, in plain units (clamped to its spec; NaN is ignored), smoothed as its spec
    /// says. The offline renderer's automation: the id must come from the [`PlanMap`](crate::plan::PlanMap) of the
    /// plan the engine runs. **Not for a real-time engine:** a plan swap happens at a segment boundary inside
    /// `process`, so an id looked up before the call can name another parameter of the new plan. Real-time hosts
    /// use [`EventKind::Slot`] (resolved by the engine through the running plan's slot table) or
    /// [`EventKind::EpochParam`] (dropped if its plan is no longer running). UI edits arrive through the command ring
    /// with their plan's epoch ([`crate::swap::Link::send_param`]).
    Param {
        /// The parameter, from the plan's [`PlanMap`](crate::plan::PlanMap).
        id: ParamId,
        /// Plain value.
        value: f32,
    },
    /// Set a macro knob (a global modulation source), 0–1, clamped; smoothed over 10 ms (lane S2b).
    Macro {
        /// 0-based, 0–7 (`macro1` is 0); others are ignored.
        index: u8,
        /// Knob position, 0–1.
        value: f32,
    },
    /// Set the mod wheel (a global modulation source), 0–1, clamped; smoothed over 10 ms (lane S2b).
    ModWheel {
        /// 0–1.
        value: f32,
    },
    /// Fade out every playing voice matching `key` over 64 frames (CLAP `NOTE_CHOKE`; lane S2b).
    Choke {
        /// The notes to silence (wildcards allowed).
        key: NoteKey,
    },
    /// Host automation of slot `slot` (0-based, 0–63; others are ignored), as the host's normalised value 0–1
    /// (clamped; NaN is ignored). The engine resolves it through the running plan's slot table, compiled from
    /// `Patch::slots`, so a plan swap inside the block can never route it to another parameter (P1b lane E+). A
    /// value bit-equal to `to_norm` of the target's patch value sets the exact patch value; any other sets
    /// `from_norm(value)`. The engine remembers the last value per slot and re-applies it after a plan swap whose
    /// slot still names the same parameter. A slot that is empty, or whose target is in a disabled module, ignores
    /// the value (and remembers it).
    Slot {
        /// 0-based slot index (`slot_1` is 0).
        slot: u8,
        /// The host's normalised value, 0–1.
        value: f32,
    },
    /// Like [`EventKind::Param`], for the plan with `epoch` only: the engine drops it (counted in
    /// `EngineStats::stale_events`) unless that plan is running when the event is due (P1b lane E+). Host
    /// automation by id: tag it with [`Engine::epoch`](crate::Engine::epoch) of the plan whose
    /// [`PlanMap`](crate::plan::PlanMap) gave the id.
    EpochParam {
        /// The epoch of the plan whose `PlanMap` gave `id`.
        epoch: u64,
        /// The parameter.
        id: ParamId,
        /// Plain value.
        value: f32,
    },
    /// CLAP polyphonic modulation of a macro (P1b lane E+): every playing voice matching `key` (use
    /// [`NoteKey::note_id`]) gets `offset` (normalised, −1..1, clamped; NaN is ignored) added to macro `index`
    /// (0-based, 0–7; others are ignored), for **voice-rate destinations only**: per voice, the macro source reads
    /// `clamp(macro + offset, 0, 1)`; global destinations keep the macro itself. Offsets start at 0 whenever a note
    /// lands on a voice (a fresh voice, a mono retrigger or a legato change) and are forgotten when it ends.
    /// Outside the CLI contract: a `Performance` cannot express it.
    MacroVoice {
        /// The voices to modulate (wildcards allowed).
        key: NoteKey,
        /// 0-based macro, 0–7.
        index: u8,
        /// Normalised offset, −1..1.
        offset: f32,
    },
    /// Set the tempo, in beats per minute (a quarter note is a beat), for the modules that sync to it (S2a, lane TMP;
    /// [`crate::tempo`]). It takes effect at the first segment boundary at or after its frame, like every event, so a
    /// tempo that changes inside a host block changes at the same boundary whatever the block size. Clamped to
    /// 20–999; NaN is ignored. The engine starts at 140 BPM and [`Engine::reset`](crate::Engine::reset) returns to
    /// it.
    Tempo {
        /// Beats per minute.
        bpm: f64,
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
