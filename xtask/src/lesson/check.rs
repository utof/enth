//! **Owner: lane LCK.** Pre-declared by lane S0l (R2); the command is built in `tools/lesson/src/check.rs`.
//!
//! `cargo xtask lesson-check [--cross] [--solve] [PACK...]`: a driver for `enth-lesson-tool check`. Until lane LCK builds
//! the tool's command it checks nothing and exits 0 (and says so), so `.github/workflows/lessons.yml` is green and wired
//! from the freeze on.

/// `cargo xtask lesson-check`: runs `enth-lesson-tool check`.
pub fn lesson_check(args: &[String]) -> Result<(), String> {
    super::drive("check", args)
}
