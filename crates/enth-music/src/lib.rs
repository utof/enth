//! enth-music (L1, see docs/architecture.md and docs/ear.md): the music side of R4's ear games. Theory tables, a 16-step
//! rhythm grid with a syncopation measure, note names, and **ear generator v1**: an `EarSpec` goes in, questions come
//! out (notes, a timbre id, an answer key and its options, all integers), frozen by a golden like Recreate's generator
//! v1. The R4 and R5 research session (`_local/sessions/2026-10-22-r4-r5-ear-games-and-duels/`, `summary.md`, report 02
//! §2) put it in a crate of its own, below `enth-challenge`.
//!
//! A core crate: no I/O, no threads, no clocks, no global RNG, **integers only**, and it builds for
//! `wasm32-unknown-unknown` (`cargo xtask wasm-check`, which covers every layer up to 5). Every draw is a keyed stream
//! of `enth-dsp`'s `derive` and `fnv1a64` (`derive(seed, fnv1a64("game/item#/level"))`), so adding a game or an item
//! never moves another's draw. It is L1 so that `enth-challenge` (L4) can embed an `EarSpec` in `Target::Ear` and the
//! UI and the CLI get labels without the challenge crate: an L0 crate could not use `enth-dsp`, and an L4 one could not
//! be embedded by `enth-challenge` (same-layer edges are refused). It cannot read the catalogue (`enth-patch` is L1
//! too), so the parameter games' items (Spot the change, Dial it in, Ear check) are v2 draws built in `enth-challenge`.
//!
//! **Owner: lane MUS** (re-exports at F1e). Lane S0e (R4/R5, freeze point F0e) created the crate and declared every file
//! up front, so the lanes that fill them never edit this one: MUS and the ladder lanes G1 to G3 and TAP name their
//! items by module path (`enth_music::theory::…`) rather than adding re-exports here. Each file names its owner at the
//! top. The reference is docs/ear.md. **Built (lane MUS, 2026-10-22):** the tables, the rhythm grid, ear generator v1
//! and level 1 of every ladder file, with proptests and the generator's golden (`tests/ear_golden.rs`, `tests/golden/`);
//! the ladder lanes add levels (a row in each file's `LEVELS`) and move the golden on purpose until INT4's friend build
//! freezes it. Start at [`ear::EarSpec`] and [`ear::question`].
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`theory`] | interval, chord, scale and progression tables (simple voice leading) and note names | MUS |
//! | [`rhythm`] | the 16-step rhythm grid, density, swing and the syncopation measure (Longuet-Higgins and Lee) | MUS |
//! | [`ear`] | `EarSpec { game, generator, level, seed, count }`, `Question`, `Answer`, `Options` and **ear generator v1**, with its golden (one block of fixed specs per game and level, the same digests on Linux, Windows and wasm) | MUS |
//! | [`ladder`] | the difficulty ladders, one file per game family (below) | MUS (level 1 of each); G1 to G3; TAP |
//!
//! The ladder files, each owned by the lane that tunes it (`ear-check` measures them, docs/ear.md, "GR4"; a ladder
//! change moves ear generator v1's golden on purpose until INT4's friend build freezes it, and the commit names the
//! games):
//!
//! | File | Game | Lane |
//! | --- | --- | --- |
//! | [`ladder::intervals`] | Interval stacks | G1 |
//! | [`ladder::pitch`] | R4b's note naming (opt-in, never in the default workout) | G1 |
//! | [`ladder::chords`] | Chord quality | G2 |
//! | [`ladder::progressions`] | R4b's progressions | G2 |
//! | [`ladder::rhythm`] | Rhythm pattern (listening only) | G2 |
//! | [`ladder::bands`] | Frequency finder's svf and EQ forms | G3 |
//! | [`ladder::taps`] | the tap-back round (after the TAP spike) | TAP |

pub mod ear;
pub mod ladder;
pub mod rhythm;
pub mod theory;
