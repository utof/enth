//! **Owner: lane EV3.** `enth lesson start <PACK> --lesson ID --step STEP --out FILE [--challenge-out FILE]`: what the
//! player loads when a step opens, written to files, because a command line has no editor to load it into. A step's
//! start is the pack patch its view names; a recreate step's is its challenge's own start, and its round (the challenge
//! file) goes with it, ready for `enth compare`, `suggest` and `match` with `--challenge`. `enth.lesson-start/1`.

use std::path::Path;

use enth_challenge::{Challenge, prepare_challenge};
use enth_learn::StepKind;

use super::{lesson_of, load_pack, step_of};
use crate::cmd::game;
use crate::cmd::{Reply, Res};
use crate::error::{self, CliError};
use crate::patchio;
use crate::schema::{LessonStartOut, ids};

pub(super) fn start(
    path: &Path,
    lesson: &str,
    which: &str,
    out: &Path,
    challenge_out: Option<&Path>,
) -> Res {
    let pack = load_pack(path)?;
    let bundle = lesson_of(&pack, lesson)?;
    let (i, step) = step_of(&bundle.lesson, which)?;
    let number = u16::try_from(i + 1).unwrap_or(u16::MAX);
    let key = bundle.key().to_string();
    let (from, patch, round_json) = if let StepKind::Recreate { challenge, .. } = &step.kind {
        let round =
            prepare_challenge(Challenge::clone(challenge)).map_err(|e| game::from_round(&e))?;
        (
            "challenge".to_owned(),
            round.start().clone(),
            Some(challenge.to_json()),
        )
    } else {
        if challenge_out.is_some() {
            return Err(error::usage(
                "no_round",
                format!(
                    "step {number} ({}) of {key} is a {} step, which has no round to write",
                    step.id,
                    step.kind.name()
                ),
                "--challenge-out is for a recreate step; leave it out",
            ));
        }
        let Some(r) = &step.view.start else {
            return Err(carried(&bundle.lesson.steps, i, &key));
        };
        let Some(patch) = bundle.patch(r) else {
            return Err(error::invalid(
                "bad_content",
                format!("the pack does not hold the patch {r} that step {number} starts from"),
                "run `cargo xtask lesson-check` on the pack",
            ));
        };
        (r.to_string(), patch.as_ref().clone(), None)
    };
    patchio::write(out, &patch)?;
    let challenge_file = match (round_json, challenge_out) {
        (Some(json), Some(file)) => {
            std::fs::write(file, format!("{json}\n"))
                .map_err(|e| error::write_failed(&patchio::show(file), &e))?;
            Some(patchio::show(file))
        }
        _ => None,
    };
    let next = match (
        &challenge_file,
        matches!(step.kind, StepKind::Recreate { .. }),
    ) {
        (Some(file), _) => format!(
            "edit {} with `enth apply`, then score it with `enth compare {} --challenge {file}` (exit 0 passes)",
            patchio::show(out),
            patchio::show(out)
        ),
        (None, true) => format!(
            "add --challenge-out round.challenge.json to also write the round, which `enth compare {} --challenge <file>` scores",
            patchio::show(out)
        ),
        (None, false) => format!(
            "edit {} with `enth apply`, then `enth lesson check {} --lesson {lesson} --step {} {}`",
            patchio::show(out),
            patchio::show(path),
            step.id,
            patchio::show(out)
        ),
    };
    Ok(Reply::new(&LessonStartOut {
        schema: ids::LESSON_START.to_owned(),
        lesson: key,
        step: step.id.to_string(),
        number,
        kind: step.kind.name().to_owned(),
        from,
        patch: patchio::show(out),
        challenge: challenge_file,
        next,
    }))
}

/// A step with no start of its own carries the patch over from the step before.
fn carried(steps: &[enth_learn::Step], i: usize, key: &str) -> CliError {
    let earlier = steps[..i]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, s)| s.view.start.is_some() || matches!(s.kind, StepKind::Recreate { .. }));
    let hint = match earlier {
        Some((j, s)) => format!(
            "keep working on the patch you ended the step before with; the nearest earlier step with a start is step {} ({})",
            j + 1,
            s.id
        ),
        None => "use `enth new --template init --out p.patch.json`, or your own patch".to_owned(),
    };
    error::usage(
        "carried_start",
        format!(
            "step {} ({}) of {key} has no start of its own: it carries the patch over from the step before",
            i + 1,
            steps[i].id
        ),
        hint,
    )
}
