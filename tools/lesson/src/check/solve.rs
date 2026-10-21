//! **Owner: lane LCK.** `check --solve` (the [parent](super)'s step 4): every step can be done.
//!
//! | Step | What reaches it | Fails when |
//! | --- | --- | --- |
//! | `do` with a sound check | **the hint follower**: from the step's start, the real player asks for its measured hint (after the authored ones), `enth_solver::hint` answers it within the step's free set (its editable, unlocked parameters), the moves are applied, up to [`MAX_HINTS`] times | it does not reach the pass, unless the witness changes structure the hints cannot move (a waveform, a switch, a route, a transposition): then the step teaches structure and its witness shows it, which is noted |
//! | `do` with patch predicates only | Show me (the playthrough already proved it) | — |
//! | `recreate` | `enth_solver::closest` from the round's start over the challenge's editable parameters (else the round's drawn ones), unlocked | it does not pass within `--budget` renders |
//! | `choose` | the audibility gate, `enth_learn::check::choice_audibility` | two options are measured closer than `MIN_JND` (2 JND): an inaudible choice. A pair `sound/1` cannot compare (another phrase, a transpose, a seed or a curve) is noted for a listening review, and `lesson-coverage` counts the step as no check (Q9) |

use enth_analysis::expect::parse_item;
use enth_challenge::v2::{Selector, Touch, touches};
use enth_challenge::{Challenge, phrases, prepare_challenge};
use enth_learn::check::{MIN_JND, PairJnd, auditions, choice_audibility, evaluate_sounds};
use enth_learn::player::{Effect, SETTLE_MS};
use enth_learn::{Check, EditSource, Example, LessonBundle, Player, StepKind, StepView};
use enth_patch::{Builtin, ParamAddr, Patch, Target};
use enth_solver::closest::{ClosestCfg, ClosestJob, RoundSerial, closest};
use enth_solver::{HintCfg, Serial, hint};

use super::ops::{moves, set_knob};
use super::play::{Played, failing_predicates, far, touched};
use super::{Log, Options};

/// The most measured hints the follower takes.
pub(crate) const MAX_HINTS: usize = 6;

/// `--solve` over one lesson's playthrough.
pub(super) fn lesson(bundle: &LessonBundle, played: &[Played], opts: Options, log: &mut Log) {
    log.line(format!("  solve {}", bundle.key()));
    for p in played {
        let step = &bundle.lesson.steps[p.index];
        let at = p.at.as_str();
        match &step.kind {
            StepKind::Do { check, witness } => {
                if check.sounds().is_empty() {
                    log.line(format!(
                        "    {} (do): patch predicates only; Show me reaches the pass (the playthrough)",
                        step.id
                    ));
                    continue;
                }
                let (Some(start), Some(witness)) = (&p.start, bundle.patch(witness)) else {
                    continue; // the playthrough already failed it
                };
                do_step(
                    bundle,
                    p.index,
                    step.id.as_str(),
                    &step.view,
                    check,
                    start,
                    witness,
                    at,
                    log,
                );
            }
            StepKind::Recreate { challenge, .. } => {
                recreate_step(step.id.as_str(), challenge, opts.budget, at, log);
            }
            StepKind::Choose { options, .. } => {
                choose_step(bundle, step.id.as_str(), options, at, log);
            }
            StepKind::Read | StepKind::Listen { .. } => {}
        }
    }
}

/// How the hint follower ended.
enum Followed {
    /// The step passed after this many measured hints.
    Reached(usize),
    /// It did not: why, and the patch it got to.
    NotReached(String, Box<Patch>),
}

#[allow(
    clippy::too_many_arguments,
    reason = "the step's parts, unpacked once by the caller"
)]
fn do_step(
    bundle: &LessonBundle,
    index: usize,
    id: &str,
    view: &StepView,
    check: &Check,
    start: &Patch,
    witness: &Patch,
    at: &str,
    log: &mut Log,
) {
    match follow(bundle, index, view, check, start) {
        Ok(Followed::Reached(n)) => log.line(format!(
            "    {id} (do): the hint follower reaches the pass in {n} measured hint(s)"
        )),
        Ok(Followed::NotReached(why, at_patch)) => {
            // The measured hint aims at the sound items only: where they pass and a patch predicate does not, it
            // stops ("it already sounds right"), and a player following it is stuck.
            let sounds = check.sounds();
            let sound_passes = evaluate_sounds(&sounds, &at_patch).is_ok_and(|v| v.passed);
            let failing = failing_predicates(check, &at_patch);
            if sound_passes && !failing.is_empty() {
                log.line(format!("    {id} (do)"));
                log.fail(
                    at,
                    format!(
                        "--solve: the measured hints stop where the sound check passes and {} still fail{}: a                          player following them is told it already sounds right; make the sound items imply the                          predicates, or the predicates follow from the sound",
                        failing.join(", "),
                        if failing.len() == 1 { "s" } else { "" }
                    ),
                );
                return;
            }
            let structure: Vec<String> = moves(start, witness)
                .unwrap_or_default()
                .iter()
                .filter(|c| !c.continuous)
                .flat_map(|c| c.touches.clone().unwrap_or_default())
                .map(|t| t.to_string())
                .collect();
            if structure.is_empty() {
                log.line(format!("    {id} (do)"));
                log.fail(
                    at,
                    format!(
                        "--solve: the hint follower does not reach the pass from the start within the editable set \
                         ({why}), and the witness changes nothing the hints cannot move"
                    ),
                );
            } else {
                log.line(format!("    {id} (do)"));
                log.note(format!(
                    "{at}: the hint follower does not reach the pass ({why}); the step teaches structure, which its \
                     witness shows: {}",
                    structure.join(", ")
                ));
            }
        }
        Err(e) => {
            log.line(format!("    {id} (do)"));
            log.fail(at, format!("--solve: the hint follower: {e}"));
        }
    }
}

