//! **Owner: lane LM** (the lesson model, freeze point F1l). The reference is docs/learn.md, "The lesson model (lane
//! LM)".
//!
//! A lesson (`lessons/<id>/lesson.json`, `enth.lesson/1`, built from `lesson.toml`): its [`Step`]s, each with a
//! [`StepKind`] (`Read`, `Listen`, `Do`, `Recreate`, `Choose`), a [`StepView`] (the layout, the start patch, what to
//! highlight, hide, mark editable and lock), its text and its authored hints. Hear, Name, Try, Do and Check are
//! authored from these (docs/learn.md, "The lesson shape"): Hear is a `Listen` step, Name a `Read` step, Try a `Do`
//! step, the Do challenges `Recreate` steps and the ear questions `Choose` steps.
//!
//! **Strict input**, as `enth.challenge/1` is: a newer schema is `lesson_too_new`, unknown fields are refused at
//! every level, and every rule the types cannot hold is checked on reading ([`Lesson::validate`]): a field that does
//! not belong to the step's kind is refused rather than ignored. The same types read the authoring TOML (the `toml`
//! feature) and the shipped JSON, so the two forms cannot drift.
//!
//! Decisions recorded for the F1l judge:
//! - **Q7** (Listen options): an [`Example`] has three optional fields, `level_match` (default on), `seed` (a render
//!   seed) and `curve` (a mod-wheel or macro curve), plus `transpose`, which the cards' "at C2 and C4" need.
//! - Ear questions are `StepKind::Choose`, judged by the player (an index); Challenge v2 has no `AnswerKind::Choice`.
//! - A Recreate step's start and locks are its challenge's (`Start`, `Constraints`): its view may not name a start,
//!   editable controls or locks, so the session gets one plan and one lock set per step.
//! - **`StepKind::Drill`** (R4, lane DRL; additive, still `enth.lesson/1`): a step that plays an ear drill ([`DrillStep`]:
//!   a game, a start level, a number of items, optionally a skill to burst on). A pack that uses one declares it in
//!   `requires.drill`, so an older build says "update enth" instead of failing inside a lesson. Playing it to the end
//!   passes the step whatever the score: no content is gated on chords, intervals or note naming.

use std::collections::BTreeSet;

use enth_challenge::{Challenge, PhraseId};
use enth_patch::{Builtin, ModSource, ModuleKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::check::Check;
use crate::drill::DrillStep;
use crate::error::{Doc, LearnError, bad_field};
use crate::pack::{LessonId, PatchRef, StepId, TextRef, check_schema, fields, syntax, unique};
use crate::review::Skill;

pub use enth_challenge::v2::{Selector, Touch, touches};

/// The lesson schema family.
pub const LESSON_KIND: &str = "enth.lesson";
/// The lesson version this build writes, and the newest it reads.
pub const LESSON_VERSION: u32 = 1;
/// The `schema` string this build writes: `enth.lesson/1`.
pub const LESSON_SCHEMA: &str = "enth.lesson/1";

/// The most steps a lesson may have.
pub const MAX_STEPS: usize = 32;
/// The most authored hints a step may have.
pub const MAX_HINTS: usize = 8;
/// The most examples a Listen step may have.
pub const MAX_EXAMPLES: usize = 8;
/// The fewest and most options of an ear question.
pub const CHOOSE_OPTIONS: core::ops::RangeInclusive<usize> = 2..=6;
/// The longest title, label or hint, in characters.
pub const MAX_LINE: usize = 300;

/// A lesson's tier, as the catalogue's learn tiers: hints by tier (docs/learn.md, "The lesson shape").
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Every coach level open.
    Basic,
    /// Levels 3 and 4 after a first Submit.
    Intermediate,
    /// No hint on the first attempt.
    Advanced,
}

/// A parameter in the catalogue's kind form, what a lesson teaches: `svf.cutoff`, `osc.wave`, `voice.glide`,
/// `master.level`, `macro.value` (the macro knob). `lesson-coverage` counts these.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamKey(String);

