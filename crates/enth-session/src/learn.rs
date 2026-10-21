//! **Owner: lane SL** (the session's Learn API, freeze point F1l). Pre-declared by lane S0l (R2).
//!
//! The pure Learn binding (docs/session.md, "Learn"; docs/learn.md, "The lesson model" and "Progress and the saved
//! state"). Like `game`, it does no I/O, spawns no threads and reads no clocks: **time comes from the timestamps the
//! shell passes to [`Session::game_tick`]** (the round and the lesson share that clock), and background work runs as
//! [`LearnJob`]s the shell executes on its own worker and hands back ([`Session::learn_job_done`]). The audio thread
//! never touches any of it: a lesson reaches the engine only by replacing the patch, one plan per step load.
//!
//! # What the session does, and what the player does
//!
//! Lane LM's player ([`enth_learn::Player`], behind the [`LessonPlayer`] trait) owns the lesson: its steps, checks,
//! hints, phases and progress records. It is pure and answers each call with [`PlayerEffect`]s. The session binds it
//! to the live patch:
//!
//! | The session | How |
//! | --- | --- |
//! | **Stashes your patch** | [`LearnEdit::learn_start`] keeps your patch (as patch-file text) and your undo history; [`LearnEdit::learn_end`] with [`EndChoice::Restore`] brings both back, [`EndChoice::Keep`] keeps the lesson's patch. A new lesson while one is live keeps the original stash |
//! | **Loads each step** | A step load gives the step a fresh history (**undo never leaves the step**) and seats the step's start patch like a load: **one plan per step load**. A Recreate step's start comes from its round instead, so it is still one plan |
//! | **Enforces the locks** | [`Session::edit`] refuses a transaction in which any op touches what the step locks ([`SessionError::Locked`], with a hint), for every op kind, before anything changes: the editor, Show me and every shell get the rule; the overlay only draws it |
//! | **Runs the round** | A Recreate step starts a real game round ([`crate::game::RoundSource::Challenge`], origin `lesson`), which never stashes: the `learn` object holds the only stash |
//! | **Sends jobs, refuses stale results** | Sound checks, measured hints, the Listen examples and reopening the lesson after a reload are jobs. A result for a job the session no longer waits for, or one that measured an older revision of the patch, is [`LearnError::StaleJob`] and changes nothing |
//! | **Keeps the read model** | [`LearnEdit::lesson`] ([`LearnView`]) for the Learn view, rebuilt every frame |
//!
//! # The lock policy (for the F1l judge)
//!
//! - **Locked edits are refused, whole:** a transaction with one op touching a locked control changes nothing. What
//!   an op touches is [`touches`], lane CV2's one table (`enth_challenge::v2::touches`), which the challenge's own
//!   lock check and the player's Show use too, so a round never asks for an edit the session refuses (F1l review
//!   M1); a lock is a [`Selector`] (`filter1.cutoff`, `filter1.*`, `lfo1 -> filter1.cutoff`, `macro3`). The session
//!   enforces the step's `locked` list only; `editable` is the editor's and lane CLS's (advisory).
//! - **Host automation of a locked control folds** (the patch-wins rules hold; the host is never refused) **but
//!   never counts toward a check:** every check, the player's patch predicates, its sound checks and a lesson round's
//!   score, sees [`Session::checked_patch`], in which each locked control holds its value from the step load, and so
//!   does every macro whose turn the lock refuses (a macro routed into a locked parameter). A fold that leaves the
//!   checked patch as it was tells the player nothing, so automation of a locked control never keeps a check from
//!   settling (F1l review, should-fix 1).
//! - **Undo never leaves the step:** each step load starts a fresh history, and the player's own history waits in
//!   the stash.
//! - **A preset load asks Restore or Keep:** [`Session::load_preset`] is refused while a lesson is live
//!   ([`SessionError::LessonLive`]); the editor asks, ends the lesson, then loads.
//! - **One stash at a time:** a lesson does not start over a live game round ([`LearnError::RoundLive`]), and the
//!   editor's `game_start` is refused while a lesson is live, so the `enth` field never holds two stashes.
//!
//! # What is saved
//!
//! The `enth` field's optional `learn` object ([`LearnState`]) holds only the in-flight [`LessonRef`] (pack,
//! version, lesson, step and content prefix) and the stash, `{"lesson":{…},"stash":"…"}`, so a reload resumes the
//! lesson at its step and brings your own patch back. Per-user progress is not here: it is a file the shell owns
//! (`<Documents>/enth/Learn/progress.json`), fed by [`LearnRequest::Record`]. The object is absent while no lesson is
//! live, so a state without a lesson keeps its bytes, and it stays within `enth.plugin-state/1`. It is read
//! leniently: an object this build cannot decode **keeps its stash**, as `GameState::lenient` does for a round (the
//! stash is your patch, the one thing a lesson must never lose); the lesson restores broken and only ending it is
//! left.
//!
//! **The budget.** The field is 1 MiB. A game round reserves [`ROUND_BYTES`] (768 KiB, a 384 KiB stash included).
//! A lesson round never stashes, so it takes at most [`LESSON_ROUND_BYTES`] (384 KiB), and the `learn` object at
//! most [`LEARN_BYTES`] (the stash plus a bounded reference). Together they leave the rest of the field more than
//! 64 KiB (a compile-time check, and a test of the largest lesson with its round).
//!
//! # The seam with lanes LM and CV2
//!
//! Lane F1L swapped SL's stand-ins for the real types (the F1l review's swap checklist): this module re-exports
//! [`LessonRef`], [`StepView`], [`StepPhase`], [`EditSource`], [`RoundEnd`], [`PlayerError`] and [`PlayerEffect`]
//! (LM's `Effect`, `non_exhaustive`: an effect this build does not know is skipped), [`Audition`], [`StepRecord`],
//! [`CheckRequest`], [`CheckVerdict`], [`HintRequest`] and [`MeasuredHint`] from `enth_learn`, and [`Selector`],
//! [`Touch`] and [`touches`] from `enth_challenge::v2`. [`LessonPlayer`] stays the session's seam: LM's `Player`
//! implements it (each method delegates; `end` maps the game's [`EndChoice`] to LM's, which has the same two
//! variants), and a test or kittest fake may too. A method added later gets a default body.

use std::fmt;
use std::sync::Arc;

use enth_challenge::Challenge;
use enth_patch::history::History;
use enth_patch::{Builtin, Op, ParamAddr, Patch, RouteAddr, Target};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::game::{
    EndChoice, EndedBy, GameError, ROUND_BYTES, RoundId, RoundOrigin, RoundSource, STASH_BYTES,
};
use crate::model::{Session, SessionError};
use crate::state::{MAX_FIELD_BYTES, StateError, decode_patch};
pub use enth_solver::closest::{Closest, ClosestCfg, ClosestStop, Delta as ClosestDelta};

// ---------------------------------------------------------------------------------------------------------------
// The budget
// ---------------------------------------------------------------------------------------------------------------

/// The longest [`LessonRef`] the `learn` object saves, as JSON. Pack and lesson ids are short; a lesson whose
/// reference is longer is refused at its start ([`LearnError::TooLarge`]).
pub const LESSON_REF_BYTES: usize = 1024;

/// The `learn` object's keys and punctuation.
const LEARN_FIXED_BYTES: usize = 1024;

/// The whole `learn` object at its largest: the stash ([`STASH_BYTES`], escaped as saved), the reference and the
/// keys.
pub const LEARN_BYTES: usize = STASH_BYTES + LESSON_REF_BYTES + LEARN_FIXED_BYTES;

/// A lesson round at its largest: a round's budget without the stash, which a lesson round never takes.
pub const LESSON_ROUND_BYTES: usize = ROUND_BYTES - STASH_BYTES;

const _: () = assert!(LEARN_BYTES + LESSON_ROUND_BYTES + 64 * 1024 < MAX_FIELD_BYTES);

/// How deep effects may call back into the player (a step load tells the player the new patch, which may answer
/// with effects of its own). A player that never settles is cut off here, with an error, instead of looping.
const MAX_EFFECT_DEPTH: u8 = 8;

/// "How close am I?"'s render budget (lane LFU): lane CLS's default, `enth_solver::closest::DEFAULT_BUDGET`, which
/// passed 57 of 59 lesson-like jobs in a median of 40–75 renders (docs/dsp/match.md, "Closest").
pub const CLOSEST_BUDGET: usize = enth_solver::closest::DEFAULT_BUDGET;

/// "Look harder": about 1,000 renders with a patient search (lane CLS's long runs, `--patience 15 --tol 0 --budget
/// 1000`: one more pass and answers 13 % nearer on average, for about 3.4 times the renders).
pub const CLOSEST_HARDER_BUDGET: usize = 1_000;

/// The largest seed a challenge takes (`enth_challenge`'s `MAX_SEED`, 2⁵³ − 1): a sibling's seed stays below it.
const MAX_SIBLING_SEED: u64 = (1 << 53) - 1;

/// The search settings of a look: the button's default, or "look harder"'s patient one.
pub fn closest_cfg(harder: bool) -> ClosestCfg {
    if harder {
        ClosestCfg {
            tol: 0.0,
            patience: 15,
            ..ClosestCfg::default()
        }
    } else {
        ClosestCfg::default()
    }
}

/// How far one control moves in "How close am I?"'s answer (lane LFU): the Learn view's words ("a touch", "a little",
/// "a lot", "switch") and rings (Good for a control that stays, Near for a touch or a little, Bad for a lot or a
/// switch). Never a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MoveSize {
    /// It stays (a move under [`MOVE_STAY`] of its travel counts as staying: about one just-noticeable step).
    Stay,
    /// Under [`MOVE_TOUCH`] of its travel.
    Touch,
    /// Under [`MOVE_LITTLE`] of its travel.
    Little,
    /// More.
    Lot,
    /// A switch, a mode or a waveform changes.
    Switch,
}

/// A move under this share of a knob's travel stays put ([`MoveSize::Stay`]).
pub const MOVE_STAY: f64 = 0.02;
/// A move under this share of a knob's travel is a touch ([`MoveSize::Touch`]).
pub const MOVE_TOUCH: f64 = 0.08;
/// A move under this share of a knob's travel is a little ([`MoveSize::Little`]); more is a lot.
pub const MOVE_LITTLE: f64 = 0.25;

/// How far `delta` moves its control ([`MoveSize`]).
pub fn move_size(delta: &ClosestDelta) -> MoveSize {
    let travel = delta.norm.abs();
    if !delta.moved() {
        MoveSize::Stay
    } else if delta.categorical {
        MoveSize::Switch
    } else if travel < MOVE_STAY {
        MoveSize::Stay
    } else if travel < MOVE_TOUCH {
        MoveSize::Touch
    } else if travel < MOVE_LITTLE {
        MoveSize::Little
    } else {
        MoveSize::Lot
    }
}