/// Follow the player's measured hints from `start` (see the [module docs](self)).
fn follow(
    bundle: &LessonBundle,
    index: usize,
    view: &StepView,
    check: &Check,
    start: &Patch,
) -> Result<Followed, String> {
    let step = u16::try_from(index).map_err(|_| "too many steps".to_owned())?;
    let (mut player, _) = Player::resume(bundle.clone(), step).map_err(|e| e.to_string())?;
    let passed = |effects: &[Effect]| effects.iter().any(|e| matches!(e, Effect::Passed { .. }));
    let mut patch = start.clone();
    let mut revision = 1_u64;
    let mut now = 1_000_u64;
    let effects = player
        .edited(now, revision, &patch, &[], EditSource::Load)
        .map_err(|e| e.to_string())?;
    if passed(&effects) {
        return Ok(Followed::Reached(0));
    }
    // "Turn it and listen": a `touched` control is turned across its span and back, as the step asks.
    for (knob, span) in touched(check) {
        let (Some(spec), Some(value)) = (knob.spec(&patch), knob.value(&patch)) else {
            continue;
        };
        for v in [far(spec, value, span), value] {
            let op = set_knob(&knob, v);
            enth_patch::apply(&mut patch, &op).map_err(|e| e.to_string())?;
            revision += 1;
            now += 10;
            let effects = player
                .edited(now, revision, &patch, &[op], EditSource::Player)
                .map_err(|e| e.to_string())?;
            if passed(&effects) {
                return Ok(Followed::Reached(0));
            }
        }
    }
    for n in 0..=MAX_HINTS {
        // Let the patch settle: the player asks for its sound check once the patch part holds.
        now += SETTLE_MS + 1;
        for effect in player.tick(now).map_err(|e| e.to_string())? {
            if let Effect::Check(request) = effect {
                let verdict = evaluate_sounds(&request.sounds, &request.patch).unwrap_or_default();
                let done = player
                    .check_done(request.revision, &verdict)
                    .map_err(|e| e.to_string())?;
                if passed(&done) {
                    return Ok(Followed::Reached(n));
                }
            }
        }
        if n == MAX_HINTS {
            break;
        }
        // The next measured hint (the authored ones come first).
        let mut request = None;
        for _ in 0..=enth_learn::lesson::MAX_HINTS {
            let effects = player.hint().map_err(|e| e.to_string())?;
            if effects.is_empty() {
                break;
            }
            if let Some(r) = effects.into_iter().find_map(|e| match e {
                Effect::HintJob(r) => Some(r),
                _ => None,
            }) {
                request = Some(r);
                break;
            }
        }
        let Some(request) = request else {
            return Ok(Followed::NotReached(
                "the player offers no measured hint".to_owned(),
                Box::new(patch),
            ));
        };
        let items = request
            .expect
            .iter()
            .map(|i| parse_item(i).map_err(|e| format!("{i:?}: {}", e.message)))
            .collect::<Result<Vec<_>, _>>()?;
        let phrase = phrases::phrase(request.phrase)
            .ok_or_else(|| format!("phrase {} is not in this build", request.phrase.0))?;
        let cfg = HintCfg {
            only: (!request.free.is_empty()).then(|| request.free.clone()),
            ..HintCfg::default()
        };
        // `RenderSpec::default()` is the scoring spec: 48 kHz at the scoring seed (enth-challenge checks the seeds
        // are one), as the plugin's measured hint renders. The type is inferred: enth-render is not a dependency of
        // the tool.
        #[allow(
            clippy::default_trait_access,
            reason = "enth_render::RenderSpec, which the tool cannot name"
        )]
        let spec = Default::default();
        let result = hint(
            &request.patch,
            &phrase.performance(),
            &spec,
            &items,
            &mut Serial::new(),
            &cfg,
        )
        .map_err(|e| e.to_string())?;
        let Some(fix) = result.fix else {
            let why = result
                .blocked
                .first()
                .map_or_else(|| "no move improves it".to_owned(), |b| b.why.clone());
            return Ok(Followed::NotReached(
                format!("after {n} hint(s): {why}"),
                Box::new(patch),
            ));
        };
        let ops = fix.ops();
        for op in &ops {
            let locked = touches(&patch, op)
                .unwrap_or_default()
                .iter()
                .any(|t| view.locks(t));
            if locked {
                return Ok(Followed::NotReached(
                    format!("the hint moves a locked control ({})", op.name()),
                    Box::new(patch),
                ));
            }
            enth_patch::apply(&mut patch, op).map_err(|e| e.to_string())?;
        }
        revision += 1;
        now += 10;
        let effects = player
            .edited(now, revision, &patch, &ops, EditSource::Player)
            .map_err(|e| e.to_string())?;
        if passed(&effects) {
            return Ok(Followed::Reached(n + 1));
        }
    }
    Ok(Followed::NotReached(
        format!("{MAX_HINTS} measured hints and the check still fails"),
        Box::new(patch),
    ))
}

