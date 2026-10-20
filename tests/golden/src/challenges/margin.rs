//! The margin assertion (docs/phase-2.md, "Golden challenges"): no pinned pass or fail decision sits within
//! [`MARGIN`](super::MARGIN) of the pass mark, so a last-bit difference in a gain can never flip a golden silently.

use enth_challenge::ScoreReport;

use super::MARGIN;

/// Whether a report's pass decision is at least [`MARGIN`] away from the pass mark.
///
/// # Errors
/// The report is too close: a message naming the case and the answer, the numbers, and what to change.
pub fn check(case: &str, answer: &str, report: &ScoreReport) -> Result<(), String> {
    let margin = report.margin();
    if margin.abs() >= MARGIN {
        return Ok(());
    }
    let side = if report.passed { "passes" } else { "fails" };
    Err(format!(
        "{case} / {answer}: sound {:.6} % {side} the pass mark {} by only {:.6}; a golden needs at least {MARGIN} so \
         that a last-bit difference in a gain cannot flip pass and fail. Pick another answer: change the recipe of \
         `{answer}` in tests/golden/challenges/cases/{case}.toml (move a parameter a little more or less; \
         `cargo run -p enth-golden -- --challenges --propose <code>` lists moves with their scores), and run \
         `cargo xtask golden --challenges --update` again.",
        report.sound.percent,
        report.pass_mark,
        margin.abs()
    ))
}
