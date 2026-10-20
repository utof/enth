//! **Owner: lane RG** (the randomizer). Pre-declared empty by lane S0r.
//!
//! The generator (design.md §9, report 01 §2.3): keyed streams, each value drawn from
//! `derive(derive(seed, fnv1a64(address)), attempt)`; the observability mask (never randomize what the phrase cannot
//! reveal); the minimum audible step; reachable quantization (3 significant digits, depths in 5 % steps); structure
//! over the starter superset. Old generator versions stay frozen, with a 1,000-hash golden. Parameter space only:
//! nothing here may depend on a render, so `random` never reaches `enth-render` (RG's acceptance checks it).
