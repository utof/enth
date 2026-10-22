//! **Owner: lane CV3** (Challenge v3, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e); empty
//! until CV3 writes it. The reference is docs/ear.md, "Round types", and docs/challenge.md, "Scoring interfaces".
//!
//! The ear scorers, each with a [`ScorerId`](crate::ScorerId) beside the old ones (a scorer that would move any score
//! gets a new id, never an edit):
//!
//! | Id | Judges | How |
//! | --- | --- | --- |
//! | `ear/1` | a label or clip index | exactly, against the seeded key; replayed from the answers for staircases |
//! | `ear-value/1` | a value on a knob or slider | the error in perceptual units (octaves, log ms) |
//! | `taps/1` | tap times | the intervals between taps after a best-fit offset, a one-to-one match, +-10 % tempo, misses and extras |
//!
//! They judge with no render, so a result verifies on any platform and engine version. CV3 hooks them into
//! [`crate::score::round`]'s dispatch from this file and adds them to [`crate::score::KNOWN_SCORERS`], additively.