impl ParamKey {
    /// A checked key: a module kind (or `voice`, `master`, `macro`) and one of its parameters.
    pub fn new(text: &str) -> Result<ParamKey, String> {
        let bad = || {
            format!(
                "{text:?} is not <kind>.<param> from the catalogue (e.g. svf.cutoff, voice.glide)"
            )
        };
        let (kind, param) = text.split_once('.').ok_or_else(bad)?;
        let known = match kind {
            "macro" => param == enth_patch::catalogue::MACRO_VALUE.key,
            _ => match Builtin::ALL.iter().find(|b| b.key() == kind) {
                Some(b) => b.param_index(param).is_some(),
                None => ModuleKind::from_key(kind).is_some_and(|k| k.param(param).is_some()),
            },
        };
        known.then(|| ParamKey(text.to_owned())).ok_or_else(bad)
    }

    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for ParamKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ParamKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        ParamKey::new(&s).map_err(serde::de::Error::custom)
    }
}

/// How the editor lays a step out. LUI draws it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// The whole editor beside the lesson pane (the default).
    #[default]
    Full,
    /// Only the modules the step highlights or leaves editable, as focus cards (the 1200 × 800 overflow case).
    Focus,
}

impl Layout {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes the field by reference"
    )]
    fn is_full(&self) -> bool {
        *self == Layout::Full
    }
}

/// What the editor shows and allows during a step. Selectors are `enth_challenge::v2::Selector`'s text form
/// (`filter1.cutoff`, `filter1.*`, `macro3`, `lfo1 -> filter1.cutoff`), each checked against the step's start when
/// the pack loads (it must name something the start has; no selector twice in one list).
///
/// - `start`: the patch the step loads (none: the patch carries over from the step before, or the player's own on
///   the first step). A Recreate step's start is its challenge's.
/// - `highlight`: the static accent outline on the new controls.
/// - `hide`: what the editor hides (usually unused modules, `noise1.*`).
/// - `editable`: **advisory**, for the UI, the measured hints' free set and `closest` (empty: no restriction).
/// - `locked`: **enforced** by the session's edit path (`SessionError::Locked`); `lesson-check` checks the witness
///   touches neither a locked nor, when `editable` is set, a non-editable control.
///
/// `editable` and `locked` are named as the challenge's `Constraints` names them (the F1l decision: the lesson's
/// `lock` became `locked`), so a step and a Recreate step's challenge read alike; on a Recreate step the player
/// fills both from the challenge.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepView {
    /// The layout.
    #[serde(default, skip_serializing_if = "Layout::is_full")]
    pub layout: Layout,
    /// The patch the step loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<PatchRef>,
    /// What to outline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlight: Vec<Selector>,
    /// What to hide.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<Selector>,
    /// What the player is meant to change (advisory).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub editable: Vec<Selector>,
    /// What the session refuses to change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked: Vec<Selector>,
}

impl StepView {
    fn is_default(&self) -> bool {
        *self == StepView::default()
    }

    /// Whether the lock list covers `touch` (the session's rule, `SessionError::Locked`).
    pub fn locks(&self, touch: &Touch) -> bool {
        self.locked.iter().any(|s| s.covers(touch))
    }

    /// Whether `touch` is something the player is meant to change: not locked, and covered by `editable` when it
    /// is set.
    pub fn allows(&self, touch: &Touch) -> bool {
        !self.locks(touch)
            && (self.editable.is_empty() || self.editable.iter().any(|s| s.covers(touch)))
    }
}

/// A control curve an example plays (Q7; the mod wheel in lesson 6.3, a macro sweep): linear between points, held
/// after the last.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCurve {
    /// `modwheel` or `macro1`…`macro8`.
    pub source: ModSource,
    /// 2 to 32 points, in time order.
    pub points: Vec<CurvePoint>,
}

/// One point of a [`ControlCurve`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurvePoint {
    /// Time from the start of the render, in ms.
    pub ms: u32,
    /// The control's position, 0–100 %.
    pub pct: u8,
}

/// The phrase an example plays unless it names one: `HELD`.
const DEFAULT_PHRASE: PhraseId = enth_challenge::phrases::HELD;

fn default_phrase() -> PhraseId {
    DEFAULT_PHRASE
}
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_default_phrase(p: &PhraseId) -> bool {
    *p == DEFAULT_PHRASE
}
fn yes() -> bool {
    true
}
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_true(b: &bool) -> bool {
    *b
}
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_zero(n: &i8) -> bool {
    *n == 0
}

