//! **Owner: lane INT4** (R4 integration). Pre-declared by lane S0e (R4/R5) as a stub that refuses to run.
//!
//! `cargo xtask gr4-report <dir>...`: the GR4 numbers (docs/ear.md, "GR4") from what each participant sent back, per
//! person: the retest pilot, the pre- and post-test thresholds on the trained and the held-out skills (Ear check on four
//! skills, two trained and two held out, counterbalanced), the workouts played from the progress file's `drills` field,
//! the debrief, and the proposed criteria. Like `gr2-report` it reads local files without the engine crates, and it
//! prints a description, not a test (N = 3 to 6). Local files only: no network code, no telemetry. The tables of record
//! are `docs/r4-gates.md`.

/// `cargo xtask gr4-report`: not built yet.
pub fn gr4_report(_args: &[String]) -> Result<(), String> {
    Err("gr4-report is not built yet (R4 lane INT4; docs/phase-4.md)".to_owned())
}
