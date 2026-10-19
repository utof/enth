//! The voice manager: which slot plays which note.
//!
//! **S2a's minimal version:** poly only. A note-on takes the lowest free slot while fewer than `voice.polyphony`
//! voices are held or releasing; otherwise the note is dropped and counted (no stealing yet). A note-off releases
//! every held voice that matches (CLAP wildcards). A voice ends when the amplitude envelope says so (past its attack
//! and below −100 dB, `VoiceModulator::voice_finished`), and its slot is free from the next segment. Everything is
//! deterministic: no randomness, no clock, no dependence on the host's block size; the same performance gives the
//! same slots, so the same summation order.
//!
//! TODO(S2b): stealing (the oldest released voice, else the oldest held one, with a 64-frame fade into a spare
//! slot, so polyphony ≤ 16 never needs more than the 32 slots), mono and legato with the 16-key last-note stack,
//! glide (linear in semitones at constant time, writing `NoteState::pitch_prev/pitch_cur`), choke, smoother reset on
//! retrigger, and the voice-end queue for CLAP `NOTE_END` (P1b). The engine already counts every note-on in
//! `note_index`, including dropped ones, so adding stealing does not move any seed.

use crate::event::NoteKey;
use crate::module::{MAX_VOICES, VoiceIdx};

/// What a voice slot is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceState {
    /// Silent and available.
    Free,
    /// Playing, key down.
    Held,
    /// Playing its release, key up.
    Released,
}

/// The slot assignment. Fixed-size; nothing here allocates.
#[derive(Clone, Debug)]
pub struct VoiceManager {
    state: [VoiceState; MAX_VOICES],
    keys: [NoteKey; MAX_VOICES],
    polyphony: usize,
    note_count: u64,
    dropped_notes: u64,
    active: [VoiceIdx; MAX_VOICES],
    active_len: usize,
}

impl VoiceManager {
    /// No voices playing; at most `polyphony` voices (clamped to 1..=16) held or releasing at once.
    #[must_use]
    pub fn new(polyphony: usize) -> Self {
        Self {
            state: [VoiceState::Free; MAX_VOICES],
            keys: [NoteKey::new(0, -1); MAX_VOICES],
            polyphony: polyphony.clamp(1, 16),
            note_count: 0,
            dropped_notes: 0,
            active: [0; MAX_VOICES],
            active_len: 0,
        }
    }

    /// Start a note: the lowest free slot and the note's index, or `None` if polyphony is full (the note is dropped;
    /// S2b steals instead). Every note-on, dropped or not, takes the next note index.
    pub fn note_on(&mut self, key: NoteKey) -> (u64, Option<VoiceIdx>) {
        let note_index = self.note_count;
        self.note_count += 1;
        if self.active_len >= self.polyphony {
            self.dropped_notes += 1;
            return (note_index, None);
        }
        let Some(v) = self.state.iter().position(|s| *s == VoiceState::Free) else {
            self.dropped_notes += 1;
            return (note_index, None);
        };
        self.state[v] = VoiceState::Held;
        self.keys[v] = key;
        self.rebuild_active();
        (note_index, Some(v))
    }

    /// Release every held voice matching `query` (wildcards allowed); `released(v)` is called for each, in
    /// ascending slot order.
    pub fn note_off(&mut self, query: NoteKey, mut released: impl FnMut(VoiceIdx)) {
        for v in 0..MAX_VOICES {
            if self.state[v] == VoiceState::Held && query.matches(&self.keys[v]) {
                self.state[v] = VoiceState::Released;
                released(v);
            }
        }
    }

    /// Free slot `v` (its amplitude envelope finished).
    pub fn end(&mut self, v: VoiceIdx) {
        if let Some(s) = self.state.get_mut(v)
            && *s != VoiceState::Free
        {
            *s = VoiceState::Free;
            self.rebuild_active();
        }
    }

