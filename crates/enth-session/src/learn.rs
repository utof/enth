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
//! Lane LM's player (`enth_learn::player::Player`) owns the lesson: its steps, checks, hints, phases and progress
//! records. It is pure and answers each call with [`PlayerEffect`]s. The session binds it to the live patch:
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
//!   an op touches is [`touches`]; a lock is a [`Selector`] (lane CV2's text grammar: `filter1.cutoff`, `filter1.*`,
//!   `lfo1 -> filter1.cutoff`, `macro3`). The session enforces the step's `locked` list only; `editable` is the
//!   editor's and lane CLS's (advisory).
//! - **Host automation of a locked parameter folds** (the patch-wins rules hold; the host is never refused) **but
//!   never counts toward a check:** every check, the player's patch predicates, its sound checks and a lesson round's
//!   score, sees [`Session::checked_patch`], in which each locked control holds its value from the step load.
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
//! Until LM's `enth_learn::player` and CV2's `enth_challenge::v2::Selector` merge, this module carries **stand-ins**
//! mirroring the types on their branches (checked against `r2/lm` at `b9f511f` and `r2/cv2` at `3219eaf`; the
//! coordinator's F1l ruling): [`LessonRef`], [`Selector`], [`Touch`], [`StepView`], [`StepPhase`], [`EditSource`],
//! [`Audition`], [`StepRecord`], [`CheckRequest`], [`CheckVerdict`], [`HintRequest`], [`MeasuredHint`],
//! [`RoundEnd`], [`PlayerError`], [`PlayerEffect`] (LM's `Effect`), and the trait [`LessonPlayer`], whose methods
//! mirror LM's `Player` one to one plus `round_challenge` (a resumed step brings no `StartRound`). The swap at F1L
//! replaces each stand-in with a re-export (the stand-ins carry only the fields the session reads; LM's carry more)
//! and implements the trait for LM's `Player` (or holds it directly). The session's `EndChoice` is the game's; LM's
//! has the same two variants.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use enth_challenge::Challenge;
use enth_patch::history::History;
use enth_patch::{
    Builtin, MacroIndex, ModSource, ModuleId, ModuleKind, Op, ParamAddr, Patch, RouteAddr, Target,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::game::{
    EndChoice, EndedBy, GameError, ROUND_BYTES, RoundOrigin, RoundSource, STASH_BYTES,
};
use crate::model::{Session, SessionError};
use crate::state::{MAX_FIELD_BYTES, StateError, decode_patch};

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

// ---------------------------------------------------------------------------------------------------------------
// The seam with lanes LM and CV2 (stand-ins until they merge)
// ---------------------------------------------------------------------------------------------------------------

/// Which lesson, at which step. **Stand-in** for lane LM's `enth_learn::pack::LessonRef` (pack id, pack version,
/// lesson id, step index, and the first 8 bytes of the lesson file's SHA-256 as 16 hex digits), with the same JSON:
/// `{"pack":"basics","version":"1.2.0","lesson":"cutoff","step":1,"content":"0123456789abcdef"}`. LM's fields are
/// typed ids that read and write as these strings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct LessonRef {
    /// The pack's id (`basics`).
    pub pack: String,
    /// The pack's version when the lesson started (semver text).
    pub version: String,
    /// The lesson's id within the pack.
    pub lesson: String,
    /// The step, 0-based.
    pub step: u16,
    /// The first 16 hex digits of the lesson file's SHA-256: a reload of an updated lesson restarts it at step 0.
    pub content: String,
}

/// The same lesson (pack and lesson id; LM's `LessonRef::key`), whatever its step, version or content.
fn same_lesson(a: &LessonRef, b: &LessonRef) -> bool {
    a.pack == b.pack && a.lesson == b.lesson
}

/// What a lesson locks, highlights or hides. **Stand-in** for lane CV2's `enth_challenge::v2::Selector`: the same
/// variants, the same covering rules and the same strict text form, `filter1.cutoff`, `filter1.*` (also `voice.*`,
/// `master.*`), `lfo1 -> filter1.cutoff`, `macro3`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Selector {
    /// One parameter.
    Param(ParamAddr),
    /// A whole module, `voice` or `master`: its parameters, the module itself (enable, remove, move, label), the
    /// routes from it and the routes into its parameters.
    Module(Target),
    /// One route: adding it, removing it, its depth.
    Route(RouteAddr),
    /// One macro's knob.
    Macro(MacroIndex),
}