/// The seed of the next sibling after `seed` (a redo): a fixed mix, so a lesson's redos are the same everywhere, never
/// the seed itself, and within a challenge's range.
pub fn sibling_seed(seed: u64) -> u64 {
    // splitmix64's finaliser on the seed and a constant ("SIBLING!").
    let mut z = seed ^ 0x5349_424c_494e_4721;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    let next = z & MAX_SIBLING_SEED;
    if next == seed {
        (next + 1) & MAX_SIBLING_SEED
    } else {
        next
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The seam with lanes LM and CV2
// ---------------------------------------------------------------------------------------------------------------

pub use enth_challenge::v2::{Selector, Touch, touches};
pub use enth_learn::player::{
    Audition, CheckRequest, CheckVerdict, HintRequest, MeasuredHint, StepRecord,
};
pub use enth_learn::{
    EditSource, Effect as PlayerEffect, LessonRef, PlayerError, RoundEnd, StepPhase, StepView,
};

/// The parameter addresses of a module or built-in in `patch` (none for a module it lacks).
fn target_params(patch: &Patch, target: &Target) -> Vec<ParamAddr> {
    let (specs, target) = match target {
        Target::Module(id) => match patch.find(id) {
            Some((_, m)) => (m.kind.params(), target),
            None => return Vec::new(),
        },
        Target::Voice => (Builtin::Voice.params(), target),
        Target::Master => (Builtin::Master.params(), target),
    };
    specs
        .iter()
        .filter_map(|spec| ParamAddr::new(target.clone(), spec.key).ok())
        .collect()
}

/// The lesson player, as the session drives it: lane LM's [`enth_learn::Player`] implements it (each method
/// delegates), and a test or kittest fake may too. Pure: `&mut self`, times from the shell, no I/O. Every mutating
/// call answers with effects, in order. Later methods join with default bodies, so a fake keeps compiling (the F1l
/// review's rule for the seam).
pub trait LessonPlayer: Send + fmt::Debug {
    /// The lesson and the current step (what the session saves).
    fn lesson_ref(&self) -> LessonRef;
    /// How many steps the lesson has.
    fn step_count(&self) -> u16;
    /// Where the current step is.
    fn phase(&self) -> StepPhase;
    /// The current step's round challenge, when it is a Recreate step: a step resumed after a reload brings no
    /// `StartRound`, so the session prepares its saved round again from this.
    fn round_challenge(&self) -> Option<Challenge>;
    /// Load step 0.
    fn start(&mut self) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// A delivered change: `patch` is the checked patch ([`Session::checked_patch`]) at `revision`.
    fn edited(
        &mut self,
        now_ms: u64,
        revision: u64,
        patch: &Patch,
        ops: &[Op],
        source: EditSource,
    ) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// Time passes: a sound check once the patch has settled ([`PlayerEffect::Check`]).
    fn tick(&mut self, now_ms: u64) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// A sound check's verdict for `revision` (the session refused stale ones first; the player checks again).
    fn check_done(
        &mut self,
        revision: u64,
        verdict: &CheckVerdict,
    ) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// The next hint: authored text ([`PlayerEffect::Hint`]), then a measured one ([`PlayerEffect::HintJob`]);
    /// nothing when none is left.
    fn hint(&mut self) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// A measured hint for `revision`.
    fn hint_done(
        &mut self,
        revision: u64,
        hint: MeasuredHint,
    ) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// Show: the witness's next op ([`PlayerEffect::Apply`]) or an ear question's answer.
    fn show(&mut self, patch: &Patch) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// Answer an ear question.
    fn choose(&mut self, option: u8) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// The step's round reached its first passing submit, or was revealed.
    fn round_ended(&mut self, end: RoundEnd) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// The next step (or Finished).
    fn next(&mut self) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// The previous step.
    fn back(&mut self) -> Result<Vec<PlayerEffect>, PlayerError>;
    /// End the lesson (the game's [`EndChoice`]; LM's has the same two variants).
    fn end(&mut self, choice: EndChoice) -> Result<Vec<PlayerEffect>, PlayerError>;
}

impl LessonPlayer for enth_learn::Player {
    fn lesson_ref(&self) -> LessonRef {
        enth_learn::Player::lesson_ref(self)
    }

    fn step_count(&self) -> u16 {
        enth_learn::Player::step_count(self)
    }

    fn phase(&self) -> StepPhase {
        enth_learn::Player::phase(self)
    }

    fn round_challenge(&self) -> Option<Challenge> {
        enth_learn::Player::round_challenge(self)
    }

    fn start(&mut self) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::start(self)
    }

    fn edited(
        &mut self,
        now_ms: u64,
        revision: u64,
        patch: &Patch,
        ops: &[Op],
        source: EditSource,
    ) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::edited(self, now_ms, revision, patch, ops, source)
    }

    fn tick(&mut self, now_ms: u64) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::tick(self, now_ms)
    }

    fn check_done(
        &mut self,
        revision: u64,
        verdict: &CheckVerdict,
    ) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::check_done(self, revision, verdict)
    }

    fn hint(&mut self) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::hint(self)
    }

    fn hint_done(
        &mut self,
        revision: u64,
        hint: MeasuredHint,
    ) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::hint_done(self, revision, hint)
    }

    fn show(&mut self, patch: &Patch) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::show(self, patch)
    }

    fn choose(&mut self, option: u8) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::choose(self, option)
    }

    fn round_ended(&mut self, end: RoundEnd) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::round_ended(self, end)
    }

    fn next(&mut self) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::next(self)
    }

    fn back(&mut self) -> Result<Vec<PlayerEffect>, PlayerError> {
        enth_learn::Player::back(self)
    }

    fn end(&mut self, choice: EndChoice) -> Result<Vec<PlayerEffect>, PlayerError> {
        let choice = match choice {
            EndChoice::Restore => enth_learn::EndChoice::Restore,
            EndChoice::Keep => enth_learn::EndChoice::Keep,
        };
        enth_learn::Player::end(self, choice)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The saved state
// ---------------------------------------------------------------------------------------------------------------

/// The Learn state saved in the `enth` field: the lesson in flight, if any, and the stash of your own patch,
/// `{"lesson":{"pack":…,"version":…,"lesson":…,"step":…,"content":…},"stash":"…"}`. The session keeps the field
/// `None` while no lesson is live, so a state without a lesson saves the same bytes as before the field existed.
/// Every field is optional with a default (state version 1), and later fields can join without touching these.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnState {
    /// The lesson and its step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lesson: Option<LessonRef>,
    /// Your own patch from before the lesson, as patch-file text (exactly what the `patch` field held, so a patch
    /// this build could not read survives too).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash: Option<String>,
}

impl LearnState {
    /// Nothing to save: every field is at its default.
    pub fn is_empty(&self) -> bool {
        *self == LearnState::default()
    }

    /// A lesson is live: a reference or a stash.
    pub fn is_live(&self) -> bool {
        self.lesson.is_some() || self.stash.is_some()
    }

    /// Read a saved `learn` object without ever failing: the state if it decodes; otherwise, when it holds a stash
    /// that is text, a lesson with that stash (and its reference if that part reads), which restores broken so
    /// ending it brings your patch back; otherwise nothing. S0l's open item, decided: **an unreadable `learn` object
    /// keeps its stash**, as `GameState::lenient` keeps a round's.
    pub fn lenient(value: &Value) -> Option<LearnState> {
        if let Ok(state) = LearnState::deserialize(value) {
            return Some(state);
        }
        let stash = value.get("stash")?.as_str()?.to_owned();
        let lesson = value
            .get("lesson")
            .and_then(|v| LessonRef::deserialize(v).ok());
        Some(LearnState {
            lesson,
            stash: Some(stash),
        })
    }
}

/// The `enth` field's `learn` reader ([`LearnState::lenient`]): never fails, so the rest of the field always loads
/// (the rule `game` follows, F1r review M3).
pub(crate) fn lenient_learn<'de, D>(deserializer: D) -> Result<Option<LearnState>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.as_ref().and_then(LearnState::lenient))
}

// ---------------------------------------------------------------------------------------------------------------
// Jobs, requests, errors and the read model
// ---------------------------------------------------------------------------------------------------------------

/// Names one Learn job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LearnJobId(pub u64);

/// Background work for the shell's worker: a pure function of its payload, so the worker never locks the session.
/// Hand the [`LearnResult`] back with [`Session::learn_job_done`] on the GUI thread (or the host main thread).
#[derive(Clone, Debug, PartialEq)]
pub struct LearnJob {
    /// Its id, to hand back with the result.
    pub id: LearnJobId,
    /// What to do.
    pub kind: LearnJobKind,
}

/// What a [`LearnJob`] does.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LearnJobKind {
    /// After a reload: find the lesson in the installed packs and resume its player at the saved step (LM's
    /// `Player::resume(bundle, step)`, which also gives the step's effects); answer [`LearnResult::Opened`], or
    /// [`LearnResult::Failed`] if the pack is gone (the lesson breaks; ending it still brings your patch back).
    Open {
        /// The saved reference.
        lesson: LessonRef,
    },
    /// Run a sound check (`enth_learn`'s evaluation: render the phrase, measure, compare). Answer
    /// [`LearnResult::Checked`].
    Check(CheckRequest),
    /// Compute a measured hint (`enth_solver::hint`). Answer [`LearnResult::Hinted`].
    Hint(HintRequest),
    /// Render the step's examples into the Target ▶ slot as a set of loops, one per example, and measure each
    /// one's loudness. Answer [`LearnResult::Examples`]. Drop the loops when [`ListenState::ready`] turns false.
    Examples {
        /// The examples, in order.
        examples: Arc<[Audition]>,
    },
    /// "How close am I?" (lane LFU): `enth_solver::closest` from the player's patch on the round the game worker
    /// holds, on its parallel evaluator, with progress and cancel. Answer [`LearnResult::Closest`].
    Closest(Box<ClosestRequest>),
}

impl LearnJobKind {
    /// The kind's name, for errors.
    pub fn name(&self) -> &'static str {
        match self {
            LearnJobKind::Open { .. } => "open",
            LearnJobKind::Check(_) => "check",
            LearnJobKind::Hint(_) => "hint",
            LearnJobKind::Examples { .. } => "examples",
            LearnJobKind::Closest(_) => "closest",
        }
    }
}