    /// The playing voices (held or releasing), in ascending slot order: the fixed voice and summation order.
    #[must_use]
    pub fn active(&self) -> &[VoiceIdx] {
        &self.active[..self.active_len]
    }

    /// What slot `v` is doing.
    #[must_use]
    pub fn state(&self, v: VoiceIdx) -> VoiceState {
        self.state[v]
    }

    /// The note playing in slot `v` (meaningful while it is not free).
    #[must_use]
    pub fn key(&self, v: VoiceIdx) -> NoteKey {
        self.keys[v]
    }

    /// Note-ons since reset (the next note index).
    #[must_use]
    pub fn note_count(&self) -> u64 {
        self.note_count
    }

    /// Note-ons dropped because polyphony was full.
    #[must_use]
    pub fn dropped_notes(&self) -> u64 {
        self.dropped_notes
    }

    /// The polyphony limit.
    #[must_use]
    pub fn polyphony(&self) -> usize {
        self.polyphony
    }

    fn rebuild_active(&mut self) {
        let mut n = 0;
        for v in 0..MAX_VOICES {
            if self.state[v] != VoiceState::Free {
                self.active[n] = v;
                n += 1;
            }
        }
        self.active_len = n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn off(key: u8) -> NoteKey {
        NoteKey {
            note_id: -1,
            ..NoteKey::new(key, 0)
        }
    }

    #[test]
    fn lowest_free_slot_and_ascending_active_list() {
        let mut m = VoiceManager::new(8);
        assert_eq!(m.note_on(NoteKey::new(60, 0)), (0, Some(0)));
        assert_eq!(m.note_on(NoteKey::new(64, 1)), (1, Some(1)));
        assert_eq!(m.note_on(NoteKey::new(67, 2)), (2, Some(2)));
        m.end(1);
        assert_eq!(m.active(), &[0, 2]);
        assert_eq!(
            m.note_on(NoteKey::new(72, 3)),
            (3, Some(1)),
            "reuses the lowest free slot"
        );
        assert_eq!(m.active(), &[0, 1, 2]);
    }

    #[test]
    fn note_off_releases_matching_held_voices_only() {
        let mut m = VoiceManager::new(8);
        m.note_on(NoteKey::new(60, 0));
        m.note_on(NoteKey::new(60, 1)); // same key, another note id: overlapping voices
        m.note_on(NoteKey::new(62, 2));
        let mut released = Vec::new();
        m.note_off(NoteKey::new(60, 1), |v| released.push(v));
        assert_eq!(released, [1]);
        released.clear();
        m.note_off(off(60), |v| released.push(v));
        assert_eq!(released, [0], "the released voice is not released twice");
        assert_eq!(m.state(2), VoiceState::Held);
        assert_eq!(m.state(1), VoiceState::Released);
        assert_eq!(
            m.active(),
            &[0, 1, 2],
            "released voices keep playing their tail"
        );
    }

    #[test]
    fn full_polyphony_drops_notes_but_counts_them() {
        let mut m = VoiceManager::new(2);
        m.note_on(NoteKey::new(60, 0));
        m.note_on(NoteKey::new(62, 1));
        m.note_off(off(60), |_| {});
        // Released voices still count against polyphony.
        assert_eq!(m.note_on(NoteKey::new(64, 2)), (2, None));
        assert_eq!(m.dropped_notes(), 1);
        m.end(0);
        assert_eq!(m.note_on(NoteKey::new(65, 3)), (3, Some(0)));
        assert_eq!(m.note_count(), 4);
    }

    #[test]
    fn polyphony_is_clamped_to_the_catalogue_range() {
        assert_eq!(VoiceManager::new(0).polyphony(), 1);
        assert_eq!(VoiceManager::new(100).polyphony(), 16);
        m_end_free_is_harmless();
    }

    fn m_end_free_is_harmless() {
        let mut m = VoiceManager::new(4);
        m.end(3);
        m.end(999);
        assert_eq!(m.active(), &[] as &[VoiceIdx]);
    }
}
