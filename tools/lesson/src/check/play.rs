//! **Owner: lane LCK.** `check`'s playthrough (the [parent](super)'s step 2): one lesson played through a headless
//! `Session` with the real `Player`, the shell's part done here, every job run inline between frames (as
//! `enth-session`'s `tests/learn_player.rs` does). From each step's start the check fails; after the reference solution
//! it passes.
//!
//! | Step | Before | The reference solution | After |
//! | --- | --- | --- | --- |
//! | `do` | the patch settles and the session's check (patch predicates, then the sound job) does not pass; a sound item fails by at least [`MIN_MARGIN`](super::MIN_MARGIN), not `thin` | the witness, applied op by op by **Show me** through the session's edit path (a locked op is refused there); a `touched` control is turned across its span and back | the step passes; every sound item passes by at least the margin, none `thin` (nuisance renders) |
//! | `recreate` | the round's start does not pass its scorers | the witness (or the round's target), edited in and submitted | the round, and so the step, passes |
//! | `choose` | each wrong pick leaves the step unpassed | the right pick | it passes |
//!
//! Before the witness is applied, its ops are checked against the step's view: none may touch a locked control, and
//! with an `editable` list every op stays inside it. Every `enth:` link and widget of a step's text resolves against
//! the step's start (a step with no start of its own resolves against the patch the earlier steps left; before any
//! step loads a patch the player's own is unknown and controls are not resolved). A generated round's challenge code
//! decodes back to its spec.

use enth_challenge::model::Constraints;
use enth_challenge::score::round::target_lufs;
use enth_challenge::{
    Challenge, ChallengeCode, PreparedRound, TargetPrefix, phrases, prepare_challenge, score_round,
    score_structure,
};
use enth_learn::check::{
    SoundOptions, SoundReport, TouchLog, evaluate_sounds, evaluate_sounds_report,
};
use enth_learn::markdown;
use enth_learn::player::CheckVerdict;
use enth_learn::{
    Check, Knob, LessonBundle, Player, Predicate, Step, StepKind, StepPhase, StepView,
};
use enth_patch::Patch;
use enth_session::Session;
use enth_session::game::{EditorView, GameEdit, GameRequest, JobKind, JobResult, Phase, Prepared};
use enth_session::learn::{LearnEdit, LearnError, LearnJobKind, LearnRequest, LearnResult};

use super::ops::{moves, set_knob};
use super::{Log, MIN_MARGIN};

/// What the playthrough found at one step, for `--cross` and `--solve`.
#[derive(Clone, Debug)]
pub(crate) struct Played {
    /// `pack/lesson#step`.
    pub at: String,
    /// The step's index.
    pub index: usize,
    /// The checked patch at the step's start (`None` while the player's own patch is all there is).
    pub start: Option<Patch>,
    /// The checked patch after its reference solution (`None` when it could not be applied).
    pub end: Option<Patch>,
}

/// The headless shell: a session, a clock, and the round its prepare job built.
struct Shell {
    s: Session,
    now: u64,
    round: Option<PreparedRound>,
    errors: Vec<String>,
}

impl Shell {
    fn new() -> Shell {
        let mut s = Session::default();
        let now = 1_000;
        s.game_tick(now, EditorView::Opened);
        Shell {
            s,
            now,
            round: None,
            errors: Vec::new(),
        }
    }

    /// Run every job the session asks for, until it asks for none.
    fn pump(&mut self) {
        for _ in 0..32 {
            let learn: Vec<LearnRequest> = self.s.drain_learn_requests().collect();
            let game: Vec<GameRequest> = self.s.drain_game_requests().collect();
            let mut ran = false;
            for r in learn {
                if let LearnRequest::Run(job) = r {
                    ran = true;
                    let result = self.learn_job(&job.kind);
                    match self.s.learn_job_done(job.id, result) {
                        Ok(()) | Err(LearnError::StaleJob(_)) => {}
                        Err(e) => self
                            .errors
                            .push(format!("the session refused a learn result: {e}")),
                    }
                }
            }
            for r in game {
                if let GameRequest::Run(job) = r {
                    ran = true;
                    let result = self.game_job(&job.kind);
                    if let Err(e) = self.s.game_job_done(job.id, result) {
                        self.errors
                            .push(format!("the session refused a game result: {e}"));
                    }
                }
            }
            if !ran {
                return;
            }
        }
        self.errors
            .push("the session kept asking for jobs (32 rounds)".to_owned());
    }

