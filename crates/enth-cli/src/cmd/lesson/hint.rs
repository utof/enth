//! **Owner: lane EV3.** `enth lesson hint <PACK> --lesson ID --step STEP [--n N] [PATCH]`: one hint of a step's
//! ladder (docs/learn.md, "The lifecycle": "`hint` gives the authored hints first, then measured ones"). `enth.lesson-hint/1`.
//!
//! - `--n 1` to the number of authored hints: the lesson's own words, no patch needed.
//! - The next one is measured on a patch file. A **do** step's is the player's: `enth_solver::hint` from the patch over
//!   the sound check's items, within the step's editable controls (the player builds the request, so the free set and
//!   the macro pins are the plugin's own), in the Learn view's words plus the move as numbers and `--ops`. A
//!   **recreate** step's is the round's coach (`enth suggest`'s), levels 1 to 4. A choose step has authored hints only.
//!
//! Stateless: `--n` names the rung, so a caller that wants the next hint asks for `n + 1`. The player marks a step
//! assisted at its first hint; here the output says so, and the caller keeps the count.

use std::path::Path;

use enth_analysis::expect::{Expect, parse_item};
use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::{Challenge, phrases, prepare_challenge};
use enth_learn::player::{EditSource, Effect};
use enth_learn::{Player, PlayerError, StepKind};
use enth_render::RenderSpec;
use enth_solver::coach::{CoachCfg, CoachTarget, HINT_LEVELS};
use enth_solver::hint::{HintCfg, HintResult, hint};

use super::{lesson_of, load_pack, step_of};
use crate::cmd::common::show;
use crate::cmd::{Reply, Res, game, suggest};
use crate::error::{self, CliError};
use crate::eval::Parallel;
use crate::fix;
use crate::patchio;
use crate::schema::{LessonBlockedOut, LessonHintOut, LessonMeasuredOut, LessonMoveOut, ids};

/// A refused player call.
fn player_error(e: &PlayerError) -> CliError {
    error::failed(e.code(), e.to_string(), e.hint())
}

