//! `enth-golden [--update]`: run every golden case and print the report. `cargo xtask golden` drives this binary.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "command-line tool"
)]

use std::process::ExitCode;

use enth_golden::{Options, run};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match args.as_slice() {
        [] => Options::check(),
        [flag] if flag == "--update" => Options::update(),
        _ => {
            eprintln!("usage: enth-golden [--update]");
            return ExitCode::from(2);
        }
    };
    match run(&opts) {
        Ok(report) => {
            print!("{}", report.render());
            if report.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(msg) => {
            eprintln!("golden: {msg}");
            ExitCode::FAILURE
        }
    }
}