    fn learn_job(&mut self, kind: &LearnJobKind) -> LearnResult {
        match kind {
            LearnJobKind::Check(request) => {
                match evaluate_sounds(&request.sounds, &request.patch) {
                    Ok(verdict) => LearnResult::Checked(verdict),
                    Err(e) => {
                        self.errors
                            .push(format!("a sound check could not run: {e} ({})", e.code()));
                        LearnResult::Checked(CheckVerdict::default())
                    }
                }
            }
            LearnJobKind::Examples { examples } => LearnResult::Examples {
                lufs: vec![None; examples.len()],
            },
            other => LearnResult::Failed {
                message: format!("lesson-check runs no {} job here", other.name()),
            },
        }
    }

    fn game_job(&mut self, kind: &JobKind) -> JobResult {
        match kind {
            JobKind::PrepareChallenge { challenge } => {
                match prepare_challenge(Challenge::clone(challenge)) {
                    Ok(round) => {
                        let lufs = phrases::phrase(round.challenge().phrase)
                            .and_then(|p| target_lufs(round.target(), p).ok().flatten())
                            .map(|l| l as f32);
                        let prepared = Prepared::from_round(&round, lufs);
                        self.round = Some(round);
                        JobResult::Prepared(Box::new(prepared))
                    }
                    Err(e) => {
                        self.errors
                            .push(format!("the round could not be prepared: {e}"));
                        JobResult::Failed {
                            message: e.to_string(),
                        }
                    }
                }
            }
            JobKind::Score { patch, .. } => match &self.round {
                Some(round) => match score_round(round, patch) {
                    Ok(report) => JobResult::Scored(Box::new(report)),
                    Err(e) => JobResult::Failed {
                        message: e.to_string(),
                    },
                },
                None => JobResult::Failed {
                    message: "no prepared round".to_owned(),
                },
            },
            JobKind::Rings { patch, .. } => match &self.round {
                Some(round) => match score_structure(round, patch) {
                    Ok(s) => JobResult::Rings(Box::new(s)),
                    Err(e) => JobResult::Failed {
                        message: e.to_string(),
                    },
                },
                None => JobResult::Failed {
                    message: "no prepared round".to_owned(),
                },
            },
            JobKind::Loudness { .. } => JobResult::Loudness { lufs: None },
            other => JobResult::Failed {
                message: format!("lesson-check runs no {} job here", other.name()),
            },
        }
    }

    /// Let the patch settle past the player's delay, twice (the check job, then its answer), running every job.
    fn settle(&mut self) {
        for _ in 0..2 {
            self.now += enth_learn::player::SETTLE_MS + 100;
            self.s.game_tick(self.now, EditorView::Visible);
            self.pump();
        }
    }

    fn phase(&self) -> Option<StepPhase> {
        self.s.lesson().map(|v| v.phase)
    }

    fn passed(&self) -> bool {
        self.phase() == Some(StepPhase::Passed)
    }

    /// Report and forget the job errors so far.
    fn flush(&mut self, at: &str, log: &mut Log) {
        for e in self.errors.drain(..) {
            log.fail(at, e);
        }
    }
}

/// A learn error as one line with its code and hint.
fn learn(e: &LearnError) -> String {
    format!("{e} ({}: {})", e.code(), e.hint())
}

