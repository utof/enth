//! **Owner: lane RC** (Result codes, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until
//! RC writes it. The reference is docs/duels.md ("The two codes") and docs/challenge.md, "Version 15: the kinds behind
//! the escape".
//!
//! The **answer code** (kind [`KIND_ANSWER`](crate::result::KIND_ANSWER)): the salt, the challenge and the patch as
//! `ops_between(start, answer)`, checked, in binary: each op a kind and a module reference, a `SetParam` adding a
//! parameter index and a value (enums by item count, toggles one bit, floats as exact `f32` bits). Indices go through
//! **`result-params-v1`**, a frozen, append-only table, as generator v1 freezes its catalogue snapshot. About 50, 135
//! and 255 symbols at levels 1, 3 and 5 (real answers may be 1.2 to 2 times longer; lane SKD measures them). A 40-bit
//! hash tail checks it, then code v1's GF(32) symbol. `SetMeta`, labels, macro names and slots are stripped before
//! encoding, so no name or `meta.author` survives. An op on a module kind this build lacks is `answer_too_new`.
