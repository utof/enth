//! **Owner: lane LM** (the API and a basic body, freeze point F1l); **PLY** (the properties, refinements). The
//! reference is docs/learn.md, "The lesson model (lane LM)", "The player".
//!
//! The pure lesson lifecycle. [`Player`] walks a [`LessonBundle`]'s steps; the session (`enth-session::learn`, lane
//! SL) owns the live patch, the stash, the undo history per step, the job ids and the saved state, and acts on the
//! [`Effect`]s every mutating call returns. `&mut self`, with times from the shell: no clocks, threads or I/O.
//!
//! - [`start`](Player::start) loads step 0. A step load carries at most one [`Effect::LoadPatch`] (its start); a
//!   Recreate step carries [`Effect::StartRound`] instead, so a step load sends exactly one plan. Every step load
//!   carries [`Effect::View`] with the step's effective locks, and Listen and Choose steps [`Effect::Examples`].
//! - [`edited`](Player::edited) after every delivered change (with the session's revision): patch predicates
//!   re-run at once; a sound check waits until the patch has settled ([`SETTLE_MS`]) and [`tick`](Player::tick)
//!   asks for it ([`Effect::Check`]). [`check_done`](Player::check_done) refuses a result for any revision but the
//!   current one (`stale_result`); the session refuses a stale job id first.
//! - [`hint`](Player::hint) gives the authored hints, then asks for a measured one ([`Effect::HintJob`], answered
//!   by [`hint_done`](Player::hint_done)); [`show`](Player::show) gives the witness's next op ([`Effect::Apply`],
//!   one undo step through the session's edit path, so locks hold) or an ear question's answer. Both mark the step
//!   assisted.
//! - [`choose`](Player::choose) answers an ear question; [`round_ended`](Player::round_ended) reports a Recreate
//!   step's round.
//! - [`next`](Player::next) and [`back`](Player::back) move between steps; [`end`](Player::end) ends the lesson
//!   (Restore or Keep, as the game's rounds do).
//!
//! A step that passes stays passed (progress never shrinks), and [`Effect::Record`] reports it at once, so a closed
//! editor loses nothing.

use std::sync::Arc;

use enth_challenge::{Challenge, PhraseId};
use enth_patch::{Op, ParamAddr, Patch, Target};

use crate::check::{Knob, SoundCheck, TouchLog};
use crate::drill::DrillStep;
use crate::lesson::{
    ControlCurve, Example, Lesson, Selector, Step, StepKind, StepView, Touch, touches,
};
use crate::pack::{LessonBundle, LessonKey, LessonRef, StepId};
use crate::progress::{LessonProgress, StepProgress};

/// How long the patch must stay unchanged before a sound check runs, in ms (as the game's loudness job waits).
pub const SETTLE_MS: u64 = 300;

/// Where the current step is. `Working` by default (the session's read model before a player is there).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StepPhase {
    /// The player is working on it (or reading, or listening).
    #[default]
    Working,
    /// A sound check for this revision is out; an edit sends the step back to `Working`.
    Checking {
        /// The revision the check is for.
        revision: u64,
    },
    /// It passed (sticky until the lesson ends).
    Passed,
}

/// Where an edit came from ([`Player::edited`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditSource {
    /// The player: an editor edit, undo, redo, or Show's op. Counts toward `Touched`.
    Player,
    /// Host automation, folded into the patch with locked parameters pinned back by the session.
    Host,
    /// A wholesale replacement: a step's start loaded, or a host state load.
    Load,
}

/// How a lesson ends ([`Player::end`]), as a game round ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndChoice {
    /// Bring back the player's own patch from before the lesson (the session's stash).
    Restore,
    /// Keep the lesson's patch.
    Keep,
}

/// How a Recreate step's round ended ([`Player::round_ended`]): at its first passing submit, on a reveal (`passed`
/// false, `assisted` true), or when the step is left with the round unpassed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RoundEnd {
    /// A submit passed.
    pub passed: bool,
    /// The round used help (a hint, a reveal, the live meter, dimming).
    pub assisted: bool,
    /// The best sound % of its submits, rounded down.
    pub best_sound_pct: Option<u8>,
}

/// A sound check for the shell's worker to run (PLY's evaluation renders `patch` with each check's pins through its
/// phrase and tests the expectations). The session wraps it in a job with its own id.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckRequest {
    /// The revision it is for: hand it back with the verdict.
    pub revision: u64,
    /// The patch at that revision.
    pub patch: Box<Patch>,
    /// The sound checks (one for a `sound` check, several for an `all`).
    pub sounds: Vec<SoundCheck>,
}

/// One expectation's result.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemVerdict {
    /// The item as written.
    pub item: String,
    /// Whether it held.
    pub passed: bool,
    /// The signed margin in the feature's unit, when it has one.
    pub margin: Option<f64>,
}

