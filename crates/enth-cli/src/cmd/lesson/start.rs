//! **Owner: lane EV3.** `enth lesson start <PACK> --lesson ID --step STEP [--out FILE] [--challenge-out FILE]
//! [--examples-out DIR]`: what the player loads when a step opens, written to files, because a command line has no
//! editor to load it into and no speakers to play it on. A step's start is the pack patch its view names; a recreate
//! step's is its challenge's own start, and its round (the challenge file) goes with it, ready for `enth compare`,
//! `suggest` and `match` with `--challenge`. A listen or choose step's sounds are written as a patch and a
//! performance each (`--examples-out`), which `enth describe PATCH --perf PERF` measures where the player would play
//! them. `enth.lesson-start/1`.

use std::path::{Path, PathBuf};

use enth_challenge::{Challenge, phrases, prepare_challenge};
use enth_learn::Example;
use enth_learn::{LessonBundle, Step, StepKind};
use serde_json::json;

use super::{lesson_of, load_pack, step_of};
use crate::cmd::game;
use crate::cmd::{Reply, Res};
use crate::error::{self, CliError};
use crate::patchio;
use crate::schema::{LessonExampleOut, LessonStartOut, ids};

/// The sounds of a listen or choose step.
fn sounds(step: &Step) -> Option<&[Example]> {
    match &step.kind {
        StepKind::Listen { examples } => Some(examples),
        StepKind::Choose { options, .. } => Some(options),
        _ => None,
    }
}

/// A label as a file stem: lower-case letters and digits, the rest `-`.
fn slug(label: &str) -> String {
    let mut out = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() { "x".to_owned() } else { out }
}

/// One sound as files to write: the patch, the performance, and what `start` reports of it.
struct Prepared {
    patch_path: PathBuf,
    perf_path: PathBuf,
    patch: enth_patch::Patch,
    perf: String,
    out: LessonExampleOut,
}

/// The files for each of a step's sounds, under `dir`; nothing is written.
fn prepare_sounds(
    bundle: &LessonBundle,
    list: &[Example],
    dir: &Path,
    number: u16,
) -> Result<Vec<Prepared>, CliError> {
    let mut out = Vec::new();
    for (k, ex) in list.iter().enumerate() {
        let stem = format!("{}-{}", k + 1, slug(&ex.label));
        let Some(patch) = bundle.patch(&ex.patch) else {
            return Err(error::invalid(
                "bad_content",
                format!(
                    "the pack does not hold the patch {} that step {number} plays",
                    ex.patch
                ),
                "run `cargo xtask lesson-check` on the pack",
            ));
        };
        let Some(entry) = phrases::entry(ex.phrase) else {
            return Err(error::invalid(
                "bad_content",
                format!(
                    "step {number} plays phrase {}, which this build lacks",
                    ex.phrase.0
                ),
                "run `cargo xtask lesson-check` on the pack",
            ));
        };
        let mut notes = Vec::new();
        for n in entry.phrase.notes {
            let key = i16::from(n.key) + i16::from(ex.transpose);
            if !(0..=127).contains(&key) {
                return Err(error::invalid(
                    "bad_content",
                    format!(
                        "the example `{}` of step {number} transposed {} leaves the keyboard",
                        ex.label, ex.transpose
                    ),
                    "run `cargo xtask lesson-check` on the pack",
                ));
            }
            notes.push(json!({
                "note": key,
                "vel": n.velocity,
                "on_s": f64::from(n.start_ms) / 1000.0,
                "off_s": f64::from(n.start_ms + n.hold_ms) / 1000.0,
            }));
        }
        let perf = serde_json::to_string_pretty(&json!({
            "schema": "enth.perf/1",
            "notes": notes,
            "dur_s": f64::from(entry.phrase.length_ms) / 1000.0,
        }))
        .unwrap_or_default();
        let patch_path = dir.join(format!("{stem}.patch.json"));
        let perf_path = dir.join(format!("{stem}.perf.json"));
        out.push(Prepared {
            out: LessonExampleOut {
                label: ex.label.clone(),
                patch: patchio::show(&patch_path),
                perf: patchio::show(&perf_path),
                phrase: entry.name.to_owned(),
                transpose: ex.transpose,
                seed: ex.seed,
            },
            patch_path,
            perf_path,
            patch: patch.as_ref().clone(),
            perf,
        });
    }
    Ok(out)
}