/// One sound to hear, in a Listen step or as an ear question's option: a pack patch through a phrase, rendered live
/// and deterministically (no pre-rendered audio). The player never renders: the session builds the render jobs
/// (examples share the Target ▶ slot by time, lane SL).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    /// What the player sees on its button ("Open", "Closed").
    pub label: String,
    /// The patch.
    pub patch: PatchRef,
    /// The phrase from the table (default 1, `HELD`).
    #[serde(default = "default_phrase", skip_serializing_if = "is_default_phrase")]
    pub phrase: PhraseId,
    /// Semitones added to every note of the phrase, −36 to 36 (default 0): "on saw at C2 and C4".
    #[serde(default, skip_serializing_if = "is_zero")]
    pub transpose: i8,
    /// Whether the examples of a step play level-matched (default on). Off where the level is the lesson (0.1, 7.3).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub level_match: bool,
    /// The render seed (default: the scoring seed). For lessons on random shapes and seeds (5.6, 8.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    /// A control curve to play with it (6.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<ControlCurve>,
}

impl Example {
    fn validate(&self, field: &str) -> Result<(), LearnError> {
        let f = |name: &str| format!("{field}.{name}");
        check_line(&self.label, &f("label"))?;
        if enth_challenge::phrases::phrase(self.phrase).is_none() {
            return Err(bad_field(
                Doc::Lesson,
                f("phrase"),
                format!("phrase {} is not in this build", self.phrase.0),
            ));
        }
        if !(-36..=36).contains(&self.transpose) {
            return Err(bad_field(
                Doc::Lesson,
                f("transpose"),
                "is −36 to 36 semitones",
            ));
        }
        if let Some(c) = &self.curve {
            if !matches!(c.source, ModSource::ModWheel | ModSource::Macro(_)) {
                return Err(bad_field(
                    Doc::Lesson,
                    f("curve.source"),
                    "is modwheel or a macro",
                ));
            }
            if !(2..=32).contains(&c.points.len()) {
                return Err(bad_field(
                    Doc::Lesson,
                    f("curve.points"),
                    "holds 2 to 32 points",
                ));
            }
            if c.points.windows(2).any(|w| w[0].ms >= w[1].ms)
                || c.points.iter().any(|p| p.pct > 100)
            {
                return Err(bad_field(
                    Doc::Lesson,
                    f("curve.points"),
                    "times rise strictly and positions are 0–100 %",
                ));
            }
        }
        Ok(())
    }
}

/// What a step asks of the player.
#[derive(Clone, Debug, PartialEq)]
pub enum StepKind {
    /// Read the text (Name). Passes on Next.
    Read,
    /// Hear examples, before anything is explained (Hear). Passes on Next.
    Listen {
        /// 1 to 8 examples.
        examples: Vec<Example>,
    },
    /// Reach a goal a [`Check`] decides, live (Try). `witness` is the reference solution: `lesson-check` plays it,
    /// and Show applies its next op.
    Do {
        /// The goal.
        check: Check,
        /// A patch that passes it.
        witness: PatchRef,
    },
    /// A real Recreate round (Do): A/B, the rings, the coach, the results drawer. Passes when the round passes.
    Recreate {
        /// The challenge (v2: a drawn, patch or seed target, its start and constraints), embedded whole.
        challenge: Box<Challenge>,
        /// A reference solution for `lesson-check`, when the target is not one patch.
        witness: Option<PatchRef>,
    },
    /// An ear question (Check, and Hear's "which is brighter?"): pick the option that answers the text.
    Choose {
        /// 2 to 6 options, rendered live.
        options: Vec<Example>,
        /// The index of the right option.
        answer: u8,
    },
    /// Play an ear drill (R4): N questions on a game's ladder, the 3-down/1-up staircase, bursts into Review's
    /// boxes. Passes when the drill is played to its end, whatever the score.
    Drill(DrillStep),
}

impl StepKind {
    /// `read`, `listen`, `do`, `recreate`, `choose`, `drill`.
    pub fn name(&self) -> &'static str {
        match self {
            StepKind::Read => "read",
            StepKind::Listen { .. } => "listen",
            StepKind::Do { .. } => "do",
            StepKind::Recreate { .. } => "recreate",
            StepKind::Choose { .. } => "choose",
            StepKind::Drill(_) => "drill",
        }
    }
}

/// One step of a lesson.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    /// Its id, append-only (progress is keyed by it).
    pub id: StepId,
    /// Its text, a markdown file beside `lesson.json` (required for a Read step).
    pub text: Option<TextRef>,
    /// What the editor shows and allows.
    pub view: StepView,
    /// Authored hints, shown in order before any measured one. Every hint marks the step assisted.
    pub hints: Vec<String>,
    /// What it asks.
    pub kind: StepKind,
}

