//! **Owner: lane LCK** (lesson check; `gen.rs` is lane DEC's). Pre-declared by lane S0l (R2); `lesson-check` is a
//! placeholder that passes, the rest refuse to run.
//!
//! The authoring loop for lessons (docs/learn.md, "`lesson-check` and the authoring loop"): an honest lesson is one
//! `cargo xtask lesson-check --solve` passes, over every pack in `content/lessons/`, deterministically and with no
//! spend. One file per command, so the lanes never edit each other's:
//!
//! | Command | File | Does | Lane |
//! | --- | --- | --- | --- |
//! | `lesson-check [--cross] [--solve] [PACK...]` | `check.rs` | schemas, text lint, the playthrough of every witness through a headless `Session`, `--cross`, `--solve`, hash pinning | LCK |
//! | `lesson-coverage` | `coverage.rs` | every P1 module and Basic-tier parameter has a step and a non-patch check; "E only" and "taught, not checked" listed apart | LCK |
//! | `lesson new` | `new.rs` | scaffolds a lesson in the authoring layout | LCK |
//! | `pack [PACK...]` | `pack.rs` | builds `.enthpack` files reproducibly (canonical JSON, sorted entries, 1980 timestamps, a fixed deflate level) and checks each bundled pack's pinned hash | LCK |
//! | `lesson-gen deconstruct` | `gen.rs` | writes the factory studies from a preset by ablation | DEC |
//!
//! `.github/workflows/lessons.yml` runs `lesson-check` on `content/lessons/**`, `crates/enth-learn/**` and `xtask/**`;
//! LCK adds the DSP, engine, patch and challenge crates (a DSP change can break a lesson) and the `gate.sh lane` hook.
//! The authoring layout is `content/lessons/<pack>/` (see the example pack there). xtask depends on `enth-learn` (with its
//! `toml` feature), `enth-session`, `enth-solver` and `enth-challenge` for this (lane S0l's `xtask/Cargo.toml`).

mod check;
mod coverage;
// `gen` is a reserved word in edition 2024, so the module is `r#gen` (the file is still `lesson/gen.rs`).
mod r#gen;
mod new;
mod pack;

pub use check::lesson_check;
pub use coverage::lesson_coverage;
pub use r#gen::lesson_gen;
pub use new::lesson;
pub use pack::pack;
