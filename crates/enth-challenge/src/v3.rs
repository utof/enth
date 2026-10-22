//! **Owner: lane CV3** (Challenge v3, freeze point F1e; the F1e judge reviews it with the other R4 and R5 APIs).
//! Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until CV3 designs it. A hook only: nothing in v1 or v2
//! changes here. The reference is docs/challenge.md, "Challenge v3 (planned, lane CV3, R4)", and docs/ear.md, "Where R4
//! lives".
//!
//! Challenge v2 has `Target::Draw` and `Target::Patch`, and every answer is a patch. R4's games need a target that is
//! not a patch and answers that are not patches. v3 is additive like v2: v1 and v2 files read as before and are written
//! at the lowest version that holds them, so no R1 to R3 challenge changes its bytes, and the golden challenges and
//! generator v1's draws do not move.
//!
//! | Piece | What |
//! | --- | --- |
//! | `Target::Ear(EarSpec)` | a question set from `enth-music`'s frozen ear generator: `EarSpec { game, generator, level, seed, count }`, integers only, the seed below 2^53; strict like every target (phrase 0, the template start) |
//! | `AnswerKind::{Choice, Value, Taps}` | a label or clip index, judged exactly; a value on a knob or slider, scored as an error in perceptual units; tap times, scored on the intervals between taps after a best-fit offset. `Patch` stays the default |
//!
//! The question-to-sound side is [`crate::ear`]; the scorers `ear/1`, `ear-value/1` and `taps/1` are in [`crate::ear::scorers`].
//! The new schema string is `enth.challenge/3`; a newer one is refused with a hint.
