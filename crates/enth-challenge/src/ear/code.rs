//! **Owner: lane CV3** (the ear-challenge code); lane RC owns the ear result (kind 4) in [`crate::result`]. Pre-declared
//! by lane S0e (R4/R5, freeze point F0e); empty until CV3 writes it. The reference is docs/challenge.md, "Version 15:
//! the kinds behind the escape", and docs/duels.md, "Ear duels".
//!
//! The **ear-challenge code**: kind [`KIND_EAR_CHALLENGE`](crate::result::KIND_EAR_CHALLENGE) (3) behind code v1's
//! reserved version-15 escape, carrying an `EarSpec` (game, ear generator, level, seed, count) on a fixed ladder, so two
//! players hear the same items. It is short, and like every version-15 kind it ends with code v1's GF(32) check symbol,
//! so an R1 to R3 build reads "update enth". Anyone holding the code can compute the answer keys, and the UI says so.