/// A "How close am I?" job (lane LFU): solver-native, `{round, patch, free, budget}` → `enth_solver::closest`
/// (docs/learn.md, "Where Learn lives"), so the worker composes nothing. The round's prepared target is the one the
/// game worker already holds from the round's prepare job (`round` and `challenge` name it, as a score job's do; a
/// worker that lost it prepares it again).
#[derive(Clone, Debug, PartialEq)]
pub struct ClosestRequest {
    /// The lesson round it measures against.
    pub round: RoundId,
    /// That round's challenge.
    pub challenge: Arc<Challenge>,
    /// The lesson's revision of the patch it starts from: a result for an older one is stale.
    pub revision: u64,
    /// The player's patch, as the round scores it ([`Session::checked_patch`]): where the search starts and what
    /// "closest" is measured from.
    pub patch: Patch,
    /// The parameters that may move: the round's editable set (else its drawn parameters), less what it locks.
    pub free: Vec<ParamAddr>,
    /// The most renders, the first included; hard ([`CLOSEST_BUDGET`], or [`CLOSEST_HARDER_BUDGET`] to look harder).
    pub budget: usize,
    /// The search's settings ([`closest_cfg`]).
    pub cfg: ClosestCfg,
}

impl ClosestRequest {
    /// The look is the harder one.
    pub fn harder(&self) -> bool {
        self.budget > CLOSEST_BUDGET
    }
}

/// How far a running "How close am I?" got (the worker reports it after every batch).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClosestProgress {
    /// Renders so far.
    pub renders: usize,
    /// The job's budget.
    pub budget: usize,
    /// It is the harder look.
    pub harder: bool,
}

/// "How close am I?" in the read model (lane LFU). The Learn view shows the button while `available`, progress and
/// Cancel while `running`, then the answer in words and rings; exact values only once the round is `revealed`.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a view model: independent facts the Learn view shows"
)]
pub struct ClosestView {
    /// The button applies: the step's round is live and its last scored submit did not pass (no reveal), and no
    /// search runs.
    pub available: bool,
    /// A search runs.
    pub running: Option<ClosestProgress>,
    /// The latest answer on this round (until the step, the round or a new look replaces it).
    pub answer: Option<Arc<Closest>>,
    /// The answer came from the harder look.
    pub harder: bool,
    /// The round was revealed: the answer's exact values may show (no peeking before).
    pub revealed: bool,
    /// "Try a new one": a sibling redo is due (after a reveal, the coach's last hint or a passing answer) and the
    /// round's challenge has a seed to vary.
    pub redo: bool,
}

/// A Learn job's answer.
#[derive(Debug)]
#[non_exhaustive]
pub enum LearnResult {
    /// For [`LearnJobKind::Open`]: the lesson's player at the saved step, and the effects its resume gave (the
    /// step's view and examples; never a start patch or a round, which the session already holds).
    Opened {
        /// The player.
        player: Box<dyn LessonPlayer>,
        /// What `Player::resume` returned beside it.
        effects: Vec<PlayerEffect>,
    },
    /// For [`LearnJobKind::Check`].
    Checked(CheckVerdict),
    /// For [`LearnJobKind::Hint`].
    Hinted(MeasuredHint),
    /// For [`LearnJobKind::Examples`]: each example's integrated loudness (LUFS; `None` if silent), in order.
    Examples {
        /// One per example.
        lufs: Vec<Option<f32>>,
    },
    /// For [`LearnJobKind::Closest`] (lane LFU): what the search found. A cancelled search is answered too (its
    /// `stop` says so); the session drops it.
    Closest(Box<Closest>),
    /// Any job that could not finish.
    Failed {
        /// Why, for the status line.
        message: String,
    },
}

impl LearnResult {
    /// The result's name, for errors.
    pub fn name(&self) -> &'static str {
        match self {
            LearnResult::Opened { .. } => "opened",
            LearnResult::Checked(_) => "checked",
            LearnResult::Hinted(_) => "hinted",
            LearnResult::Examples { .. } => "examples",
            LearnResult::Closest(_) => "closest",
            LearnResult::Failed { .. } => "failed",
        }
    }
}

/// Something the shell must do for the lesson, drained with [`Session::drain_learn_requests`] under the lock and
/// acted on **after unlocking** (like `ShellRequest` and `GameRequest`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LearnRequest {
    /// Run this job on the worker; hand the result back with [`Session::learn_job_done`].
    Run(LearnJob),
    /// The lesson no longer needs this job: drop it if it has not run (a late result is refused as stale).
    Cancel(LearnJobId),
    /// Merge this step outcome into the per-user progress file (re-read, `Progress::merge`, temporary file, rename).
    Record {
        /// The lesson (its current step).
        lesson: LessonRef,
        /// The outcome.
        record: StepRecord,
    },
    /// The Listen state changed: store it where `process()` reads it, and drop the example loops when `ready` is
    /// false.
    Listen(ListenState),
    /// The lesson changed what the `enth` field saves (a start, a step, an end): mark the project dirty, coalesced
    /// with `ShellRequest::PatchEdited`.
    Dirty,
    /// A played Recreate round ended (any origin: the game, a lesson, a Review item; lane REV, additive): apply its
    /// first submit's rings to the per-user progress file's review boxes with today's day (re-read,
    /// `Progress::observe`, temporary file, rename). See `crate::review`.
    Rings(enth_learn::review::RoundRings),
}

/// What the audio side plays of a Listen or Choose step's examples: they share the Target ▶ slot by time (a Listen
/// or Choose step never has a round), as a set of loops with the playing one picked here, so a flip between examples
/// is position-locked and level-matched like Target ▶ and Mine ▶.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListenState {
    /// The example playing, if any.
    pub example: Option<u8>,
    /// The loops are rendered.
    pub ready: bool,
    /// Each example's gain in dB (≤ 0): the louder level-matched examples are attenuated to the quietest, never the
    /// quieter boosted; an unmeasured or silent one, or one with `level_match` off, plays at 0 dB.
    pub gains_db: Vec<f32>,
}

/// What the lesson is waiting for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags the Learn view shows, not a state machine"
)]
pub struct LearnPending {
    /// The lesson is being found again after a reload.
    pub opening: bool,
    /// A sound check is out.
    pub checking: bool,
    /// A measured hint is being computed.
    pub hinting: bool,
    /// The examples are being rendered.
    pub examples: bool,
}

/// The editor's read model of the lesson, rebuilt every frame ([`LearnEdit::lesson`]). Build one with `Default` and
/// set fields (it is `non_exhaustive`): a kittest fake can.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a view model: independent facts the Learn view shows"
)]
pub struct LearnView {
    /// The lesson and its current step (the saved reference while the lesson is being found again; none when the
    /// saved one could not be read).
    pub lesson: Option<LessonRef>,
    /// How many steps (0 until the player is there).
    pub steps: u16,
    /// Where the step stands.
    pub phase: StepPhase,
    /// What the step highlights, hides and locks.
    pub view: StepView,
    /// The step's Recreate round is live: draw the Recreate view (`GameEdit::round`).
    pub round: bool,
    /// What the lesson waits for.
    pub pending: LearnPending,
    /// The latest hint, authored or measured.
    pub hint: Option<String>,
    /// The ear question's last pick, and whether it was right.
    pub chose: Option<(u8, bool)>,
    /// The ear question's answer, once shown.
    pub answer: Option<u8>,
    /// The examples' playback.
    pub listen: ListenState,
    /// The lesson is done; ending it (Restore or Keep) is what is left.
    pub finished: bool,
    /// The lesson could not be found again after a reload: only ending it is left, and Restore brings your patch
    /// back.
    pub broken: bool,
    /// The last thing that went wrong.
    pub error: Option<LearnError>,
    /// When the lesson next needs a frame without input (lane SKL, F2l; additive): the moment the patch will have
    /// settled after the last change, so the player's tick can ask for its sound check (`enth_learn::player::SETTLE_MS`
    /// after the change). `None` once that tick has run: the lesson asks for no frames of its own otherwise. The shell
    /// asks the editor for a repaint at the earlier of this and the round's `wake_at_ms`.
    pub wake_at_ms: Option<u64>,
    /// "How close am I?" on the step's round (lane LFU; additive).
    pub closest: ClosestView,
}

/// Why a Learn call failed. Every variant has a stable snake-case [`code`](LearnError::code) and a one-line
/// [`hint`](LearnError::hint).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LearnError {
    /// No lesson is live.
    NoLesson,
    /// A Recreate round of the game's own is live and holds a stash: end it first.
    RoundLive,
    /// The lesson is being found again after a reload.
    NotReady,
    /// The lesson could not be found again: only ending it is left.
    Broken,
    /// The lesson would not fit the saved state.
    TooLarge {
        /// What does not fit (`stash`, `lesson`).
        what: &'static str,
        /// Its size, as saved.
        bytes: usize,
        /// The most it may take.
        max: usize,
    },
    /// A result for a job the lesson no longer waits for, or one that measured an older patch: ignore it.
    StaleJob(LearnJobId),
    /// A result of the wrong kind for its job (a shell bug).
    WrongResult {
        /// The job's kind.
        job: &'static str,
        /// The result's kind.
        result: &'static str,
    },
    /// A job failed.
    JobFailed {
        /// Its kind.
        job: &'static str,
        /// The shell's message.
        message: String,
    },
    /// The shell reopened another lesson than the saved one.
    OtherLesson {
        /// The saved one (`pack/lesson`).
        expected: String,
        /// The reopened one.
        found: String,
    },
    /// The player refused the call (its own code and hint).
    Player(PlayerError),
    /// The player kept answering its own effects past the session's depth limit (a player bug).
    PlayerLoops,
    /// No hint is left.
    NoHint,
    /// Show me has nothing more to show.
    NothingToShow,
    /// No such example, or the examples are not rendered yet.
    NoExample(u8),
    /// Show me's op was refused by the edit path (a witness that touches a lock: a content bug `lesson-check`
    /// catches).
    Edit(SessionError),
    /// The step's round could not start.
    Round(GameError),
    /// "How close am I?" answers after a Submit that did not pass, on a lesson round with something free to move
    /// (lane LFU).
    ClosestNotNow,
    /// "Try a new one" follows a reveal, the coach's last hint or "How close am I?", on a round whose challenge has a
    /// seed (lane LFU).
    NoRedo,
}