/// Play one lesson. One [`Played`] per step reached.
pub(super) fn lesson(bundle: &LessonBundle, log: &mut Log) -> Vec<Played> {
    let key = bundle.key().to_string();
    let lesson = &bundle.lesson;
    log.line(format!(
        "  lesson {key}: {} ({} steps)",
        lesson.title,
        lesson.steps.len()
    ));
    let mut shell = Shell::new();
    if let Err(e) = shell.s.learn_start(Box::new(Player::new(bundle.clone()))) {
        log.fail(&key, format!("the lesson does not start: {}", learn(&e)));
        return Vec::new();
    }
    shell.pump();
    let mut out = Vec::with_capacity(lesson.steps.len());
    let mut known = false;
    for (index, step) in lesson.steps.iter().enumerate() {
        let at = format!("{key}#{}", step.id);
        log.step();
        known |= step.view.start.is_some() || matches!(step.kind, StepKind::Recreate { .. });
        let start = known.then(|| shell.s.checked_patch());
        resolve(bundle, step, start.as_ref(), &at, log);
        let end = match &step.kind {
            StepKind::Read | StepKind::Listen { .. } => {
                log.line(format!("    {} ({}): ok", step.id, step.kind.name()));
                start.clone()
            }
            StepKind::Do { check, witness } => {
                let witness = bundle.patch(witness).map(|p| Patch::clone(p));
                do_step(
                    &mut shell,
                    step,
                    check,
                    witness.as_ref(),
                    start.as_ref(),
                    &at,
                    log,
                )
            }
            StepKind::Recreate { challenge, witness } => {
                let witness = witness
                    .as_ref()
                    .and_then(|w| bundle.patch(w))
                    .map(|p| Patch::clone(p));
                recreate_step(&mut shell, step, challenge, witness, &at, log)
            }
            StepKind::Choose { options, answer } => {
                choose_step(&mut shell, step, options, *answer, &at, log);
                start.clone()
            }
            StepKind::Drill(_) => {
                // A drill is a game with its own tests (the staircase against simulated listeners): the
                // playthrough leaves the step as it is, like the Read steps around it.
                log.line(format!("    {} ({}): ok", step.id, step.kind.name()));
                start.clone()
            }
        };
        shell.flush(&at, log);
        out.push(Played {
            at: at.clone(),
            index,
            start,
            end,
        });
        if let Err(e) = shell.s.learn_next() {
            log.fail(&at, format!("Next: {}", learn(&e)));
            break;
        }
        shell.pump();
        shell.flush(&at, log);
    }
    if shell.s.lesson().is_some_and(|v| !v.finished) {
        log.fail(&key, "the last Next does not finish the lesson");
    }
    if let Err(e) = shell.s.learn_end(enth_session::game::EndChoice::Restore) {
        log.fail(&key, format!("End: {}", learn(&e)));
    }
    out
}

/// Every `enth:` link and widget of the step's text against its start and its examples.
fn resolve(bundle: &LessonBundle, step: &Step, start: Option<&Patch>, at: &str, log: &mut Log) {
    let Some(t) = &step.text else {
        return;
    };
    let Some(text) = bundle.text(t) else {
        return;
    };
    let examples = match &step.kind {
        StepKind::Listen { examples } => examples.len(),
        StepKind::Choose { options, .. } => options.len(),
        _ => 0,
    };
    for issue in markdown::parse(text).resolve(start, examples) {
        log.fail(
            at,
            format!(
                "{}:{}: {} ({})",
                t.path_in(&bundle.lesson.id),
                issue.line(text),
                issue.message,
                issue.code
            ),
        );
    }
}

/// The witness's ops against a view: locked controls never, and only editable ones when the view lists any.
fn witness_touches(
    start: &Patch,
    witness: &Patch,
    locked: impl Fn(&enth_challenge::v2::Touch) -> Option<String>,
    editable: impl Fn(&enth_challenge::v2::Touch) -> bool,
    at: &str,
    log: &mut Log,
) -> Option<usize> {
    let Some(list) = moves(start, witness) else {
        log.fail(at, "no ops lead from the start to the witness");
        return None;
    };
    for c in &list {
        match &c.touches {
            None => log.fail(
                at,
                format!(
                    "the witness needs a {} op this build cannot judge",
                    c.op.name()
                ),
            ),
            Some(touched) => {
                for t in touched {
                    if let Some(lock) = locked(t) {
                        log.fail(
                            at,
                            format!("the witness touches {t}, which the step locks ({lock})"),
                        );
                    } else if !editable(t) {
                        log.fail(
                            at,
                            format!("the witness touches {t}, which the step's `editable` list leaves out"),
                        );
                    }
                }
            }
        }
    }
    Some(list.len())
}

