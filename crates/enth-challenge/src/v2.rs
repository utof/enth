//! **Owner: lane CV2** (Challenge v2, freeze point F1l; the F1l judge reviews it with the lesson model and the
//! session's Learn API). Pre-declared by lane S0l (R2); empty until CV2 designs it. A hook only: nothing in v1
//! changes here.
//!
//! Challenge v1 has only `Target::Seed`, `Start::Template`, an empty `Constraints` and `AnswerKind::Patch`, so it
//! cannot express a lesson. Challenge v2 is additive and reads v1 (docs/challenge.md, "Challenge v2"; docs/learn.md,
//! "Challenge v2"; the open questions Q4, Q5 and Q7 are decided at F1l):
//!
//! | Piece | What |
//! | --- | --- |
//! | `Target::Draw` | a seeded draw over a parameter list with value sets, in parameter space with design §9's keyed streams and just-noticeable steps, so a new seed gives a sibling for a redo or a parallel test form |
//! | `Target::Patch` | an explicit target patch (hand-made or generated) |
//! | `Target::Expectations` | an expectation list, as `enth check` reads it |
//! | `Start::Patch` | the step's own start patch, not a template |
//! | `Constraints { editable, locked }` | what the player may change; what the session refuses |
//! | `AnswerKind::Choice` | an ear question: options rendered live, pairwise at least 2 just-noticeable steps apart |
//!
//! A v1 challenge reads as v2, the golden challenges and `enth compare --target` stay byte-identical, and the
//! generator's v1 draws do not move. The new schema string is `enth.challenge/2`; a newer one is refused with a hint.
