//! **Owner: lane SE** (Session, freeze point F1e; the F1e judge reviews the API with the other R4 and R5 APIs).
//! Pre-declared by lane S0e (R4/R5, freeze point F0e); written by lane SE.
//!
//! The pure ear state (docs/ear.md, "The session and the Ear view"; docs/session.md, "Ear"). Like
//! [`game`](crate::game), [`learn`](crate::learn) and [`create`](crate::create) it does no I/O, spawns no threads and
//! reads no clocks: **time comes from timestamps the shell passes in**, and background work runs as jobs the shell
//! executes on its game worker and hands back. The audio thread never touches any of it: **an ear run never reaches
//! the engine at all** (no plan, no event, no host write; the shell plays the stimuli through the Target ▶ examples
//! slot, which is its own).
//!
//! - **A run is a set of questions** ([`EarPlan`]: `enth-music`'s `EarSpec`, the one ear spec, and the item it starts
//!   at, a burst's focus from `enth_challenge::ear::find_item`). [`EarEdit::ear_start`] asks the shell for them ([`EarRequest::Prepare`]: the frozen
//!   generator, then the renders, about 0.3 s for ten); the shell posts them back ([`EarPost::Ready`]), and the run
//!   asks the first. **One question is a round: asked, answered, judged.** [`EarEdit::ear_answer`] judges at once and
//!   hands the shell the result ([`EarRequest::Result`]: the skill, the level, right or wrong, the time), which the
//!   drill player turns into its staircase and Review's boxes and the progress file keeps; the question's key stays
//!   hidden until then, and [`EarEdit::ear_next`] moves on. After the answer the replay is open: every example,
//!   including the ones that give the answer away ([`EarExample::after_answer`]).
//! - **The seam is data, on the one ear spec.** The plan holds `EarSpec` itself (lane F1E, the F1e review's M3: SE's
//!   mirror of its integers is gone) and a start item (should-fix S3); a question arrives as labels and an integer key,
//!   and a result leaves as plain fields. The session takes no type from `enth-learn`'s drill player. Choice answers are judged here; value and tap
//!   answers join when `enth-challenge`'s `ear-value/1` and `taps/1` exist (the enums are `non_exhaustive`, so that is
//!   additive).
//! - **Nothing is saved.** An ear session is not saved in the project, like a create job: this module adds **no
//!   field to the `enth` state blob**, `enth.plugin-state/1` is untouched, and a host state load leaves a run alone.
//!   Thresholds and practised days live in the per-user progress file the shell owns (`drills`).
//! - **Stale results are refused**, harmlessly, as the other modules refuse theirs: a post for a run that was
//!   cleared or replaced, a second `Ready`, an answer for a question the run has moved past or already judged.
//!
//! After F2e the session's ear types change only additively (docs/phase-4.md, "Freezes"): the enums and the structs
//! the shell builds are `non_exhaustive`, with constructors.

use crate::model::Session;

/// The most questions one run holds (a game's session is 10 to 20; an Ear check's trials come in small runs).
pub const MAX_QUESTIONS: usize = 64;
/// The most options a question offers (Spot the change offers 2 to 6 knob names).
pub const MAX_OPTIONS: usize = 12;
/// The most clips a question renders (the Target ▶ examples slot holds at most 8 position-locked loops).
pub const MAX_EXAMPLES: usize = 8;
/// The largest seed of a plan: below 2^53, `EarSpec`'s own limit.
pub const MAX_SEED: u64 = enth_music::ear::MAX_SEED;

/// A run's id: every post carries it; a post for a run the session no longer holds is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EarRunId(pub u64);

pub use enth_challenge::model::EarSpec;

/// What a run is asked for (the F1e review's M3 and should-fix S3): **the one ear spec** (`enth-music`'s `EarSpec`,
/// as `enth-challenge` re-exports it: game, generator, level, seed, count) and **the item it starts at**. Question
/// `k` of the run is item `first + k` of the spec's keyed streams (`enth_challenge::ear::question::questions_at`), so
/// a burst on a skill starts where `enth_challenge::ear::find_item(&spec, skill, from)` found it; `first` is 0
/// otherwise. A drill item becomes a plan through `enth_learn::drill::Item::plan` (one documented mapping).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct EarPlan {
    /// The questions: the game, the generator, the level (the staircase's, chosen by the drill player), the seed (at
    /// most [`MAX_SEED`]) and how many (1 to [`MAX_QUESTIONS`]).
    pub spec: EarSpec,
    /// The item of the spec the run starts at.
    pub first: u32,
}