/// The `touched` predicates of a check: each control and its span, 0–1.
pub(super) fn touched(check: &Check) -> Vec<(Knob, f32)> {
    match check {
        Check::Patch { predicates } => predicates
            .iter()
            .filter_map(|p| match p {
                Predicate::Touched { knob, min_span_pct } => {
                    Some((knob.clone(), f32::from(*min_span_pct) / 100.0))
                }
                _ => None,
            })
            .collect(),
        Check::Sound { .. } => Vec::new(),
        Check::All { checks } => checks.iter().flat_map(touched).collect(),
    }
}

/// Where a `touched` control is turned to cover `span` (0–1) of its travel from `value`: up when there is room, else
/// down.
pub(super) fn far(spec: &enth_params::ParamSpec, value: f32, span: f32) -> f32 {
    let n = spec.to_norm(value);
    spec.from_norm(if n + span <= 1.0 {
        n + span
    } else {
        (n - span).max(0.0)
    })
}

/// The patch predicates of a check that fail on `patch` (`touched` ones aside: the edit log is the player's).
pub(super) fn failing_predicates(check: &Check, patch: &Patch) -> Vec<String> {
    let log = TouchLog::default();
    match check {
        Check::Patch { predicates } => predicates
            .iter()
            .filter(|p| !matches!(p, Predicate::Touched { .. }) && !p.holds(patch, &log))
            .map(ToString::to_string)
            .collect(),
        Check::All { checks } => checks
            .iter()
            .flat_map(|c| failing_predicates(c, patch))
            .collect(),
        Check::Sound { .. } => Vec::new(),
    }
}

/// One line per sound item: the reading, its margin, its noise.
fn items(reports: &[SoundReport]) -> Vec<String> {
    reports
        .iter()
        .flat_map(|r| r.report.results.iter())
        .map(|i| {
            let num = |v: Option<f64>| v.map_or_else(|| "none".to_owned(), |v| format!("{v:.4}"));
            format!(
                "{} got {} {:?}, margin {}, noise {}{}",
                i.expect,
                num(i.compared),
                i.unit,
                i.margin
                    .map_or_else(|| "none".to_owned(), |m| format!("{m:+.4}")),
                num(i.noise),
                if i.thin { ", THIN" } else { "" }
            )
        })
        .collect()
}

