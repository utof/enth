//! **Owner: lane INT2** (R2 integration). Pre-declared by lane S0l (R2) as a stub that refuses to run.
//!
//! `cargo xtask gr2-report <progress file>...`: the GR2 numbers from the friends' and the owner's local progress files
//! (docs/learn.md, "GR2"): per person, the pre- and post-test challenge codes (pass within 3 minutes without hints, plus
//! structure %), the staggered mid-test, the per-lesson votes and the "name up to three things you learned" notes, with
//! the proposed pass criteria beside them. Local files only: no network code, no telemetry. The tables of record are
//! `docs/r2-gates.md`.

/// `cargo xtask gr2-report`: not built yet.
pub fn gr2_report(_args: &[String]) -> Result<(), String> {
    Err("gr2-report is not built yet (R2 lane INT2; docs/phase-3.md)".to_owned())
}