/// What a sound check found ([`Player::check_done`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckVerdict {
    /// Every item of every sound check held.
    pub passed: bool,
    /// Per item, for the feedback line.
    pub items: Vec<ItemVerdict>,
}

/// A measured hint to compute (`enth_solver::hint`: a bounded move from `patch` over the failing items, within
/// `free`). Solver-native, so this crate never calls the solver; the session wraps it in a job.
#[derive(Clone, Debug, PartialEq)]
pub struct HintRequest {
    /// The revision it is for.
    pub revision: u64,
    /// The patch at that revision.
    pub patch: Box<Patch>,
    /// The phrase.
    pub phrase: PhraseId,
    /// The expectation items.
    pub expect: Vec<String>,
    /// The parameters the move may change (the step's editable, unlocked parameters; empty: any).
    pub free: Vec<ParamAddr>,
}

/// A measured hint, as the Learn view shows it ([`Player::hint_done`]): the session builds it from the solver's
/// answer.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredHint {
    /// The text, graded like the coach (aspect, module, parameter, range).
    pub text: String,
    /// What to point at.
    pub highlight: Vec<Selector>,
}

/// An example ready to render: its fields and its patch, resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Audition {
    /// The label.
    pub label: String,
    /// The patch.
    pub patch: Arc<Patch>,
    /// The phrase.
    pub phrase: PhraseId,
    /// Semitones added to the phrase.
    pub transpose: i8,
    /// Level-matched with the step's other examples.
    pub level_match: bool,
    /// The render seed, when not the scoring seed.
    pub seed: Option<u32>,
    /// A control curve.
    pub curve: Option<ControlCurve>,
}

/// A step's outcome for the progress file: the shell merges it with `Progress::record`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRecord {
    /// The lesson.
    pub lesson: LessonKey,
    /// The step.
    pub step: StepId,
    /// What it achieved.
    pub progress: StepProgress,
}

/// What the session does after a call. Every mutating call returns them in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Effect {
    /// Load this patch as the step's start (one plan, one undo boundary). Report it back with
    /// [`EditSource::Load`].
    LoadPatch(Arc<Patch>),
    /// Start a Recreate step's round (`RoundSource::Challenge`), which brings its own start.
    StartRound(Box<Challenge>),
    /// End the live round: its step was left.
    EndRound,
    /// Start a Drill step's drill (R4): the shell builds a `DrillSpec` from it ([`DrillStep::spec`]), plays the
    /// items, and reports the end with [`Player::drill_ended`].
    StartDrill(DrillStep),
    /// The step's view: layout, highlight, hide, editable (advisory) and the **effective locks** the session
    /// enforces (for a Recreate step, its challenge's).
    View(StepView),
    /// The examples or options of a Listen or Choose step, to render on demand.
    Examples(Arc<[Audition]>),
    /// Run this sound check.
    Check(CheckRequest),
    /// Compute this measured hint.
    HintJob(HintRequest),
    /// Show an authored hint.
    Hint(String),
    /// Show a measured hint.
    Measured(MeasuredHint),
    /// Apply this op through the edit path, as one undo step (Show).
    Apply(Op),
    /// The player picked an option.
    Chose {
        /// Which.
        option: u8,
        /// Whether it is the answer.
        correct: bool,
    },
    /// Show an ear question's answer.
    Reveal {
        /// The right option.
        answer: u8,
    },
    /// The step passed.
    Passed {
        /// Which step.
        step: u16,
        /// With help.
        assisted: bool,
    },
    /// Merge this into the progress file.
    Record(StepRecord),
    /// The last step was left forward: the lesson is complete.
    Finished,
    /// The lesson ended: restore the stash or keep the patch.
    End(EndChoice),
}

/// Why a call is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlayerError {
    /// A result for another revision than the current one, or for no outstanding request.
    Stale {
        /// The result's revision.
        revision: u64,
        /// The player's.
        current: u64,
    },
    /// The call does not apply to this step's kind (a hint on a Recreate step: the round's coach gives those).
    NotHere {
        /// The call.
        call: &'static str,
        /// The step's kind.
        kind: &'static str,
    },
    /// No such step or option.
    OutOfRange {
        /// What.
        what: &'static str,
        /// The index.
        index: usize,
    },
    /// The lesson has ended.
    Ended,
    /// Show found no op from the patch to the witness (the witness is unreachable by ops).
    NoWitnessPath,
}