impl Selector {
    /// Whether this selector covers `touch` (lane CV2's `Selector::covers`).
    pub fn covers(&self, touch: &Touch) -> bool {
        match (self, touch) {
            (Selector::Param(a), Touch::Param(b)) => a == b,
            (Selector::Route(a), Touch::Route(b)) => a == b,
            (Selector::Macro(a), Touch::Macro(b)) => a == b,
            (Selector::Module(t), Touch::Module(u)) => t == u,
            (Selector::Module(t), Touch::Param(a)) => a.target() == t,
            (Selector::Module(t), Touch::Route(r)) => {
                r.dst.target() == t
                    || matches!((t, &r.src), (Target::Module(id), ModSource::Module(src)) if id == src)
            }
            _ => false,
        }
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Selector::Param(addr) => write!(f, "{addr}"),
            Selector::Module(target) => write!(f, "{target}.*"),
            Selector::Route(route) => write!(f, "{route}"),
            Selector::Macro(index) => write!(f, "macro{}", index.one_based()),
        }
    }
}

impl FromStr for Selector {
    type Err = String;

    /// Strict, as lane CV2's: the text must be the canonical form [`Display`](fmt::Display) prints.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let parsed = if text.contains("->") {
            text.parse::<RouteAddr>()
                .map(Selector::Route)
                .map_err(|e| e.to_string())?
        } else if let Some(owner) = text.strip_suffix(".*") {
            Selector::Module(match owner {
                "voice" => Target::Voice,
                "master" => Target::Master,
                id => Target::Module(ModuleId::new(id).map_err(|e| e.to_string())?),
            })
        } else if !text.contains('.') && text.starts_with("macro") {
            match text.parse::<ModSource>().map_err(|e| e.to_string())? {
                ModSource::Macro(i) => Selector::Macro(i),
                _ => return Err(format!("'{text}' is not a selector")),
            }
        } else {
            text.parse::<ParamAddr>()
                .map(Selector::Param)
                .map_err(|e| e.to_string())?
        };
        let canonical = parsed.to_string();
        if canonical != text {
            return Err(format!("write '{text}' as '{canonical}'"));
        }
        Ok(parsed)
    }
}

/// One thing an op changes ([`touches`]). **Stand-in** for lane CV2's `enth_challenge::v2::Touch`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Touch {
    /// A parameter's value (or its effect).
    Param(ParamAddr),
    /// A route: added, removed, or its depth.
    Route(RouteAddr),
    /// A module itself (enabled, removed, moved, relabelled, its role) or a built-in.
    Module(Target),
    /// A macro's knob (or its name).
    Macro(MacroIndex),
}

impl fmt::Display for Touch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Touch::Param(addr) => write!(f, "{addr}"),
            Touch::Route(route) => write!(f, "{route}"),
            Touch::Module(target) => write!(f, "{target}"),
            Touch::Macro(index) => write!(f, "macro{}", index.one_based()),
        }
    }
}