#[allow(
    clippy::too_many_lines,
    reason = "one command: the three kinds of step, each with its own ladder"
)]
pub(super) fn hint_cmd(
    path: &Path,
    lesson: &str,
    which: &str,
    n: u16,
    patch_path: Option<&Path>,
    workers: Option<usize>,
) -> Res {
    let pack = load_pack(path)?;
    let bundle = lesson_of(&pack, lesson)?;
    let (i, step) = step_of(&bundle.lesson, which)?;
    let number = u16::try_from(i + 1).unwrap_or(u16::MAX);
    let key = bundle.key().to_string();
    let authored = u16::try_from(step.hints.len()).unwrap_or(u16::MAX);
    game::check_workers(workers)?;
    if n == 0 {
        return Err(error::usage(
            "bad_n",
            "--n 0: hints are numbered from 1",
            "--n 1 is the first hint; `enth lesson list` shows how many authored hints a step has",
        ));
    }
    let kind = step.kind.name();
    // How many hints the step has in all, and whether the rung after the authored ones needs a patch.
    let measured_rungs: u16 = match &step.kind {
        StepKind::Do { check, .. } => u16::from(!check.sounds().is_empty()),
        StepKind::Recreate { .. } => u16::from(HINT_LEVELS),
        StepKind::Choose { .. } => 0,
        _ => {
            return Err(error::usage(
                "not_hintable",
                format!(
                    "step {number} ({}) of {key} is a {kind} step: there is nothing to get wrong",
                    step.id
                ),
                "`enth lesson list` shows the do, recreate and choose steps",
            ));
        }
    };
    let total = authored + measured_rungs;
    let next_hint = |n: u16| -> String {
        if n < total {
            format!(
                "`--n {}` is the next hint; any hint marks the step assisted in the player",
                n + 1
            )
        } else {
            "that is the last hint of this step; any hint marks the step assisted in the player"
                .to_owned()
        }
    };
    let mut out = LessonHintOut {
        schema: ids::LESSON_HINT.to_owned(),
        lesson: key.clone(),
        step: step.id.to_string(),
        number,
        kind: kind.to_owned(),
        n,
        authored,
        source: "authored".to_owned(),
        text: String::new(),
        patch: None,
        measured: None,
        coach: None,
        more: n < total,
        renders: 0,
        next: next_hint(n),
    };
    if n <= authored {
        out.text.clone_from(&step.hints[usize::from(n) - 1]);
        return Ok(Reply::new(&out));
    }
    if n > total {
        let (message, hint) = if total == 0 {
            (
                format!("step {number} ({}) of {key} has no hints", step.id),
                "`enth lesson list` shows each step's authored hints; a do or recreate step also has measured ones"
                    .to_owned(),
            )
        } else {
            let measured_note = if measured_rungs == 0 {
                "no measured one".to_owned()
            } else {
                format!("{measured_rungs} measured")
            };
            (
                format!(
                    "--n {n}: step {number} ({}) of {key} has {total} hints: {authored} authored and {measured_note}",
                    step.id
                ),
                format!("ask for --n 1 to --n {total}"),
            )
        };
        return Err(error::usage("no_more_hints", message, hint));
    }
    let Some(patch_path) = patch_path else {
        return Err(error::usage(
            "patch_required",
            format!(
                "hint {n} of step {number} ({}) is measured on your patch (the authored ones are --n 1 to --n {authored})",
                step.id
            ),
            format!(
                "give the patch file: `enth lesson hint {} --lesson {lesson} --step {} --n {n} <patch>`",
                patchio::show(path),
                step.id
            ),
        ));
    };
    let (patch, mut notes) = patchio::load(patch_path)?;
    out.patch = Some(patchio::show(patch_path));
    match &step.kind {
        StepKind::Do { .. } => {
            // The player builds the request, so the free set and the macro pins are the plugin's.
            let idx = u16::try_from(i).unwrap_or(u16::MAX);
            let (mut player, _) =
                Player::resume(bundle.clone(), idx).map_err(|e| player_error(&e))?;
            player
                .edited(0, 1, &patch, &[], EditSource::Load)
                .map_err(|e| player_error(&e))?;
            let mut effects = Vec::new();
            for _ in 0..=authored {
                effects = player.hint().map_err(|e| player_error(&e))?;
            }
            let request = effects.into_iter().find_map(|e| match e {
                Effect::HintJob(r) => Some(r),
                _ => None,
            });
            let Some(request) = request else {
                return Err(error::failed(
                    "no_measured_hint",
                    format!(
                        "the player gave no measured hint for step {number} ({})",
                        step.id
                    ),
                    "report this with the pack and the step",
                ));
            };
            let phrase = phrases::phrase(request.phrase).ok_or_else(|| {
                error::invalid(
                    "unknown_phrase",
                    format!("phrase {} is not in this build", request.phrase.0),
                    "update enth",
                )
            })?;
            let items: Vec<Expect> = request
                .expect
                .iter()
                .map(|item| parse_item(item))
                .collect::<Result<_, _>>()
                .map_err(|e| error::from_expect(&e))?;
            let cfg = HintCfg {
                only: (!request.free.is_empty()).then(|| request.free.clone()),
                ..HintCfg::default()
            };
            let spec = RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED);
            let result = game::with_workers(workers, || {
                hint(
                    &request.patch,
                    &phrase.performance(),
                    &spec,
                    &items,
                    &mut Parallel,
                    &cfg,
                )
            })?
            .map_err(|e| {
                error::failed(
                    e.code(),
                    format!("the hint search could not run: {e}"),
                    "check the patch renders: `enth render <patch>`",
                )
            })?;
            let (text, measured) = measured(&request.patch, &result);
            "measured".clone_into(&mut out.source);
            out.text = text;
            out.measured = Some(measured);
            out.renders = u32::try_from(result.stats.renders).unwrap_or(u32::MAX);
            out.next = format!(
                "apply the move with `enth apply {} --ops '<ops>'`, then `enth lesson check`; {}",
                patchio::show(patch_path),
                next_hint(n)
            );
        }
        StepKind::Recreate { challenge, .. } => {
            let level = u8::try_from(n - authored).unwrap_or(u8::MAX);
            let round =
                prepare_challenge(Challenge::clone(challenge)).map_err(|e| game::from_round(&e))?;
            let cfg = CoachCfg::default();
            let (target_renders, plan) = game::with_workers(workers, || {
                let mut eval = Parallel;
                let target = CoachTarget::for_round(&round, &mut eval, &cfg)
                    .map_err(|e| game::from_coach(&e))?;
                let plan = target
                    .plan(&patch, &mut eval, &cfg)
                    .map_err(|e| game::from_coach(&e))?;
                Ok::<_, CliError>((target.sensitivity().renders, plan))
            })??;
            let spec_of =
                |addr: &enth_patch::ParamAddr| -> Option<&'static enth_params::ParamSpec> {
                    patch
                        .spec(addr)
                        .or_else(|| round.start().spec(addr))
                        .or_else(|| round.target().spec(addr))
                };
            let mut sug =
                suggest::suggest_out(patch_path, None, level, &plan, target_renders, &spec_of);
            "coach".clone_into(&mut out.source);
            out.text.clone_from(&sug.text);
            out.renders = u32::try_from(sug.renders).unwrap_or(u32::MAX);
            out.next = format!(
                "{}; or `enth lesson closest` for the nearest passing patch",
                next_hint(n)
            );
            // The embedded object's own advice is `enth suggest`'s (`--level`); the lesson's is `--n`.
            sug.hint.clone_from(&out.next);
            out.coach = Some(sug);
        }
        _ => {}
    }
    let mut reply = Reply::new(&out);
    notes.dedup();
    for note in &notes {
        reply = reply.note(note);
    }
    Ok(reply)
}