impl PlayerError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            PlayerError::Stale { .. } => "stale_result",
            PlayerError::NotHere { .. } => "not_here",
            PlayerError::OutOfRange { .. } => "out_of_range",
            PlayerError::Ended => "lesson_ended",
            PlayerError::NoWitnessPath => "no_witness_path",
        }
    }

    /// What to do about it.
    pub fn hint(&self) -> String {
        match self {
            PlayerError::Stale { .. } => {
                "drop the result: the patch changed since it was asked for".to_owned()
            }
            PlayerError::NotHere { call, .. } if *call == "hint" || *call == "show" => {
                "a Recreate step's help comes from its round's coach and Reveal".to_owned()
            }
            PlayerError::NotHere { .. } => "this step does not take that action".to_owned(),
            PlayerError::OutOfRange { .. } => "use an index the lesson has".to_owned(),
            PlayerError::Ended => "start the lesson again".to_owned(),
            PlayerError::NoWitnessPath => {
                "the lesson's witness is broken: run `cargo xtask lesson-check`".to_owned()
            }
        }
    }
}

impl std::fmt::Display for PlayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayerError::Stale { revision, current } => {
                write!(
                    f,
                    "a result for revision {revision}, the patch is at {current}"
                )
            }
            PlayerError::NotHere { call, kind } => {
                write!(f, "{call} does not apply to a {kind} step")
            }
            PlayerError::OutOfRange { what, index } => write!(f, "no {what} {index}"),
            PlayerError::Ended => f.write_str("the lesson has ended"),
            PlayerError::NoWitnessPath => f.write_str("no op leads from the patch to the witness"),
        }
    }
}

impl std::error::Error for PlayerError {}

/// What the player remembers about a step it has visited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Visit {
    passed: bool,
    assisted: bool,
    wrong_picks: u8,
    best: Option<u8>,
}

/// The pure lesson player. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct Player {
    bundle: LessonBundle,
    step: usize,
    phase: StepPhase,
    visits: Vec<Visit>,
    /// The session's revision of the last patch the player saw, and that patch.
    revision: u64,
    patch: Option<Box<Patch>>,
    /// When the last edit arrived, for the settle delay.
    edited_at: Option<u64>,
    touched: TouchLog,
    hints_given: usize,
    hint_pending: Option<u64>,
    finished: bool,
    ended: bool,
}

impl Player {
    /// A player at step 0 of `bundle`, not started.
    pub fn new(bundle: LessonBundle) -> Player {
        let n = bundle.lesson.steps.len();
        Player {
            bundle,
            step: 0,
            phase: StepPhase::Working,
            visits: vec![Visit::default(); n],
            revision: 0,
            patch: None,
            edited_at: None,
            touched: TouchLog::default(),
            hints_given: 0,
            hint_pending: None,
            finished: false,
            ended: false,
        }
    }