impl EarPlan {
    /// A plan of `spec`'s questions from item 0; [`EarPlan::check`] says whether it is in range.
    #[must_use]
    pub fn new(spec: EarSpec) -> Self {
        EarPlan { spec, first: 0 }
    }

    /// The same plan from item `first` (a burst's focus).
    #[must_use]
    pub fn starting_at(mut self, first: u32) -> Self {
        self.first = first;
        self
    }

    /// Whether the plan is in range. A game or level this build cannot play is the shell's prepare job's refusal
    /// (`EarPost::Failed`), not this check's.
    ///
    /// # Errors
    /// [`EarError::BadPlan`] naming the first field out of range.
    pub fn check(&self) -> Result<(), EarError> {
        if self.spec.seed > MAX_SEED {
            return Err(EarError::BadPlan("the seed must be below 2^53"));
        }
        if !(1..=MAX_QUESTIONS).contains(&usize::from(self.spec.count)) {
            return Err(EarError::BadPlan("count must be 1 to 64"));
        }
        if self.first.checked_add(u32::from(self.spec.count)).is_none() {
            return Err(EarError::BadPlan("the first item is too far"));
        }
        Ok(())
    }
}

/// A clip the shell rendered for a question, in the order of the question's examples (the Target ▶ examples slot).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EarExample {
    /// What the button says ("A", "B", "Example", "The answer").
    pub label: String,
    /// The clip gives the answer away (the right option's sound, B with the change): it plays only after the question
    /// is answered, in the replay ([`EarError::Hidden`] before).
    pub after_answer: bool,
}

impl EarExample {
    /// A clip that plays while the question is open.
    #[must_use]
    pub fn open(label: impl Into<String>) -> Self {
        EarExample {
            label: label.into(),
            after_answer: false,
        }
    }

    /// A clip that plays only in the replay.
    #[must_use]
    pub fn replay(label: impl Into<String>) -> Self {
        EarExample {
            label: label.into(),
            after_answer: true,
        }
    }
}

/// The right answer of a question. Integers, so it is the same on every platform and engine version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarKey {
    /// The index of the right option.
    Choice(u8),
}

/// What the player answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarAnswer {
    /// The index of the option picked.
    Choice(u8),
}

/// One question as the shell posts it ([`EarPost::Ready`]): the labels the generator gave, the clips it rendered and
/// the key. The shell builds it from `enth-music`'s question (labels, key) and `enth-challenge`'s rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EarQuestion {
    /// The skill the question trains or tests (`ear.interval.m3`, `svf.cutoff`): what Review's boxes and the progress
    /// file key on. Empty for none.
    pub skill: String,
    /// The options' labels, in the order shown (2 to [`MAX_OPTIONS`]).
    pub options: Vec<String>,
    /// The clips the shell rendered, in slot order (at most [`MAX_EXAMPLES`]).
    pub examples: Vec<EarExample>,
    /// The right answer; hidden from the read model until the question is answered.
    pub key: EarKey,
}

impl EarQuestion {
    /// A label-choice question: `options` shown in order, `key` the index of the right one.
    #[must_use]
    pub fn choice(skill: impl Into<String>, options: Vec<String>, key: u8) -> Self {
        EarQuestion {
            skill: skill.into(),
            options,
            examples: Vec::new(),
            key: EarKey::Choice(key),
        }
    }

    /// With the clips the shell rendered.
    #[must_use]
    pub fn with_examples(mut self, examples: Vec<EarExample>) -> Self {
        self.examples = examples;
        self
    }

    fn check(&self) -> Result<(), EarError> {
        if !(2..=MAX_OPTIONS).contains(&self.options.len()) {
            return Err(EarError::BadPost("a question needs 2 to 12 options"));
        }
        if self.examples.len() > MAX_EXAMPLES {
            return Err(EarError::BadPost("a question renders at most 8 clips"));
        }
        let EarKey::Choice(key) = self.key;
        if usize::from(key) >= self.options.len() {
            return Err(EarError::BadPost("the key is not one of the options"));
        }
        Ok(())
    }
}