#[allow(
    clippy::too_many_arguments,
    reason = "the step's parts, unpacked once by the caller"
)]
fn do_step(
    shell: &mut Shell,
    step: &Step,
    check: &Check,
    witness: Option<&Patch>,
    start: Option<&Patch>,
    at: &str,
    log: &mut Log,
) -> Option<Patch> {
    log.line(format!("    {} (do)", step.id));
    let Some(start) = start else {
        log.fail(
            at,
            "the step plays from the player's own patch (no start of its own, and no earlier step loads one), so \
             its check cannot be proven: give it `view.start`",
        );
        return None;
    };
    let Some(witness) = witness else {
        log.fail(at, "its witness is not in the pack");
        return None;
    };
    let sounds = check.sounds();
    let options = SoundOptions { nuisance: true };

    // Before: the session's own check, after the patch settles.
    shell.settle();
    if shell.passed() {
        log.fail(at, "the start already passes the check");
    }
    if !sounds.is_empty() {
        if check.patch_holds(start, &TouchLog::default()) {
            match evaluate_sounds_report(&sounds, start, options) {
                Ok(reports) => {
                    for line in items(&reports) {
                        log.line(format!("      start:   {line}"));
                    }
                    let clear = reports
                        .iter()
                        .flat_map(|r| r.report.results.iter())
                        .any(|i| !i.pass && !i.thin && i.margin.is_none_or(|m| m <= -MIN_MARGIN));
                    if !clear {
                        log.fail(
                            at,
                            format!(
                                "the start does not fail a sound item clearly (by {MIN_MARGIN} or more, not thin)"
                            ),
                        );
                    }
                }
                Err(e) => log.fail(at, format!("the start's sound check: {e} ({})", e.code())),
            }
        } else {
            log.line("      start:   the patch part fails, so the sound part is not reached");
        }
    }

    // The witness against the view, then applied by Show me through the session's edit path.
    let view: &StepView = &step.view;
    let count = witness_touches(
        start,
        witness,
        |t| {
            view.locked
                .iter()
                .find(|s| s.covers(t))
                .map(ToString::to_string)
        },
        |t| view.allows(t),
        at,
        log,
    )?;
    if count == 0 {
        log.fail(at, "the witness is the start: nothing to apply");
    }
    let mut shows = 0_usize;
    loop {
        let left = moves(shell.s.patch(), witness).map_or(0, |m| m.len());
        if left == 0 {
            break;
        }
        if shows > count + 2 {
            log.fail(
                at,
                format!("Show me does not reach the witness ({left} op(s) left)"),
            );
            break;
        }
        match shell.s.learn_show() {
            Ok(()) => {
                shows += 1;
                shell.pump();
            }
            Err(e) => {
                log.fail(at, format!("Show me: {}", learn(&e)));
                break;
            }
        }
    }
    // A `touched` control: turned across its span and back, as the player would.
    for (knob, span) in touched(check) {
        let patch = shell.s.patch().clone();
        let (Some(spec), Some(value)) = (knob.spec(&patch), knob.value(&patch)) else {
            log.fail(
                at,
                format!("`touched {knob}`: the patch has no such control"),
            );
            continue;
        };
        for v in [far(spec, value, span), value] {
            if let Err(e) = shell.s.edit(vec![set_knob(&knob, v)], None) {
                log.fail(
                    at,
                    format!("turning {knob} for `touched`: {e} ({})", e.code()),
                );
            }
        }
        shell.pump();
    }

    // After: the step passes, and every sound item holds by the margin.
    shell.settle();
    let end = shell.s.checked_patch();
    if !shell.passed() {
        let failing = failing_predicates(check, &end);
        log.fail(
            at,
            if failing.is_empty() {
                "the witness does not pass the check".to_owned()
            } else {
                format!(
                    "the witness does not pass the check: {} {}",
                    failing.join(", "),
                    if failing.len() == 1 { "fails" } else { "fail" }
                )
            },
        );
    }
    if !sounds.is_empty() {
        match evaluate_sounds_report(&sounds, &end, options) {
            Ok(reports) => {
                for line in items(&reports) {
                    log.line(format!("      witness: {line}"));
                }
                for i in reports.iter().flat_map(|r| r.report.results.iter()) {
                    if !i.pass {
                        log.fail(at, format!("at the witness, {} fails", i.expect));
                    } else if i.thin {
                        log.fail(
                            at,
                            format!(
                                "at the witness, {} is thin (an inaudible change could flip it)",
                                i.expect
                            ),
                        );
                    } else if i.margin.is_some_and(|m| m < MIN_MARGIN) {
                        log.fail(
                            at,
                            format!(
                                "at the witness, {} passes by less than {MIN_MARGIN}",
                                i.expect
                            ),
                        );
                    }
                }
            }
            Err(e) => log.fail(at, format!("the witness's sound check: {e} ({})", e.code())),
        }
    }
    log.line(format!(
        "      Show me applied {shows} op(s); the check fails at the start and {} at the witness",
        if shell.passed() {
            "passes"
        } else {
            "does NOT pass"
        }
    ));
    Some(end)
}