    /// Start: load step 0 (its start patch or round, its view, its examples).
    pub fn start(&mut self) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        Ok(self.enter(0, true))
    }

    /// A player resumed at `step` after a reload: the session already holds the patch (and a Recreate step's
    /// round), so the effects carry the view and examples only, never a `LoadPatch` or `StartRound`. The session
    /// checks the ids against its saved [`LessonRef`] first and resumes at 0 when the content differs.
    pub fn resume(bundle: LessonBundle, step: u16) -> Result<(Player, Vec<Effect>), PlayerError> {
        let mut p = Player::new(bundle);
        let i = usize::from(step);
        if i >= p.visits.len() {
            return Err(PlayerError::OutOfRange {
                what: "step",
                index: i,
            });
        }
        let effects = p.enter(i, false);
        Ok((p, effects))
    }

    // --- read accessors ----------------------------------------------------------------------------------------

    /// The lesson.
    pub fn lesson(&self) -> &Lesson {
        &self.bundle.lesson
    }

    /// The bundle (patches and texts).
    pub fn bundle(&self) -> &LessonBundle {
        &self.bundle
    }

    /// The current step's index.
    pub fn step(&self) -> u16 {
        u16::try_from(self.step).unwrap_or(u16::MAX)
    }

    /// Every step.
    pub fn steps(&self) -> &[Step] {
        &self.bundle.lesson.steps
    }

    /// The current step.
    pub fn current(&self) -> &Step {
        &self.bundle.lesson.steps[self.step]
    }

    /// The current step's text.
    pub fn text(&self) -> Option<&str> {
        self.current()
            .text
            .as_ref()
            .and_then(|t| self.bundle.text(t))
    }

    /// Where the current step is.
    pub fn phase(&self) -> StepPhase {
        self.phase
    }

    /// Whether step `i` has passed.
    pub fn passed(&self, i: usize) -> bool {
        self.visits.get(i).is_some_and(|v| v.passed)
    }

    /// Whether the current step used help.
    pub fn assisted(&self) -> bool {
        self.visits[self.step].assisted
    }

    /// Whether the last step was left forward.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The reference the session saves, at the current step.
    pub fn lesson_ref(&self) -> LessonRef {
        self.bundle.lesson_ref(self.step())
    }

    /// How many steps the lesson has.
    pub fn step_count(&self) -> u16 {
        u16::try_from(self.visits.len()).unwrap_or(u16::MAX)
    }

    /// The current step's drill, when it is a Drill step. A step resumed after a reload sends no
    /// [`Effect::StartDrill`], so the shell starts its drill again from this.
    pub fn drill_step(&self) -> Option<DrillStep> {
        match &self.current().kind {
            StepKind::Drill(drill) => Some(drill.clone()),
            _ => None,
        }
    }

    /// The current step's round challenge, when it is a Recreate step. A step resumed after a reload sends no
    /// [`Effect::StartRound`], so the session prepares its saved round again from this.
    pub fn round_challenge(&self) -> Option<Challenge> {
        match &self.current().kind {
            StepKind::Recreate { challenge, .. } => Some(Challenge::clone(challenge)),
            _ => None,
        }
    }

    // --- the lifecycle -----------------------------------------------------------------------------------------

    /// A delivered change: the patch now at `revision` (the session's counter, rising), the ops that made it (empty
    /// for a load) and where they came from. Patch predicates re-run at once; a sound check waits for the patch to
    /// settle.
    pub fn edited(
        &mut self,
        now_ms: u64,
        revision: u64,
        patch: &Patch,
        ops: &[Op],
        source: EditSource,
    ) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        self.revision = revision;
        self.patch = Some(Box::new(patch.clone()));
        self.edited_at = Some(now_ms);
        if source == EditSource::Player {
            for op in ops {
                self.note_touch(patch, op);
            }
        }
        if let StepPhase::Checking { .. } = self.phase {
            self.phase = StepPhase::Working;
        }
        if self.phase != StepPhase::Working {
            return Ok(Vec::new());
        }
        let StepKind::Do { check, .. } = &self.current().kind else {
            return Ok(Vec::new());
        };
        if check.sounds().is_empty() && check.patch_holds(patch, &self.touched) {
            return Ok(self.pass(None));
        }
        Ok(Vec::new())
    }

    /// Time passes: once the patch has settled for [`SETTLE_MS`], a Do step whose patch predicates hold asks for
    /// its sound check ([`Effect::Check`]), once per revision.
    pub fn tick(&mut self, now_ms: u64) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        if self.phase != StepPhase::Working {
            return Ok(Vec::new());
        }
        let (Some(at), Some(patch)) = (self.edited_at, self.patch.as_ref()) else {
            return Ok(Vec::new());
        };
        if now_ms < at.saturating_add(SETTLE_MS) {
            return Ok(Vec::new());
        }
        let StepKind::Do { check, .. } = &self.current().kind else {
            return Ok(Vec::new());
        };
        let sounds = check.sounds();
        if sounds.is_empty() || !check.patch_holds(patch, &self.touched) {
            return Ok(Vec::new());
        }
        let request = CheckRequest {
            revision: self.revision,
            patch: patch.clone(),
            sounds,
        };
        self.phase = StepPhase::Checking {
            revision: self.revision,
        };
        Ok(vec![Effect::Check(request)])
    }

    /// A sound check's verdict for `revision`. Refused (`stale_result`) unless it is for the check now out.
    pub fn check_done(
        &mut self,
        revision: u64,
        verdict: &CheckVerdict,
    ) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        if self.phase != (StepPhase::Checking { revision }) {
            return Err(PlayerError::Stale {
                revision,
                current: self.revision,
            });
        }
        if verdict.passed {
            Ok(self.pass(None))
        } else {
            self.phase = StepPhase::Working;
            // Wait for the next edit before checking again.
            self.edited_at = None;
            Ok(Vec::new())
        }
    }

    /// The next hint: the step's authored hints in order, then (for a step with a sound check) a measured one
    /// ([`Effect::HintJob`]). Marks the step assisted. Recreate steps take their hints from the round's coach.
    pub fn hint(&mut self) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        let step = self.current();
        match &step.kind {
            StepKind::Do { .. } | StepKind::Choose { .. } => {}
            other => {
                return Err(PlayerError::NotHere {
                    call: "hint",
                    kind: other.name(),
                });
            }
        }
        if let Some(text) = step.hints.get(self.hints_given).cloned() {
            self.hints_given += 1;
            self.visits[self.step].assisted = true;
            return Ok(vec![Effect::Hint(text)]);
        }
        let (StepKind::Do { check, .. }, Some(patch)) = (&step.kind, self.patch.as_ref()) else {
            return Ok(Vec::new());
        };
        let Some(sound) = check.sounds().into_iter().next() else {
            return Ok(Vec::new());
        };
        if self.hint_pending == Some(self.revision) {
            return Ok(Vec::new());
        }
        let free = free_params(&effective_view(step), patch);
        let request = HintRequest {
            revision: self.revision,
            patch: Box::new(sound.pinned(patch)),
            phrase: sound.phrase,
            expect: sound.expect,
            free,
        };
        self.hint_pending = Some(self.revision);
        self.visits[self.step].assisted = true;
        Ok(vec![Effect::HintJob(request)])
    }

    /// A measured hint for `revision`. Refused (`stale_result`) unless it answers the request now out for the
    /// current revision.
    pub fn hint_done(
        &mut self,
        revision: u64,
        hint: MeasuredHint,
    ) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        if self.hint_pending != Some(revision) || revision != self.revision {
            return Err(PlayerError::Stale {
                revision,
                current: self.revision,
            });
        }
        self.hint_pending = None;
        Ok(vec![Effect::Measured(hint)])
    }

    /// Show: a Do step's witness's next op from `patch` ([`Effect::Apply`], one undo step; the session applies it
    /// through its edit path and reports it with [`EditSource::Player`]; the knob's value in `patch` is noted for
    /// `touched` first, so the op's jump counts as a turn across its travel): the first op whose every touch
    /// ([`touches`], the session's own table) the view allows, else the first that touches no lock. A witness every
    /// remaining op of which touches a lock is `no_witness_path` (a content bug `lesson-check` catches), never an op
    /// the session would refuse. An ear question's answer ([`Effect::Reveal`], and the step passes, assisted). Marks
    /// the step assisted.
    pub fn show(&mut self, patch: &Patch) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        let step = self.current().clone();
        match &step.kind {
            StepKind::Do { witness, .. } => {
                let target = self
                    .bundle
                    .patch(witness)
                    .ok_or(PlayerError::NoWitnessPath)?;
                let ops =
                    enth_patch::ops_between(patch, target).ok_or(PlayerError::NoWitnessPath)?;
                let view = &effective_view(&step);
                // The ops that change something a control shows (an unknown kind counts; metadata does not).
                let moves: Vec<(&Op, Option<Vec<Touch>>)> = ops
                    .iter()
                    .map(|op| (op, touches(patch, op)))
                    .filter(|(_, t)| t.as_ref().is_none_or(|t| !t.is_empty()))
                    .collect();
                if moves.is_empty() {
                    return Ok(Vec::new());
                }
                let allowed = |(_, t): &&(&Op, Option<Vec<Touch>>)| {
                    t.as_ref().is_some_and(|t| t.iter().all(|t| view.allows(t)))
                };
                let unlocked = |(_, t): &&(&Op, Option<Vec<Touch>>)| {
                    t.as_ref().is_some_and(|t| !t.iter().any(|t| view.locks(t)))
                };
                let (pick, _) = moves
                    .iter()
                    .find(allowed)
                    .or_else(|| moves.iter().find(unlocked))
                    .ok_or(PlayerError::NoWitnessPath)?;
                let pick = Op::clone(pick);
                // The touch spans from where the knob is now (lane LFU): the session reports the op's result with
                // `EditSource::Player`, so one jump of Show me covers the travel it moves, as a drag would.
                self.note_touch(patch, &pick);
                self.visits[self.step].assisted = true;
                Ok(vec![Effect::Apply(pick)])
            }
            StepKind::Choose { answer, .. } => {
                self.visits[self.step].assisted = true;
                let mut out = vec![Effect::Reveal { answer: *answer }];
                if self.phase != StepPhase::Passed {
                    out.extend(self.pass(None));
                }
                Ok(out)
            }
            other => Err(PlayerError::NotHere {
                call: "show",
                kind: other.name(),
            }),
        }
    }

    /// Answer the current ear question with `option`. The right one passes the step (clean only with no wrong
    /// pick and no help); a wrong one can be followed by another pick.
    pub fn choose(&mut self, option: u8) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        let StepKind::Choose { options, answer } = &self.current().kind else {
            return Err(PlayerError::NotHere {
                call: "choose",
                kind: self.current().kind.name(),
            });
        };
        if usize::from(option) >= options.len() {
            return Err(PlayerError::OutOfRange {
                what: "option",
                index: usize::from(option),
            });
        }
        let correct = option == *answer;
        let mut out = vec![Effect::Chose { option, correct }];
        if correct {
            if self.phase != StepPhase::Passed {
                out.extend(self.pass(None));
            }
        } else {
            let v = &mut self.visits[self.step];
            v.wrong_picks = v.wrong_picks.saturating_add(1);
        }
        Ok(out)
    }

    /// A Recreate step's round ended (or reached its first passing submit). A pass passes the step; help marks it
    /// assisted; the best sound % is kept.
    pub fn round_ended(&mut self, end: RoundEnd) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        if !matches!(self.current().kind, StepKind::Recreate { .. }) {
            return Err(PlayerError::NotHere {
                call: "round_ended",
                kind: self.current().kind.name(),
            });
        }
        let v = &mut self.visits[self.step];
        v.assisted |= end.assisted;
        v.best = v.best.max(end.best_sound_pct);
        if end.passed && self.phase != StepPhase::Passed {
            return Ok(self.pass(end.best_sound_pct));
        }
        Ok(Vec::new())
    }

    /// A Drill step's drill was played to its end (R4): the step passes **whatever the score**, since no content is
    /// gated on chords, intervals or note naming and a drill's score feeds the staircase and the boxes, never the
    /// lesson. The shell records the drill itself (`Progress::observe_drill`); the step's own record is `done`.
    pub fn drill_ended(&mut self) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        if !matches!(self.current().kind, StepKind::Drill(_)) {
            return Err(PlayerError::NotHere {
                call: "drill_ended",
                kind: self.current().kind.name(),
            });
        }
        if self.phase == StepPhase::Passed {
            return Ok(Vec::new());
        }
        Ok(self.pass(None))
    }

    /// Leave the step forward. A Read or Listen step passes on the way; an unpassed step is left as it is (skipped).
    /// From the last step: [`Effect::Finished`].
    #[allow(
        clippy::should_implement_trait,
        reason = "the lesson's Next button, agreed with lane SL; a Player is not an iterator"
    )]
    pub fn next(&mut self) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        let mut out = Vec::new();
        if matches!(
            self.current().kind,
            StepKind::Read | StepKind::Listen { .. }
        ) && self.phase != StepPhase::Passed
        {
            out.extend(self.pass(None));
        }
        out.extend(self.leave());
        if self.step + 1 < self.visits.len() {
            out.extend(self.enter(self.step + 1, true));
        } else {
            self.finished = true;
            out.push(Effect::Finished);
        }
        Ok(out)
    }

    /// Go back one step (it loads again; a passed step stays passed).
    pub fn back(&mut self) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        if self.step == 0 {
            return Err(PlayerError::OutOfRange {
                what: "step",
                index: 0,
            });
        }
        let mut out = self.leave();
        out.extend(self.enter(self.step - 1, true));
        Ok(out)
    }

    /// End the lesson: the session restores its stash or keeps the patch. Every later call is refused.
    pub fn end(&mut self, choice: EndChoice) -> Result<Vec<Effect>, PlayerError> {
        self.live()?;
        let mut out = self.leave();
        self.ended = true;
        out.push(Effect::End(choice));
        Ok(out)
    }

    // --- internals ---------------------------------------------------------------------------------------------

    fn live(&self) -> Result<(), PlayerError> {
        if self.ended {
            Err(PlayerError::Ended)
        } else {
            Ok(())
        }
    }

    /// Enter step `i`: its load effects (with or without the plan) and a fresh working state.
    fn enter(&mut self, i: usize, load: bool) -> Vec<Effect> {
        self.step = i;
        self.finished = false;
        self.phase = if self.visits[i].passed {
            StepPhase::Passed
        } else {
            StepPhase::Working
        };
        self.touched.clear();
        self.hints_given = 0;
        self.hint_pending = None;
        self.edited_at = None;
        let step = self.bundle.lesson.steps[i].clone();
        let mut out = Vec::new();
        let view = effective_view(&step);
        match &step.kind {
            StepKind::Recreate { challenge, .. } => {
                if load {
                    out.push(Effect::StartRound(challenge.clone()));
                }
            }
            StepKind::Drill(drill) => {
                if load {
                    out.push(Effect::StartDrill(drill.clone()));
                }
            }
            _ => {
                if load
                    && let Some(start) = step.view.start.as_ref().and_then(|r| self.bundle.patch(r))
                {
                    out.push(Effect::LoadPatch(Arc::clone(start)));
                }
            }
        }
        out.push(Effect::View(view));
        if let StepKind::Listen { examples: list } | StepKind::Choose { options: list, .. } =
            &step.kind
        {
            let auditions: Vec<Audition> = list.iter().filter_map(|e| self.audition(e)).collect();
            out.push(Effect::Examples(auditions.into()));
        }
        out
    }

    /// Leave the current step: a live round ends.
    fn leave(&mut self) -> Vec<Effect> {
        if matches!(self.current().kind, StepKind::Recreate { .. })
            && self.phase != StepPhase::Passed
        {
            vec![Effect::EndRound]
        } else {
            Vec::new()
        }
    }

    fn pass(&mut self, best: Option<u8>) -> Vec<Effect> {
        self.phase = StepPhase::Passed;
        let v = &mut self.visits[self.step];
        v.passed = true;
        v.best = v.best.max(best);
        let clean = !v.assisted && v.wrong_picks == 0;
        let best = v.best;
        let record = StepRecord {
            lesson: self.bundle.key(),
            step: self.current().id.clone(),
            progress: StepProgress {
                done: true,
                clean,
                best,
                extra: std::collections::BTreeMap::new(),
            },
        };
        vec![
            Effect::Passed {
                step: self.step(),
                assisted: !clean,
            },
            Effect::Record(record),
        ]
    }

    fn audition(&self, e: &Example) -> Option<Audition> {
        Some(Audition {
            label: e.label.clone(),
            patch: Arc::clone(self.bundle.patch(&e.patch)?),
            phrase: e.phrase,
            transpose: e.transpose,
            level_match: e.level_match,
            seed: e.seed,
            curve: e.curve.clone(),
        })
    }

    fn note_touch(&mut self, patch: &Patch, op: &Op) {
        let knob = match op {
            Op::SetParam { addr, .. } => Knob::Param(addr.clone()),
            Op::SetMacro { index, .. } => Knob::Macro(*index),
            _ => return,
        };
        if let (Some(spec), Some(value)) = (knob.spec(patch), knob.value(patch)) {
            self.touched.note(knob, spec, value);
        }
    }

    // --- resuming with the progress file (lane SKL, F2l; phase-3's open check "a resumed lesson forgets which
    // steps passed", F1l review should-fix 7) ---------------------------------------------------------------------

    /// Bring back what the per-user progress file knows about this lesson (the shell reads it and calls this right
    /// after [`resume`](Player::resume), before the session sees the player): each step whose id is `done` there is
    /// passed again (assisted unless it was ever `clean`, its best sound % kept), and the current step's phase is
    /// `Passed` if it is one. It never un-passes a step and emits nothing: a passed step stays passed, so after a
    /// reload a step passed earlier neither looks unpassed nor records again. Steps the lesson no longer has are
    /// ignored.
    pub fn restore(&mut self, progress: &LessonProgress) {
        for (i, step) in self.bundle.lesson.steps.iter().enumerate() {
            let Some(p) = progress.steps.get(&step.id) else {
                continue;
            };
            if !p.done {
                continue;
            }
            let v = &mut self.visits[i];
            if !v.passed {
                v.assisted |= !p.clean;
            }
            v.passed = true;
            v.best = v.best.max(p.best);
        }
        if self.visits[self.step].passed && !self.ended {
            self.phase = StepPhase::Passed;
        }
    }
}

