//! **Owner: lane SKL** (freeze point F2l), then **EV3** (additive). The output types of `enth lesson show`
//! (`enth.lesson-show/1`) and `enth lesson check` (`enth.lesson-check/1`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `enth.lesson-show/1`: a pack's lessons, step by step, as a lesson's text says them. No answers (a choose step's
/// answer, a do step's witness) unless `--reveal`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonShowOut {
    /// `enth.lesson-show/1`.
    pub schema: String,
    /// The pack.
    pub pack: LessonPackOut,
    /// The lessons shown (one with `--lesson`, else every lesson of the pack, in its order).
    pub lessons: Vec<LessonOut>,
    /// What to do next.
    pub next: String,
}

/// A pack's identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonPackOut {
    /// Its id (`basics`): progress is keyed by it.
    pub id: String,
    /// Its version (semver).
    pub version: String,
    /// Its title.
    pub title: String,
    /// The engine it was checked on (recorded, never a gate; read from the authoring TOML, the pack is sealed on
    /// this build's).
    pub engine: String,
}

/// One lesson.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonOut {
    /// `pack/lesson`.
    pub key: String,
    /// The lesson's id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// `basic`, `intermediate` or `advanced`.
    pub tier: String,
    /// What it teaches, in a sentence.
    pub objective: String,
    /// The controls it teaches (catalogue keys: `svf.cutoff`).
    pub teaches: Vec<String>,
    /// Its steps, in order.
    pub steps: Vec<LessonStepOut>,
}

/// One step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonStepOut {
    /// Its place, 1-based ("Step 2 of 3"); `enth lesson check --step` takes it or the id.
    pub number: u16,
    /// Its id (append-only; progress is keyed by it).
    pub id: String,
    /// `read`, `listen`, `do`, `recreate` or `choose`.
    pub kind: String,
    /// The step's text (markdown), when it has one.
    pub text: Option<String>,
    /// The authored hints, in order.
    pub hints: Vec<String>,
    /// The patch the step starts from (a pack path; null: the patch carries over).
    pub start: Option<String>,
    /// What the step outlines.
    pub highlight: Vec<String>,
    /// What the step hides.
    pub hide: Vec<String>,
    /// What the player is meant to change (advisory).
    pub editable: Vec<String>,
    /// What the session refuses to change.
    pub locked: Vec<String>,
    /// A do step's goal, as the lesson file holds it (`{"kind":"sound","phrase":1,"expect":[…]}`).
    pub check: Option<Value>,
    /// A listen step's examples or a choose step's options: their labels, in order.
    pub examples: Vec<String>,
    /// A recreate step's challenge, as the lesson file holds it.
    pub challenge: Option<Value>,
    /// With `--reveal`: a choose step's answer (0-based).
    pub answer: Option<u8>,
    /// With `--reveal`: a do or recreate step's witness, the reference solution (a pack path).
    pub witness: Option<String>,
}

/// `enth.lesson-check/1`: one do step's check run against a patch file: its patch predicates (no edit log, so a
/// `touched` predicate cannot hold here) and its sound checks (each rendered through its phrase and judged). Exit 1
/// when it fails.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonCheckOut {
    /// `enth.lesson-check/1`.
    pub schema: String,
    /// `pack/lesson`.
    pub lesson: String,
    /// The step's id.
    pub step: String,
    /// Its place, 1-based.
    pub number: u16,
    /// The patch file checked.
    pub patch: String,
    /// Every predicate and every sound item passed.
    pub pass: bool,
    /// The patch predicates, in order.
    pub predicates: Vec<LessonPredicateOut>,
    /// The sound checks' items, in order.
    pub sounds: Vec<LessonSoundItemOut>,
    /// A note for the reader (why a predicate cannot hold here), when there is one.
    pub note: Option<String>,
    /// What to do next.
    pub next: String,
}

/// One patch predicate's result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonPredicateOut {
    /// The predicate, as the lesson file holds it.
    pub predicate: Value,
    /// Whether it holds for the patch.
    pub pass: bool,
}

/// One sound item's result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonSoundItemOut {
    /// The phrase it is heard through (id).
    pub phrase: u16,
    /// The item as written (`brightness<500Hz`).
    pub item: String,
    /// Whether it held.
    pub pass: bool,
    /// The signed margin in the feature's unit, when it has one.
    pub margin: Option<f64>,
}
