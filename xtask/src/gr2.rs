//! **Owner: lane INT2** (R2 integration). Pre-declared by lane S0l (R2); the command is built in
//! `tools/lesson/src/gr2.rs`.
//!
//! `cargo xtask gr2-report <progress file>...`: a driver for `enth-lesson-tool gr2`, the GR2 numbers from the friends'
//! and the owner's local progress files (docs/learn.md, "GR2"). Local files only: no network code, no telemetry. The
//! tables of record are `docs/r2-gates.md`. If INT2 can read the JSON without the tool's dependencies (as `gr1-report`
//! reads round logs), the work moves into this file and the tool's `gr2.rs` goes.

/// `cargo xtask gr2-report`: runs `enth-lesson-tool gr2`.
pub fn gr2_report(args: &[String]) -> Result<(), String> {
    crate::lesson::drive("gr2", args)
}
