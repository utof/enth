//! `enth-listen gr3`, which lane S0s pre-declared (R3): hidden from `--help`, and it refuses to run (exit 1) and names its
//! lane, whatever follows it. The lane that builds it (LT) removes this file and gives the command real tests, with
//! `--dry-run` and a recording mock player: nothing here plays audio.
#![allow(
    clippy::disallowed_methods,
    reason = "the test runs the tool's own binary"
)]

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_enth-listen"))
        .args(args)
        .output()
        .expect("the tool runs")
}

#[test]
fn gr3_refuses_and_names_its_lane() {
    for args in [
        vec!["gr3"],
        vec!["gr3", "--export-kit", "kit", "--dry-run"],
        vec!["gr3", "--analyze", "answers"],
    ] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(
            text.contains("not built yet") && text.contains("LT"),
            "{args:?}: {text}"
        );
    }
}

#[test]
fn gr3_stays_out_of_help() {
    let out = run(&["--help"]);
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("abx"), "{help}");
    assert!(!help.contains("gr3"), "gr3 is listed in --help:\n{help}");
}
