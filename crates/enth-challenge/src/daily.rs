//! **Owner: lane RC** (Result codes, freeze point F1e; the F1e judge freezes the formula). Pre-declared by lane S0e
//! (R4/R5, freeze point F0e); empty until RC writes it. The reference is docs/challenge.md, "The daily challenge
//! formula", and docs/duels.md, "The daily challenge".
//!
//! `daily::v1`: one challenge per level per day with no server and no organiser. Integer arithmetic on `enth-dsp`'s
//! pinned `rng`: `seed = derive(derive(fnv1a64(b"enth-daily-v1"), day), level) >> 24` (the top 40 bits), and the code is
//! the short code of preset `level` (1 to 5) with generator 1 and that seed. The day is whole days since 1970-01-01 in
//! UTC and **the shell supplies it**: a core crate reads no clock. A golden of (day, level) to code pins it; after F1e it
//! never changes, and a daily on a newer generator is a new formula (`enth-daily-v2`).
