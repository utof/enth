//! `enth-golden [--update]`: run every golden case and print the report. `cargo xtask golden` drives this binary.
//!
//! `enth-golden --challenges [--update] [--only <case>]` runs the golden challenges instead
//! (`tests/golden/challenges/`, `cargo xtask golden --challenges`); `--challenges --propose <code or settings>` prints
//! the helper for writing a new case.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "command-line tool"
)]

use std::process::ExitCode;

use enth_golden::challenges;
use enth_golden::{Options, run};

const USAGE: &str = "usage: enth-golden [--update]\n       enth-golden --challenges [--update] [--only <case>]\n       enth-golden --challenges --propose <code or settings>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--challenges") {
        return golden_challenges(&args[1..]);
    }
    let opts = match args.as_slice() {
        [] => Options::check(),
        [flag] if flag == "--update" => Options::update(),
        _ => {
            eprintln!("{USAGE}");
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

fn golden_challenges(args: &[String]) -> ExitCode {
    if let [flag, input] = args
        && flag == "--propose"
    {
        return match challenges::propose::propose(input) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(msg) => {
                eprintln!("golden challenges: {msg}");
                ExitCode::FAILURE
            }
        };
    }
    let mut opts = challenges::Options::check();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--update" => opts.update = true,
            "--only" => {
                let Some(name) = rest.next() else {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                };
                opts.only = Some(name.clone());
            }
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    match challenges::run(&opts) {
        Ok(report) => {
            print!("{}", report.render());
            if report.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(msg) => {
            eprintln!("golden challenges: {msg}");
            ExitCode::FAILURE
        }
    }
}