/// A step's effective view: its own, and on a Recreate step its challenge's constraints as `locked` and `editable`
/// (F1l review M4): the session enforces `locked`, the editor and the measured hints read `editable`.
fn effective_view(step: &Step) -> StepView {
    let mut view = step.view.clone();
    if let StepKind::Recreate { challenge, .. } = &step.kind {
        view.locked.clone_from(&challenge.constraints.locked);
        view.editable.clone_from(&challenge.constraints.editable);
    }
    view
}

/// Whether the view allows setting `addr` in `patch`, by the session's table ([`touches`]).
fn allows_param(view: &StepView, patch: &Patch, addr: &ParamAddr) -> bool {
    let op = Op::SetParam {
        addr: addr.clone(),
        value: patch.value(addr).unwrap_or(0.0),
    };
    touches(patch, &op).is_some_and(|t| t.iter().all(|t| view.allows(t)))
}

/// The parameters a measured hint may move: those the view allows in `patch` (every one when nothing restricts).
fn free_params(view: &StepView, patch: &Patch) -> Vec<ParamAddr> {
    if view.editable.is_empty() && view.locked.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (_, m) in patch.modules() {
        for spec in m.kind.params() {
            if let Ok(a) = ParamAddr::new(Target::Module(m.id.clone()), spec.key)
                && allows_param(view, patch, &a)
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
                && allows_param(view, patch, &a)
            {
                out.push(a);
            }
        }
    }
    out
}