#[allow(
    clippy::too_many_lines,
    reason = "one command: the three things a step can load, each checked before anything is written"
)]
pub(super) fn start(
    path: &Path,
    lesson: &str,
    which: &str,
    out: Option<&Path>,
    challenge_out: Option<&Path>,
    examples_out: Option<&Path>,
) -> Res {
    let pack = load_pack(path)?;
    let bundle = lesson_of(&pack, lesson)?;
    let (i, step) = step_of(&bundle.lesson, which)?;
    let number = u16::try_from(i + 1).unwrap_or(u16::MAX);
    let key = bundle.key().to_string();
    let kind = step.kind.name();
    let is_round = matches!(step.kind, StepKind::Recreate { .. });
    let list = sounds(step);
    if out.is_none() && examples_out.is_none() {
        let (what, hint) = match kind {
            "listen" | "choose" => (
                "has sounds, not a start patch",
                "add --examples-out DIR to write its sounds",
            ),
            "read" => (
                "has nothing to load",
                "read its text with `enth lesson show`",
            ),
            _ => ("has a start patch", "add --out FILE.patch.json to write it"),
        };
        return Err(error::usage(
            "nothing_to_write",
            format!(
                "step {number} ({}) of {key} is a {kind} step, which {what}",
                step.id
            ),
            hint,
        ));
    }
    if examples_out.is_some() && list.is_none() {
        return Err(error::usage(
            "no_examples",
            format!(
                "step {number} ({}) of {key} is a {kind} step, which has no sounds to write",
                step.id
            ),
            "--examples-out is for a listen or choose step; leave it out",
        ));
    }
    if challenge_out.is_some() && !is_round {
        return Err(error::usage(
            "no_round",
            format!(
                "step {number} ({}) of {key} is a {kind} step, which has no round to write",
                step.id
            ),
            "--challenge-out is for a recreate step; leave it out",
        ));
    }

    // Everything is worked out before anything is written, so a refusal leaves no file behind.
    let mut from = None;
    let mut start_patch = None;
    let mut round_json = None;
    if out.is_some() {
        if let StepKind::Recreate { challenge, .. } = &step.kind {
            let round =
                prepare_challenge(Challenge::clone(challenge)).map_err(|e| game::from_round(&e))?;
            from = Some("challenge".to_owned());
            start_patch = Some(round.start().clone());
            round_json = Some(challenge.to_json());
        } else {
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
            from = Some(r.to_string());
            start_patch = Some(patch.as_ref().clone());
        }
    } else if challenge_out.is_some() {
        if let StepKind::Recreate { challenge, .. } = &step.kind {
            round_json = Some(challenge.to_json());
        }
    }
    let prepared = match (list, examples_out) {
        (Some(list), Some(dir)) => prepare_sounds(&bundle, list, dir, number)?,
        _ => Vec::new(),
    };

    if let (Some(out), Some(patch)) = (out, &start_patch) {
        patchio::write(out, patch)?;
    }
    let challenge_file = match (round_json, challenge_out) {
        (Some(json), Some(file)) => {
            std::fs::write(file, format!("{json}\n"))
                .map_err(|e| error::write_failed(&patchio::show(file), &e))?;
            Some(patchio::show(file))
        }
        _ => None,
    };
    if let Some(dir) = examples_out {
        std::fs::create_dir_all(dir).map_err(|e| error::write_failed(&patchio::show(dir), &e))?;
        for p in &prepared {
            patchio::write(&p.patch_path, &p.patch)?;
            std::fs::write(&p.perf_path, format!("{}\n", p.perf))
                .map_err(|e| error::write_failed(&patchio::show(&p.perf_path), &e))?;
        }
    }
    let shown = out.map(patchio::show);
    let next = if prepared.is_empty() {
        let patch = shown.clone().unwrap_or_default();
        match (&challenge_file, is_round) {
            (Some(file), _) => format!(
                "edit {patch} with `enth apply`, then score it with `enth compare {patch} --challenge {file}` (exit 0 passes)"
            ),
            (None, true) => format!(
                "add --challenge-out round.challenge.json to also write the round, which `enth compare {patch} --challenge <file>` scores"
            ),
            (None, false) => format!(
                "edit {patch} with `enth apply`, then `enth lesson check {} --lesson {lesson} --step {} {patch}`",
                patchio::show(path),
                step.id
            ),
        }
    } else if kind == "choose" {
        "measure each sound with `enth describe <patch> --perf <perf>` (`--seed` where the sound sets one), then answer with the label of the one that fits the step's question".to_owned()
    } else {
        "measure each sound with `enth describe <patch> --perf <perf>` (`--seed` where the sound sets one); a listen step has no answer".to_owned()
    };
    Ok(Reply::new(&LessonStartOut {
        schema: ids::LESSON_START.to_owned(),
        lesson: key,
        step: step.id.to_string(),
        number,
        kind: kind.to_owned(),
        from,
        patch: shown,
        challenge: challenge_file,
        examples: prepared.into_iter().map(|p| p.out).collect(),
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
