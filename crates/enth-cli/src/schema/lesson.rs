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

// ---------------------------------------------------------------------------------------------------------------
// Lane EV3: `enth lesson list`, `start`, `hint` and `closest`
// ---------------------------------------------------------------------------------------------------------------

/// `enth.lesson-list/1`: the packs at the given paths and the lessons in them, each with its steps in short (number,
/// id, kind and how many authored hints). A folder that holds packs lists each; one that a build refuses is named in
/// `refused` with its reason and hint, not hidden.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonListOut {
    /// `enth.lesson-list/1`.
    pub schema: String,
    /// The packs read, in path order.
    pub packs: Vec<LessonListPackOut>,
    /// Packs found in a folder that this build refused.
    pub refused: Vec<LessonRefusedOut>,
    /// What to do next.
    pub next: String,
}

/// One pack in a list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonListPackOut {
    /// Where it was read from (a folder or an `.enthpack`).
    pub path: String,
    /// Its id.
    pub id: String,
    /// Its version (semver).
    pub version: String,
    /// Its title.
    pub title: String,
    /// The engine it was sealed on.
    pub engine: String,
    /// The module kinds it covers.
    pub covers: Vec<String>,
    /// Its tracks (ordered lists of `pack/lesson` keys).
    pub tracks: Vec<LessonTrackOut>,
    /// Its lessons, in the pack's order.
    pub lessons: Vec<LessonListEntryOut>,
}

/// A track: an ordered list of lessons that may cross packs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonTrackOut {
    /// Its id (`tour`).
    pub id: String,
    /// Its title.
    pub title: String,
    /// The lessons, as `pack/lesson` keys.
    pub lessons: Vec<String>,
}

/// One lesson in a list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonListEntryOut {
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
    /// The controls it teaches (catalogue keys).
    pub teaches: Vec<String>,
    /// Its steps, in order.
    pub steps: Vec<LessonStepRefOut>,
}

/// A step in short.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonStepRefOut {
    /// Its place, 1-based.
    pub number: u16,
    /// Its id.
    pub id: String,
    /// `read`, `listen`, `do`, `recreate` or `choose`.
    pub kind: String,
    /// How many authored hints it has (`enth lesson hint --n 1` to `--n <hints>`).
    pub hints: u16,
}

/// A pack the build refused, found while listing a folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonRefusedOut {
    /// Where it was.
    pub path: String,
    /// The reader's code (`pack_too_new`, `hash_mismatch`, …).
    pub code: String,
    /// Why.
    pub message: String,
    /// What to do about it.
    pub hint: String,
}

/// `enth.lesson-start/1`: what the player loads when a step opens, written to files: the step's start patch and, for
/// a recreate step, its round (the challenge file `enth compare`, `suggest` and `match` take with `--challenge`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonStartOut {
    /// `enth.lesson-start/1`.
    pub schema: String,
    /// `pack/lesson`.
    pub lesson: String,
    /// The step's id.
    pub step: String,
    /// Its place, 1-based.
    pub number: u16,
    /// `read`, `listen`, `do`, `recreate` or `choose`.
    pub kind: String,
    /// Where the start came from: the pack patch the step names (`patches/saw.patch.json`), or `challenge` for a
    /// recreate step's own start; null when only the examples were asked for.
    pub from: Option<String>,
    /// The start patch, as written (null when only the examples were asked for).
    pub patch: Option<String>,
    /// A recreate step's challenge file, as written.
    pub challenge: Option<String>,
    /// A listen or choose step's sounds, as files (`--examples-out`); empty otherwise. A choose step's answer is one
    /// of the labels.
    pub examples: Vec<LessonExampleOut>,
    /// What to do next.
    pub next: String,
}

/// One sound of a listen or choose step, as files: `enth describe PATCH --perf PERF` measures it, where the player
/// would play it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonExampleOut {
    /// What the player sees on its button ("A", "Open").
    pub label: String,
    /// The example's patch, as written.
    pub patch: String,
    /// The performance (`enth.perf/1`): the step's phrase, transposed, as written.
    pub perf: String,
    /// The phrase's name (`held`).
    pub phrase: String,
    /// Semitones added to every note of the phrase.
    pub transpose: i8,
    /// The render seed, when the example sets one: pass it to `describe --seed`.
    pub seed: Option<u32>,
}