/// What the shell posts back for a run ([`Session::ear_post`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarPost {
    /// The questions: the generator ran and the clips are rendered. There must be exactly the plan's count.
    Ready {
        /// The questions, in order.
        questions: Vec<EarQuestion>,
    },
    /// The run could not be prepared.
    Failed {
        /// A stable snake-case code.
        code: String,
        /// What happened.
        message: String,
        /// What to do about it.
        hint: String,
    },
}

/// A job for the shell's game worker.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EarPrepare {
    /// Its id (the post carries it back).
    pub id: EarRunId,
    /// What it is asked for: run the frozen generator, then render the stimuli at the host's rate.
    pub plan: EarPlan,
}

/// One judged answer, for the drill player and the progress file ([`EarRequest::Result`]). The session keeps no
/// staircase and no box: the shell feeds this to `enth-learn`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EarResult {
    /// The run.
    pub run: EarRunId,
    /// The question's number in the run (0 = the first).
    pub item: u32,
    /// The plan the run played (the game, the level the staircase chose).
    pub plan: EarPlan,
    /// The skill (empty for none).
    pub skill: String,
    /// The answer matched the key.
    pub correct: bool,
    /// Milliseconds from the question opening to the answer (the shell's clock).
    pub response_ms: u64,
}

/// The ear outbox the shell drains ([`Session::drain_ear_requests`]) and acts on after unlocking.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarRequest {
    /// Prepare a run on the game worker.
    Prepare(EarPrepare),
    /// The run is gone before its preparation finished: drop the job (a post that arrives anyway is stale).
    Cancel(EarRunId),
    /// Play clip `example` of question `item` from the top, through the Target ▶ examples slot.
    Play {
        /// The run.
        id: EarRunId,
        /// The question.
        item: u32,
        /// The clip (an index into the question's examples).
        example: usize,
    },
    /// Stop playing.
    Stop,
    /// One judged answer.
    Result(Box<EarResult>),
}

/// Where a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarPhase {
    /// Asked for: the shell generates and renders the questions.
    Preparing,
    /// A question is open and waits for the player's answer.
    Asking,
    /// The question is judged; its replay and its key are open, and the run waits for [`EarEdit::ear_next`].
    Answered,
    /// Every question is answered and the run moved past the last.
    Done,
    /// The run could not be prepared ([`EarView::error`]).
    Failed,
}

/// An error the shell posted, or a post the session refused to build a run from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarFailure {
    /// A stable snake-case code.
    pub code: String,
    /// What happened.
    pub message: String,
    /// What to do about it.
    pub hint: String,
}

/// A judged question, kept for the replay and the summary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EarItemResult {
    /// The question's number in the run.
    pub item: u32,
    /// Its skill.
    pub skill: String,
    /// What the player answered.
    pub answer: EarAnswer,
    /// The right answer.
    pub key: EarKey,
    /// The answer matched the key.
    pub correct: bool,
    /// Milliseconds from the question opening to the answer.
    pub response_ms: u64,
}

/// The open question, as the Ear view draws it. The key is `None` until the question is answered.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EarQuestionView {
    /// Its number in the run (0 = the first).
    pub item: u32,
    /// Its skill.
    pub skill: String,
    /// The options' labels.
    pub options: Vec<String>,
    /// The clips, with which of them wait for the answer.
    pub examples: Vec<EarExample>,
    /// What the player answered, once they have.
    pub chosen: Option<EarAnswer>,
    /// The right answer, **only once the question is answered** (the replay with the answer).
    pub key: Option<EarKey>,
    /// Whether the answer was right, once given.
    pub correct: Option<bool>,
}

/// The Ear view's read model ([`EarEdit::ear`]), rebuilt on every call.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EarView {
    /// The run.
    pub id: EarRunId,
    /// Where it stands.
    pub phase: EarPhase,
    /// What it was asked for (its level is the staircase's).
    pub plan: EarPlan,
    /// The questions in the run (the plan's count until they arrive).
    pub count: u32,
    /// The open question's number; `None` while preparing or failed, and once done.
    pub item: Option<u32>,
    /// The open question.
    pub question: Option<EarQuestionView>,
    /// Questions judged so far.
    pub answered: u32,
    /// Of those, the right ones.
    pub correct: u32,
    /// Every judged question, in order: the replay and the summary.
    pub results: Vec<EarItemResult>,
    /// Why it failed.
    pub error: Option<EarFailure>,
}

