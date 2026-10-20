//! enth-challenge (L4, see docs/architecture.md and design.md §6 and §9): the [`Challenge`](model) behind a Recreate
//! round (and later a lesson step, a daily, a duel and "create from sound"), the Recreate randomizer, challenge codes,
//! the phrases a challenge plays, and the scorers. The R1/S1 research session
//! (`_local/sessions/2026-10-20-r1-s1-recreate-and-instrument/`, `summary.md`) placed all of it here.
//!
//! A core crate: no I/O, no threads, no clocks, no global RNG, and it builds for `wasm32-unknown-unknown` (`cargo
//! xtask wasm-check`). Every random draw is keyed by its parameter address (`derive(derive(seed, fnv1a64(address)),
//! attempt)`), never taken in sequence, and target generation works in parameter space only (design.md §9), so a
//! seed gives the same target on every machine. Time comes from the shell: the session's `game` module passes it in.
//!
//! Lane S0r (R1/S1, freeze point F0r) created the crate and declared every file up front, so the lanes that fill them
//! never edit this one. Each file names its owner at the top.
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`spec`] | [`RandomSpec`] and [`Category`]: the randomizer's integer-only settings (a stub; finalised at F1r) | CT, with RG |
//! | [`model`] | Challenge v1: target, start, phrase, constraints, assists, scoring spec; versioned, with fixtures | CT |
//! | [`code`] | challenge codes: Crockford base32 with a check symbol, short and long forms, `code_too_new` | CT |
//! | [`score`] | the `Scorer` trait and `ScoreReport` ([`score`] itself); sound, structure and combined scores | CT; SS; ST |
//! | [`random`] | the generator: keyed draws, the observability mask, minimum step, frozen generator versions | RG |
//! | [`priors`] | hand-written per-parameter priors relative to the phrase's note; the `musical` blend | RG |
//! | [`phrases`] | the versioned phrase table (one held note, an octave pair, a legato pair) | RG |
//! | [`calibrate`] | per-challenge just-noticeable-difference units and the fitted sound-% curve | SS |
//! | [`loudness`] | an in-house BS.1770 loudness on `libm` for the scored path, bit-exact on every platform | SS |
//!
//! Until those lanes merge, every module except [`spec`] is empty.

pub mod calibrate;
pub mod code;
pub mod loudness;
pub mod model;
pub mod phrases;
pub mod priors;
pub mod random;
pub mod score;
pub mod spec;

pub use spec::{Category, RandomSpec};