/// What `op` changes, applied to `before`, for the lock check. `None` for an op kind this build does not know
/// (`Op` is `non_exhaustive`): the session refuses it while anything is locked (fail closed).
///
/// | Op | Touches |
/// | --- | --- |
/// | `SetParam` | the parameter |
/// | `InsertRoute`, `RemoveRoute`, `SetDepth` | the route, and its destination parameter (modulation moves it) |
/// | `SetMacro` | the macro, and each route from it with its destination |
/// | `SetMacroName` | the macro |
/// | `AddModule`, `RemoveModule`, `SetEnabled` | the module, every parameter it has and every route from or into it |
/// | `Move`, `SetLabel` | the module |
/// | `SetAmpEnv` | the new amplitude envelope and the old one |
/// | `SetSlot` | the slot's new target and its old one (a slot lets the host move a parameter) |
/// | `SetMeta` | nothing |
pub fn touches(before: &Patch, op: &Op) -> Option<Vec<Touch>> {
    let mut out = Vec::new();
    match op {
        Op::SetParam { addr, .. } => out.push(Touch::Param(addr.clone())),
        Op::AddModule { module, .. } => {
            module_whole(&mut out, before, &module.id, Some(module.kind));
        }
        Op::RemoveModule { id } | Op::SetEnabled { id, .. } => {
            module_whole(&mut out, before, id, None);
        }
        Op::Move { id, .. } | Op::SetLabel { id, .. } => {
            out.push(Touch::Module(Target::Module(id.clone())));
        }
        Op::SetAmpEnv { id } => {
            out.push(Touch::Module(Target::Module(id.clone())));
            out.push(Touch::Module(Target::Module(before.voice.amp_env.clone())));
        }
        Op::InsertRoute { route, .. } => route_touches(
            &mut out,
            &RouteAddr {
                src: route.src.clone(),
                dst: route.dst.clone(),
            },
        ),
        Op::RemoveRoute { route } | Op::SetDepth { route, .. } => route_touches(&mut out, route),
        Op::SetMacro { index, .. } => {
            out.push(Touch::Macro(*index));
            for r in before
                .routes
                .iter()
                .filter(|r| r.src == ModSource::Macro(*index))
            {
                route_touches(
                    &mut out,
                    &RouteAddr {
                        src: r.src.clone(),
                        dst: r.dst.clone(),
                    },
                );
            }
        }
        Op::SetMacroName { index, .. } => out.push(Touch::Macro(*index)),
        Op::SetSlot { slot, addr } => {
            out.extend(addr.iter().cloned().map(Touch::Param));
            out.extend(before.slots[slot.get()].iter().cloned().map(Touch::Param));
        }
        Op::SetMeta { .. } => {}
        _ => return None,
    }
    Some(out)
}

fn route_touches(out: &mut Vec<Touch>, route: &RouteAddr) {
    out.push(Touch::Route(route.clone()));
    out.push(Touch::Param(route.dst.clone()));
}

/// A module, all its parameters (from `kind`, or its kind in `before`) and every route from or into it.
fn module_whole(out: &mut Vec<Touch>, before: &Patch, id: &ModuleId, kind: Option<ModuleKind>) {
    out.push(Touch::Module(Target::Module(id.clone())));
    let kind = kind.or_else(|| before.find(id).map(|(_, m)| m.kind));
    if let Some(kind) = kind {
        out.extend(
            kind.params()
                .iter()
                .filter_map(|spec| ParamAddr::module(id.clone(), spec.key).ok())
                .map(Touch::Param),
        );
    }
    let source = ModSource::Module(id.clone());
    out.extend(
        before
            .routes
            .iter()
            .filter(|r| r.dst.module_id() == Some(id) || r.src == source)
            .map(|r| {
                Touch::Route(RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                })
            }),
    );
}

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

/// What a step shows. **Stand-in** for lane LM's `enth_learn::lesson::StepView` (which also names the layout, the
/// start and the advisory `editable` list). The session enforces `lock`, the step's **effective** locks (on a
/// Recreate step, its challenge's `Constraints.locked`), and passes the rest to the editor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepView {
    /// Controls to outline.
    pub highlight: Vec<Selector>,
    /// Controls to hide.
    pub hide: Vec<Selector>,
    /// Controls the session refuses to change.
    pub lock: Vec<Selector>,
}

/// Where the current step is. **Stand-in** for lane LM's `StepPhase`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StepPhase {
    /// The player is working on it (or reading, or listening).
    #[default]
    Working,
    /// A sound check for this revision is out.
    Checking {
        /// The revision the check is for.
        revision: u64,
    },
    /// It passed (sticky until the lesson ends).
    Passed,
}

/// Where an edit came from. **Stand-in** for lane LM's `EditSource`: `Touched` counts `Player` only, and host
/// automation never makes a step pass by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditSource {
    /// An editor edit, undo, redo, or Show's op (the edit path).
    Player,
    /// Host automation folded into the patch (the session passes the checked patch: locked controls pinned).
    Host,
    /// The whole patch was replaced: a step's start, a round's start, a reveal, a host state load, a resume.
    Load,
}

