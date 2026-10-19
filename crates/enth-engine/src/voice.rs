//! The voice manager: which slot plays which note, stealing, mono and legato, glide, and voice-end reports.
//!
//! Everything here is deterministic: no randomness, no clock, no dependence on the host's block size. Events reach
//! it on the segment grid in arrival order, so the same performance gives the same slots, and the same slots give
//! the same summation order. Fixed-size state; nothing allocates.
//!
//! **Poly** (`voice.mode = poly`): a note-on takes the lowest free of the 32 slots while fewer than
//! `voice.polyphony` (≤ 16) voices are held or releasing. When polyphony is full it **steals**: the oldest released
//! voice, else the oldest held one ("oldest" = the earliest note-on, so ties cannot happen). The victim fades to
//! silence over 64 frames (two segments, linear) while the new note starts in a spare slot; polyphony ≤ 16 leaves
//! 16 spares for fades. Only a burst of more than 16 steals within two segments can find every slot busy; then the
//! new note takes over the fading voice with the least fade left (a cut, counted). A same-key note-on starts a new
//! voice; the old one keeps its release (CLAP note ids keep overlapping notes apart).
//!
//! **Mono and legato** play one voice with a 16-key last-note stack (the newest key sounds; a 17th key pushes out
//! the oldest). A note-on while a key is held: mono retriggers the envelopes from their current level, legato only
//! changes the pitch. A note-on with no key held retriggers the voice while it still sounds (both modes), or starts
//! a fresh voice once it has ended. Releasing the sounding key returns to the newest key still held (a pitch change
//! without a retrigger); releasing the last key releases the voice. The voice keeps its first note's note index,
//! and so its seeds, until it ends.
//!
//! **Glide** (`voice.glide` ms, 0 = off): linear in semitones at constant time, `max(1, round(glide · sr / 1000 /
//! 32))` segments from the current pitch to the new one, whatever the interval; a new note mid-glide starts a new
//! glide from where the pitch is. `glide_mode = always` glides every new note (in poly mode, from the previous
//! note-on's pitch); `legato` only when another key is held at the note-on. Writes `NoteState::pitch_prev` (the
//! pitch at the segment start) and `pitch_cur` (at its end); modules ramp between them.
//!
//! **Choke** (CLAP `NOTE_CHOKE`): matching voices fade out over 64 frames like a steal.
//!
//! **Voice end**: at the end of a segment a voice is freed when its amplitude envelope is past its attack and below
//! −100 dB, or when its steal fade completes. Every end (and every cut) is reported as a [`NoteEnd`] for CLAP
//! `NOTE_END`; [`Engine::note_ends`](crate::Engine::note_ends) lists those of the last `process` call. A freed slot
//! costs nothing.

use enth_patch::catalogue::{GlideMode, VoiceMode};

use crate::event::NoteKey;
use crate::module::{MAX_VOICES, NoteState, SEG, VoiceIdx};

/// What a voice slot is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceState {
    /// Silent and available.
    Free,
    /// Playing, key down.
    Held,
    /// Playing its release, key up.
    Released,
    /// Fading out over 64 frames after a steal or a choke; freed when the fade ends (lane S2b).
    Stolen,
}

/// Segments a steal fade lasts: 64 frames.
pub const STEAL_FADE_SEGMENTS: u8 = 2;

/// Keys the mono/legato stack remembers.
pub const NOTE_STACK: usize = 16;

/// Voice ends one `process` call can report; more are counted in `EngineStats::dropped_note_ends`.
pub const NOTE_END_CAPACITY: usize = 256;

/// A voice that ended, for CLAP `NOTE_END`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteEnd {
    /// The note the voice was playing.
    pub key: NoteKey,
    /// The absolute engine frame at which it went silent (a segment boundary).
    pub frame: u64,
}

/// What a note-on did, for the engine to apply to the modules and the notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteOn {
    /// A fresh voice starts in this slot (every voice module gets `start_voice`).
    Start {
        /// The slot.
        voice: VoiceIdx,
        /// Another key was held at the note-on (for `glide_mode = legato`).
        overlapping: bool,
    },
    /// The mono voice restarts its envelopes from their current level, with the new pitch and velocity.
    Retrigger {
        /// The slot.
        voice: VoiceIdx,
        /// Another key was held at the note-on.
        overlapping: bool,
    },
    /// The legato voice changes pitch only (another key is held).
    Legato {
        /// The slot.
        voice: VoiceIdx,
    },
}

