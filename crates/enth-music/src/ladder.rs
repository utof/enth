//! **Owner: lane MUS** (level 1 of each ladder file), then the lane named in each file: G1, G2, G3 and TAP. Pre-declared
//! by lane S0e (R4/R5, freeze point F0e); empty until they write them.
//!
//! The difficulty ladders of the music games (docs/ear.md, "The games" and "Difficulty and the staircase"): per level,
//! which items a game may draw and how far apart its options are. Placement is by measured distance, never by random
//! settings, and `ear-check` (lane ECK, `cargo xtask ear-check`) measures every level (docs/ear.md, "GR4"). One file per
//! game family, so the ladder lanes never edit each other's: a ladder change moves ear generator v1's golden on purpose
//! until INT4's friend build freezes it, and the commit names the games.

pub mod bands;
pub mod chords;
pub mod intervals;
pub mod pitch;
pub mod progressions;
pub mod rhythm;
pub mod taps;