/// The parameters a selector names in `patch`.
fn selector_params(selector: &Selector, patch: &Patch) -> Vec<ParamAddr> {
    let of_target = |target: &Target| -> Vec<ParamAddr> {
        let specs = match target {
            Target::Module(id) => patch.find(id).map_or(&[][..], |(_, m)| m.kind.params()),
            Target::Voice => Builtin::Voice.params(),
            Target::Master => Builtin::Master.params(),
        };
        specs
            .iter()
            .filter_map(|s| ParamAddr::new(target.clone(), s.key).ok())
            .collect()
    };
    match selector {
        Selector::Param(a) => vec![a.clone()],
        Selector::Module(target) => of_target(target),
        _ => Vec::new(),
    }
}

fn recreate_step(id: &str, challenge: &Challenge, budget: usize, at: &str, log: &mut Log) {
    let round = match prepare_challenge(challenge.clone()) {
        Ok(r) => r,
        Err(e) => {
            log.line(format!("    {id} (recreate)"));
            log.fail(at, format!("--solve: the round cannot be prepared: {e}"));
            return;
        }
    };
    let c = &challenge.constraints;
    let mut free: Vec<ParamAddr> = if c.editable.is_empty() {
        round.params().to_vec()
    } else {
        c.editable
            .iter()
            .flat_map(|s| selector_params(s, round.start()))
            .collect()
    };
    free.retain(|a| !c.is_locked(&Touch::Param(a.clone())));
    free.dedup();
    if free.is_empty() {
        log.line(format!("    {id} (recreate)"));
        log.fail(
            at,
            "--solve: no parameter is free to move (editable, unlocked)",
        );
        return;
    }
    let job = ClosestJob {
        round: &round,
        patch: round.start(),
        free: &free,
        budget,
    };
    match closest(&job, &mut RoundSerial::new(), &ClosestCfg::default()) {
        Ok(found) if found.passed => log.line(format!(
            "    {id} (recreate): closest passes in {} render(s) over {} free parameter(s) (sound {:.1} %, knob \
             distance {:.3})",
            found.renders,
            free.len(),
            found.percent,
            found.distance
        )),
        Ok(found) => {
            log.line(format!("    {id} (recreate)"));
            log.fail(
                at,
                format!(
                    "--solve: closest does not pass the round within {budget} renders over {} free parameter(s) \
                     (best sound {:.1} %, stopped: {})",
                    free.len(),
                    found.percent,
                    found.stop.name()
                ),
            );
        }
        Err(e) => {
            log.line(format!("    {id} (recreate)"));
            log.fail(at, format!("--solve: closest: {e} ({})", e.code()));
        }
    }
}

fn choose_step(bundle: &LessonBundle, id: &str, options: &[Example], at: &str, log: &mut Log) {
    let audibility = match choice_audibility(&auditions(bundle, options)) {
        Ok(a) => a,
        Err(e) => {
            log.line(format!("    {id} (choose)"));
            log.fail(
                at,
                format!(
                    "--solve: the options cannot be measured: {e} ({})",
                    e.code()
                ),
            );
            return;
        }
    };
    let label = |i: usize| options.get(i).map_or("?", |o| o.label.as_str());
    let mut parts = Vec::new();
    let mut unmeasured: Vec<(String, &str)> = Vec::new();
    log.line(format!("    {id} (choose)"));
    for pair in &audibility.pairs {
        let (a, b) = (label(pair.a), label(pair.b));
        match pair.jnd {
            PairJnd::Measured(j) if j >= MIN_JND => parts.push(format!("{a}-{b} {j:.2}")),
            PairJnd::Measured(j) => log.fail(
                at,
                format!(
                    "--solve: options {a:?} and {b:?} are {j:.2} JND apart under sound/1, below {MIN_JND}: an \
                     inaudible choice"
                ),
            ),
            PairJnd::NotMeasured(why) => unmeasured.push((format!("{a}-{b}"), why)),
        }
    }
    if let Some((_, why)) = unmeasured.first() {
        let pairs: Vec<&str> = unmeasured.iter().map(|(p, _)| p.as_str()).collect();
        log.note(format!(
            "{at}: sound/1 cannot compare options {} ({why}): a listening review decides, and lesson-coverage \
             counts the step as no check (taught, not checked)",
            pairs.join(", ")
        ));
    }
    if !parts.is_empty() {
        log.line(format!("      audible (JND): {}", parts.join(", ")));
    }
}