/// Why the session refused an ear intent or post.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarError {
    /// The plan is out of range.
    BadPlan(&'static str),
    /// No run (start one first).
    NoRun,
    /// The run is still being prepared, or failed: there is no question yet.
    NotReady,
    /// Every question is answered and the run is done.
    Finished,
    /// A post for a run the session no longer holds, or a post the run no longer waits for.
    Stale(EarRunId),
    /// The question is not the open one: the run moved on (or has not reached it).
    NotCurrent {
        /// The question asked about.
        item: u32,
    },
    /// The question was judged already: the first answer stands.
    AlreadyAnswered {
        /// The question.
        item: u32,
    },
    /// The question is open: answer it before moving on.
    NotAnswered,
    /// The answer is not one of the question's options.
    BadAnswer(&'static str),
    /// The question has no such clip.
    NoExample {
        /// The clip asked for.
        example: usize,
    },
    /// The clip gives the answer away and the question is still open.
    Hidden {
        /// The clip asked for.
        example: usize,
    },
    /// The shell's post does not fit the plan (a shell bug); the run fails.
    BadPost(&'static str),
}

impl EarError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            EarError::BadPlan(_) => "bad_ear_plan",
            EarError::NoRun => "no_ear_run",
            EarError::NotReady => "ear_not_ready",
            EarError::Finished => "ear_finished",
            EarError::Stale(_) => "stale_post",
            EarError::NotCurrent { .. } => "not_current",
            EarError::AlreadyAnswered { .. } => "already_answered",
            EarError::NotAnswered => "not_answered",
            EarError::BadAnswer(_) => "bad_answer",
            EarError::NoExample { .. } => "no_example",
            EarError::Hidden { .. } => "example_hidden",
            EarError::BadPost(_) => "wrong_result",
        }
    }

    /// What to do about it.
    #[must_use]
    pub fn hint(&self) -> String {
        match self {
            EarError::BadPlan(what) => format!("fix the plan: {what}"),
            EarError::NoRun => "start a run first".to_owned(),
            EarError::NotReady => "the questions are still being prepared: try again in a moment".to_owned(),
            EarError::Finished => "every question is answered: start another run".to_owned(),
            EarError::Stale(_) => "a result for a run that is gone: nothing to do".to_owned(),
            EarError::NotCurrent { .. } => "that question is not the open one: nothing to do".to_owned(),
            EarError::AlreadyAnswered { .. } => "the first answer stands: move on to the next question".to_owned(),
            EarError::NotAnswered => "answer the question first".to_owned(),
            EarError::BadAnswer(what) => format!("answer with one of the options: {what}"),
            EarError::NoExample { .. } => "this question has no such clip".to_owned(),
            EarError::Hidden { .. } => "this clip gives the answer away: answer first, then replay it".to_owned(),
            EarError::BadPost(_) => "report this: the shell answered an ear job with a result that does not fit its plan".to_owned(),
        }
    }
}

impl std::fmt::Display for EarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EarError::BadPlan(what) => write!(f, "bad ear plan: {what}"),
            EarError::NoRun => f.write_str("no ear run"),
            EarError::NotReady => f.write_str("the ear run has no question yet"),
            EarError::Finished => f.write_str("the ear run is done"),
            EarError::Stale(id) => write!(f, "a post for ear run {} which is gone", id.0),
            EarError::NotCurrent { item } => write!(f, "question {item} is not the open one"),
            EarError::AlreadyAnswered { item } => write!(f, "question {item} is already answered"),
            EarError::NotAnswered => f.write_str("the question is not answered yet"),
            EarError::BadAnswer(what) => write!(f, "bad answer: {what}"),
            EarError::NoExample { example } => write!(f, "no clip {example}"),
            EarError::Hidden { example } => write!(f, "clip {example} waits for the answer"),
            EarError::BadPost(what) => write!(f, "the posted questions do not fit: {what}"),
        }
    }
}

impl std::error::Error for EarError {}

