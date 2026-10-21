//! The commands lane S0l pre-declared in `enth-lesson-tool` that are still stubs: each refuses to run (exit 1) and names
//! its lane, and an unknown command prints the usage. The lane that builds a command removes its row here and gives it
//! real tests (lane LCK's four: `tests/check.rs`, `tests/commands.rs`; lane DEC's `gen`: `tests/gen.rs`).
#![allow(
    clippy::disallowed_methods,
    reason = "the test runs the tool's own binary"
)]

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_enth-lesson-tool"))
        .args(args)
        .output()
        .expect("the tool runs")
}

#[test]
fn every_stub_refuses_and_names_its_lane() {
    // One stub is left (a loop over one row would be clippy's `single_element_loop`).
    let (args, lane) = (["gr2", "progress.json"], "INT2");
    let out = run(&args);
    assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("not built yet") && text.contains(lane),
        "{args:?}: {text}"
    );
}

#[test]
fn an_unknown_command_prints_the_usage() {
    for args in [vec![], vec!["nope"]] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("usage: enth-lesson-tool"));
    }
}