/// A Listen or Choose example, ready to render. **Stand-in** for lane LM's `Audition` (which also has the phrase, a
/// transpose, a render seed and a control curve, Q7); the session reads only `level_match` (the gains).
#[derive(Clone, Debug, PartialEq)]
pub struct Audition {
    /// What the editor calls it.
    pub label: String,
    /// The patch it plays.
    pub patch: Arc<Patch>,
    /// Level-matched with the step's other examples (default on).
    pub level_match: bool,
}

/// A step's outcome for the progress file. **Stand-in** for lane LM's `StepRecord` (the lesson key, the step id and
/// its `StepProgress`); the shell merges it with `Progress::record`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StepRecord {
    /// The step.
    pub step: u16,
    /// Its check passed.
    pub passed: bool,
    /// With help.
    pub assisted: bool,
}

/// A sound check to run. **Stand-in** for lane LM's `CheckRequest` (which also carries the sound checks).
#[derive(Clone, Debug, PartialEq)]
pub struct CheckRequest {
    /// The revision it is for.
    pub revision: u64,
    /// The checked patch at that revision.
    pub patch: Box<Patch>,
}

/// A sound check's verdict. **Stand-in** for lane LM's `CheckVerdict` (which also has per-item results).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckVerdict {
    /// Every item of every sound check held.
    pub passed: bool,
}

/// A measured hint to compute (`enth_solver::hint`). **Stand-in** for lane LM's `HintRequest` (which also carries
/// the phrase, the expectation items and the free parameters).
#[derive(Clone, Debug, PartialEq)]
pub struct HintRequest {
    /// The revision it is for.
    pub revision: u64,
    /// The checked patch at that revision.
    pub patch: Box<Patch>,
}

/// A measured hint, as the Learn view shows it. **Stand-in** for lane LM's `MeasuredHint` (which also says what to
/// point at).
#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredHint {
    /// What to say.
    pub text: String,
}

/// How a Recreate step's round went. **Stand-in** for lane LM's `RoundEnd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RoundEnd {
    /// A submit passed.
    pub passed: bool,
    /// The round used help.
    pub assisted: bool,
    /// The best sound % of its submits, rounded down.
    pub best_sound_pct: Option<u8>,
}

/// The player refused a call. **Stand-in** for lane LM's `PlayerError` (`stale_result`, `not_here`,
/// `out_of_range`, `lesson_ended`, `no_witness_path`, each with a hint).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerError {
    /// A stable snake-case code.
    pub code: &'static str,
    /// What to do about it.
    pub hint: String,
}

/// What the session does after a player call, in order. **Stand-in** for lane LM's `Effect`, variant for variant.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEffect {
    /// Seat this patch as the step's start: one plan, a fresh history. A step load carries at most one, and a
    /// Recreate step none (its round brings its start).
    LoadPatch(Arc<Patch>),
    /// Start the step's Recreate round (`RoundSource::Challenge`, origin `lesson`).
    StartRound(Box<Challenge>),
    /// End the live round: its step was left.
    EndRound,
    /// The step's view, with the effective locks the session enforces.
    View(StepView),
    /// A Listen or Choose step's examples, for the shell to render into the Target ▶ slot.
    Examples(Arc<[Audition]>),
    /// Run this sound check (a job).
    Check(CheckRequest),
    /// Compute this measured hint (a job).
    HintJob(HintRequest),
    /// Show an authored hint.
    Hint(String),
    /// Show a measured hint.
    Measured(MeasuredHint),
    /// Apply this op through the edit path as one undo step (Show): the locks hold.
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
    /// The last step was left forward: the lesson is complete (it stays live until the editor ends it).
    Finished,
    /// The lesson ended (the session restores its stash or keeps the patch itself).
    End(EndChoice),
}

