//! **Owner: lane DUI** (the duel card, feature `duel`). Pre-declared by lane S0e (R4/R5); empty until the lane writes
//! it.
//!
//! The local duel history (docs/duels.md, "The duel history"): the codes, both re-scored results and the date you played,
//! with optional names the receiver types ("from: Sam"). It lives in a per-user file the shell owns (`enth-plugin`'s
//! `duel/`), and leaves the machine only if you send it.
