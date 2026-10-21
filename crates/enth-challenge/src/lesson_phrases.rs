//! **Owner: lane CV2** (Challenge v2, freeze point F1l). Pre-declared by lane S0l (R2); empty until CV2 writes the
//! split.
//!
//! The lesson-only phrase ids and their invariants (docs/learn.md, "Challenge v2"; Q8 decided at F1l). The phrase
//! table is append-only and the generator's phrases (ids 1 to 3) stay untouched, so generator v1's draws do not move;
//! the table splits into generator phrases and lesson-only phrases with their own invariants (short holds and chords
//! are lesson material, not generator material). New ids: `CHORD`, `LONG_HOLD`, `STACCATO` and `STACCATO_RUN`, for
//! lessons 6.1, 8.2, 5.3, 5.4 and 7.1 to 7.4 (content/lessons/PLAN.md, §8.2). [`phrases`](crate::phrases) keeps the
//! generator's table.
