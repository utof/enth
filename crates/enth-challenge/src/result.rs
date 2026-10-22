//! **Owner: lane RC** (Result codes, freeze point F1e; the F1e judge fixes each layout). Pre-declared by lane S0e (R4/R5,
//! freeze point F0e), which reserved the version-15 kind numbers below and nothing else. The reference is
//! docs/challenge.md, "Version 15: the kinds behind the escape", and docs/duels.md, "The two codes".
//!
//! The kinds behind code v1's reserved version-15 escape ([`CODE_VERSION_ESCAPE`](crate::code::CODE_VERSION_ESCAPE)): a
//! code whose first symbol carries version 15 continues with a 5-bit kind and a 4-bit layout version, then the kind's own
//! fields. This build refuses every one of them today, and a later build refuses a kind or layout it does not know
//! before it reads a field. **Every kind ends with code v1's GF(32) check symbol**, one symbol on top of its own check,
//! so an R1 to R3 build reads "update enth" (`code_too_new`) for a code of 64 symbols or fewer; only longer ones read
//! `bad_length`, which no later change can fix.
//!
//! **The numbers are the proposal in docs/challenge.md.** The F1e judge may renumber them before anything ships; after
//! F1e they never change. Kind 0 is never written (`bad_field`).
//!
//! | Kind | Constant | What it carries | Lane |
//! | --- | --- | --- | --- |
//! | 1 | [`KIND_SCORECARD`] | the spoiler-free scorecard: the challenge, engine, scorer set, scores, time, assists and a salted commitment | RC |
//! | 2 | [`KIND_ANSWER`] | the answer code: the patch as a diff from the round's start through the frozen `result-params-v1` table | RC |
//! | 3 | [`KIND_EAR_CHALLENGE`] | an `EarSpec` on a fixed ladder, for ear duels | CV3 (`ear::code`) |
//! | 4 | [`KIND_EAR_RESULT`] | the ear result: the answers, the time and the assists, verified by replay with no render | RC |
//! | 5 | [`KIND_LESSON_SIBLING`] | reserved: (pack, lesson, step, seed); nothing plays it yet | none |
//!
//! RC writes the scorecard and the ear result here, [`answer`](crate::answer) the answer code, [`verify`](crate::verify)
//! the receiver's re-score, [`paste`](crate::paste) the paste-anything decoding and [`daily`](crate::daily) the daily
//! formula.

/// Kind 1 behind version 15: the **scorecard**, sent on finishing. It spoils nothing.
pub const KIND_SCORECARD: u8 = 1;
/// Kind 2 behind version 15: the **answer code**, swapped after both players have played. It carries the patch as a
/// diff, so it spoils the answer.
pub const KIND_ANSWER: u8 = 2;
/// Kind 3 behind version 15: the **ear challenge** (an `EarSpec` on a fixed ladder), for ear duels.
pub const KIND_EAR_CHALLENGE: u8 = 3;
/// Kind 4 behind version 15: the **ear result** (the answers, the time and the assists), verified by replay.
pub const KIND_EAR_RESULT: u8 = 4;
/// Kind 5 behind version 15: reserved for a lesson sibling (pack, lesson, step, seed). Nothing plays it yet.
pub const KIND_LESSON_SIBLING: u8 = 5;

/// Every reserved kind, in number order.
pub const KINDS: [u8; 5] = [
    KIND_SCORECARD,
    KIND_ANSWER,
    KIND_EAR_CHALLENGE,
    KIND_EAR_RESULT,
    KIND_LESSON_SIBLING,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::CODE_VERSION_ESCAPE;

    #[test]
    fn the_reserved_kinds_are_the_documented_numbers() {
        assert_eq!(
            [
                KIND_SCORECARD,
                KIND_ANSWER,
                KIND_EAR_CHALLENGE,
                KIND_EAR_RESULT,
                KIND_LESSON_SIBLING
            ],
            [1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn kinds_are_distinct_nonzero_and_fit_five_bits() {
        for (i, kind) in KINDS.iter().enumerate() {
            assert!(
                (1..32).contains(kind),
                "kind {kind} must be 1..=31 (0 is never written)"
            );
            assert!(!KINDS[..i].contains(kind), "kind {kind} is reserved twice");
        }
    }

    #[test]
    fn the_kinds_sit_behind_code_v1s_escape() {
        assert_eq!(CODE_VERSION_ESCAPE, 15);
    }
}
