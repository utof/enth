//! The `enth` binary (P1a lane C): a thin shell over `enth_cli::run`. Empty at freeze point F0.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "shell crate: owns I/O, clocks and threads, and prints to stdout and stderr (root clippy.toml)"
)]

fn main() -> std::process::ExitCode {
    std::process::ExitCode::SUCCESS
}