/// `enth.lesson-hint/1`: one hint of a step's ladder. `--n 1` to the number of authored hints are the lesson's own
/// words; the next is measured on a patch file: a do step's by `enth_solver::hint` (the move over the failing
/// items, within the step's editable controls), a recreate step's by the round's coach, four graded levels. Any hint
/// marks a step assisted in the player.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonHintOut {
    /// `enth.lesson-hint/1`.
    pub schema: String,
    /// `pack/lesson`.
    pub lesson: String,
    /// The step's id.
    pub step: String,
    /// Its place, 1-based.
    pub number: u16,
    /// The step's kind: `do`, `recreate` or `choose`.
    pub kind: String,
    /// Which hint this is (the `--n` asked for), 1-based.
    pub n: u16,
    /// How many authored hints the step has.
    pub authored: u16,
    /// `authored`, `measured` (a do step's solver move) or `coach` (a recreate step's graded hint).
    pub source: String,
    /// The hint, in words.
    pub text: String,
    /// The patch file a measured or coach hint is for.
    pub patch: Option<String>,
    /// A do step's measured move: what to turn, by how much, and the same as `enth apply --ops` takes.
    pub measured: Option<LessonMeasuredOut>,
    /// A recreate step's coach hint: an `enth.suggest/1` object for the round, at level `n - authored`.
    pub coach: Option<crate::schema::SuggestOut>,
    /// Whether another hint follows this one (`--n <n + 1>`).
    pub more: bool,
    /// Renders the measured hint cost (0 for an authored one).
    pub renders: u32,
    /// What to do next.
    pub next: String,
}

/// A do step's measured hint: the solver's verified move, from the player's patch, within the step's editable set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonMeasuredOut {
    /// The controls the Learn view points at.
    pub highlight: Vec<String>,
    /// Each parameter the move changes, the largest first.
    pub moves: Vec<LessonMoveOut>,
    /// The move as ops, for `enth apply <patch> --ops '<json>'` (plain values with units).
    pub ops: Vec<Value>,
    /// Applying every move makes the whole sound check pass.
    pub pass_all: bool,
    /// Items no move within the step's controls can satisfy, with the reason.
    pub blocked: Vec<LessonBlockedOut>,
}

/// One parameter of a measured move.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonMoveOut {
    /// The address (`filter1.cutoff`).
    pub param: String,
    /// Its name on the card (`Cutoff`).
    pub name: String,
    /// Where it is now, with its unit.
    pub from: String,
    /// Where the move puts it, with its unit.
    pub to: String,
    /// `up` or `down`.
    pub direction: String,
}

/// An item a measured hint cannot reach.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonBlockedOut {
    /// The item as written.
    pub item: String,
    /// The stable reason (`at_bound`, `conflict`, `no_effect`, `absent`, `performance`, `unmodelled`).
    pub reason: String,
    /// Why, in a sentence.
    pub why: String,
    /// Parameters sitting at a bound.
    pub at_bound: Vec<String>,
}

/// `enth.lesson-closest/1`: "How close am I?" for a recreate step: the nearest patch that passes the round, found by
/// CMA-ES from your patch over the step's editable controls, with how far each moved. `passed: false` means none was
/// found within the budget, and `deltas` is the nearest to passing, not a solution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonClosestOut {
    /// `enth.lesson-closest/1`.
    pub schema: String,
    /// `pack/lesson`.
    pub lesson: String,
    /// The step's id.
    pub step: String,
    /// Its place, 1-based.
    pub number: u16,
    /// The patch file the search started from.
    pub patch: String,
    /// The parameters that were free to move (the step's editable set, or `--free`).
    pub free: Vec<String>,
    /// The most renders (`--budget`).
    pub budget: u32,
    /// The answer passes the round.
    pub passed: bool,
    /// The sound score of your patch, 0 to 100.
    pub start_percent: f64,
    /// The sound score of the answer.
    pub percent: f64,
    /// The knob travel from your patch to the answer: the distance on the free controls' 0-1 axes, plus 1 for each
    /// switch or choice that changes.
    pub distance: f64,
    /// Renders used.
    pub renders: u32,
    /// The render at which a pass was first seen.
    pub first_pass_renders: Option<u32>,
    /// Why it ended: `start_passes`, `converged`, `budget` or `exhausted`.
    pub stop: String,
    /// One entry per free parameter, in order.
    pub deltas: Vec<LessonDeltaOut>,
    /// The answer patch file, with `--out`.
    pub out: Option<String>,
    /// The result in a sentence.
    pub text: String,
    /// What to do next.
    pub next: String,
}

/// How one free parameter moved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonDeltaOut {
    /// The address.
    pub param: String,
    /// Your value, with its unit.
    pub from: String,
    /// The answer's value.
    pub to: String,
    /// The signed move on the knob's 0-1 axis (0 or 1 for a switch or choice).
    pub norm: f64,
    /// A switch or a choice, not a knob.
    pub categorical: bool,
    /// It changed.
    pub moved: bool,
}
