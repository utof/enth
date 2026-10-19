//! One test over every case in `cases/`: `cargo test`, `cargo nextest` and `cargo xtask golden` all run it.
//! It only compares; `--update` exists in the binary (`cargo xtask golden --update`) and never in a test.

use enth_golden::{Options, run};

#[test]
fn golden_renders() {
    let report = run(&Options::check()).expect("the harness runs");
    assert!(report.is_ok(), "\n{}", report.render());
}