/// The lesson player, as the session drives it. **A stand-in trait** whose methods mirror lane LM's concrete
/// `enth_learn::player::Player` one to one (the F1l ruling), so the session, its tests and a kittest fake can drive
/// any player until LM's merges; then F1L implements it for `Player` (each method delegates; `round_challenge` reads
/// the current step's `StepKind::Recreate { challenge }`) or holds `Player` directly. Pure: `&mut self`, times from
/// the shell, no I/O. Every mutating call answers with effects, in order.
pub trait LessonPlayer: Send + fmt::Debug {
    /// The lesson and the current step (what the session saves).
    fn lesson_ref(&self) -> LessonRef;
    /// How many steps the lesson has.
    fn steps(&self) -> u16;
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
    /// End the lesson.
    fn end(&mut self, choice: EndChoice) -> Result<Vec<PlayerEffect>, PlayerError>;
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
}

impl LearnJobKind {
    /// The kind's name, for errors.
    pub fn name(&self) -> &'static str {
        match self {
            LearnJobKind::Open { .. } => "open",
            LearnJobKind::Check(_) => "check",
            LearnJobKind::Hint(_) => "hint",
            LearnJobKind::Examples { .. } => "examples",
        }
    }
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
    /// The player refused the call.
    Player(PlayerError),
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
            LearnError::Player(e) => e.code,
            LearnError::NoHint => "no_hint",
            LearnError::NothingToShow => "nothing_to_show",
            LearnError::NoExample(_) => "no_example",
            LearnError::Edit(e) => e.code(),
            LearnError::Round(e) => e.code(),
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
            LearnError::Player(e) => e.hint.clone(),
            LearnError::NoHint => "every hint is used: try Show me, or keep going".to_owned(),
            LearnError::NothingToShow => "the step is already where Show me would take it".to_owned(),
            LearnError::NoExample(_) => "wait for the examples to be ready, then pick one of them".to_owned(),
            LearnError::Edit(e) => e.hint(),
            LearnError::Round(e) => e.hint(),
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
            LearnError::Player(e) => write!(f, "the lesson refused: {}", e.code),
            LearnError::NoHint => f.write_str("no hint left"),
            LearnError::NothingToShow => f.write_str("nothing to show"),
            LearnError::NoExample(i) => write!(f, "no example {i}"),
            LearnError::Edit(e) => write!(f, "Show me's edit was refused: {e}"),
            LearnError::Round(e) => write!(f, "the step's round did not start: {e}"),
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
    error: Option<LearnError>,
    requests: Vec<LearnRequest>,
}

impl fmt::Debug for LearnLive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LearnLive")
            .field("player", &self.player.is_some())
            .field("locks", &self.view.lock.len())
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
        &self.learn.view.lock
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
            (job, other) => Err(LearnError::WrongResult {
                job,
                result: other.name(),
            }),
        }
    }

    // --- hooks the rest of the session calls (each one line at its call site) ----------------------------------

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
        let locks = &self.learn.view.lock;
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
        if self.learn.broken || self.learn.player.is_none() {
            return;
        }
        let _ = self.learn_drive(|p| p.tick(now), false);
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
        ];
        if all {
            ids.push(jobs.open.take());
        }
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
            let err = LearnError::Player(PlayerError {
                code: "player_loops",
                hint: "report this: the lesson kept answering its own effects".to_owned(),
            });
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
    fn learn_changed(&mut self, ops: &[Op], source: EditSource) {
        if self.learn.player.is_none() || self.learn.broken {
            return;
        }
        self.learn.revision += 1;
        let revision = self.learn.revision;
        let patch = self.checked_patch();
        let now = self.game_now();
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
                // lesson's end is `learn_end`'s, which restores or keeps.
                PlayerEffect::Passed { .. } | PlayerEffect::End(_) => {}
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
        for lock in &self.learn.view.lock {
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
        if !same_lesson(&found, &saved) {
            let err = LearnError::OtherLesson {
                expected: format!("{}/{}", saved.pack, saved.lesson),
                found: format!("{}/{}", found.pack, found.lesson),
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
            steps: player.map_or(0, LessonPlayer::steps),
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let ok = serde_json::json!({"lesson": {"pack": "basics", "step": 2}, "stash": "{}"});
        let state = LearnState::lenient(&ok).unwrap();
        assert_eq!(state.lesson.as_ref().map(|l| l.step), Some(2));
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