fn recreate_step(
    shell: &mut Shell,
    step: &Step,
    challenge: &Challenge,
    witness: Option<Patch>,
    at: &str,
    log: &mut Log,
) -> Option<Patch> {
    log.line(format!("    {} (recreate)", step.id));
    let Some(round) = shell.round.take() else {
        log.fail(at, "the step's round did not start");
        return None;
    };
    // A generated target's code decodes back to its spec.
    if let Some(spec) = challenge.spec() {
        match challenge.code(TargetPrefix::of(&round.fingerprint())) {
            Some(code) => {
                let text = code.encode();
                match ChallengeCode::decode(&text) {
                    Ok(back) if back.spec() == spec => {
                        log.line(format!("      code {text}: decodes"));
                    }
                    Ok(_) => log.fail(at, format!("the code {text} decodes to another spec")),
                    Err(e) => log.fail(at, format!("the code {text} does not decode: {e}")),
                }
            }
            None => log.fail(at, "the generated target has no code"),
        }
    }
    // Before: the start does not pass.
    match score_round(&round, round.start()) {
        Ok(r) => {
            log.line(format!("      start:   sound {:.1} %", r.sound.percent));
            if r.passed {
                log.fail(
                    at,
                    format!(
                        "the round's start already passes (sound {:.1} %)",
                        r.sound.percent
                    ),
                );
            }
        }
        Err(e) => log.fail(at, format!("the start cannot be scored: {e}")),
    }
    // The witness (or the target) against the challenge's constraints, edited in and submitted.
    let witness = witness.unwrap_or_else(|| round.target().clone());
    let constraints: &Constraints = &challenge.constraints;
    let start = round.start().clone();
    witness_touches(
        &start,
        &witness,
        |t| constraints.lock_of(t).map(ToString::to_string),
        |t| constraints.is_editable(t),
        at,
        log,
    )?;
    let ops: Vec<_> = moves(&start, &witness)
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.op)
        .collect();
    let witness_score = score_round(&round, &witness).ok();
    shell.round = Some(round);
    if !ops.is_empty()
        && let Err(e) = shell.s.edit(ops, None)
    {
        log.fail(at, format!("editing the witness in: {e} ({})", e.code()));
        return None;
    }
    shell.now += 500;
    shell.s.game_tick(shell.now, EditorView::Visible);
    if let Err(e) = shell.s.game_submit() {
        log.fail(at, format!("Submit: {e} ({})", e.code()));
        return None;
    }
    shell.pump();
    let round_passed = shell.s.round().is_some_and(|r| r.phase == Phase::Passed);
    let percent = witness_score.map_or(f64::NAN, |r| r.sound.percent);
    log.line(format!("      witness: sound {percent:.1} %"));
    if !round_passed || !shell.passed() {
        log.fail(
            at,
            format!("the witness does not pass the round (sound {percent:.1} %)"),
        );
    }
    Some(shell.s.checked_patch())
}

fn choose_step(
    shell: &mut Shell,
    step: &Step,
    options: &[enth_learn::Example],
    answer: u8,
    at: &str,
    log: &mut Log,
) {
    for (k, option) in options.iter().enumerate() {
        let Ok(k) = u8::try_from(k) else {
            continue;
        };
        if k == answer {
            continue;
        }
        if let Err(e) = shell.s.learn_choose(k) {
            log.fail(at, format!("picking {:?}: {}", option.label, learn(&e)));
        }
        if shell.passed() {
            log.fail(at, format!("the wrong pick {:?} passes", option.label));
        }
    }
    if let Err(e) = shell.s.learn_choose(answer) {
        log.fail(at, format!("picking the answer: {}", learn(&e)));
    }
    let label = options
        .get(usize::from(answer))
        .map_or("?", |o| o.label.as_str());
    if shell.passed() {
        log.line(format!(
            "    {} (choose): every wrong pick leaves it open, the answer {label:?} passes",
            step.id
        ));
    } else {
        log.fail(at, format!("the answer {label:?} does not pass"));
    }
}
