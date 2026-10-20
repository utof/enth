//! **Owner: lane CL.** `enth suggest <patch> --code <code> [--level N]`: the Recreate game's hint button on the
//! command line. It runs lane SN's coach (`enth_solver::coach`) on the patch against the challenge's target: the
//! target's descriptors become an expectation list, the solver measures a move on the player's patch, and the
//! verified move is turned into four graded hints.
//!
//! - **Level 1** says which aspect is off and how ("Your sound is darker than the target"), **2** the module to look
//!   at, **3** the parameter and which way to turn it, **4** a range for it. Levels 1 to 3 hold no number at all, and
//!   level 4 a range around the fix, never the target's own value: the output carries a field only from its level on
//!   (`tests/suggest.rs` checks that no target value is printed early).
//! - It costs one coach plan: the target's sensitivity (one render per randomized parameter) and about 24 renders of
//!   your patch, on rayon. The answer is identical for any worker count.
//! - A hint makes a round *assisted* in the game; here it is only information. The exact fix the solver found is not
//!   printed (`enth check --expect` gives that kind of move for an expectation list).

use std::path::PathBuf;

use enth_params::ParamSpec;
use enth_patch::ParamAddr;
use enth_solver::coach::{
    CoachCfg, CoachPlan, CoachTarget, Difference, HINT_LEVELS, Turn, Verdict,
};

use super::common::show;
use super::game::{self, Resolved, WhichChallenge};
use super::{Reply, Res};
use crate::error::{self, CliError};
use crate::eval::Parallel;
use crate::patchio;
use crate::schema::{SuggestOut, ids};

/// `enth suggest`'s arguments.
#[derive(Clone, Debug, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// The patch to coach (*.patch.json).
    pub patch: PathBuf,
    #[command(flatten)]
    pub which: WhichChallenge,
    /// How much to reveal, 1 to 4: 1 which aspect is off, 2 which module, 3 which parameter and which way, 4 a range.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub level: u8,
    /// Worker threads for the renders (default: every core). The answer is the same for any number.
    #[arg(long, value_name = "N")]
    pub workers: Option<usize>,
}

/// Run `enth suggest`.
pub fn run(args: &Args) -> Res {
    let level = check_level(args.level)?;
    game::check_workers(args.workers)?;
    let (patch, mut notes) = patchio::load(&args.patch)?;
    let Resolved {
        round,
        code,
        notes: round_notes,
    } = game::resolve(&args.which)?;
    notes.extend(round_notes);

    let cfg = CoachCfg::default();
    let (target_renders, plan) = game::with_workers(args.workers, || {
        let mut eval = Parallel;
        let target =
            CoachTarget::for_round(&round, &mut eval, &cfg).map_err(|e| game::from_coach(&e))?;
        let plan = target
            .plan(&patch, &mut eval, &cfg)
            .map_err(|e| game::from_coach(&e))?;
        Ok::<_, CliError>((target.sensitivity().renders, plan))
    })??;

    let spec_of = |addr: &ParamAddr| -> Option<&'static ParamSpec> {
        patch
            .spec(addr)
            .or_else(|| round.start().spec(addr))
            .or_else(|| round.target().spec(addr))
    };
    let out = suggest_out(
        &args.patch,
        code.as_deref(),
        level,
        &plan,
        target_renders,
        &spec_of,
    );
    let mut reply = Reply::new(&out).text(format!("{}\n", out.text));
    for n in &notes {
        reply = reply.note(n);
    }
    Ok(reply)
}

/// `--level`, 1 to 4.
fn check_level(level: u8) -> Result<u8, CliError> {
    if (1..=HINT_LEVELS).contains(&level) {
        Ok(level)
    } else {
        Err(error::usage(
            "bad_level",
            format!("--level {level}: a hint has levels 1 to {HINT_LEVELS}"),
            "1 which aspect is off, 2 which module, 3 which parameter and which way, 4 a value range",
        ))
    }
}

/// The output for the hint at `level`. A field is set only from its level on, so a higher level's information never
/// reaches a lower level's output.
fn suggest_out(
    patch: &std::path::Path,
    code: Option<&str>,
    level: u8,
    plan: &CoachPlan,
    target_renders: usize,
    spec_of: &dyn Fn(&ParamAddr) -> Option<&'static ParamSpec>,
) -> SuggestOut {
    let hint = plan.hint(level);
    let close = plan.verdict == Verdict::Close;
    let range = hint.range.and_then(|(lo, hi)| {
        let spec = hint.param.as_ref().and_then(spec_of);
        spec.map(|s| vec![show(Some(s), lo), show(Some(s), hi)])
    });
    let next = if close {
        "nothing measured is off: submit it, or compare with `enth compare`".to_owned()
    } else if level < HINT_LEVELS {
        format!(
            "`--level {}` reveals more; each level costs one coach run, and the game counts a hint as assistance",
            level + 1
        )
    } else {
        "that is the last level: make the move, then ask again for the next one".to_owned()
    };
    SuggestOut {
        schema: ids::SUGGEST.to_owned(),
        patch: patchio::show(patch),
        code: code.map(str::to_owned),
        level,
        levels: HINT_LEVELS,
        close,
        measured: hint.measured,
        aspect: hint.aspect.map(|a| a.code().to_owned()),
        difference: hint.difference.map(|d| difference(d).to_owned()),
        module: hint.module.as_ref().map(ToString::to_string),
        param: hint.param.as_ref().map(ToString::to_string),
        turn: hint.turn.map(|t| turn(t).to_owned()),
        range,
        text: hint.text.clone(),
        renders: target_renders + plan.renders(),
        hint: next,
    }
}

fn difference(d: Difference) -> &'static str {
    match d {
        Difference::Higher => "higher",
        Difference::Lower => "lower",
        Difference::Missing => "missing",
        Difference::Extra => "extra",
        _ => "different",
    }
}

fn turn(t: Turn) -> &'static str {
    match t {
        Turn::Raise => "raise",
        Turn::Lower => "lower",
    }
}