impl NoteOn {
    /// The slot the note plays in.
    #[must_use]
    pub const fn voice(&self) -> VoiceIdx {
        match *self {
            NoteOn::Start { voice, .. }
            | NoteOn::Retrigger { voice, .. }
            | NoteOn::Legato { voice } => voice,
        }
    }

    /// Whether another key was held at the note-on.
    #[must_use]
    pub const fn overlapping(&self) -> bool {
        match *self {
            NoteOn::Start { overlapping, .. } | NoteOn::Retrigger { overlapping, .. } => {
                overlapping
            }
            NoteOn::Legato { .. } => true,
        }
    }
}

/// The last-note stack of mono and legato: the newest key on top.
#[derive(Clone, Copy, Debug)]
struct NoteStack {
    keys: [NoteKey; NOTE_STACK],
    len: usize,
}

impl NoteStack {
    const fn new() -> Self {
        Self {
            keys: [NoteKey::new(0, -1); NOTE_STACK],
            len: 0,
        }
    }

    fn push(&mut self, key: NoteKey) {
        if self.len == NOTE_STACK {
            self.keys.copy_within(1.., 0);
            self.len -= 1;
        }
        self.keys[self.len] = key;
        self.len += 1;
    }

    /// Remove every key the query matches (wildcards allowed), keeping the order of the rest.
    fn remove(&mut self, query: NoteKey) {
        let mut kept = 0;
        for i in 0..self.len {
            if !query.matches(&self.keys[i]) {
                self.keys[kept] = self.keys[i];
                kept += 1;
            }
        }
        self.len = kept;
    }

