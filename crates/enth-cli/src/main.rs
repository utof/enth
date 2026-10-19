//! The `enth` binary: a thin shell over `enth_cli::run_with` (docs/agent-cli.md). It reads stdin only when an
//! argument is `-` (`--ops -`), tells the library whether stdout is a terminal, prints, and exits with the code.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "shell crate: owns I/O, clocks and threads, and prints to stdout and stderr (root clippy.toml)"
)]

use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let wants_stdin = args.iter().any(|a| {
        let a = a.to_string_lossy();
        a == "-" || a.ends_with("=-")
    });
    let mut stdin = String::new();
    if wants_stdin && std::io::stdin().read_to_string(&mut stdin).is_err() {
        stdin.clear();
    }
    let env = enth_cli::Env {
        stdout_tty: std::io::stdout().is_terminal(),
    };
    let out = enth_cli::run_with(args, &stdin, env);
    // A closed pipe (`enth guide | head`) is not an error worth a panic.
    let _ = std::io::stdout().write_all(out.stdout.as_bytes());
    let _ = std::io::stderr().write_all(out.stderr.as_bytes());
    ExitCode::from(u8::try_from(out.code).unwrap_or(1))
}
