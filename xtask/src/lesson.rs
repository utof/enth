//! **Owner: lane LCK** (lesson check; `gen.rs` is lane DEC's). Pre-declared by lane S0l (R2); LCK built `lesson-check`,
//! `lesson-coverage`, `lesson new` and `pack` in the tool.
//!
//! The authoring loop for lessons (docs/learn.md, "`lesson-check` and the authoring loop"): an honest lesson is one
//! `cargo xtask lesson-check --solve` passes, over every pack in `content/lessons/`, deterministically and with no
//! spend. The work is in the tooling package `enth-lesson-tool` (`tools/lesson/`), which holds the heavy dependencies
//! (`enth-learn`, `enth-session`, `enth-solver`, `enth-challenge`); xtask stays light, with no engine crate (S0r's rule).
//! Each command here is a thin driver that runs `cargo run --release -p enth-lesson-tool -- <command> <args>` and
//! forwards the exit status, the way `cargo xtask golden` drives `enth-golden`. One file per command, so the lanes never
//! edit each other's (the tool has the same files in `tools/lesson/src/`, where the commands are built):
//!
//! | xtask command | Tool command | File | Does | Lane |
//! | --- | --- | --- | --- | --- |
//! | `lesson-check [--cross] [--solve] [PACK...]` | `check` | `check.rs` | schemas, text lint, the playthrough of every witness through a headless `Session`, `--cross`, `--solve`, hash pinning | LCK |
//! | `lesson-coverage` | `coverage` | `coverage.rs` | every P1 module and Basic-tier parameter has a step and a non-patch check; "E only" and "taught, not checked" listed apart | LCK |
//! | `lesson new <pack> <id>` | `new` | `new.rs` | scaffolds a lesson in the authoring layout | LCK |
//! | `pack [PACK...]` | `pack` | `pack.rs` | builds `.enthpack` files reproducibly (canonical JSON, sorted entries, 1980 timestamps, a fixed deflate level) and checks each bundled pack's pinned hash | LCK |
//! | `lesson-gen deconstruct <preset>` | `gen deconstruct <preset>` | `gen.rs` | writes the factory studies from a preset by ablation | DEC |
//!
//! `.github/workflows/lessons.yml` runs `lesson-check --cross --solve`, `lesson-coverage` (a report until GR2) and `pack
//! --check` on `content/lessons/**`, the lesson tool, xtask and every crate a lesson's verdict depends on (a DSP change
//! can break a lesson); `scripts/gate.sh lane` runs `lesson-check` when `content/lessons/` changes. The authoring layout
//! is `content/lessons/<pack>/` (see the example pack there).

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

/// Runs `enth-lesson-tool <command> <args>` through cargo (release: the playthroughs and the solver are slow in debug)
/// and succeeds if the tool does. The tool's own report is on the terminal already, so a failure only says which command.
pub(crate) fn drive(command: &str, args: &[String]) -> Result<(), String> {
    let status = super::cargo()
        .args([
            "run",
            "--quiet",
            "--release",
            "--package",
            "enth-lesson-tool",
            "--bin",
            "enth-lesson-tool",
            "--",
            command,
        ])
        .args(args)
        .status()
        .map_err(|e| format!("cannot run cargo run: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("enth-lesson-tool {command}: failed (report above)"))
    }
}