/// The editor's ear intents and read model. [`Session`] implements it; a kittest fake can. Object-safe. Every call is
/// made on the GUI thread under the session lock, at the end of the frame like `SessionEdit`'s; `at_ms` is the shell's
/// monotonic time in milliseconds, the clock [`Session::game_tick`] gets (a tap's arrival stamp is one too).
pub trait EarEdit {
    /// The run, if there is one.
    fn ear(&self) -> Option<EarView>;
    /// Start a run: a [`EarRequest::Prepare`] goes out, and a run already held is replaced (its pending job is
    /// cancelled; the answers it judged were already handed to the shell).
    ///
    /// # Errors
    /// [`EarError::BadPlan`].
    fn ear_start(&mut self, plan: EarPlan) -> Result<EarRunId, EarError>;
    /// Answer the open question `item` at `at_ms`. It is judged at once against the key, a [`EarRequest::Result`] goes
    /// out, and the key and the replay open. The first answer stands.
    ///
    /// # Errors
    /// [`EarError::NoRun`], [`EarError::NotReady`], [`EarError::Finished`], [`EarError::NotCurrent`] for a question
    /// other than the open one, [`EarError::AlreadyAnswered`], [`EarError::BadAnswer`].
    fn ear_answer(
        &mut self,
        item: u32,
        answer: EarAnswer,
        at_ms: u64,
    ) -> Result<EarItemResult, EarError>;
    /// Move past the judged question: the next one opens at `at_ms`, or the run is done after the last.
    ///
    /// # Errors
    /// [`EarError::NoRun`], [`EarError::NotReady`], [`EarError::Finished`], [`EarError::NotAnswered`].
    fn ear_next(&mut self, at_ms: u64) -> Result<(), EarError>;
    /// Play clip `example` of the open question from the top. A clip that gives the answer away plays only after the
    /// answer.
    ///
    /// # Errors
    /// [`EarError::NoRun`], [`EarError::NotReady`], [`EarError::Finished`], [`EarError::NoExample`],
    /// [`EarError::Hidden`].
    fn ear_play(&mut self, example: usize) -> Result<(), EarError>;
    /// Stop playing.
    fn ear_stop(&mut self);
    /// Close the run: a preparing one is cancelled, and its later posts are stale.
    fn ear_clear(&mut self);
}

#[derive(Debug)]
struct RunLive {
    id: EarRunId,
    plan: EarPlan,
    phase: EarPhase,
    questions: Vec<EarQuestion>,
    results: Vec<EarItemResult>,
    /// The open (or last judged) question.
    cursor: usize,
    /// When the open question opened (the shell's clock).
    opened_ms: u64,
    /// The latest time the run saw (a timestamp older than it counts as it).
    last_ms: u64,
    playing: bool,
    error: Option<EarFailure>,
}

/// The ear state the session keeps between calls (never saved).
#[derive(Debug, Default)]
pub(crate) struct EarLive {
    next_id: u64,
    run: Option<RunLive>,
    requests: Vec<EarRequest>,
}

impl EarLive {
    fn drop_run(&mut self) {
        if let Some(run) = self.run.take() {
            if run.phase == EarPhase::Preparing {
                self.requests.push(EarRequest::Cancel(run.id));
            }
            if run.playing {
                self.requests.push(EarRequest::Stop);
            }
        }
    }
}

impl RunLive {
    fn stamp(&mut self, at_ms: u64) -> u64 {
        self.last_ms = self.last_ms.max(at_ms);
        self.last_ms
    }

    fn open(&self) -> Option<&EarQuestion> {
        matches!(self.phase, EarPhase::Asking | EarPhase::Answered)
            .then(|| self.questions.get(self.cursor))
            .flatten()
    }

    /// The run holds a question to act on, or says why not.
    fn require_question(&self) -> Result<&EarQuestion, EarError> {
        match self.phase {
            EarPhase::Preparing | EarPhase::Failed => Err(EarError::NotReady),
            EarPhase::Done => Err(EarError::Finished),
            EarPhase::Asking | EarPhase::Answered => self.open().ok_or(EarError::NotReady),
        }
    }
}