/// The Learn view's words for a hint result, and the move as numbers. The words are the plugin's: the largest move's
/// parameter and its direction, or the reason an item is out of reach.
fn measured(patch: &enth_patch::Patch, result: &HintResult) -> (String, LessonMeasuredOut) {
    let mut out = LessonMeasuredOut {
        highlight: Vec::new(),
        moves: Vec::new(),
        ops: Vec::new(),
        pass_all: false,
        blocked: result
            .blocked
            .iter()
            .map(|b| LessonBlockedOut {
                item: b.expect.clone(),
                reason: b.reason.code().to_owned(),
                why: b.why.clone(),
                at_bound: b.at_bound.iter().map(ToString::to_string).collect(),
            })
            .collect(),
    };
    if let Some(f) = &result.fix {
        let mut moves: Vec<_> = f.moves.iter().collect();
        moves.sort_by(|a, b| b.normalised().abs().total_cmp(&a.normalised().abs()));
        if let Some(first) = moves.first() {
            let way = |m: &enth_solver::Move| if m.to < m.from { "down" } else { "up" };
            out.moves = moves
                .iter()
                .map(|m| LessonMoveOut {
                    param: m.addr.to_string(),
                    name: m.spec.name.to_owned(),
                    from: show(Some(m.spec), m.from),
                    to: m.to_text(),
                    direction: way(m).to_owned(),
                })
                .collect();
            out.highlight = moves.iter().map(|m| m.addr.to_string()).collect();
            out.ops = fix::ops_json(patch, &f.ops());
            out.pass_all = f.pass_all;
            let text = format!(
                "Try turning {} ({}) {}.",
                first.spec.name,
                first.addr,
                way(first)
            );
            return (text, out);
        }
    }
    if let Some(b) = result.blocked.first() {
        out.highlight = b.at_bound.iter().map(ToString::to_string).collect();
        return (b.why.clone(), out);
    }
    (
        "It already sounds right: `enth lesson check` will say so.".to_owned(),
        out,
    )
}
