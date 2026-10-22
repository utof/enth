//! The commands lane S0l pre-declared in `enth-lesson-tool` were stubs that refused to run and named their lane; each
//! lane that built one removed its row here and gave it real tests (lane LCK's four: `tests/check.rs`,
//! `tests/commands.rs`; lane DEC's `gen`: `tests/gen.rs`). The last, `gr2`, went: lane INT2 built `cargo xtask
//! gr2-report` in xtask (`xtask/src/gr2.rs`), so `gr2` is now an unknown command here, and an unknown command prints
//! the usage.
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
fn an_unknown_command_prints_the_usage() {
    for args in [vec![], vec!["nope"], vec!["gr2", "progress.json"]] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("usage: enth-lesson-tool"));
    }
}
