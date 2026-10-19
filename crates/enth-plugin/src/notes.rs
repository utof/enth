//! Notes: the note tuple, `NOTE_END` events, voice info and polyphonic modulation keyed by the host's voice
//! ids (read `VoiceID::id()`, never `id_or_fallback`) (lane PL).
//!
//! Declared empty by lane S0b (P1b); the owning lane fills it.
