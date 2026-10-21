//! **Owner: lane LCK.** Pre-declared by lane S0l (R2); the command is built in `tools/lesson/src/coverage.rs`.
//!
//! `cargo xtask lesson-coverage`: a driver for `enth-lesson-tool coverage` (every P1 module has a lesson and a
//! non-patch check; GR2's mechanical half).

/// `cargo xtask lesson-coverage`: runs `enth-lesson-tool coverage`.
pub fn lesson_coverage(args: &[String]) -> Result<(), String> {
    super::drive("coverage", args)
}
