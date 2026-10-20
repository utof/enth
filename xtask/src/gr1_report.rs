//! **Owner: lane GK** (the GR1 kit). Pre-declared by lane S0r (R1/S1) as a stub that refuses to run.
//!
//! `cargo xtask gr1-report <round logs>...`: read the friends' and the owner's round logs (JSONL, round-log schema v1:
//! code, patch, scores, time, the agreement vote) and print the GR1 numbers: rounds played, pass rates by level,
//! agreement with the score, time to pass, and the agent baseline beside the no-op and CMA-ES floors
//! (docs/phase-2.md). Local files only: no network code, no telemetry.

/// `cargo xtask gr1-report`: not built yet.
pub fn gr1_report(_args: &[String]) -> Result<(), String> {
    Err("gr1-report is not built yet (R1/S1 lane GK; docs/phase-2.md)".to_owned())
}