/// [`Player::restore`] (lane SKL, F2l).
#[cfg(test)]
mod restore_tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::pack::{ContentId, PackVersion, TextRef};
    use crate::progress::LessonProgress;

    fn bundle() -> LessonBundle {
        let lesson = Lesson::from_json(
            r#"{"schema":"enth.lesson/1","id":"three","title":"Three","tier":"basic","objective":"Read three steps.",
            "teaches":["svf.cutoff"],"steps":[{"id":"a","kind":"read","text":"t.md"},{"id":"b","kind":"read","text":"t.md"},
            {"id":"c","kind":"read","text":"t.md"}]}"#,
        )
        .unwrap();
        let mut texts = BTreeMap::new();
        texts.insert(TextRef::new("t.md").unwrap(), Arc::<str>::from("# Text"));
        LessonBundle {
            pack: "example".parse().unwrap(),
            version: PackVersion::new(0, 1, 0),
            content: ContentId([0; 8]),
            lesson: Arc::new(lesson),
            patches: BTreeMap::new(),
            texts,
        }
    }

    fn progress(steps: &[(&str, bool, bool)]) -> LessonProgress {
        let mut p = LessonProgress::default();
        for (id, done, clean) in steps {
            p.steps.insert(
                id.parse().unwrap(),
                StepProgress {
                    done: *done,
                    clean: *clean,
                    best: None,
                    extra: BTreeMap::new(),
                },
            );
        }
        p
    }

    #[test]
    fn a_resumed_step_passed_before_stays_passed_and_records_nothing_again() {
        let (mut p, _) = Player::resume(bundle(), 1).unwrap();
        assert_eq!(p.phase(), StepPhase::Working, "resume alone forgets");
        p.restore(&progress(&[
            ("a", true, true),
            ("b", true, false),
            ("gone", true, true),
        ]));
        assert!(p.passed(0) && p.passed(1) && !p.passed(2));
        assert_eq!(
            p.phase(),
            StepPhase::Passed,
            "the current step passed before the reload"
        );
        assert!(p.assisted(), "b was never clean");
        // Next leaves a passed step without passing (and recording) it again.
        let effects = p.next().unwrap();
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Record(_) | Effect::Passed { .. })),
            "{effects:?}"
        );
        assert_eq!(p.phase(), StepPhase::Working, "c was not passed");
    }

    #[test]
    fn restore_never_unpasses_and_ignores_steps_not_done() {
        let mut p = Player::new(bundle());
        let _ = p.start().unwrap();
        let _ = p.next().unwrap(); // a passes (a read step left forward)
        p.restore(&progress(&[("a", false, false), ("b", false, false)]));
        assert!(p.passed(0), "a step passed in this session stays passed");
        assert!(!p.passed(1));
        assert_eq!(p.phase(), StepPhase::Working);
    }
}

