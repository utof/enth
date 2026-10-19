//! The audio block: translate host events and notes into engine events, run `Engine::process`, write the
//! output (lanes SK, PL). Nothing here allocates, frees, locks, logs or does I/O.
//!
//! Declared empty by lane S0b (P1b); the owning lane fills it.
