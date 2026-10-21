//! **Owner: lane LCK** (`check`, `coverage`, `new`, `pack`), **DEC** (`gen`) and **INT2** (`gr2`). Pre-declared by lane
//! S0l (R2, freeze point F0l); `check` is a placeholder that passes, the rest refuse to run.
//!
//! `enth-lesson-tool`: the lessons' authoring loop (docs/learn.md, "`lesson-check` and the authoring loop"), as a
//! tooling package so that `cargo xtask` stays light (no engine crate in xtask, S0r's rule). `cargo xtask lesson-check`,
//! `lesson-coverage`, `lesson new`, `pack`, `lesson-gen` and `gr2-report` each run `cargo run --release -p
//! enth-lesson-tool -- <command> <args>` and forward the exit status, the way `cargo xtask golden` drives `enth-golden`.
//! One file per command, so the lanes never edit each other's:
//!
//! | Command | File | Does | Lane |
//! | --- | --- | --- | --- |
//! | `check [--cross] [--solve] [PACK...]` | `check.rs` | schemas, text lint, the playthrough of every witness through a headless `Session`, `--cross`, `--solve`, hash pinning | LCK |
//! | `coverage` | `coverage.rs` | every P1 module and Basic-tier parameter has a step and a non-patch check; "E only" and "taught, not checked" listed apart | LCK |
//! | `new <pack> <id>` | `new.rs` | scaffolds a lesson in the authoring layout | LCK |
//! | `pack [PACK...]` | `pack.rs` | builds `.enthpack` files reproducibly (canonical JSON, sorted entries, 1980 timestamps, a fixed deflate level) and checks each bundled pack's pinned hash | LCK |
//! | `gen deconstruct <preset>` | `gen.rs` | writes the factory studies from a preset by ablation | DEC |
//! | `gr2 <progress file>...` | `gr2.rs` | the GR2 numbers from the progress files | INT2 |
//!
//! `.github/workflows/lessons.yml` runs `cargo xtask lesson-check` on `content/lessons/**`, `crates/enth-learn/**`,
//! `tools/lesson/**` and `xtask/**`; LCK adds the DSP, engine, patch and challenge crates (a DSP change can break a
//! lesson) and the `gate.sh lane` hook. The authoring layout is `content/lessons/<pack>/` (see the example pack there).
//! INT2 may read the progress files without this package's dependencies (as `gr1-report` reads round logs), in which
//! case `gr2.rs` here goes and `xtask/src/gr2.rs` does the work.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool: reports go to stdout and stderr"
)]

mod check;
mod coverage;
mod r#gen; // `gen` is a reserved word in edition 2024; the file is still `gen.rs`
mod gr2;
mod new;
mod pack;

use std::process::ExitCode;

const USAGE: &str = "usage: enth-lesson-tool <command> [args...]
  check [--cross] [--solve] [<pack>...]   (placeholder: checks nothing and passes until R2 lane LCK builds it)
  coverage                                (not built yet: R2 lane LCK)
  new <pack> <id>                         (not built yet: R2 lane LCK)
  pack [<pack>...]                        (not built yet: R2 lane LCK)
  gen deconstruct <preset>                (not built yet: R2 lane DEC)
  gr2 <progress file>...                  (not built yet: R2 lane INT2)
Run it through `cargo xtask lesson-check`, `lesson-coverage`, `lesson new`, `pack`, `lesson-gen` and `gr2-report`.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check") => check::run(&args[1..]),
        Some("coverage") => coverage::run(&args[1..]),
        Some("new") => new::run(&args[1..]),
        Some("pack") => pack::run(&args[1..]),
        Some("gen") => r#gen::run(&args[1..]),
        Some("gr2") => gr2::run(&args[1..]),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("enth-lesson-tool: {msg}");
            ExitCode::FAILURE
        }
    }
}