/// Show me on a `touched` step (lane LFU, LUI's note for PLY): one jump of Show me counts as a turn across the travel
/// it moves.
#[cfg(test)]
mod show_touch_tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::pack::{ContentId, PackVersion, PatchRef};

    fn bundle(min_span_pct: u8) -> LessonBundle {
        let lesson = Lesson::from_json(&format!(
            r#"{{"schema":"enth.lesson/1","id":"turn","title":"Turn","tier":"basic","objective":"Turn it.",
            "teaches":["master.level"],"steps":[{{"id":"turn","kind":"do","view":{{"start":"patches/start.patch.json"}},
            "check":{{"kind":"patch","predicates":["touched master.level >= {min_span_pct}%"]}},
            "witness":"patches/witness.patch.json"}}]}}"#
        ))
        .unwrap();
        let start = Patch::init();
        let addr: ParamAddr = "master.level".parse().unwrap();
        let spec = start.spec(&addr).unwrap();
        let from = start.value(&addr).unwrap();
        let mut witness = start.clone();
        // Half the travel, down (the starter sits near the top of the level's range).
        let to = spec.from_norm((spec.to_norm(from) - 0.5).max(0.0));
        Op::SetParam {
            addr: addr.clone(),
            value: to,
        }
        .apply(&mut witness)
        .unwrap();
        let mut patches = BTreeMap::new();
        patches.insert(
            PatchRef::new("patches/start.patch.json").unwrap(),
            Arc::new(start),
        );
        patches.insert(
            PatchRef::new("patches/witness.patch.json").unwrap(),
            Arc::new(witness),
        );
        LessonBundle {
            pack: "example".parse().unwrap(),
            version: PackVersion::new(0, 1, 0),
            content: ContentId([0; 8]),
            lesson: Arc::new(lesson),
            patches,
            texts: BTreeMap::new(),
        }
    }

    /// Play Show me as the session does: its op applied, then reported as the player's edit.
    fn show_once(p: &mut Player, patch: &mut Patch, revision: u64) -> Vec<Effect> {
        let fx = p.show(patch).unwrap();
        let Some(Effect::Apply(op)) = fx.first() else {
            panic!("Show me gives an op: {fx:?}");
        };
        op.apply(patch).unwrap();
        p.edited(
            10 * revision,
            revision,
            patch,
            std::slice::from_ref(op),
            EditSource::Player,
        )
        .unwrap()
    }

    #[test]
    fn one_show_me_covers_the_travel_it_moves() {
        let b = bundle(30);
        let mut patch =
            Patch::clone(&b.patches[&PatchRef::new("patches/start.patch.json").unwrap()]);
        let mut p = Player::new(b);
        let _ = p.start().unwrap();
        let _ = p.edited(0, 1, &patch, &[], EditSource::Load).unwrap();
        assert_eq!(p.phase(), StepPhase::Working, "the start fails");
        let fx = show_once(&mut p, &mut patch, 2);
        assert!(
            fx.iter().any(|e| matches!(e, Effect::Passed { .. })),
            "a 50 % jump passes a 30 % span: {fx:?}"
        );
        assert!(p.assisted(), "Show me helps");
    }

    #[test]
    fn a_jump_shorter_than_the_span_still_fails() {
        let b = bundle(60);
        let mut patch =
            Patch::clone(&b.patches[&PatchRef::new("patches/start.patch.json").unwrap()]);
        let mut p = Player::new(b);
        let _ = p.start().unwrap();
        let _ = p.edited(0, 1, &patch, &[], EditSource::Load).unwrap();
        let fx = show_once(&mut p, &mut patch, 2);
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::Passed { .. })),
            "{fx:?}"
        );
        assert_eq!(p.phase(), StepPhase::Working);
    }
}
