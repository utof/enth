//! **Owner: lane CT** (code v1, freeze point F1r). Pre-declared empty by lane S0r.
//!
//! Challenge codes (report 01 §2.5, summary "`RandomSpec` and codes"): Crockford base32 with a check symbol, a short
//! form of about 13 characters and a long form of about 32, carrying the generator and engine versions and a
//! target-hash prefix. A code from a newer version or generator is refused (`code_too_new`), never guessed.
