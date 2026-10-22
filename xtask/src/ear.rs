//! **Owner: lane ECK** (Ear check). Pre-declared by lane S0e (R4/R5) as a placeholder that passes; the command is built
//! in `tools/lesson/src/ear/` (`ear-check`).
//!
//! `cargo xtask ear-check`: until lane ECK writes it, this checks nothing and exits 0, so `.github/workflows/ear.yml` is
//! green (and wired) from the freeze on. It says so on stdout; do not read a pass from it. ECK's version is GR4's
//! mechanical half (docs/ear.md, "GR4"): determinism (1,000 items per game regenerate identical hashes on the six golden
//! targets), audibility (every option pair at least 2 JND apart under `sound/1`, at least 6 at level 1; items scored on
//! parameter steps listed apart), solvability (an ideal-observer bot at 95 % or more at the top level), no leaks (bias
//! bots stay at chance +-3 % over 10,000 items), staircases (simulated listeners get a 40-trial estimate within one step in
//! at least 90 % of runs; 75 to 85 % success) and coverage (every observable Basic-tier parameter is in game 1 or 2).
//! Under 10 minutes in CI. **xtask stays light** (no engine crate here): the work lives in the tooling package
//! `enth-lesson-tool` (`tools/lesson/`), which holds the engine-bound dependencies, and this becomes a thin driver the way
//! `lesson.rs` is: `cargo run --release -p enth-lesson-tool -- ear-check <args>`, forwarding the exit status.

/// `cargo xtask ear-check`: a placeholder that checks nothing.
#[allow(
    clippy::unnecessary_wraps,
    reason = "the placeholder keeps the signature of the real command, which fails on an unfair or unsolvable game"
)]
pub fn ear_check(_args: &[String]) -> Result<(), String> {
    println!(
        "ear-check: placeholder (R4 lane ECK has not built it); nothing was checked, so this pass means nothing yet"
    );
    Ok(())
}
