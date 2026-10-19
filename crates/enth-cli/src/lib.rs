//! enth-cli (L8, see docs/architecture.md and docs/agent-cli.md): the `enth` command line (P1a lane C).
//!
//! A thin binary over [`run`]`(args, stdin) -> `[`Output`]` { code, stdout, stderr }`, so snapshot tests run in
//! process; a handful of end-to-end tests spawn the built binary for TTY detection and exit codes.
//! Shells translate; they never decide (architecture rule 6): renders come from `enth-render`, measurements and
//! expectations from `enth-analysis`, edits and errors from `enth-patch`, and hints from `enth-solver`
//! through the seam in [`fix`].
//!
//! Conventions every command follows (docs/agent-cli.md):
//!
//! - One JSON object on stdout when stdout is not a TTY, a table (or markdown) when it is; `--format json|table`
//!   overrides. Every object carries `"schema": "enth.<command>/<version>"`; keys are sorted; every non-integral
//!   number is rounded to 4 significant digits (integral numbers print exactly), except inside `ops` and `inverse`
//!   arrays, which stay exact so they can be replayed.
//! - Real units in and out (`800Hz`, `40ms`, `-6dB`); normalised values never appear.
//! - Stable exit codes ([`error`]): 0 ok, 1 `check` had failures, 2 usage, 3 validation, 4 render or analysis, 5
//!   file not found. Errors are `enth.error/1` objects on stdout with a `hint` and `did_you_mean`.
//! - Every output is a Rust type in [`schema`] (`Serialize + Deserialize + JsonSchema`, `deny_unknown_fields`);
//!   `enth schema <name>` prints its JSON Schema.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "shell crate: owns I/O, clocks and threads, and prints to stdout and stderr (root clippy.toml)"
)]

mod aliases;
mod cli;
mod cmd;
pub mod error;
pub mod eval;
pub mod fix;
mod guide;
pub mod knobs;
pub mod out;
mod patchio;
pub mod perf;
pub mod schema;
mod table;
mod templates;
pub mod wav;

use std::ffi::OsString;

pub use error::{
    CliError, EXIT_CHECK_FAILED, EXIT_NOT_FOUND, EXIT_OK, EXIT_RENDER, EXIT_USAGE, EXIT_VALIDATION,
};

/// What a call printed and how it exited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// The exit code ([`error`] lists them).
    pub code: i32,
    /// One JSON object and a newline (or a table, or help text).
    pub stdout: String,
    /// Free text for humans; never needed by an agent.
    pub stderr: String,
}

/// What the shell around [`run_with`] knows about its environment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Env {
    /// stdout is a terminal: print tables instead of JSON (unless `--format` says otherwise).
    pub stdout_tty: bool,
}

/// Run one `enth` command, as if stdout were not a terminal (JSON output). `args` excludes the program name:
/// `run(["check", "p.patch.json", "--expect", "attack<10ms"], "")`. `stdin` is read by arguments given as `-`
/// (`--ops -`).
pub fn run<I, T>(args: I, stdin: &str) -> Output
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_with(args, stdin, Env::default())
}

/// [`run`] with an explicit environment (the binary passes whether stdout is a terminal).
pub fn run_with<I, T>(args: I, stdin: &str, env: Env) -> Output
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let argv: Vec<OsString> = std::iter::once(OsString::from("enth"))
        .chain(args.into_iter().map(Into::into))
        .collect();
    cmd::dispatch(&argv, stdin, env)
}