impl Session {
    /// The ear outbox: [`EarRequest`]s for the shell to act on after unlocking (wherever `drain_requests` is drained).
    pub fn drain_ear_requests(&mut self) -> std::vec::Drain<'_, EarRequest> {
        self.ear.requests.drain(..)
    }

    /// Hand back a post of the shell's for run `id` (under the lock, on the GUI or host main thread). The first
    /// question opens at the latest tick's time ([`Session::game_tick`]'s clock).
    ///
    /// # Errors
    /// [`EarError::Stale`] for a run the session no longer holds, or one that no longer waits for a post (nothing
    /// changes); [`EarError::BadPost`] for questions that do not fit the plan (the run fails).
    pub fn ear_post(&mut self, id: EarRunId, post: EarPost) -> Result<(), EarError> {
        let now = self.game_now();
        let Some(run) = self
            .ear
            .run
            .as_mut()
            .filter(|r| r.id == id && r.phase == EarPhase::Preparing)
        else {
            return Err(EarError::Stale(id));
        };
        match post {
            EarPost::Ready { questions } => {
                let fits = if questions.len() == usize::from(run.plan.spec.count) {
                    questions.iter().try_for_each(EarQuestion::check)
                } else {
                    Err(EarError::BadPost("the count differs from the plan's"))
                };
                if let Err(e) = fits {
                    run.phase = EarPhase::Failed;
                    run.error = Some(EarFailure {
                        code: e.code().to_owned(),
                        message: e.to_string(),
                        hint: e.hint(),
                    });
                    return Err(e);
                }
                run.questions = questions;
                run.phase = EarPhase::Asking;
                run.cursor = 0;
                run.opened_ms = run.stamp(now);
            }
            EarPost::Failed {
                code,
                message,
                hint,
            } => {
                run.phase = EarPhase::Failed;
                run.error = Some(EarFailure {
                    code,
                    message,
                    hint,
                });
            }
        }
        Ok(())
    }
}

impl EarEdit for Session {
    fn ear(&self) -> Option<EarView> {
        let run = self.ear.run.as_ref()?;
        let answered = run.results.len();
        let current = run.open().map(|q| {
            let judged = run
                .results
                .iter()
                .find(|r| r.item as usize == run.cursor)
                .filter(|_| run.phase == EarPhase::Answered);
            EarQuestionView {
                item: run.cursor as u32,
                skill: q.skill.clone(),
                options: q.options.clone(),
                examples: q.examples.clone(),
                chosen: judged.map(|r| r.answer),
                key: judged.map(|r| r.key),
                correct: judged.map(|r| r.correct),
            }
        });
        Some(EarView {
            id: run.id,
            phase: run.phase,
            plan: run.plan,
            count: if run.questions.is_empty() {
                u32::from(run.plan.spec.count)
            } else {
                run.questions.len() as u32
            },
            item: current.as_ref().map(|q| q.item),
            question: current,
            answered: answered as u32,
            correct: run.results.iter().filter(|r| r.correct).count() as u32,
            results: run.results.clone(),
            error: run.error.clone(),
        })
    }

    fn ear_start(&mut self, plan: EarPlan) -> Result<EarRunId, EarError> {
        plan.check()?;
        self.ear.drop_run();
        self.ear.next_id += 1;
        let id = EarRunId(self.ear.next_id);
        self.ear
            .requests
            .push(EarRequest::Prepare(EarPrepare { id, plan }));
        self.ear.run = Some(RunLive {
            id,
            plan,
            phase: EarPhase::Preparing,
            questions: Vec::new(),
            results: Vec::new(),
            cursor: 0,
            opened_ms: 0,
            last_ms: self.game_now(),
            playing: false,
            error: None,
        });
        Ok(id)
    }

    fn ear_answer(
        &mut self,
        item: u32,
        answer: EarAnswer,
        at_ms: u64,
    ) -> Result<EarItemResult, EarError> {
        let run = self.ear.run.as_mut().ok_or(EarError::NoRun)?;
        let (skill, key, options) = {
            let question = run.require_question()?;
            (question.skill.clone(), question.key, question.options.len())
        };
        if item as usize != run.cursor {
            return Err(EarError::NotCurrent { item });
        }
        if run.phase == EarPhase::Answered {
            return Err(EarError::AlreadyAnswered { item });
        }
        let EarAnswer::Choice(picked) = answer;
        if usize::from(picked) >= options {
            return Err(EarError::BadAnswer("the option does not exist"));
        }
        let EarKey::Choice(right) = key;
        let at = run.stamp(at_ms);
        let result = EarItemResult {
            item,
            skill: skill.clone(),
            answer,
            key,
            correct: picked == right,
            response_ms: at.saturating_sub(run.opened_ms),
        };
        run.results.push(result.clone());
        run.phase = EarPhase::Answered;
        let request = EarRequest::Result(Box::new(EarResult {
            run: run.id,
            item,
            plan: run.plan,
            skill,
            correct: result.correct,
            response_ms: result.response_ms,
        }));
        self.ear.requests.push(request);
        Ok(result)
    }

