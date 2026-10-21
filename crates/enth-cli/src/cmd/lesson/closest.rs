//! **Owner: lane EV3.** `enth lesson closest <PACK> --lesson ID --step STEP <PATCH> [--budget N] [--free A,B] [--out FILE]`:
//! the "How close am I?" button of a recreate step (docs/dsp/match.md, "Closest"). It runs `enth_solver::closest` from
//! your patch over the step's editable controls and answers the nearest patch that passes the step's round, with how far
//! each control moved. It costs a search (tens of renders), so it is a command and not a reading; in the player, using
//! it makes the redo a sibling challenge. `enth.lesson-closest/1`.
//!
//! The answer is deterministic for any `--workers`. A `passed: false` result means none was found within the budget
//! (the free controls may not be able to pass the round at all); its `deltas` are the nearest to passing, not a
//! solution. Exit 0 whenever the search ran, like `enth match`.

use std::path::Path;

use enth_challenge::{Challenge, PreparedRound, prepare_challenge};
use enth_learn::{StepKind, StepView};
use enth_patch::{Op, ParamAddr, Patch, Target};
use enth_solver::closest::{
    ClosestCfg, ClosestError, ClosestJob, DEFAULT_BUDGET, Reading, closest, reading,
};
use rayon::prelude::*;

use super::{lesson_of, load_pack, step_of};
use crate::cmd::common::show;
use crate::cmd::{Reply, Res, game};
use crate::error::{self, CliError, nearest};
use crate::out::round4;
use crate::patchio;
use crate::schema::{LessonClosestOut, LessonDeltaOut, ids};

/// The parameters of `patch` the view allows moving, in the player's order (modules, then voice, then master); the
/// same table the session's locks use.
fn allowed(view: &StepView, patch: &Patch) -> Vec<ParamAddr> {
    let allows = |addr: &ParamAddr| {
        let op = Op::SetParam {
            addr: addr.clone(),
            value: patch.value(addr).unwrap_or(0.0),
        };
        enth_challenge::v2::touches(patch, &op).is_some_and(|t| t.iter().all(|t| view.allows(t)))
    };
    let mut out = Vec::new();
    for (_, m) in patch.modules() {
        for spec in m.kind.params() {
            if let Ok(a) = ParamAddr::new(Target::Module(m.id.clone()), spec.key)
                && allows(&a)
            {
                out.push(a);
            }
        }
    }
    for (target, params) in [
        (Target::Voice, enth_patch::Builtin::Voice.params()),
        (Target::Master, enth_patch::Builtin::Master.params()),
    ] {
        for spec in params {
            if let Ok(a) = ParamAddr::new(target.clone(), spec.key)
                && allows(&a)
            {
                out.push(a);
            }
        }
    }
    out
}

/// `--free`: full addresses that the patch has.
fn named(free: &[String], patch: &Patch) -> Result<Vec<ParamAddr>, CliError> {
    free.iter()
        .map(|text| {
            text.parse::<ParamAddr>()
                .ok()
                .filter(|a| patch.spec(a).is_some())
                .ok_or_else(|| {
                    let have: Vec<String> = patch
                        .modules()
                        .flat_map(|(_, m)| {
                            m.kind
                                .params()
                                .iter()
                                .map(|s| format!("{}.{}", m.id, s.key))
                                .collect::<Vec<_>>()
                        })
                        .collect();
                    error::invalid(
                        "unknown_param",
                        format!("--free {text}: the patch has no such parameter"),
                        "name full addresses, such as filter1.cutoff (`enth get <patch>` lists them)",
                    )
                    .suggest(nearest(text, have.iter().map(String::as_str)))
                })
        })
        .collect()
}

