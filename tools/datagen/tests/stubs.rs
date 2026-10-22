//! The commands lane S0s pre-declared in `enth-datagen`. Lane DG built `ds1`, `shards` and `exclude` (their tests are in
//! `ds1.rs`); an unknown command, or `ds1` without a subcommand, prints the usage.
#![allow(
    clippy::disallowed_methods,
    reason = "the test runs the tool's own binary"
)]

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_enth-datagen"))
        .args(args)
        .output()
        .expect("the tool runs")
}

#[test]
fn an_unknown_command_prints_the_usage() {
    for args in [vec![], vec!["nope"]] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("usage: enth-datagen"));
    }
}

#[test]
fn the_built_commands_name_what_they_need() {
    for (args, needle) in [
        (vec!["ds1"], "usage: enth-datagen ds1"),
        (vec!["ds1", "gen"], "--count is required"),
        (
            vec!["ds1", "gen", "--count", "1", "--seed", "1"],
            "--out is required",
        ),
        (vec!["ds1", "gen", "--nope"], "unknown argument"),
        (vec!["shards"], "usage: shards"),
        (vec!["exclude"], "usage: exclude"),
    ] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(text.contains(needle), "{args:?}: {text}");
    }
}
