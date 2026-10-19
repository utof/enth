//! enth-cli (L8, see docs/architecture.md and docs/agent-cli.md): the `enth` command line (P1a lane C).
//!
//! Empty at freeze point F0. The plan is a thin binary over `enth_cli::run(args, stdin) -> Output { code, stdout,
//! stderr }`, so insta snapshots run in process; a handful of end-to-end tests spawn the built binary.
//! Shells translate; they never decide (architecture rule 6): no product rule lives here.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "shell crate: owns I/O, clocks and threads, and prints to stdout and stderr (root clippy.toml)"
)]