    fn top(&self) -> Option<NoteKey> {
        self.len.checked_sub(1).map(|i| self.keys[i])
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// The slot assignment. Fixed-size; nothing here allocates.
#[derive(Clone, Debug)]
pub struct VoiceManager {
    state: [VoiceState; MAX_VOICES],
    keys: [NoteKey; MAX_VOICES],
    /// The note index of the note that started the voice: the steal order.
    age: [u64; MAX_VOICES],
    /// Segments of steal fade left (`Stolen` voices).
    fade_left: [u8; MAX_VOICES],
    mode: VoiceMode,
    polyphony: usize,
    note_count: u64,
    dropped_notes: u64,
    stolen: u64,
    cut: u64,
    active: [VoiceIdx; MAX_VOICES],
    active_len: usize,
    stack: NoteStack,
    /// The mono/legato voice, while it is held or releasing.
    mono: Option<VoiceIdx>,
    ends: [NoteEnd; NOTE_END_CAPACITY],
    ends_len: usize,
    ends_dropped: u64,
}

impl VoiceManager {
    /// No voices playing. Poly mode with at most `polyphony` voices (clamped to 1..=16) held or releasing at once.
    #[must_use]
    pub fn new(polyphony: usize) -> Self {
        Self::with_mode(VoiceMode::Poly, polyphony)
    }

    /// No voices playing, in `mode`.
    #[must_use]
    pub fn with_mode(mode: VoiceMode, polyphony: usize) -> Self {
        Self {
            state: [VoiceState::Free; MAX_VOICES],
            keys: [NoteKey::new(0, -1); MAX_VOICES],
            age: [0; MAX_VOICES],
            fade_left: [0; MAX_VOICES],
            mode,
            polyphony: polyphony.clamp(1, 16),
            note_count: 0,
            dropped_notes: 0,
            stolen: 0,
            cut: 0,
            active: [0; MAX_VOICES],
            active_len: 0,
            stack: NoteStack::new(),
            mono: None,
            ends: [NoteEnd {
                key: NoteKey::new(0, -1),
                frame: 0,
            }; NOTE_END_CAPACITY],
            ends_len: 0,
            ends_dropped: 0,
        }
    }

    /// Adopt a new plan's mode and polyphony (a structural edit). Playing voices keep playing. Entering mono or
    /// legato rebuilds the key stack from the held voices (oldest first) and adopts the newest held voice;
    /// entering poly forgets the stack. A lower polyphony is enforced by the next note-on's steals.
    pub fn configure(&mut self, mode: VoiceMode, polyphony: usize) {
        self.polyphony = polyphony.clamp(1, 16);
        let was_poly = self.mode == VoiceMode::Poly;
        self.mode = mode;
        if mode == VoiceMode::Poly {
            self.stack.clear();
            self.mono = None;
        } else if was_poly {
            self.stack.clear();
            self.mono = None;
            let mut order = [0usize; MAX_VOICES];
            let mut n = 0;
            for v in 0..MAX_VOICES {
                if self.state[v] == VoiceState::Held {
                    order[n] = v;
                    n += 1;
                }
            }
            order[..n].sort_unstable_by_key(|&v| self.age[v]);
            for &v in &order[..n] {
                self.stack.push(self.keys[v]);
            }
            self.mono = order[..n].last().copied();
        }
    }

    /// Start a note. Every note-on, whatever happens to it, takes the next note index (so stealing and mode
    /// changes never move a seed). `frame` is the segment start, for the end report of a voice cut to make room.
    pub fn note_on(&mut self, key: NoteKey, frame: u64) -> (u64, NoteOn) {
        let note_index = self.note_count;
        self.note_count += 1;
        let action = if self.mode == VoiceMode::Poly {
            self.poly_on(key, note_index, frame)
        } else {
            self.mono_on(key, note_index, frame)
        };
        self.rebuild_active();
        (note_index, action)
    }

    fn poly_on(&mut self, key: NoteKey, note_index: u64, frame: u64) -> NoteOn {
        let overlapping = self.state.contains(&VoiceState::Held);
        let mut playing = self
            .state
            .iter()
            .filter(|s| matches!(s, VoiceState::Held | VoiceState::Released))
            .count();
        while playing >= self.polyphony {
            let victim = self
                .oldest(VoiceState::Released)
                .or_else(|| self.oldest(VoiceState::Held));
            let Some(v) = victim else { break };
            self.steal(v);
            playing -= 1;
        }
        let v = self.take_slot(frame);
        self.start(v, key, note_index);
        NoteOn::Start {
            voice: v,
            overlapping,
        }
    }

    fn mono_on(&mut self, key: NoteKey, note_index: u64, frame: u64) -> NoteOn {
        let overlapping = self.stack.top().is_some();
        self.stack.push(key);
        let sounding = self
            .mono
            .filter(|&m| matches!(self.state[m], VoiceState::Held | VoiceState::Released));
        match sounding {
            Some(m) if overlapping && self.state[m] == VoiceState::Held => {
                self.keys[m] = key;
                if self.mode == VoiceMode::Legato {
                    NoteOn::Legato { voice: m }
                } else {
                    NoteOn::Retrigger {
                        voice: m,
                        overlapping,
                    }
                }
            }
            Some(m) => {
                self.state[m] = VoiceState::Held;
                self.keys[m] = key;
                NoteOn::Retrigger {
                    voice: m,
                    overlapping,
                }
            }
            None => {
                let v = self.take_slot(frame);
                self.start(v, key, note_index);
                self.mono = Some(v);
                NoteOn::Start {
                    voice: v,
                    overlapping,
                }
            }
        }
    }

    /// The `state` voice with the earliest note-on.
    fn oldest(&self, state: VoiceState) -> Option<VoiceIdx> {
        (0..MAX_VOICES)
            .filter(|&v| self.state[v] == state)
            .min_by_key(|&v| self.age[v])
    }

    fn steal(&mut self, v: VoiceIdx) {
        self.fade_out(v);
        self.stolen += 1;
    }

    fn fade_out(&mut self, v: VoiceIdx) {
        self.state[v] = VoiceState::Stolen;
        self.fade_left[v] = STEAL_FADE_SEGMENTS;
        if self.mono == Some(v) {
            self.mono = None;
        }
    }

    /// The lowest free slot; if every slot is busy, the fading voice with the least fade left (lowest slot on a
    /// tie) is cut and reported as ended at `frame`.
    fn take_slot(&mut self, frame: u64) -> VoiceIdx {
        if let Some(v) = self.state.iter().position(|s| *s == VoiceState::Free) {
            return v;
        }
        let v = (0..MAX_VOICES)
            .filter(|&v| self.state[v] == VoiceState::Stolen)
            .min_by_key(|&v| self.fade_left[v])
            // Unreachable with polyphony ≤ 16 (more than 16 voices busy means some are fading); the oldest voice is
            // the fallback, so a note is never lost.
            .or_else(|| (0..MAX_VOICES).min_by_key(|&v| self.age[v]))
            .unwrap_or(0);
        self.cut += 1;
        self.report_end(v, frame);
        if self.mono == Some(v) {
            self.mono = None;
        }
        v
    }

    fn start(&mut self, v: VoiceIdx, key: NoteKey, note_index: u64) {
        self.state[v] = VoiceState::Held;
        self.keys[v] = key;
        self.age[v] = note_index;
        self.fade_left[v] = 0;
    }

    /// Release every held voice matching `query` (wildcards allowed); `released(v)` is called for each, in
    /// ascending slot order. In mono and legato, releasing the sounding key while others are held returns the voice
    /// to the newest held key without a retrigger: that is returned as `(voice, key)`.
    pub fn note_off(
        &mut self,
        query: NoteKey,
        mut released: impl FnMut(VoiceIdx),
    ) -> Option<(VoiceIdx, NoteKey)> {
        if self.mode == VoiceMode::Poly {
            for v in 0..MAX_VOICES {
                if self.state[v] == VoiceState::Held && query.matches(&self.keys[v]) {
                    self.state[v] = VoiceState::Released;
                    released(v);
                }
            }
            return None;
        }
        // Held voices left over from poly mode (a structural edit) release as in poly.
        for v in 0..MAX_VOICES {
            if self.state[v] == VoiceState::Held
                && Some(v) != self.mono
                && query.matches(&self.keys[v])
            {
                self.state[v] = VoiceState::Released;
                released(v);
            }
        }
        let top = self.stack.top();
        self.stack.remove(query);
        let m = self.mono.filter(|&m| self.state[m] == VoiceState::Held)?;
        match self.stack.top() {
            None => {
                self.state[m] = VoiceState::Released;
                released(m);
                None
            }
            Some(new_top) if Some(new_top) != top => {
                self.keys[m] = new_top;
                Some((m, new_top))
            }
            Some(_) => None,
        }
    }

    /// Fade out every playing voice matching `query` over 64 frames (CLAP `NOTE_CHOKE`); `choked(v)` per voice.
    pub fn choke(&mut self, query: NoteKey, mut choked: impl FnMut(VoiceIdx)) {
        for v in 0..MAX_VOICES {
            if matches!(self.state[v], VoiceState::Held | VoiceState::Released)
                && query.matches(&self.keys[v])
            {
                self.fade_out(v);
                choked(v);
            }
        }
        self.stack.remove(query);
        if self.mono.is_none() {
            self.stack.clear();
        }
    }

    /// The steal fade of voice `v` this segment as (gain at the start, gain at the end), or `None` if it is not
    /// fading.
    #[inline]
    #[must_use]
    pub fn fade(&self, v: VoiceIdx) -> Option<(f32, f32)> {
        if self.state[v] != VoiceState::Stolen {
            return None;
        }
        let left = f32::from(self.fade_left[v]);
        let n = f32::from(STEAL_FADE_SEGMENTS);
        Some((left / n, (left - 1.0) / n))
    }

    /// End of a segment that started at `frame`: count down the steal fades, and free every voice whose fade is
    /// over or for which `finished(v)` (its amplitude envelope is done). Returns whether any voice ended.
    pub fn end_segment(&mut self, frame: u64, mut finished: impl FnMut(VoiceIdx) -> bool) -> bool {
        let end = frame + SEG as u64;
        let mut any = false;
        for i in 0..self.active_len {
            let v = self.active[i];
            let done = if self.state[v] == VoiceState::Stolen {
                self.fade_left[v] = self.fade_left[v].saturating_sub(1);
                self.fade_left[v] == 0 || finished(v)
            } else {
                finished(v)
            };
            if done {
                self.report_end(v, end);
                self.state[v] = VoiceState::Free;
                if self.mono == Some(v) {
                    self.mono = None;
                    // The voice ended under held keys (a zero-sustain envelope): the stack keeps them, so the next
                    // note-on overlaps for glide but starts a fresh voice.
                }
                any = true;
            }
        }
        if any {
            self.rebuild_active();
        }
        any
    }

    fn report_end(&mut self, v: VoiceIdx, frame: u64) {
        if let Some(slot) = self.ends.get_mut(self.ends_len) {
            *slot = NoteEnd {
                key: self.keys[v],
                frame,
            };
            self.ends_len += 1;
        } else {
            self.ends_dropped += 1;
        }
    }

    /// Free slot `v` at once (no report). For tests and teardown.
    pub fn end(&mut self, v: VoiceIdx) {
        if let Some(s) = self.state.get_mut(v)
            && *s != VoiceState::Free
        {
            *s = VoiceState::Free;
            if self.mono == Some(v) {
                self.mono = None;
            }
            self.rebuild_active();
        }
    }

    /// The voice ends reported since the last [`VoiceManager::clear_ends`].
    #[must_use]
    pub fn ends(&self) -> &[NoteEnd] {
        &self.ends[..self.ends_len]
    }

    /// Forget the reported voice ends (the engine does this at the start of each `process` call).
    pub fn clear_ends(&mut self) {
        self.ends_len = 0;
    }

    /// The playing voices (held, releasing or fading), in ascending slot order: the fixed voice and summation
    /// order.
    #[must_use]
    pub fn active(&self) -> &[VoiceIdx] {
        &self.active[..self.active_len]
    }

    /// What slot `v` is doing.
    #[must_use]
    pub fn state(&self, v: VoiceIdx) -> VoiceState {
        self.state[v]
    }

    /// The note playing in slot `v` (meaningful while it is not free; the newest held key for the mono voice).
    #[must_use]
    pub fn key(&self, v: VoiceIdx) -> NoteKey {
        self.keys[v]
    }

    /// The voice mode.
    #[must_use]
    pub fn mode(&self) -> VoiceMode {
        self.mode
    }

    /// Whether any voice has its key down.
    #[must_use]
    pub fn any_held(&self) -> bool {
        self.state.contains(&VoiceState::Held)
    }

    /// Note-ons since reset (the next note index).
    #[must_use]
    pub fn note_count(&self) -> u64 {
        self.note_count
    }

    /// Note-ons dropped. Always 0 since S2b: a full engine steals instead.
    #[must_use]
    pub fn dropped_notes(&self) -> u64 {
        self.dropped_notes
    }

    /// Voices faded out to make room for a note.
    #[must_use]
    pub fn stolen(&self) -> u64 {
        self.stolen
    }

    /// Voices cut without a fade because all 32 slots were busy.
    #[must_use]
    pub fn cut(&self) -> u64 {
        self.cut
    }

    /// Voice ends that did not fit the report (more than [`NOTE_END_CAPACITY`] in one `process` call).
    #[must_use]
    pub fn dropped_ends(&self) -> u64 {
        self.ends_dropped
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

/// Glide state per voice: linear in semitones at constant time (see the module docs).
#[derive(Clone, Debug)]
pub struct Glide {
    target: [f32; MAX_VOICES],
    step: [f32; MAX_VOICES],
    left: [u32; MAX_VOICES],
    /// The pitch of the latest note-on (or legato return): where an `always` glide starts in poly mode.
    last_pitch: Option<f32>,
}

impl Default for Glide {
    fn default() -> Self {
        Self::new()
    }
}

/// Segments a glide of `ms` lasts at `sample_rate`: `max(1, round(ms · sr / 1000 / 32))`, or 0 when `ms` is 0.
#[must_use]
pub fn glide_segments(ms: f32, sample_rate: f32) -> u32 {
    if ms.is_nan() || ms <= 0.0 {
        return 0;
    }
    let n = f64::from(ms) * f64::from(sample_rate) / 1000.0 / SEG as f64;
    n.round().clamp(1.0, f64::from(u32::MAX)) as u32
}

impl Glide {
    /// No glide in flight.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            target: [0.0; MAX_VOICES],
            step: [0.0; MAX_VOICES],
            left: [0; MAX_VOICES],
            last_pitch: None,
        }
    }

    /// Where a glide into a new note starts, if it glides at all: the voice's current pitch for a retrigger or a
    /// legato change, the previous note-on's pitch for a fresh voice. `None`: jump.
    #[must_use]
    pub fn origin(
        &self,
        action: NoteOn,
        mode: GlideMode,
        segments: u32,
        notes: &NoteState,
    ) -> Option<f32> {
        if segments == 0 || (mode == GlideMode::Legato && !action.overlapping()) {
            return None;
        }
        match action {
            NoteOn::Start { .. } => self.last_pitch,
            NoteOn::Retrigger { voice, .. } | NoteOn::Legato { voice } => {
                Some(notes.pitch_cur[voice])
            }
        }
    }

    /// Voice `v` moves to `pitch`: from `from` over `segments` (a glide), or at once. A fresh voice (`fresh`) or a
    /// jump sets the pitch at the segment start too; a glide starts from the current pitch.
    pub fn go(
        &mut self,
        v: VoiceIdx,
        pitch: f32,
        from: Option<f32>,
        segments: u32,
        fresh: bool,
        notes: &mut NoteState,
    ) {
        self.last_pitch = Some(pitch);
        match from {
            Some(start) if segments > 0 && start.to_bits() != pitch.to_bits() => {
                if fresh {
                    notes.pitch_prev[v] = start;
                }
                notes.pitch_cur[v] = start;
                self.target[v] = pitch;
                self.left[v] = segments;
                self.step[v] = (pitch - start) / segments as f32;
            }
            _ => {
                notes.pitch_prev[v] = pitch;
                notes.pitch_cur[v] = pitch;
                self.left[v] = 0;
            }
        }
    }

    /// Advance every gliding active voice by one segment (after the segment's note events); the last step lands on
    /// the target exactly.
    pub fn advance(&mut self, voices: &[VoiceIdx], notes: &mut NoteState) {
        for &v in voices {
            if self.left[v] > 0 {
                self.left[v] -= 1;
                notes.pitch_cur[v] = if self.left[v] == 0 {
                    self.target[v]
                } else {
                    notes.pitch_cur[v] + self.step[v]
                };
            }
        }
    }

    /// Whether voice `v` is gliding.
    #[must_use]
    pub fn is_gliding(&self, v: VoiceIdx) -> bool {
        self.left[v] > 0
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

    fn slot(action: NoteOn) -> VoiceIdx {
        action.voice()
    }

    #[test]
    fn lowest_free_slot_and_ascending_active_list() {
        let mut m = VoiceManager::new(8);
        assert_eq!(slot(m.note_on(NoteKey::new(60, 0), 0).1), 0);
        assert_eq!(slot(m.note_on(NoteKey::new(64, 1), 0).1), 1);
        assert_eq!(m.note_on(NoteKey::new(67, 2), 0).0, 2);
        m.end(1);
        assert_eq!(m.active(), &[0, 2]);
        let (index, action) = m.note_on(NoteKey::new(72, 3), 0);
        assert_eq!((index, slot(action)), (3, 1), "reuses the lowest free slot");
        assert_eq!(m.active(), &[0, 1, 2]);
    }

    #[test]
    fn note_off_releases_matching_held_voices_only() {
        let mut m = VoiceManager::new(8);
        m.note_on(NoteKey::new(60, 0), 0);
        m.note_on(NoteKey::new(60, 1), 0); // same key, another note id: overlapping voices
        m.note_on(NoteKey::new(62, 2), 0);
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
    fn full_polyphony_steals_the_oldest_released_then_the_oldest_held() {
        let mut m = VoiceManager::new(3);
        m.note_on(NoteKey::new(60, 0), 0); // slot 0, age 0
        m.note_on(NoteKey::new(62, 1), 0); // slot 1, age 1
        m.note_on(NoteKey::new(64, 2), 0); // slot 2, age 2
        m.note_off(off(62), |_| {});
        // Full: the released voice (slot 1) is the victim, though slot 0 is older.
        let (_, a) = m.note_on(NoteKey::new(65, 3), 32);
        assert_eq!(m.state(1), VoiceState::Stolen);
        assert_eq!(slot(a), 3, "the new note takes a spare slot");
        // Full again, nothing released: the oldest held voice (slot 0) is the victim.
        let (_, b) = m.note_on(NoteKey::new(67, 4), 32);
        assert_eq!(m.state(0), VoiceState::Stolen);
        assert_eq!(slot(b), 4);
        assert_eq!(m.stolen(), 2);
        assert_eq!(m.dropped_notes(), 0);
        assert_eq!(m.fade(0), Some((1.0, 0.5)));
        assert_eq!(m.active(), &[0, 1, 2, 3, 4]);
        // Two segments later both fades are over; the slots are free and reported.
        m.clear_ends();
        m.end_segment(32, |_| false);
        assert_eq!(m.fade(0), Some((0.5, 0.0)));
        m.end_segment(64, |_| false);
        assert_eq!(m.active(), &[2, 3, 4]);
        let ends: Vec<(i16, u64)> = m.ends().iter().map(|e| (e.key.key, e.frame)).collect();
        assert_eq!(ends, [(60, 96), (62, 96)]);
    }

    #[test]
    fn a_burst_of_steals_never_loses_a_note() {
        let mut m = VoiceManager::new(16);
        for i in 0..200 {
            let (index, a) = m.note_on(NoteKey::new((i % 128) as u8, i), 0);
            assert_eq!(index, i as u64);
            assert_eq!(m.state(slot(a)), VoiceState::Held);
            let playing = (0..MAX_VOICES)
                .filter(|&v| matches!(m.state(v), VoiceState::Held | VoiceState::Released))
                .count();
            assert!(playing <= 16);
        }
        assert!(m.cut() > 0, "32 busy slots force cuts");
        assert_eq!(m.active().len(), MAX_VOICES);
    }

    #[test]
    fn the_same_performance_gives_the_same_slots() {
        let run = || {
            let mut m = VoiceManager::new(4);
            let mut slots = Vec::new();
            for i in 0..64u32 {
                let key = 40 + (i * 7 % 40) as u8;
                let (_, a) = m.note_on(NoteKey::new(key, i as i32), 0);
                slots.push(slot(a));
                if i % 3 == 0 {
                    m.note_off(off(40 + (i * 5 % 40) as u8), |_| {});
                }
                m.end_segment(u64::from(i) * 32, |v| v % 5 == 0);
            }
            slots
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn mono_retriggers_and_returns_to_the_newest_held_key() {
        let mut m = VoiceManager::with_mode(VoiceMode::Mono, 8);
        let (i0, a) = m.note_on(NoteKey::new(60, 0), 0);
        assert_eq!(
            a,
            NoteOn::Start {
                voice: 0,
                overlapping: false
            }
        );
        let (i1, b) = m.note_on(NoteKey::new(64, 1), 0);
        assert_eq!((i0, i1), (0, 1));
        assert_eq!(
            b,
            NoteOn::Retrigger {
                voice: 0,
                overlapping: true
            }
        );
        m.note_on(NoteKey::new(67, 2), 0);
        // Releasing a key that is not sounding changes nothing.
        assert_eq!(m.note_off(off(64), |_| panic!("no release")), None);
        // Releasing the sounding key returns to the newest held one (60).
        let back = m.note_off(off(67), |_| panic!("no release"));
        assert_eq!(back, Some((0, NoteKey::new(60, 0))));
        let mut released = Vec::new();
        assert_eq!(m.note_off(off(60), |v| released.push(v)), None);
        assert_eq!(released, [0]);
        assert_eq!(m.state(0), VoiceState::Released);
        // A note in the release tail retriggers the same voice.
        let (_, c) = m.note_on(NoteKey::new(62, 3), 0);
        assert_eq!(
            c,
            NoteOn::Retrigger {
                voice: 0,
                overlapping: false
            }
        );
        assert_eq!(m.active(), &[0]);
        // Once it has ended, a note starts a fresh voice.
        m.note_off(off(62), |_| {});
        m.end_segment(0, |_| true);
        let (_, d) = m.note_on(NoteKey::new(62, 4), 0);
        assert!(matches!(d, NoteOn::Start { voice: 0, .. }));
    }

    #[test]
    fn legato_changes_pitch_only_while_keys_overlap() {
        let mut m = VoiceManager::with_mode(VoiceMode::Legato, 8);
        m.note_on(NoteKey::new(60, 0), 0);
        assert_eq!(
            m.note_on(NoteKey::new(62, 1), 0).1,
            NoteOn::Legato { voice: 0 }
        );
        m.note_off(off(62), |_| {});
        m.note_off(off(60), |_| {});
        assert!(matches!(
            m.note_on(NoteKey::new(64, 2), 0).1,
            NoteOn::Retrigger {
                voice: 0,
                overlapping: false
            }
        ));
    }

    #[test]
    fn the_stack_holds_16_keys_and_drops_the_oldest() {
        let mut m = VoiceManager::with_mode(VoiceMode::Legato, 8);
        for k in 0..20u8 {
            m.note_on(NoteKey::new(40 + k, i32::from(k)), 0);
        }
        // Release the newest 15: the voice walks back down the stack, never to keys 40–43 (pushed out).
        for k in (5..20u8).rev() {
            let back = m.note_off(off(40 + k), |_| panic!("still held"));
            assert_eq!(back.map(|(_, key)| key.key), Some(i16::from(40 + k - 1)));
        }
        let mut released = false;
        m.note_off(off(44), |_| released = true);
        assert!(released, "keys 40-43 fell off the 16-key stack");
    }

    #[test]
    fn mode_changes_adopt_the_held_voices() {
        let mut m = VoiceManager::new(8);
        m.note_on(NoteKey::new(60, 0), 0);
        m.note_on(NoteKey::new(64, 1), 0);
        m.configure(VoiceMode::Mono, 8);
        // Slot 1 (newest held) is the mono voice; a new key retriggers it.
        assert!(matches!(
            m.note_on(NoteKey::new(67, 2), 0).1,
            NoteOn::Retrigger { voice: 1, .. }
        ));
        // The left-over poly voice (slot 0) still releases.
        let mut released = Vec::new();
        m.note_off(off(60), |v| released.push(v));
        assert_eq!(released, [0]);
        m.configure(VoiceMode::Poly, 2);
        assert_eq!(m.polyphony(), 2);
    }

    #[test]
    fn choke_fades_matching_voices() {
        let mut m = VoiceManager::new(8);
        m.note_on(NoteKey::new(60, 0), 0);
        m.note_on(NoteKey::new(62, 1), 0);
        let mut choked = Vec::new();
        m.choke(off(62), |v| choked.push(v));
        assert_eq!(choked, [1]);
        assert_eq!(m.state(1), VoiceState::Stolen);
        assert_eq!(m.stolen(), 0, "a choke is not a steal");
    }

    #[test]
    fn glide_is_linear_in_semitones_at_constant_time() {
        assert_eq!(glide_segments(0.0, 48_000.0), 0);
        assert_eq!(glide_segments(100.0, 48_000.0), 150);
        assert_eq!(
            glide_segments(0.1, 48_000.0),
            1,
            "any glide is at least one segment"
        );
        let mut g = Glide::new();
        let mut notes = NoteState::new();
        g.go(0, 60.0, None, 4, true, &mut notes);
        assert_eq!((notes.pitch_prev[0], notes.pitch_cur[0]), (60.0, 60.0));
        let action = NoteOn::Legato { voice: 0 };
        let from = g.origin(action, GlideMode::Always, 4, &notes);
        assert_eq!(from, Some(60.0));
        g.go(0, 72.0, from, 4, false, &mut notes);
        let mut seen = Vec::new();
        for _ in 0..6 {
            notes.pitch_prev[0] = notes.pitch_cur[0];
            g.advance(&[0], &mut notes);
            seen.push(notes.pitch_cur[0]);
        }
        assert_eq!(seen, [63.0, 66.0, 69.0, 72.0, 72.0, 72.0]);
        // Legato glide mode: only overlapping notes glide.
        let start = NoteOn::Start {
            voice: 1,
            overlapping: false,
        };
        assert_eq!(g.origin(start, GlideMode::Legato, 4, &notes), None);
        assert_eq!(g.origin(start, GlideMode::Always, 4, &notes), Some(72.0));
    }
}
