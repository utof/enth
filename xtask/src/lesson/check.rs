//! **Owner: lane LCK.** Pre-declared by lane S0l (R2); the command is built in `tools/lesson/src/check.rs`.
//!
//! `cargo xtask lesson-check [--cross] [--solve] [--budget N] [--jobs N] [--lesson ID]... [PACK...]`: a driver for
//! `enth-lesson-tool check` (the files, the playthrough of every step through a headless session, `--cross`, `--solve`;
//! `tools/lesson/src/check.rs` says what each proves). `.github/workflows/lessons.yml` and `scripts/gate.sh lane` run it.

/// `cargo xtask lesson-check`: runs `enth-lesson-tool check`.
pub fn lesson_check(args: &[String]) -> Result<(), String> {
    super::drive("check", args)
}