impl LearnError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            LearnError::NoLesson => "no_lesson",
            LearnError::RoundLive => "round_live",
            LearnError::NotReady => "not_ready",
            LearnError::Broken => "lesson_broken",
            LearnError::TooLarge { .. } => "too_large",
            LearnError::StaleJob(_) => "stale_job",
            LearnError::WrongResult { .. } => "wrong_result",
            LearnError::JobFailed { .. } => "job_failed",
            LearnError::OtherLesson { .. } => "other_lesson",
            LearnError::Player(e) => e.code(),
            LearnError::PlayerLoops => "player_loops",
            LearnError::NoHint => "no_hint",
            LearnError::NothingToShow => "nothing_to_show",
            LearnError::NoExample(_) => "no_example",
            LearnError::Edit(e) => e.code(),
            LearnError::Round(e) => e.code(),
            LearnError::ClosestNotNow => "closest_not_now",
            LearnError::NoRedo => "no_redo",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            LearnError::NoLesson => "start a lesson first".to_owned(),
            LearnError::RoundLive => {
                "a Recreate round is in progress: end it first (Restore brings your own patch back)".to_owned()
            }
            LearnError::NotReady => "the lesson is being reopened: try again in a moment".to_owned(),
            LearnError::Broken => {
                "this lesson cannot continue (its pack is missing or changed): end it, and Restore brings your patch back"
                    .to_owned()
            }
            LearnError::TooLarge { what: "lesson", .. } => {
                "this lesson's pack or lesson id is too long to save: report it to the pack's author".to_owned()
            }
            LearnError::TooLarge { .. } => {
                "your patch is too large to keep during a lesson: save it as a preset, then start from Init".to_owned()
            }
            LearnError::StaleJob(_) => "nothing to do: the lesson no longer waits for this result".to_owned(),
            LearnError::WrongResult { .. } => {
                "report this: the shell answered a job with the wrong result".to_owned()
            }
            LearnError::JobFailed { .. } => "try again; report it if it keeps failing".to_owned(),
            LearnError::OtherLesson { .. } => {
                "the saved lesson was not found: end it (Restore brings your patch back)".to_owned()
            }
            LearnError::Player(e) => e.hint(),
            LearnError::PlayerLoops => {
                "report this: the lesson kept answering its own effects".to_owned()
            }
            LearnError::NoHint => "every hint is used: try Show me, or keep going".to_owned(),
            LearnError::NothingToShow => "the step is already where Show me would take it".to_owned(),
            LearnError::NoExample(_) => "wait for the examples to be ready, then pick one of them".to_owned(),
            LearnError::Edit(e) => e.hint(),
            LearnError::Round(e) => e.hint(),
            LearnError::ClosestNotNow => {
                "Submit first: How close am I? answers after a Submit that does not pass".to_owned()
            }
            LearnError::NoRedo => {
                "a new sound like it comes after Reveal, the last hint or How close am I?".to_owned()
            }
        }
    }
}

impl fmt::Display for LearnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LearnError::NoLesson => f.write_str("no lesson is live"),
            LearnError::RoundLive => f.write_str("a Recreate round is live"),
            LearnError::NotReady => f.write_str("the lesson is not ready"),
            LearnError::Broken => f.write_str("the lesson cannot continue"),
            LearnError::TooLarge { what, bytes, max } => {
                write!(
                    f,
                    "the {what} takes {bytes} bytes, over the {max} a lesson may save"
                )
            }
            LearnError::StaleJob(id) => write!(f, "learn job {} is not awaited", id.0),
            LearnError::WrongResult { job, result } => {
                write!(f, "a {job} job answered with a {result} result")
            }
            LearnError::JobFailed { job, message } => write!(f, "the {job} job failed: {message}"),
            LearnError::OtherLesson { expected, found } => {
                write!(f, "reopened {found}, not the saved {expected}")
            }
            LearnError::Player(e) => write!(f, "the lesson refused: {e}"),
            LearnError::PlayerLoops => f.write_str("the lesson kept answering its own effects"),
            LearnError::NoHint => f.write_str("no hint left"),
            LearnError::NothingToShow => f.write_str("nothing to show"),
            LearnError::NoExample(i) => write!(f, "no example {i}"),
            LearnError::Edit(e) => write!(f, "Show me's edit was refused: {e}"),
            LearnError::Round(e) => write!(f, "the step's round did not start: {e}"),
            LearnError::ClosestNotNow => f.write_str("How close am I? is not available now"),
            LearnError::NoRedo => f.write_str("no new sound to try yet"),
        }
    }
}

impl std::error::Error for LearnError {}

/// The editor's Learn intents (lane LUI). [`Session`] implements it; a kittest fake may too. Object-safe. Every call
/// is made on the GUI thread under the session lock, at the end of the frame like `SessionEdit`'s.
pub trait LearnEdit {
    /// The lesson, if one is live.
    fn lesson(&self) -> Option<LearnView>;
    /// Start a lesson with its player (the shell builds it from the pack it read). Stashes your patch and undo
    /// history (a live lesson's stash carries over) and loads the first step. Refused over a live game round
    /// ([`LearnError::RoundLive`]) and for a patch too large to stash ([`LearnError::TooLarge`]); nothing changes
    /// then.
    fn learn_start(&mut self, player: Box<dyn LessonPlayer>) -> Result<(), LearnError>;
    /// The next step (or Finished).
    fn learn_next(&mut self) -> Result<(), LearnError>;
    /// The previous step.
    fn learn_back(&mut self) -> Result<(), LearnError>;
    /// The next hint: authored text at once, or a measured hint on the worker.
    fn learn_hint(&mut self) -> Result<(), LearnError>;
    /// Show me: the witness's next op, applied through the edit path as one undo step (locks hold).
    fn learn_show(&mut self) -> Result<(), LearnError>;
    /// Answer an ear question.
    fn learn_choose(&mut self, option: u8) -> Result<(), LearnError>;
    /// Play an example (`None`: stop).
    fn learn_listen(&mut self, example: Option<u8>) -> Result<(), LearnError>;
    /// End the lesson: [`EndChoice::Restore`] brings your patch and history back, [`EndChoice::Keep`] keeps the
    /// lesson's. The step's round ends first, logged.
    fn learn_end(&mut self, choice: EndChoice) -> Result<(), LearnError>;
    /// "How close am I?" (lane LFU): after a Submit that did not pass, search for the nearest passing patch from
    /// yours on the worker (`harder`: "look harder", about 1,000 renders). Its answer marks the round assisted, like a
    /// reveal, and a sibling redo follows. Refused otherwise ([`LearnError::ClosestNotNow`]).
    fn learn_closest(&mut self, harder: bool) -> Result<(), LearnError> {
        let _ = harder;
        Err(LearnError::ClosestNotNow)
    }
    /// Stop a running "How close am I?" (nothing when none runs).
    fn learn_closest_cancel(&mut self) -> Result<(), LearnError> {
        Ok(())
    }
    /// "Try a new one" (lane LFU): after a reveal, the coach's last hint or "How close am I?", the step's round ends
    /// and a sibling challenge (the same controls, a new seed) starts in its place. Refused otherwise
    /// ([`LearnError::NoRedo`]).
    fn learn_redo(&mut self) -> Result<(), LearnError> {
        Err(LearnError::NoRedo)
    }
    /// Play a Review item (lane REV, additive): its seeded sibling challenge as a round of origin `review`, which
    /// stashes your patch like a game round (see `crate::review`). Refused while a lesson is live. The default body
    /// does nothing, so a fake keeps compiling.
    fn review_start(&mut self, item: enth_learn::review::ReviewItem) -> Result<(), LearnError> {
        let _ = item;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The session's transient lesson state
// ---------------------------------------------------------------------------------------------------------------

/// A job out for the player, with the revision it measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InFlight {
    id: LearnJobId,
    revision: u64,
}

/// The jobs the lesson waits for, at most one per kind.
#[derive(Debug, Default)]
struct LearnJobs {
    next: u64,
    open: Option<LearnJobId>,
    check: Option<InFlight>,
    hint: Option<InFlight>,
    examples: Option<LearnJobId>,
    closest: Option<InFlight>,
}

/// The lesson's state that is not saved: the player, your stashed history, the step's view, locks and pinned values,
/// jobs, the examples and the outbox. Lives in [`Session`] beside the saved [`LearnState`] (in `InstanceState::learn`).
#[derive(Default)]
pub(crate) struct LearnLive {
    player: Option<Box<dyn LessonPlayer>>,
    /// Your own undo history, while the lesson runs in this process (not saved: no history is).
    stash_history: Option<History>,
    view: StepView,
    /// Each locked control at its step-load value: [`Session::checked_patch`] applies them.
    pins: Vec<Op>,
    /// Counts changes to the patch over the whole lesson; jobs measure one revision.
    revision: u64,
    /// The checked patch the player was last told about: a host fold that leaves it as it was is not a change.
    told: Option<Patch>,
    jobs: LearnJobs,
    examples: Option<Arc<[Audition]>>,
    lufs: Option<Vec<Option<f32>>>,
    listen: Option<u8>,
    /// The Listen state last pushed to the shell.
    listen_sent: Option<ListenState>,
    hint: Option<String>,
    chose: Option<(u8, bool)>,
    answer: Option<u8>,
    finished: bool,
    broken: bool,
    /// The step's effects resume the saved step after a reload: no start patch, the saved round resumed.
    resuming: bool,
    /// How deep effects are calling back into the player.
    depth: u8,
    /// When the patch will have settled after the last change the player was told about ([`LearnView::wake_at_ms`]).
    settle_at: Option<u64>,
    /// "How close am I?": the running search's progress, and the latest answer with the round it answered and
    /// whether it was the harder look (lane LFU).
    closest_progress: Option<ClosestProgress>,
    closest_answer: Option<(RoundId, Arc<Closest>, bool)>,
    error: Option<LearnError>,
    requests: Vec<LearnRequest>,
}

impl fmt::Debug for LearnLive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LearnLive")
            .field("player", &self.player.is_some())
            .field("locks", &self.view.locked.len())
            .field("revision", &self.revision)
            .field("jobs", &self.jobs)
            .field("requests", &self.requests.len())
            .finish_non_exhaustive()
    }
}

impl LearnLive {
    /// Forget the lesson (keeps the job counter, the revision, the Listen state last sent and the outbox).
    fn reset(&mut self) {
        let keep = LearnLive {
            jobs: LearnJobs {
                next: self.jobs.next,
                ..LearnJobs::default()
            },
            revision: self.revision,
            listen_sent: self.listen_sent.take(),
            requests: std::mem::take(&mut self.requests),
            ..LearnLive::default()
        };
        *self = keep;
    }