/// A lesson: one idea in 5–10 minutes. See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct Lesson {
    /// Its id within the pack, and its directory's name.
    pub id: LessonId,
    /// Its title.
    pub title: String,
    /// Its tier.
    pub tier: Tier,
    /// What the player can do after it, one sentence (GR2 codes "what did you learn?" against it).
    pub objective: String,
    /// The parameters it teaches (`lesson-coverage`).
    pub teaches: Vec<ParamKey>,
    /// Its steps, in order.
    pub steps: Vec<Step>,
}

impl Lesson {
    /// Read `lesson.json`: parse, check the schema (`missing_schema`, `bad_schema`, `lesson_too_new`), decode
    /// (unknown fields refused, a repeated key refused), [`validate`](Self::validate).
    pub fn from_json(text: &str) -> Result<Lesson, LearnError> {
        Lesson::from_json_versioned(text).map(|(l, _)| l)
    }

    /// [`from_json`](Self::from_json), and the schema version the file had.
    pub(crate) fn from_json_versioned(text: &str) -> Result<(Lesson, u32), LearnError> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| syntax(Doc::Lesson, e.to_string()))?;
        let version = check_schema(Doc::Lesson, &value, LESSON_VERSION)?;
        let file: LessonFile =
            serde_json::from_str(text).map_err(|e| fields(Doc::Lesson, e.to_string()))?;
        Ok((file.into_lesson()?, version))
    }

    /// Read the authoring `lesson.toml`, with the same checks.
    #[cfg(feature = "toml")]
    pub fn from_toml(text: &str) -> Result<Lesson, LearnError> {
        let value: Value = toml::from_str(text).map_err(|e| syntax(Doc::Lesson, e.to_string()))?;
        check_schema(Doc::Lesson, &value, LESSON_VERSION)?;
        let file: LessonFile =
            serde_json::from_value(value).map_err(|e| fields(Doc::Lesson, e.to_string()))?;
        file.into_lesson()
    }

    /// The JSON text: compact, schema first, byte-deterministic (what `lesson.json` holds).
    pub fn to_json(&self) -> String {
        serde_json::to_string(&LessonFile::from(self)).unwrap_or_default()
    }

    /// The JSON text, indented.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(&LessonFile::from(self)).unwrap_or_default()
    }

    /// The step with this id.
    pub fn step(&self, id: &StepId) -> Option<(usize, &Step)> {
        self.steps.iter().enumerate().find(|(_, s)| &s.id == id)
    }

    /// Every text the lesson names, once each.
    pub fn texts(&self) -> impl Iterator<Item = &TextRef> {
        let set: BTreeSet<&TextRef> = self.steps.iter().filter_map(|s| s.text.as_ref()).collect();
        set.into_iter()
    }

    /// Every patch the lesson names (starts, examples, options, witnesses), once each, in path order.
    pub fn patches(&self) -> impl Iterator<Item = &PatchRef> {
        let mut set: BTreeSet<&PatchRef> = BTreeSet::new();
        for s in &self.steps {
            set.extend(s.view.start.as_ref());
            match &s.kind {
                StepKind::Read | StepKind::Drill(_) => {}
                StepKind::Listen { examples: list } | StepKind::Choose { options: list, .. } => {
                    set.extend(list.iter().map(|e| &e.patch));
                }
                StepKind::Do { witness, .. } => {
                    set.insert(witness);
                }
                StepKind::Recreate { witness, .. } => set.extend(witness.as_ref()),
            }
        }
        set.into_iter()
    }

    /// Check what the types cannot: a title and objective, 1 to [`MAX_STEPS`] steps with unique ids, and each
    /// step's rules (a Read step has a text; Listen has 1 to 8 examples; Choose 2 to 6 options and an answer among
    /// them; a Recreate step's view names no start, editable set or locks; hints are 1 to 300 characters, at most
    /// 8; every check's expectations parse and its values are well-formed).
    pub fn validate(&self) -> Result<(), LearnError> {
        check_line(&self.title, "title")?;
        check_line(&self.objective, "objective")?;
        if self.steps.is_empty() || self.steps.len() > MAX_STEPS {
            return Err(bad_field(
                Doc::Lesson,
                "steps",
                format!("holds 1 to {MAX_STEPS} steps"),
            ));
        }
        unique(self.steps.iter().map(|s| &s.id), |id| {
            bad_field(Doc::Lesson, "steps", format!("has two steps {id}"))
        })?;
        for (i, s) in self.steps.iter().enumerate() {
            let at = |name: &str| format!("steps[{i}].{name}");
            if s.hints.len() > MAX_HINTS {
                return Err(bad_field(
                    Doc::Lesson,
                    at("hints"),
                    format!("holds at most {MAX_HINTS}"),
                ));
            }
            for (h, hint) in s.hints.iter().enumerate() {
                check_line(hint, &at(&format!("hints[{h}]")))?;
            }
            match &s.kind {
                StepKind::Read if s.text.is_none() => {
                    return Err(bad_field(Doc::Lesson, at("text"), "a read step has a text"));
                }
                StepKind::Read => {}
                StepKind::Listen { examples } => {
                    if examples.is_empty() || examples.len() > MAX_EXAMPLES {
                        return Err(bad_field(
                            Doc::Lesson,
                            at("examples"),
                            "holds 1 to 8 examples",
                        ));
                    }
                    for (e, ex) in examples.iter().enumerate() {
                        ex.validate(&at(&format!("examples[{e}]")))?;
                    }
                }
                StepKind::Choose { options, answer } => {
                    if !CHOOSE_OPTIONS.contains(&options.len()) {
                        return Err(bad_field(
                            Doc::Lesson,
                            at("options"),
                            "holds 2 to 6 options",
                        ));
                    }
                    if usize::from(*answer) >= options.len() {
                        return Err(bad_field(
                            Doc::Lesson,
                            at("answer"),
                            "is not one of the options (0-based)",
                        ));
                    }
                    for (e, ex) in options.iter().enumerate() {
                        ex.validate(&at(&format!("options[{e}]")))?;
                    }
                }
                StepKind::Do { check, .. } => check.validate(&at("check"))?,
                StepKind::Drill(drill) => {
                    if let Some((field, message)) = drill.problem() {
                        return Err(bad_field(Doc::Lesson, at(field), message));
                    }
                    if let Some(skill) = drill.skill.as_ref()
                        && !skill.is_ear()
                        && let Err(message) = ParamKey::new(skill.as_str())
                    {
                        return Err(bad_field(Doc::Lesson, at("skill"), message));
                    }
                }
                StepKind::Recreate { .. } => {
                    if s.view.start.is_some()
                        || !s.view.editable.is_empty()
                        || !s.view.locked.is_empty()
                    {
                        return Err(bad_field(
                            Doc::Lesson,
                            at("view"),
                            "a recreate step's start, editable controls and locks are its challenge's",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// A title, label, objective or hint: not blank, at most [`MAX_LINE`] characters.
fn check_line(text: &str, field: &str) -> Result<(), LearnError> {
    if text.trim().is_empty() || text.chars().count() > MAX_LINE {
        return Err(bad_field(
            Doc::Lesson,
            field,
            format!("is 1 to {MAX_LINE} characters"),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// The file forms
// ---------------------------------------------------------------------------------------------------------------

/// The on-disk lesson: the schema first.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LessonFile {
    schema: String,
    id: LessonId,
    title: String,
    tier: Tier,
    objective: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    teaches: Vec<ParamKey>,
    steps: Vec<StepFile>,
}

/// The kind tag of a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KindTag {
    Read,
    Listen,
    Do,
    Recreate,
    Choose,
    Drill,
}

/// The on-disk step: flat, with the kind's fields beside the common ones, so `lesson.toml` reads naturally
/// (`kind = "do"`, then `[steps.check]`). A field the kind does not have is refused.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StepFile {
    id: StepId,
    kind: KindTag,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<TextRef>,
    #[serde(default, skip_serializing_if = "StepView::is_default")]
    view: StepView,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    examples: Vec<Example>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    check: Option<Check>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    witness: Option<PatchRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    challenge: Option<Box<Challenge>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    options: Vec<Example>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    answer: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    game: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    items: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skill: Option<Skill>,
}

impl LessonFile {
    fn into_lesson(self) -> Result<Lesson, LearnError> {
        let steps = self
            .steps
            .into_iter()
            .enumerate()
            .map(|(i, s)| s.into_step(i))
            .collect::<Result<_, _>>()?;
        let lesson = Lesson {
            id: self.id,
            title: self.title,
            tier: self.tier,
            objective: self.objective,
            teaches: self.teaches,
            steps,
        };
        lesson.validate()?;
        Ok(lesson)
    }
}

impl StepFile {
    fn into_step(self, i: usize) -> Result<Step, LearnError> {
        let kind_name = format!("{:?}", self.kind).to_ascii_lowercase();
        let extra = |name: &str| {
            bad_field(
                Doc::Lesson,
                format!("steps[{i}].{name}"),
                format!("a {kind_name} step has no {name}"),
            )
        };
        let need = |name: &str| {
            bad_field(
                Doc::Lesson,
                format!("steps[{i}].{name}"),
                format!("a {kind_name} step needs {name}"),
            )
        };
        // Refuse every field the kind does not have, so nothing is silently ignored.
        let present = [
            ("examples", !self.examples.is_empty()),
            ("check", self.check.is_some()),
            ("witness", self.witness.is_some()),
            ("challenge", self.challenge.is_some()),
            ("options", !self.options.is_empty()),
            ("answer", self.answer.is_some()),
            ("game", self.game.is_some()),
            ("level", self.level.is_some()),
            ("items", self.items.is_some()),
            ("skill", self.skill.is_some()),
        ];
        let allowed: &[&str] = match self.kind {
            KindTag::Read => &[],
            KindTag::Listen => &["examples"],
            KindTag::Do => &["check", "witness"],
            KindTag::Recreate => &["challenge", "witness"],
            KindTag::Choose => &["options", "answer"],
            KindTag::Drill => &["game", "level", "items", "skill"],
        };
        if let Some((name, _)) = present.iter().find(|(n, p)| *p && !allowed.contains(n)) {
            return Err(extra(name));
        }
        let kind = match self.kind {
            KindTag::Read => StepKind::Read,
            KindTag::Listen => StepKind::Listen {
                examples: self.examples,
            },
            KindTag::Do => StepKind::Do {
                check: self.check.ok_or_else(|| need("check"))?,
                witness: self.witness.ok_or_else(|| need("witness"))?,
            },
            KindTag::Recreate => StepKind::Recreate {
                challenge: self.challenge.ok_or_else(|| need("challenge"))?,
                witness: self.witness,
            },
            KindTag::Choose => StepKind::Choose {
                options: self.options,
                answer: self.answer.ok_or_else(|| need("answer"))?,
            },
            KindTag::Drill => {
                let mut drill = DrillStep::new(self.game.ok_or_else(|| need("game"))?);
                drill.level = self.level.unwrap_or(drill.level);
                drill.items = self.items.unwrap_or(drill.items);
                drill.skill = self.skill;
                StepKind::Drill(drill)
            }
        };
        Ok(Step {
            id: self.id,
            text: self.text,
            view: self.view,
            hints: self.hints,
            kind,
        })
    }
}

impl From<&Lesson> for LessonFile {
    fn from(l: &Lesson) -> Self {
        LessonFile {
            schema: LESSON_SCHEMA.to_owned(),
            id: l.id.clone(),
            title: l.title.clone(),
            tier: l.tier,
            objective: l.objective.clone(),
            teaches: l.teaches.clone(),
            steps: l.steps.iter().map(StepFile::from).collect(),
        }
    }
}

impl From<&Step> for StepFile {
    fn from(s: &Step) -> Self {
        let mut f = StepFile {
            id: s.id.clone(),
            kind: KindTag::Read,
            text: s.text.clone(),
            view: s.view.clone(),
            hints: s.hints.clone(),
            examples: Vec::new(),
            check: None,
            witness: None,
            challenge: None,
            options: Vec::new(),
            answer: None,
            game: None,
            level: None,
            items: None,
            skill: None,
        };
        match &s.kind {
            StepKind::Read => {}
            StepKind::Listen { examples } => {
                f.kind = KindTag::Listen;
                f.examples.clone_from(examples);
            }
            StepKind::Do { check, witness } => {
                f.kind = KindTag::Do;
                f.check = Some(check.clone());
                f.witness = Some(witness.clone());
            }
            StepKind::Recreate { challenge, witness } => {
                f.kind = KindTag::Recreate;
                f.challenge = Some(challenge.clone());
                f.witness.clone_from(witness);
            }
            StepKind::Choose { options, answer } => {
                f.kind = KindTag::Choose;
                f.options.clone_from(options);
                f.answer = Some(*answer);
            }
            StepKind::Drill(drill) => {
                f.kind = KindTag::Drill;
                f.game = Some(drill.game);
                // The defaults are left out, so a step written plainly reads back as it was written.
                f.level = (drill.level != 1).then_some(drill.level);
                f.items = (drill.items != crate::drill::DEFAULT_STEP_ITEMS).then_some(drill.items);
                f.skill.clone_from(&drill.skill);
            }
        }
        f
    }
}
