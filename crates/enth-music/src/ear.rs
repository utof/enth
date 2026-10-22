//! **Owner: lane MUS** (Music, freeze point F1e; the F1e judge fixes the real types). Pre-declared by lane S0e (R4/R5,
//! freeze point F0e); empty until MUS writes it. The reference is docs/ear.md, "The ear generator".
//!
//! **Ear generator v1**: an `EarSpec` (game, generator, level, seed, count; the seed below 2^53) becomes `count`
//! questions. A question is integers all the way down: notes (key, velocity, start and hold in ms), a length, a timbre
//! id into `enth-challenge`'s frozen pool, an optional post-EQ band, an answer (a choice index, a value as `f32` bits or
//! tap times) and its options. Report 02 §2's sketch:
//!
//! ```text
//! EarSpec  { game: u8, generator: u8, level: u8, seed: u64, count: u8 }
//! Question { timbre: u8, notes: Vec<NoteSpec>, length_ms: u32, post: Option<EqBand>, answer: Answer, options: Options }
//! Answer   { Choice(u8), Value { key: &'static str, bits: u32 }, Taps(Vec<u32>) }
//! ```
//!
//! - **Keyed streams.** Item `i` draws from `derive(seed, fnv1a64("game/item#/level"))`, so adding a game or an item
//!   never moves another's draw; a practice session is (game, seed, settings) plus the answer log.
//! - **Valid by construction.** Every question stays within C1 to C7, at most 8 notes at once and at most 10 s, and every
//!   answer key is one of its options (proptests).
//! - **Frozen by a golden.** 1,000 specs to a hash, on Linux, Windows and wasm (macOS weekly). It freezes at INT4's
//!   friend build, the first that shares ear codes; until then the ladder lanes (G1 to G3) update the golden on purpose
//!   and the commit names the games and why. After it, a change is ear generator v2.
//!
//! The parameter games (Spot the change, Dial it in, Ear check) are not here: they need the catalogue, and live in
//! `enth-challenge` on v2 draws.
