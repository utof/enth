//! **Owner: lane MUS** (level 1 of each ladder file, written), then the lane named in each file: G1, G2, G3 and TAP add
//! the other levels. Pre-declared by lane S0e (R4/R5, freeze point F0e).
//!
//! The difficulty ladders of the music games (docs/ear.md, "The games" and "Difficulty and the staircase"): per level,
//! which items a game may draw and how far apart its options are. Placement is by measured distance, never by random
//! settings, and `ear-check` (lane ECK, `cargo xtask ear-check`) measures every level (docs/ear.md, "GR4"). One file per
//! game family, so the ladder lanes never edit each other's: a ladder change moves ear generator v1's golden on purpose
//! until INT4's friend build freezes it, and the commit names the games.
//!
//! **Every file has the same four things**, so the generator (`crate::ear::question`) and the tools treat them alike:
//!
//! | Item | What |
//! | --- | --- |
//! | `Level` | one row of the ladder: the pool of answers, the number of options, the distance rule between them, the root and timbre ranges, the hold. Its fields are `pub`: `ear-check` lists every level's distances from them |
//! | `LEVELS` | the rows; level `n` is `LEVELS[n - 1]` |
//! | `MAX_LEVEL` | `LEVELS.len()`; `Game::max_level` and `EarSpec::validate_music` read it, so a new row is playable and proptested the moment it lands |
//! | `question(level, rng)` | the item: it draws from the keyed stream `rng` in the order the file's docs name (**part of the frozen form**) and returns a valid `Question` |
//!
//! To add a level: add a row to `LEVELS` (and the code its fields need, which must not change what the existing rows
//! draw), run `cargo test -p enth-music` (the proptests check validity at every level), then `cargo test -p enth-music
//! --test ear_golden -- --ignored regenerate` and say in the commit which games moved and why. A new level appends its
//! own block to the golden and no existing line changes (tuning a level changes that block alone). A level must stay
//! within C1 to C7, at most 8 notes at once and at most 10 s ([`crate::ear::Question::validate`]).

//! **A fifth thing, kept beside them** ([`with_answer`], lane ECK): what a question would sound like if its right answer
//! were another option, everything else held. `ear-check` measures every option pair with it, and a change to a game's
//! `question` that its builder has not followed fails `cargo test -p enth-music` (`tests/with_answer.rs`), so change the
//! builder in the same commit. Tap back has no options and no builder.

pub mod bands;
pub mod chords;
pub mod intervals;
pub mod pitch;
pub mod progressions;
pub mod rhythm;
pub mod taps;
pub mod with_answer;