    fn dirty(&mut self) {
        if !self.requests.contains(&LearnRequest::Dirty) {
            self.requests.push(LearnRequest::Dirty);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The session's Learn entries
// ---------------------------------------------------------------------------------------------------------------

impl Session {
    /// The saved `learn` object, if a lesson is live (what the `enth` field would save; tests read it).
    pub fn learn_record(&self) -> Option<&LearnState> {
        self.instance.learn.as_ref().filter(|l| l.is_live())
    }

    /// The lesson's player, while it is there (the Learn view reads the step's text through it).
    pub fn learn_player(&self) -> Option<&dyn LessonPlayer> {
        self.learn.player.as_deref()
    }

    /// What the current step locks: [`Session::edit`] refuses any op touching one of these.
    pub fn locks(&self) -> &[Selector] {
        &self.learn.view.locked
    }

    /// The patch every check sees: the live patch with each control the step locks at its value from the step load,
    /// so host automation of a locked parameter (which folds into the patch) never counts toward a check. Outside a
    /// lesson, the patch itself.
    pub fn checked_patch(&self) -> Patch {
        let mut patch = self.patch.clone();
        for op in &self.learn.pins {
            let _ = enth_patch::apply(&mut patch, op);
        }
        patch
    }

    /// Take everything the shell must do for the lesson, oldest first: jobs to run or cancel, progress records, the
    /// Listen state, dirty marks. Drain it with [`Session::drain_requests`], under the lock, and act **after
    /// unlocking**.
    pub fn drain_learn_requests(&mut self) -> std::vec::Drain<'_, LearnRequest> {
        let listen = self.learn_listen_state();
        if self.learn.listen_sent.as_ref() != Some(&listen)
            && (self.learn.listen_sent.is_some() || listen != ListenState::default())
        {
            self.learn.listen_sent = Some(listen.clone());
            self.learn.requests.push(LearnRequest::Listen(listen));
        }
        self.learn.requests.drain(..)
    }

    /// Hand back a Learn job's result. Called on the GUI thread (or the host main thread), under the lock, **never on
    /// the worker**: an opened lesson resumes its step, which may prepare a round. A result for a job the lesson no
    /// longer waits for, or a check or hint that measured an older revision of the patch, is
    /// [`LearnError::StaleJob`] and changes nothing (the player asks again once the patch settles).
    pub fn learn_job_done(
        &mut self,
        id: LearnJobId,
        result: LearnResult,
    ) -> Result<(), LearnError> {
        let jobs = &self.learn.jobs;
        let job = if jobs.open == Some(id) {
            "open"
        } else if jobs.check.is_some_and(|f| f.id == id) {
            "check"
        } else if jobs.hint.is_some_and(|f| f.id == id) {
            "hint"
        } else if jobs.examples == Some(id) {
            "examples"
        } else if jobs.closest.is_some_and(|f| f.id == id) {
            "closest"
        } else {
            return Err(LearnError::StaleJob(id));
        };
        match (job, result) {
            (_, LearnResult::Failed { message }) => {
                self.learn_job_failed(job, message);
                Ok(())
            }
            ("open", LearnResult::Opened { player, effects }) => {
                self.learn.jobs.open = None;
                self.learn_opened(player, effects)
            }
            ("check", LearnResult::Checked(verdict)) => {
                let flight = self.learn_take_fresh(id, |jobs| &mut jobs.check)?;
                self.learn_drive(|p| p.check_done(flight.revision, &verdict), false)
            }
            ("hint", LearnResult::Hinted(hint)) => {
                let flight = self.learn_take_fresh(id, |jobs| &mut jobs.hint)?;
                self.learn_drive(|p| p.hint_done(flight.revision, hint), false)
            }
            ("examples", LearnResult::Examples { lufs }) => {
                let count = self.learn.examples.as_ref().map_or(0, |e| e.len());
                if lufs.len() != count {
                    return Err(LearnError::WrongResult {
                        job,
                        result: "examples of another count",
                    });
                }
                self.learn.jobs.examples = None;
                self.learn.lufs = Some(lufs);
                Ok(())
            }
            ("closest", LearnResult::Closest(found)) => {
                self.learn_take_fresh(id, |jobs| &mut jobs.closest)?;
                let harder = self.learn.closest_progress.take().is_some_and(|p| p.harder);
                self.learn_closest_answered(*found, harder);
                Ok(())
            }
            (job, other) => Err(LearnError::WrongResult {
                job,
                result: other.name(),
            }),
        }
    }

    /// A running job's progress (lane LFU): "How close am I?"'s renders so far, which the worker reports after every
    /// batch and the shell hands over before the frame. Anything but the awaited search is ignored.
    pub fn learn_job_progress(&mut self, id: LearnJobId, renders: usize) {
        if self.learn.jobs.closest.is_some_and(|f| f.id == id)
            && let Some(p) = self.learn.closest_progress.as_mut()
        {
            p.renders = renders.min(p.budget);
        }
    }

    // --- hooks the rest of the session calls (each one line at its call site) ----------------------------------

    /// Queue a request for the shell (lane REV's review rings use the lesson's outbox).
    pub(crate) fn learn_request(&mut self, request: LearnRequest) {
        self.learn.requests.push(request);
    }

    /// A lesson is live: the `learn` object holds a lesson or a stash.
    pub(crate) fn learn_live(&self) -> bool {
        self.instance
            .learn
            .as_ref()
            .is_some_and(LearnState::is_live)
    }

    /// The edit path's lock check, before anything changes ([`Session::edit`]): a transaction with any op touching a
    /// locked control is refused whole. Each op is judged against the patch as the ops before it leave it.
    pub(crate) fn learn_refuse(&mut self, ops: &[Op]) -> Result<(), SessionError> {
        let locks = &self.learn.view.locked;
        if locks.is_empty() {
            return Ok(());
        }
        let mut scratch = self.patch.clone();
        for op in ops {
            let refused = match touches(&scratch, op) {
                Some(touched) => touched.iter().find_map(|t| {
                    locks
                        .iter()
                        .find(|lock| lock.covers(t))
                        .map(|lock| (t.to_string(), lock.to_string()))
                }),
                // An op kind this build does not know: fail closed while anything is locked.
                None => Some((format!("a {} op", op.name()), locks[0].to_string())),
            };
            if let Some((touched, lock)) = refused {
                let err = SessionError::Locked {
                    op: op.name(),
                    touched,
                    lock,
                };
                self.last_error = Some(err.clone());
                return Err(err);
            }
            if enth_patch::apply(&mut scratch, op).is_err() {
                break; // the history reports the failing op itself
            }
        }
        Ok(())
    }

    /// The edit path delivered `ops` (an edit, undo, redo or Show's op). Called last in [`Session::deliver`].
    pub(crate) fn learn_note_edit(&mut self, ops: &[Op]) {
        self.learn_changed(ops, EditSource::Player);
    }

    /// A host fold wrote into the patch. Called at the end of [`Session::fold_host`].
    pub(crate) fn learn_note_fold(&mut self) {
        self.learn_changed(&[], EditSource::Host);
    }

    /// The host loaded a patch. Called at the end of [`Session::load_patch`].
    pub(crate) fn learn_note_load(&mut self) {
        self.learn_changed(&[], EditSource::Load);
    }

    /// The lesson's clock: the player may ask for a sound check once the patch has settled. Called from
    /// [`Session::game_tick`], once per editor frame.
    pub(crate) fn learn_tick(&mut self, now: u64) {
        if !self.learn.broken && self.learn.player.is_some() {
            let _ = self.learn_drive(|p| p.tick(now), false);
        }
        // The settle moment has passed: the lesson needs no frame of its own until the next change.
        if self.learn.settle_at.is_some_and(|at| now >= at) {
            self.learn.settle_at = None;
        }
    }

    /// A lesson round's start patch was seated (the step's one plan): pin the locks and show the player the patch.
    pub(crate) fn learn_round_seated(&mut self) {
        self.learn_pin();
        self.learn_changed(&[], EditSource::Load);
    }

    /// The lesson round first passed (`passed`) or was revealed, with its assists and best sound score in
    /// hundredths of a percent.
    pub(crate) fn learn_round_result(&mut self, passed: bool, assisted: bool, best: Option<u16>) {
        let end = RoundEnd {
            passed,
            assisted,
            best_sound_pct: best.map(|b| u8::try_from(b / 100).unwrap_or(u8::MAX)),
        };
        let _ = self.learn_drive(|p| p.round_ended(end), false);
    }

    /// The lesson round was revealed: the target is the patch now, and the step is given up.
    pub(crate) fn learn_round_revealed(&mut self, best: Option<u16>) {
        self.learn_changed(&[], EditSource::Load);
        self.learn_round_result(false, true, best);
    }

    /// A host state load set the `enth` field ([`Session::load_instance`]): `live` is the lesson that was live, and
    /// `self.instance.learn` holds what the host loaded. The same lesson at the same step with the same stash
    /// (REAPER's Undo within a step) goes on; anything else is the host's: the live lesson ends without a restore
    /// (the patch is the host's) and the loaded one, if any, is reopened. A lesson round with no lesson to resume it
    /// breaks.
    pub(crate) fn learn_reconcile_load(&mut self, live: Option<LearnState>) {
        let loaded = self.instance.learn.take();
        if let (Some(l), Some(n)) = (&live, &loaded)
            && l == n
            && l.is_live()
        {
            self.instance.learn = live;
            // The round the host restored may be another of this step's: prepare it again from the step.
            self.learn_resume_round();
            return;
        }
        if live.as_ref().is_some_and(LearnState::is_live) {
            self.learn_cancel_jobs(true);
            self.learn.reset();
        }
        match loaded {
            Some(state) if state.is_live() => self.learn_restore(state),
            other => self.instance.learn = other,
        }
        if !self.learn_live() {
            self.game_break_lesson_round();
        }
    }

    // --- the lesson's machinery ---------------------------------------------------------------------------------

    /// Ask the worker for a job.
    fn learn_job(&mut self, kind: LearnJobKind) -> LearnJobId {
        self.learn.jobs.next += 1;
        let id = LearnJobId(self.learn.jobs.next);
        self.learn
            .requests
            .push(LearnRequest::Run(LearnJob { id, kind }));
        id
    }

    /// Take the awaited check or hint `id` from its slot, refusing it as stale when it measured an older revision.
    fn learn_take_fresh(
        &mut self,
        id: LearnJobId,
        slot: impl FnOnce(&mut LearnJobs) -> &mut Option<InFlight>,
    ) -> Result<InFlight, LearnError> {
        let flight = slot(&mut self.learn.jobs)
            .take()
            .ok_or(LearnError::StaleJob(id))?;
        if flight.revision == self.learn.revision {
            Ok(flight)
        } else {
            Err(LearnError::StaleJob(id))
        }
    }

    /// Cancel the step's jobs (and reopening, with `all`).
    fn learn_cancel_jobs(&mut self, all: bool) {
        let jobs = &mut self.learn.jobs;
        let mut ids = vec![
            jobs.check.take().map(|f| f.id),
            jobs.hint.take().map(|f| f.id),
            jobs.examples.take(),
            jobs.closest.take().map(|f| f.id),
        ];
        if all {
            ids.push(jobs.open.take());
        }
        self.learn.closest_progress = None;
        self.learn
            .requests
            .extend(ids.into_iter().flatten().map(LearnRequest::Cancel));
    }

    fn learn_job_failed(&mut self, job: &'static str, message: String) {
        match job {
            "open" => {
                self.learn.jobs.open = None;
                self.learn.broken = true;
                self.game_break_lesson_round();
            }
            "check" => self.learn.jobs.check = None,
            "hint" => self.learn.jobs.hint = None,
            "closest" => {
                self.learn.jobs.closest = None;
                self.learn.closest_progress = None;
            }
            _ => self.learn.jobs.examples = None,
        }
        self.learn.error = Some(LearnError::JobFailed { job, message });
    }

    /// Ready for an intent: a live lesson, not broken, with its player.
    fn learn_ready(&self) -> Result<(), LearnError> {
        if !self.learn_live() {
            Err(LearnError::NoLesson)
        } else if self.learn.broken {
            Err(LearnError::Broken)
        } else if self.learn.player.is_none() {
            Err(LearnError::NotReady)
        } else {
            Ok(())
        }
    }

    /// Call the player and act on its effects. `step_load` forces a step load (the start); otherwise a step load is
    /// a change of the player's step.
    fn learn_drive<F>(&mut self, call: F, step_load: bool) -> Result<(), LearnError>
    where
        F: FnOnce(&mut dyn LessonPlayer) -> Result<Vec<PlayerEffect>, PlayerError>,
    {
        if self.learn.depth >= MAX_EFFECT_DEPTH {
            let err = LearnError::PlayerLoops;
            self.learn.error = Some(err.clone());
            return Err(err);
        }
        let Some(player) = self.learn.player.as_mut() else {
            return Err(LearnError::NoLesson);
        };
        let before = player.lesson_ref().step;
        let answer = call(player.as_mut());
        let after = player.lesson_ref().step;
        match answer {
            Ok(effects) => {
                self.learn.depth += 1;
                let applied = self.learn_effects(effects, step_load || before != after);
                self.learn.depth -= 1;
                applied
            }
            Err(e) => {
                let err = LearnError::Player(e);
                self.learn.error = Some(err.clone());
                Err(err)
            }
        }
    }

    /// The patch changed: count it and show the player the checked patch.
    ///
    /// Host automation that leaves the checked patch as it was (a locked control, or a macro routed into one, both
    /// pinned) is no change: the player is not told and the revision does not move, so automation of a locked control
    /// never keeps a sound check from settling (F1l review, should-fix 1).
    fn learn_changed(&mut self, ops: &[Op], source: EditSource) {
        if self.learn.player.is_none() || self.learn.broken {
            return;
        }
        let patch = self.checked_patch();
        if source == EditSource::Host && self.learn.told.as_ref() == Some(&patch) {
            return;
        }
        self.learn.revision += 1;
        // A search from the patch that was would answer for an older patch: stop it (lane LFU).
        self.learn_closest_stop();
        let revision = self.learn.revision;
        let now = self.game_now();
        self.learn.settle_at = Some(now.saturating_add(enth_learn::player::SETTLE_MS));
        self.learn.told = Some(patch.clone());
        let _ = self.learn_drive(|p| p.edited(now, revision, &patch, ops, source), false);
    }

    /// Act on the player's effects, in order. A step load ends the step that was (its round, its jobs, its
    /// history) and seats the new step's start: one plan, from the round when the step has one. Show's op goes
    /// through the edit path last, so its refusal is the call's error.
    fn learn_effects(
        &mut self,
        effects: Vec<PlayerEffect>,
        step_load: bool,
    ) -> Result<(), LearnError> {
        if step_load {
            self.learn_leave_step();
        }
        let mut load = None;
        let mut round = None;
        let mut apply = Vec::new();
        let mut viewed = false;
        for effect in effects {
            match effect {
                PlayerEffect::View(view) => {
                    self.learn.view = view;
                    viewed = true;
                }
                PlayerEffect::LoadPatch(patch) => load = Some(patch),
                PlayerEffect::StartRound(challenge) => round = Some(challenge),
                PlayerEffect::EndRound => {
                    if self.lesson_round_live() {
                        let _ = self.game_close(EndedBy::Keep);
                    }
                }
                PlayerEffect::Examples(examples) => self.learn_examples(examples),
                PlayerEffect::Check(request) => {
                    if let Some(old) = self.learn.jobs.check.take() {
                        self.learn.requests.push(LearnRequest::Cancel(old.id));
                    }
                    let revision = request.revision;
                    let id = self.learn_job(LearnJobKind::Check(request));
                    self.learn.jobs.check = Some(InFlight { id, revision });
                }
                PlayerEffect::HintJob(request) => {
                    if let Some(old) = self.learn.jobs.hint.take() {
                        self.learn.requests.push(LearnRequest::Cancel(old.id));
                    }
                    let revision = request.revision;
                    let id = self.learn_job(LearnJobKind::Hint(request));
                    self.learn.jobs.hint = Some(InFlight { id, revision });
                }
                PlayerEffect::Hint(text) => self.learn.hint = Some(text),
                PlayerEffect::Measured(hint) => self.learn.hint = Some(hint.text),
                PlayerEffect::Apply(op) => apply.push(op),
                PlayerEffect::Chose { option, correct } => {
                    self.learn.chose = Some((option, correct));
                }
                PlayerEffect::Reveal { answer } => self.learn.answer = Some(answer),
                PlayerEffect::Record(record) => {
                    if let Some(lesson) = self.learn_current_ref() {
                        self.learn
                            .requests
                            .push(LearnRequest::Record { lesson, record });
                    }
                }
                PlayerEffect::Finished => self.learn.finished = true,
                // The step's pass shows through the player's phase and reaches the progress file as a `Record`; the
                // lesson's end is `learn_end`'s, which restores or keeps. And a kind of effect this build does not
                // know (`Effect` is `non_exhaustive`): a later player's addition an older session can do without.
                _ => {}
            }
        }
        let resuming = std::mem::take(&mut self.learn.resuming);
        if resuming {
            // A step resumed after a reload: the `patch` field holds its patch (and the saved round its own).
            self.learn_resume_round();
            self.learn_pin();
            self.learn_changed(&[], EditSource::Load);
        } else if let Some(challenge) = round {
            // The round's start is the step's one plan.
            if let Err(e) = self.game_open(RoundSource::Challenge(challenge), RoundOrigin::Lesson) {
                self.learn.error = Some(LearnError::Round(e));
            }
        } else if let Some(patch) = load {
            self.learn_seat(Patch::clone(&patch));
        } else if step_load || viewed {
            self.learn_pin();
        }
        self.learn_save_ref();
        for op in apply {
            if let Err(e) = self.edit(vec![op], None) {
                let err = LearnError::Edit(e);
                self.learn.error = Some(err.clone());
                return Err(err);
            }
        }
        Ok(())
    }

    /// Stop a running "How close am I?" (lane LFU): the worker is told to cancel, and its late answer is stale.
    fn learn_closest_stop(&mut self) {
        if let Some(flight) = self.learn.jobs.closest.take() {
            self.learn.requests.push(LearnRequest::Cancel(flight.id));
        }
        self.learn.closest_progress = None;
    }

    /// "How close am I?" answered (lane LFU). A cancelled search shows nothing. An answer that found a passing patch
    /// (or found yours passing already) helped: the round is marked assisted, like a reveal, and so is the step (the
    /// player's `round_ended` with `assisted`); then a sibling redo is due. An answer that found no pass says so and
    /// helps no one: its moves are not a solution.
    ///
    /// A fresh answer (its revision is the patch's now) is for the live round: a new round's start and a step load
    /// both move the revision, and both stop the search.
    fn learn_closest_answered(&mut self, found: Closest, harder: bool) {
        if found.stop == ClosestStop::Cancelled {
            return;
        }
        let Some(round) = self
            .game_record()
            .filter(|r| r.origin == RoundOrigin::Lesson)
            .map(|r| r.id)
        else {
            return;
        };
        let helped = found.passed;
        self.learn.closest_answer = Some((round, Arc::new(found), harder));
        if helped {
            self.game_mark_closest(round);
            let best = self
                .game_record()
                .and_then(|r| r.submits.iter().map(|s| s.score.sound).max());
            self.learn_round_result(false, true, best);
        }
    }

    /// The parameters "How close am I?" may move on `challenge`'s round: its editable selectors' parameters in
    /// `patch` (else the round's drawn ones), less what it locks, each once (lane LCK's `--solve` takes the same set).
    fn closest_free(challenge: &Challenge, patch: &Patch, drawn: &[ParamAddr]) -> Vec<ParamAddr> {
        let c = &challenge.constraints;
        let named: Vec<ParamAddr> = if c.editable.is_empty() {
            drawn.to_vec()
        } else {
            c.editable
                .iter()
                .flat_map(|s| match s {
                    Selector::Param(a) => vec![a.clone()],
                    Selector::Module(target) => target_params(patch, target),
                    Selector::Route(_) | Selector::Macro(_) => Vec::new(),
                })
                .collect()
        };
        let mut free: Vec<ParamAddr> = Vec::new();
        for a in named {
            if patch.value(&a).is_some()
                && !c.is_locked(&Touch::Param(a.clone()))
                && !free.contains(&a)
            {
                free.push(a);
            }
        }
        free
    }

    /// The read model's "How close am I?" part.
    fn learn_closest_view(&self) -> ClosestView {
        let round = self
            .game_record()
            .filter(|r| r.origin == RoundOrigin::Lesson);
        let answer = match (&self.learn.closest_answer, round) {
            (Some((id, answer, harder)), Some(r)) if *id == r.id => {
                Some((Arc::clone(answer), *harder))
            }
            _ => None,
        };
        let running = self.learn.closest_progress;
        let redo = self
            .lesson_round_redo()
            .is_some_and(|(c, due)| due && challenge_seed(&c).is_some());
        ClosestView {
            available: running.is_none() && self.lesson_round_closest().is_some(),
            running,
            harder: answer.as_ref().is_some_and(|(_, h)| *h),
            answer: answer.map(|(a, _)| a),
            revealed: round.is_some_and(|r| r.revealed),
            redo,
        }
    }

    /// A restored lesson round waits for its challenge: the current step's, from the player.
    fn learn_resume_round(&mut self) {
        if !self.lesson_round_waiting() {
            return;
        }
        let challenge = self.learn.player.as_ref().and_then(|p| p.round_challenge());
        match challenge {
            Some(challenge) => {
                self.game_resume_lesson_round(challenge);
            }
            None => self.game_break_lesson_round(),
        }
    }

    /// End the step that was: its round (logged, origin `lesson`), its jobs, its examples, its hint and pick; a
    /// fresh history and no locks until the new step's view arrives. A resume keeps the restored round and the
    /// history.
    fn learn_leave_step(&mut self) {
        if !self.learn.resuming {
            if self.lesson_round_live() {
                let _ = self.game_close(EndedBy::Keep);
            }
            self.close_drag();
            self.history = History::with_cap(self.config.history_cap);
        }
        self.learn_cancel_jobs(false);
        self.learn.view = StepView::default();
        self.learn.pins.clear();
        self.learn.hint = None;
        self.learn.chose = None;
        self.learn.answer = None;
        self.learn.examples = None;
        self.learn.lufs = None;
        self.learn.listen = None;
        self.learn.error = None;
        self.learn.closest_answer = None;
    }

    /// Seat a step's start patch like a load: one plan, a fresh history, the locks pinned, the player told.
    fn learn_seat(&mut self, patch: Patch) {
        self.close_drag();
        self.history = History::with_cap(self.config.history_cap);
        self.seat_patch(patch, None);
        self.learn_pin();
        self.learn_changed(&[], EditSource::Load);
    }

    /// Pin each locked control at its value now ([`Session::checked_patch`]).
    fn learn_pin(&mut self) {
        fn pin(patch: &Patch, addr: &ParamAddr, pins: &mut Vec<Op>) {
            if let Some(value) = patch.value(addr) {
                pins.push(Op::SetParam {
                    addr: addr.clone(),
                    value,
                });
            }
        }
        let patch = &self.patch;
        let mut pins = Vec::new();
        for lock in &self.learn.view.locked {
            match lock {
                Selector::Param(addr) => pin(patch, addr, &mut pins),
                Selector::Module(target) => {
                    for addr in target_params(patch, target) {
                        pin(patch, &addr, &mut pins);
                    }
                    for r in patch.routes.iter().filter(|r| r.dst.target() == target) {
                        pins.push(Op::SetDepth {
                            route: RouteAddr {
                                src: r.src.clone(),
                                dst: r.dst.clone(),
                            },
                            depth: r.depth,
                        });
                    }
                }
                Selector::Route(route) => {
                    if let Some(r) = patch
                        .routes
                        .iter()
                        .find(|r| r.src == route.src && r.dst == route.dst)
                    {
                        pins.push(Op::SetDepth {
                            route: route.clone(),
                            depth: r.depth,
                        });
                    }
                }
                Selector::Macro(index) => pins.push(Op::SetMacro {
                    index: *index,
                    value: patch.macros[index.get()].value,
                }),
            }
        }
        // A macro whose turn the locks refuse (it is routed into a locked control: `touches` counts the route and its
        // destination) holds too, so host automation of it never counts either (F1l review, nit 9).
        let locked = &self.learn.view.locked;
        for (i, m) in patch.macros.iter().enumerate() {
            let Some(index) = enth_patch::MacroIndex::new(i) else {
                continue;
            };
            let op = Op::SetMacro {
                index,
                value: m.value,
            };
            let refused = touches(patch, &op)
                .is_some_and(|t| t.iter().any(|t| locked.iter().any(|l| l.covers(t))));
            if refused && !pins.contains(&op) {
                pins.push(op);
            }
        }
        self.learn.pins = pins;
    }

    /// The step's examples: render them (the loops replace any earlier set).
    fn learn_examples(&mut self, examples: Arc<[Audition]>) {
        if let Some(old) = self.learn.jobs.examples.take() {
            self.learn.requests.push(LearnRequest::Cancel(old));
        }
        self.learn.lufs = None;
        self.learn.listen = None;
        self.learn.examples = Some(Arc::clone(&examples));
        if !examples.is_empty() {
            let id = self.learn_job(LearnJobKind::Examples { examples });
            self.learn.jobs.examples = Some(id);
        }
    }

    /// The examples' playback, with the level-match gains.
    fn learn_listen_state(&self) -> ListenState {
        let (Some(examples), Some(lufs)) = (&self.learn.examples, &self.learn.lufs) else {
            return ListenState::default();
        };
        let measured = |example: &Audition, lufs: &Option<f32>| {
            lufs.filter(|l| example.level_match && l.is_finite())
        };
        let quietest = examples
            .iter()
            .zip(lufs)
            .filter_map(|(e, l)| measured(e, l))
            .reduce(f32::min);
        let gains_db = examples
            .iter()
            .zip(lufs)
            .map(|(e, l)| match (measured(e, l), quietest) {
                (Some(l), Some(q)) => (q - l).min(0.0),
                _ => 0.0,
            })
            .collect();
        ListenState {
            example: self.learn.listen,
            ready: true,
            gains_db,
        }
    }

    /// The lesson's current reference: the player's, or the saved one while it is being found again.
    fn learn_current_ref(&self) -> Option<LessonRef> {
        match &self.learn.player {
            Some(player) => Some(player.lesson_ref()),
            None => self.instance.learn.as_ref().and_then(|l| l.lesson.clone()),
        }
    }

    /// Save the player's current step in the `learn` object (dirty when it moved).
    fn learn_save_ref(&mut self) {
        let Some(player) = &self.learn.player else {
            return;
        };
        let current = player.lesson_ref();
        if let Some(state) = self.instance.learn.as_mut()
            && state.lesson.as_ref() != Some(&current)
        {
            state.lesson = Some(current);
            self.learn.dirty();
        }
    }

    /// Restore a saved lesson: ask the shell to find it again. A lesson without a readable reference restores
    /// broken, with its stash.
    fn learn_restore(&mut self, state: LearnState) {
        self.learn.reset();
        let lesson = state.lesson.clone();
        self.instance.learn = Some(state);
        if let Some(lesson) = lesson {
            let id = self.learn_job(LearnJobKind::Open { lesson });
            self.learn.jobs.open = Some(id);
        } else {
            self.learn.broken = true;
            self.learn.error = Some(LearnError::Broken);
            self.game_break_lesson_round();
        }
    }

    /// The shell found the lesson again: check it is the saved one, then resume its step with the effects its
    /// resume gave (the view and examples), or, when the lesson changed (another content prefix), start it again at
    /// step 0.
    fn learn_opened(
        &mut self,
        player: Box<dyn LessonPlayer>,
        effects: Vec<PlayerEffect>,
    ) -> Result<(), LearnError> {
        let Some(saved) = self.instance.learn.as_ref().and_then(|l| l.lesson.clone()) else {
            return Err(LearnError::NoLesson);
        };
        let found = player.lesson_ref();
        if found.key() != saved.key() {
            let err = LearnError::OtherLesson {
                expected: saved.key().to_string(),
                found: found.key().to_string(),
            };
            self.learn.broken = true;
            self.learn.error = Some(err.clone());
            self.game_break_lesson_round();
            return Err(err);
        }
        self.learn.player = Some(player);
        self.learn.error = None;
        if found.content == saved.content && found.step == saved.step {
            self.learn.resuming = true;
            self.learn.depth += 1;
            let resumed = self.learn_effects(effects, true);
            self.learn.depth -= 1;
            resumed
        } else {
            // The lesson changed under its saved step: it starts again, and the round of the old step ends.
            self.learn_drive(|p| p.start(), true)
        }
    }
}

impl LearnEdit for Session {
    fn lesson(&self) -> Option<LearnView> {
        let state = self.learn_record()?;
        let player = self.learn.player.as_deref();
        Some(LearnView {
            lesson: self.learn_current_ref(),
            steps: player.map_or(0, LessonPlayer::step_count),
            phase: player.map_or_else(StepPhase::default, LessonPlayer::phase),
            view: self.learn.view.clone(),
            round: self.lesson_round_live(),
            pending: LearnPending {
                opening: self.learn.jobs.open.is_some(),
                checking: self.learn.jobs.check.is_some(),
                hinting: self.learn.jobs.hint.is_some(),
                examples: self.learn.jobs.examples.is_some(),
            },
            hint: self.learn.hint.clone(),
            chose: self.learn.chose,
            answer: self.learn.answer,
            listen: self.learn_listen_state(),
            finished: self.learn.finished,
            broken: self.learn.broken || state.lesson.is_none(),
            error: self.learn.error.clone(),
            wake_at_ms: self.learn.settle_at,
            closest: self.learn_closest_view(),
        })
    }

    fn learn_start(&mut self, player: Box<dyn LessonPlayer>) -> Result<(), LearnError> {
        // One stash at a time: a game round of its own holds one.
        if self.game_record().is_some() && !self.lesson_round_live() {
            return Err(LearnError::RoundLive);
        }
        let lesson = player.lesson_ref();
        let bytes = serde_json::to_string(&lesson).map_or(usize::MAX, |t| t.len());
        if bytes > LESSON_REF_BYTES {
            return Err(LearnError::TooLarge {
                what: "lesson",
                bytes,
                max: LESSON_REF_BYTES,
            });
        }
        let carried = self
            .instance
            .learn
            .as_ref()
            .is_some_and(|l| l.stash.is_some());
        let own = self.own_patch_bytes();
        if !carried && own > STASH_BYTES {
            return Err(LearnError::TooLarge {
                what: "stash",
                bytes: own,
                max: STASH_BYTES,
            });
        }
        // The live lesson's round (or one a lesson left behind) ends first.
        if self.lesson_round_live() {
            let _ = self.game_close(EndedBy::Keep);
        }
        let (stash, history) = if carried {
            // A new lesson while one is live: the original stash, your patch from before the first, carries over.
            self.learn_cancel_jobs(true);
            let stash = self.instance.learn.take().and_then(|l| l.stash);
            (stash, self.learn.stash_history.take())
        } else {
            self.close_drag();
            let text = self
                .unloaded_patch
                .clone()
                .unwrap_or_else(|| self.patch.to_json());
            let fresh = History::with_cap(self.config.history_cap);
            (
                Some(text),
                Some(std::mem::replace(&mut self.history, fresh)),
            )
        };
        self.learn.reset();
        self.unloaded_instance = None; // the user's own state now (a newer build's `enth` text is replaced)
        self.instance.learn = Some(LearnState {
            lesson: Some(lesson),
            stash,
        });
        self.learn.stash_history = history;
        self.learn.player = Some(player);
        self.learn.dirty();
        self.learn_drive(|p| p.start(), true)
    }

    fn learn_next(&mut self) -> Result<(), LearnError> {
        self.learn_ready()?;
        self.learn_drive(|p| p.next(), false)
    }

    fn learn_back(&mut self) -> Result<(), LearnError> {
        self.learn_ready()?;
        self.learn_drive(|p| p.back(), false)
    }

    fn learn_hint(&mut self) -> Result<(), LearnError> {
        self.learn_ready()?;
        let mut none_left = false;
        self.learn_drive(
            |p| {
                let effects = p.hint()?;
                none_left = effects.is_empty();
                Ok(effects)
            },
            false,
        )?;
        if none_left {
            Err(LearnError::NoHint)
        } else {
            Ok(())
        }
    }

    fn learn_show(&mut self) -> Result<(), LearnError> {
        self.learn_ready()?;
        let patch = self.patch.clone();
        let mut nothing = false;
        self.learn_drive(
            |p| {
                let effects = p.show(&patch)?;
                nothing = effects.is_empty();
                Ok(effects)
            },
            false,
        )?;
        if nothing {
            Err(LearnError::NothingToShow)
        } else {
            Ok(())
        }
    }

    fn learn_choose(&mut self, option: u8) -> Result<(), LearnError> {
        self.learn_ready()?;
        self.learn_drive(|p| p.choose(option), false)
    }

    fn learn_listen(&mut self, example: Option<u8>) -> Result<(), LearnError> {
        if !self.learn_live() {
            return Err(LearnError::NoLesson);
        }
        if let Some(i) = example {
            let count = self.learn.examples.as_ref().map_or(0, |e| e.len());
            if self.learn.lufs.is_none() || usize::from(i) >= count {
                return Err(LearnError::NoExample(i));
            }
        }
        self.learn.listen = example;
        Ok(())
    }

    fn review_start(&mut self, item: enth_learn::review::ReviewItem) -> Result<(), LearnError> {
        self.review_open(item) // lane REV: crate::review
    }

    fn learn_end(&mut self, choice: EndChoice) -> Result<(), LearnError> {
        if !self.learn_live() {
            return Err(LearnError::NoLesson);
        }
        // The step's round ends first, logged with the same choice; it never stashed, so it restores nothing.
        if self.lesson_round_live() {
            let why = match choice {
                EndChoice::Restore => EndedBy::Restore,
                EndChoice::Keep => EndedBy::Keep,
            };
            let _ = self.game_close(why);
        }
        // The player's last word: its records reach the progress file; the rest is the session's to do.
        if let Some(player) = self.learn.player.as_mut() {
            let lesson = player.lesson_ref();
            for effect in player.end(choice).unwrap_or_default() {
                if let PlayerEffect::Record(record) = effect {
                    self.learn.requests.push(LearnRequest::Record {
                        lesson: lesson.clone(),
                        record,
                    });
                }
            }
        }
        self.learn_cancel_jobs(true);
        let history = self.learn.stash_history.take();
        let stash = self.instance.learn.as_mut().and_then(|l| {
            l.lesson = None;
            l.stash.take()
        });
        if self
            .instance
            .learn
            .as_ref()
            .is_some_and(LearnState::is_empty)
        {
            self.instance.learn = None;
        }
        self.learn.reset();
        self.learn.dirty();
        if choice == EndChoice::Restore
            && let Some(text) = stash
        {
            let (patch, unloaded) = match decode_patch(&text) {
                Ok(loaded) => (loaded.patch, None),
                Err(e) => {
                    // The same fallback as a load: the init patch plays and a readable-elsewhere text is kept.
                    let keep = !matches!(e, StateError::TooLarge { .. });
                    self.last_error = Some(SessionError::State(e));
                    (Patch::init(), keep.then_some(text))
                }
            };
            self.close_drag();
            self.history = history.unwrap_or_else(|| History::with_cap(self.config.history_cap));
            self.seat_patch(patch, unloaded);
        }
        Ok(())
    }

    fn learn_closest(&mut self, harder: bool) -> Result<(), LearnError> {
        self.learn_ready()?;
        if self.learn.jobs.closest.is_some() {
            return Ok(()); // one search at a time: the running one answers
        }
        let Some((round, challenge, drawn)) = self.lesson_round_closest() else {
            return Err(LearnError::ClosestNotNow);
        };
        let patch = self.checked_patch();
        let free = Session::closest_free(&challenge, &patch, &drawn);
        if free.is_empty() {
            return Err(LearnError::ClosestNotNow);
        }
        let budget = if harder {
            CLOSEST_HARDER_BUDGET
        } else {
            CLOSEST_BUDGET
        };
        let revision = self.learn.revision;
        let id = self.learn_job(LearnJobKind::Closest(Box::new(ClosestRequest {
            round,
            challenge,
            revision,
            patch,
            free,
            budget,
            cfg: closest_cfg(harder),
        })));
        self.learn.jobs.closest = Some(InFlight { id, revision });
        self.learn.closest_progress = Some(ClosestProgress {
            renders: 0,
            budget,
            harder,
        });
        self.learn.closest_answer = None;
        self.learn.error = None;
        Ok(())
    }

    fn learn_closest_cancel(&mut self) -> Result<(), LearnError> {
        self.learn_closest_stop();
        Ok(())
    }

    fn learn_redo(&mut self) -> Result<(), LearnError> {
        self.learn_ready()?;
        let Some((challenge, true)) = self.lesson_round_redo() else {
            return Err(LearnError::NoRedo);
        };
        let seed = challenge_seed(&challenge).ok_or(LearnError::NoRedo)?;
        let next = sibling_seed(seed);
        let sibling = challenge.with_seed(next).map_err(|_| LearnError::NoRedo)?;
        // The search and its answer belong to the round that ends.
        self.learn_closest_stop();
        self.learn.closest_answer = None;
        if let Err(e) = self.game_open_sibling(sibling, next) {
            let err = LearnError::Round(e);
            self.learn.error = Some(err.clone());
            return Err(err);
        }
        self.learn.error = None;
        self.learn.dirty();
        Ok(())
    }
}

/// A challenge's seed, when it has one to vary (a seed or a draw target; a patch target has none).
fn challenge_seed(challenge: &Challenge) -> Option<u64> {
    challenge
        .spec()
        .map(|s| s.seed)
        .or_else(|| challenge.draw().map(|d| d.seed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::{MacroIndex, ModuleId};

    fn sel(text: &str) -> Selector {
        text.parse().unwrap()
    }

    fn addr(text: &str) -> ParamAddr {
        text.parse().unwrap()
    }

    #[test]
    fn selectors_read_and_print_the_lesson_grammar_strictly() {
        for text in [
            "filter1.cutoff",
            "voice.glide",
            "filter1.*",
            "voice.*",
            "master.*",
            "lfo1 -> filter1.cutoff",
            "macro3 -> osc1.pw",
            "macro3",
        ] {
            assert_eq!(sel(text).to_string(), text, "{text}");
        }
        assert_eq!(sel("macro8"), Selector::Macro(MacroIndex::new(7).unwrap()));
        assert!(matches!(
            sel("filter1.*"),
            Selector::Module(Target::Module(_))
        ));
        for bad in [
            "",
            "filter1",
            "macro9",
            "filter1.nope",
            "Bad.*",
            "lfo1 ->",
            " macro8 ",
            "lfo1->filter1.cutoff",
        ] {
            assert!(bad.parse::<Selector>().is_err(), "{bad:?}");
        }
        assert_eq!(
            "lfo1->filter1.cutoff".parse::<Selector>().unwrap_err(),
            "write 'lfo1->filter1.cutoff' as 'lfo1 -> filter1.cutoff'"
        );
    }

    #[test]
    fn a_module_selector_covers_its_parameters_itself_and_its_routes() {
        let module = sel("filter1.*");
        let filter1 = Target::Module(ModuleId::new("filter1").unwrap());
        assert!(module.covers(&Touch::Param(addr("filter1.cutoff"))));
        assert!(module.covers(&Touch::Module(filter1.clone())));
        assert!(module.covers(&Touch::Route("lfo1 -> filter1.cutoff".parse().unwrap())));
        assert!(sel("lfo1.*").covers(&Touch::Route("lfo1 -> filter1.cutoff".parse().unwrap())));
        assert!(!module.covers(&Touch::Param(addr("osc1.level"))));
        assert!(sel("voice.*").covers(&Touch::Param(addr("voice.glide"))));
        assert!(sel("voice.*").covers(&Touch::Module(Target::Voice)));
        assert!(!sel("voice.*").covers(&Touch::Param(addr("master.level"))));
        let param = sel("filter1.cutoff");
        assert!(param.covers(&Touch::Param(addr("filter1.cutoff"))));
        assert!(!param.covers(&Touch::Param(addr("filter1.resonance"))));
        assert!(!param.covers(&Touch::Module(filter1)));
        // A macro selector covers the knob only (lane CV2's rule); `touches` adds a macro's routes as their own
        // touches, so a route or parameter lock still holds against the knob.
        let m3 = MacroIndex::new(2).unwrap();
        assert!(sel("macro3").covers(&Touch::Macro(m3)));
        assert!(!sel("macro3").covers(&Touch::Route("macro3 -> osc1.pw".parse().unwrap())));
        assert!(!sel("macro3").covers(&Touch::Macro(MacroIndex::new(0).unwrap())));
        let route = sel("lfo1 -> filter1.cutoff");
        assert!(route.covers(&Touch::Route("lfo1 -> filter1.cutoff".parse().unwrap())));
        assert!(!route.covers(&Touch::Param(addr("filter1.cutoff"))));
    }

    #[test]
    fn an_unreadable_learn_object_keeps_its_stash() {
        let lesson = serde_json::json!({
            "pack": "basics", "version": "1.2.0", "lesson": "cutoff", "step": 2, "content": "0123456789abcdef"
        });
        let ok = serde_json::json!({"lesson": lesson, "stash": "{}"});
        let state = LearnState::lenient(&ok).unwrap();
        assert_eq!(state.lesson.as_ref().map(|l| l.step), Some(2));
        // A reference missing a field (LM's typed reference has no defaults) reads as no reference; the stash stays.
        let partial = serde_json::json!({"lesson": {"pack": "basics", "step": 2}, "stash": "{}"});
        assert_eq!(
            LearnState::lenient(&partial),
            Some(LearnState {
                lesson: None,
                stash: Some("{}".to_owned())
            })
        );
        let bad_ref = serde_json::json!({"lesson": {"step": "two"}, "stash": "{}"});
        assert_eq!(
            LearnState::lenient(&bad_ref),
            Some(LearnState {
                lesson: None,
                stash: Some("{}".to_owned())
            })
        );
        let bad_stash = serde_json::json!({"lesson": {"pack": "basics"}, "stash": 7});
        assert_eq!(LearnState::lenient(&bad_stash), None);
        assert!(LearnState::default().is_empty());
        assert_eq!(serde_json::to_string(&LearnState::default()).unwrap(), "{}");
    }

    #[test]
    fn the_learn_types_cross_threads() {
        fn send<T: Send + 'static>() {}
        send::<LearnJob>();
        send::<LearnResult>();
        send::<LearnRequest>();
        send::<LearnView>();
        send::<Session>();
    }
}
