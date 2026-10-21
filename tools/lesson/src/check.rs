//! **Owner: lane LCK.** Pre-declared by lane S0l (R2) as a placeholder that passes.
//!
//! `check [--cross] [--solve] [PACK...]` (`cargo xtask lesson-check`): until lane LCK writes it, this checks nothing and
//! exits 0, so `.github/workflows/lessons.yml` is green (and wired) from the freeze on. It says so on stdout; do not read a
//! pass from it. LCK's version runs the seven checks in docs/learn.md ("`lesson-check` and the authoring loop"): schemas,
//! text, the playthrough from each step's start (the start fails, the witness passes with a margin of at least 1e-3, no
//! `thin` item, only editable controls touched), `--cross`, `--solve` (the hint follower reaches the pass, `match` passes
//! Recreate steps within a budget, ear-question options are pairwise audible), coverage and hash pinning. Under 10
//! minutes in CI.

/// `check`: a placeholder that checks nothing.
#[allow(
    clippy::unnecessary_wraps,
    reason = "the placeholder keeps the signature of the real command, which fails on a dishonest lesson"
)]
pub fn run(_args: &[String]) -> Result<(), String> {
    println!(
        "lesson-check: placeholder (R2 lane LCK has not built it); nothing was checked, so this pass means nothing yet"
    );
    Ok(())
}