fn from_closest(e: &ClosestError) -> CliError {
    match e {
        ClosestError::Search(inner) => game::from_search(inner),
        _ => error::failed(e.code(), e.to_string(), e.hint()),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "one command: every flag of it is a parameter"
)]
pub(super) fn closest_cmd(
    path: &Path,
    lesson: &str,
    which: &str,
    patch_path: &Path,
    budget: Option<u32>,
    free: &[String],
    out_path: Option<&Path>,
    workers: Option<usize>,
) -> Res {
    let pack = load_pack(path)?;
    let bundle = lesson_of(&pack, lesson)?;
    let (i, step) = step_of(&bundle.lesson, which)?;
    let number = u16::try_from(i + 1).unwrap_or(u16::MAX);
    let key = bundle.key().to_string();
    let StepKind::Recreate { challenge, .. } = &step.kind else {
        let how = match step.kind {
            StepKind::Do { .. } => {
                "a do step's help is `enth lesson hint` (a measured move) and `enth lesson check`"
            }
            _ => "only a recreate step is a round with a nearest passing patch",
        };
        return Err(error::usage(
            "not_a_round",
            format!(
                "step {number} ({}) of {key} is a {} step, which has no round to search",
                step.id,
                step.kind.name()
            ),
            how,
        ));
    };
    game::check_workers(workers)?;
    let budget = match budget {
        None => DEFAULT_BUDGET,
        Some(0) => {
            return Err(error::usage(
                "bad_budget",
                "--budget 0: the search needs at least one render (your own patch is the first)",
                "the default is 300; 40 to 80 renders settle a step with one to three free controls",
            ));
        }
        Some(b) => b as usize,
    };
    let (patch, notes) = patchio::load(patch_path)?;
    let round: PreparedRound =
        prepare_challenge(Challenge::clone(challenge)).map_err(|e| game::from_round(&e))?;
    let free_set = if free.is_empty() {
        let view = StepView {
            editable: challenge.constraints.editable.clone(),
            locked: challenge.constraints.locked.clone(),
            ..StepView::default()
        };
        if view.editable.is_empty() {
            return Err(error::usage(
                "no_editable_set",
                format!(
                    "step {number} ({}) of {key} does not say which controls are editable, so there is no small set to search",
                    step.id
                ),
                "name the controls to search: --free filter1.cutoff,filter1.resonance",
            ));
        }
        allowed(&view, &patch)
    } else {
        named(free, &patch)?
    };
    let job = ClosestJob {
        round: &round,
        patch: &patch,
        free: &free_set,
        budget,
    };
    let found = game::with_workers(workers, || {
        let mut eval = |r: &PreparedRound, candidates: &[Patch]| -> Vec<Option<Reading>> {
            candidates.par_iter().map(|p| reading(r, p)).collect()
        };
        closest(&job, &mut eval, &ClosestCfg::default())
    })?
    .map_err(|e| from_closest(&e))?;
    if let Some(file) = out_path {
        patchio::write(file, &found.patch)?;
    }
    let deltas: Vec<LessonDeltaOut> = found
        .deltas
        .iter()
        .map(|d| {
            let spec = patch.spec(&d.addr);
            LessonDeltaOut {
                param: d.addr.to_string(),
                from: show(spec, d.from),
                to: show(spec, d.to),
                norm: round4(d.norm),
                categorical: d.categorical,
                moved: d.moved(),
            }
        })
        .collect();
    let moved: Vec<&str> = deltas
        .iter()
        .filter(|d| d.moved)
        .map(|d| d.param.as_str())
        .collect();
    let text = if found.passed && moved.is_empty() {
        "Your patch already passes the round.".to_owned()
    } else if found.passed {
        format!(
            "The nearest passing patch found moves {}.",
            moved.join(", ")
        )
    } else {
        format!(
            "No passing patch within {budget} renders; the nearest to passing moves {}.",
            if moved.is_empty() {
                "nothing".to_owned()
            } else {
                moved.join(", ")
            }
        )
    };
    let result = LessonClosestOut {
        schema: ids::LESSON_CLOSEST.to_owned(),
        lesson: key,
        step: step.id.to_string(),
        number,
        patch: patchio::show(patch_path),
        free: free_set.iter().map(ToString::to_string).collect(),
        budget: u32::try_from(budget).unwrap_or(u32::MAX),
        passed: found.passed,
        start_percent: round4(found.start_percent),
        percent: round4(found.percent),
        distance: round4(found.distance),
        renders: u32::try_from(found.renders).unwrap_or(u32::MAX),
        first_pass_renders: found
            .first_pass_renders
            .map(|r| u32::try_from(r).unwrap_or(u32::MAX)),
        stop: found.stop.name().to_owned(),
        deltas,
        out: out_path.map(patchio::show),
        text,
        next: if found.passed {
            "the round is passed with the answer; a lesson counts a pass that used this help as assisted".to_owned()
        } else {
            "raise --budget, free more controls with --free, or `enth lesson hint` for a graded hint".to_owned()
        },
    };
    let mut reply = Reply::new(&result);
    for n in &notes {
        reply = reply.note(n);
    }
    Ok(reply)
}