    fn ear_next(&mut self, at_ms: u64) -> Result<(), EarError> {
        let run = self.ear.run.as_mut().ok_or(EarError::NoRun)?;
        run.require_question()?;
        if run.phase != EarPhase::Answered {
            return Err(EarError::NotAnswered);
        }
        let at = run.stamp(at_ms);
        if run.cursor + 1 < run.questions.len() {
            run.cursor += 1;
            run.phase = EarPhase::Asking;
            run.opened_ms = at;
        } else {
            run.phase = EarPhase::Done;
        }
        if run.playing {
            run.playing = false;
            self.ear.requests.push(EarRequest::Stop);
        }
        Ok(())
    }

    fn ear_play(&mut self, example: usize) -> Result<(), EarError> {
        let run = self.ear.run.as_mut().ok_or(EarError::NoRun)?;
        let question = run.require_question()?;
        let clip = question
            .examples
            .get(example)
            .ok_or(EarError::NoExample { example })?;
        if clip.after_answer && run.phase != EarPhase::Answered {
            return Err(EarError::Hidden { example });
        }
        run.playing = true;
        let request = EarRequest::Play {
            id: run.id,
            item: run.cursor as u32,
            example,
        };
        self.ear.requests.push(request);
        Ok(())
    }

    fn ear_stop(&mut self) {
        if let Some(run) = self.ear.run.as_mut()
            && run.playing
        {
            run.playing = false;
            self.ear.requests.push(EarRequest::Stop);
        }
    }

    fn ear_clear(&mut self) {
        self.ear.drop_run();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn the_state_and_its_messages_move_between_threads() {
        assert_send::<EarRequest>();
        assert_send::<EarPost>();
        assert_send::<EarView>();
    }

    #[test]
    fn plans_out_of_range_are_refused() {
        assert_eq!(
            EarPlan::new(EarSpec {
                game: 1,
                generator: 1,
                level: 1,
                seed: 7,
                count: 10
            })
            .check(),
            Ok(())
        );
        assert_eq!(
            EarPlan::new(EarSpec {
                game: 1,
                generator: 1,
                level: 1,
                seed: MAX_SEED,
                count: 64
            })
            .check(),
            Ok(())
        );
        for bad in [
            EarPlan::new(EarSpec {
                game: 1,
                generator: 1,
                level: 1,
                seed: MAX_SEED + 1,
                count: 10,
            }),
            EarPlan::new(EarSpec {
                game: 1,
                generator: 1,
                level: 1,
                seed: u64::MAX,
                count: 10,
            }),
            EarPlan::new(EarSpec {
                game: 1,
                generator: 1,
                level: 1,
                seed: 7,
                count: 0,
            }),
            EarPlan::new(EarSpec {
                game: 1,
                generator: 1,
                level: 1,
                seed: 7,
                count: 65,
            }),
        ] {
            assert_eq!(bad.check().map_err(|e| e.code()), Err("bad_ear_plan"));
        }
        // A burst's plan starts at its item (should-fix S3), as far as the items go.
        let spec = EarSpec {
            game: 5,
            generator: 1,
            level: 1,
            seed: 7,
            count: 10,
        };
        let burst = EarPlan::new(spec).starting_at(41);
        assert_eq!((burst.spec, burst.first), (spec, 41));
        assert_eq!(burst.check(), Ok(()));
        assert!(EarPlan::new(spec).starting_at(u32::MAX).check().is_err());
    }

    #[test]
    fn a_question_must_fit_its_options() {
        let q = |options: usize, key: u8| {
            EarQuestion::choice("ear.test", vec!["x".to_owned(); options], key)
        };
        assert_eq!(q(2, 1).check(), Ok(()));
        assert_eq!(q(12, 11).check(), Ok(()));
        assert!(q(1, 0).check().is_err(), "one option is no question");
        assert!(q(13, 0).check().is_err());
        assert!(q(3, 3).check().is_err(), "the key is not an option");
        let too_many = q(2, 0).with_examples(vec![EarExample::open("A"); MAX_EXAMPLES + 1]);
        assert!(too_many.check().is_err());
    }
}
