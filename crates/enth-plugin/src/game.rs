//! **Owner: lane SKR** (the round skeleton, freeze point F2r). Pre-declared empty by lane S0r (R1/S1).
//!
//! The plugin's side of a Recreate round (report 01 §2.6, summary "A/B"): the clock it passes to the session's
//! `game` module, the worker thread that renders the target and scores, and in `process()` the target player (a
//! preallocated, loudness-matched loop handed over through an `rtrb` slot with a return ring) and the phrase player
//! that feeds the live engine in sync, with a 10 ms A/B crossfade. Nothing here may allocate, lock or log on the
//! audio thread. The local round log (JSONL; lane GK's schema) is written from here too, never from `process()`.
