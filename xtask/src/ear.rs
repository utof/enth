//! **Owner: lane ECK** (Ear check). Pre-declared by lane S0e (R4/R5); built in `tools/lesson/src/ear/` (`ear-check`).
//!
//! `cargo xtask ear-check [options]`: GR4's mechanical half (docs/ear.md, "GR4" and "`ear-check` as built"):
//! determinism (the music generator regenerates its golden digests; every game's items regenerate theirs), audibility
//! (every option pair at least 2 JND apart under `sound/1`, at least 6 at level 1; what `sound/1` cannot hear is listed
//! as not measured), solvability (an ideal observer at 95 % or more at the top level), no leaks (bias bots stay at
//! chance +-3 % over 10,000 items), staircases (simulated listeners get a 40-trial estimate within one level in at
//! least 90 % of runs; 75 to 85 % success) and coverage. Under 10 minutes in CI (`.github/workflows/ear.yml`).
//!
//! **xtask stays light** (no engine crate here): the work lives in the tooling package `enth-lesson-tool`
//! (`tools/lesson/`), which holds the engine-bound dependencies, and this is a thin driver the way `lesson.rs` is: `cargo
//! run --release -p enth-lesson-tool -- ear-check <args>`, forwarding the exit status. The options are the tool's
//! (`--items N`, `--bot-items N`, `--runs N`, `--jobs N`, `--seed N`, `--strict`, `--ladders enforce|report`, `--only
//! GAME`, `--update-golden`; `cargo xtask ear-check --help`).

/// `cargo xtask ear-check`: runs the checks and exits non-zero when a hard one fails (the report names it).
///
/// # Errors
/// The tool failed.
pub fn ear_check(args: &[String]) -> Result<(), String> {
    super::lesson::drive("ear-check", args)
}
