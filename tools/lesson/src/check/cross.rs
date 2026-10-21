//! **Owner: lane LCK.** `check --cross` (the [parent](super)'s step 3): a Do step's check does not pass another Do
//! step's witness, so a check tells its own goal apart from its neighbours'. The neighbours are the other Do steps of
//! the lesson and of the lessons on either side of it in the pack's order (the agent evals' `audit --cross` compares
//! every goal with every other witness; within a pack the near ones are the ones that get confused).
//!
//! **Built-up steps are not confused:** when step B starts from a patch that already passes A's check (B builds on
//! A's result), B's witness passing A's check is inherited, not a loose check, and is not flagged. A `touched`
//! predicate reads the span a straight turn from B's start to its witness leaves.

use enth_learn::check::evaluate_sounds;
use enth_learn::{Check, LessonBundle, StepKind};
use enth_patch::Patch;

use super::Log;
use super::ops::touch_log;
use super::play::Played;

/// One Do step the playthrough completed.
struct DoStep<'a> {
    at: &'a str,
    lesson: usize,
    check: &'a Check,
    start: &'a Patch,
    witness: &'a Patch,
}

/// Whether `check` passes `patch`, reached from `start`.
fn passes(check: &Check, start: &Patch, patch: &Patch) -> bool {
    if !check.patch_holds(patch, &touch_log(start, patch)) {
        return false;
    }
    let sounds = check.sounds();
    sounds.is_empty() || evaluate_sounds(&sounds, patch).is_ok_and(|v| v.passed)
}

/// `--cross` over one pack's lessons and their playthroughs.
pub(super) fn run(lessons: &[LessonBundle], played: &[Vec<Played>], log: &mut Log) {
    let mut steps: Vec<DoStep<'_>> = Vec::new();
    for (li, (bundle, runs)) in lessons.iter().zip(played).enumerate() {
        for p in runs {
            if let (StepKind::Do { check, .. }, Some(start), Some(witness)) =
                (&bundle.lesson.steps[p.index].kind, &p.start, &p.end)
            {
                steps.push(DoStep {
                    at: &p.at,
                    lesson: li,
                    check,
                    start,
                    witness,
                });
            }
        }
    }
    let before = log.failures();
    let mut pairs = 0_usize;
    for a in &steps {
        for b in &steps {
            if std::ptr::eq(a, b) || a.lesson.abs_diff(b.lesson) > 1 {
                continue;
            }
            pairs += 1;
            if passes(a.check, b.start, b.witness) && !passes(a.check, b.start, b.start) {
                log.fail(
                    a.at,
                    format!(
                        "--cross: its check passes {}'s witness, whose start does not pass it: the check does not \
                         tell the two goals apart",
                        b.at
                    ),
                );
            }
        }
    }
    log.line(format!(
        "  cross: {} Do step(s), {pairs} check-witness pair(s){}",
        steps.len(),
        if log.failures() == before {
            ", none confused"
        } else {
            ""
        }
    ));
}
