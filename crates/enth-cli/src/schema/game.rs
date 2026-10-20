//! **Owner: lane CL.** The objects of the R1 game commands: `suggest` (`enth.suggest/1`), `match` (`enth.match/1`),
//! `explain` (`enth.explain/1`), `compare` (`enth.compare/2`) and `challenge decode` (`enth.challenge-decode/1`), plus
//! [`ScoreOut`], the three scores of a round. They live in their own file (not in [`super::types`]) so the lanes that
//! add commands never edit one another's. (`challenge new` and `show` stay with lane SKR's types in
//! [`super::types`], at version 2.)
//!
//! Same rules as every output type: `Serialize + Deserialize + JsonSchema`, `deny_unknown_fields`, a shape change
//! or a change of meaning bumps the version in [`super::ids`] and updates the snapshots. Optional fields are
//! absent (not null) until their level or until they apply, so an output never holds a field it has no value for.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::{ChangeOut, PerfOut, Reading, SpecOut};

// ---------------------------------------------------------------------------------------------------------------
// suggest
// ---------------------------------------------------------------------------------------------------------------

/// `enth.suggest/1`: the coach's hint for a patch against a challenge's target, at one reveal level.
///
/// The fields appear level by level, as in the game: `aspect` and `difference` from level 1, `module` from 2,
/// `param` and `turn` from 3, `range` at 4. **Levels 1 to 3 hold no number but `level`, `levels` and `renders`:
/// the target's values are never printed before the last level, and even then as a range around the fix, not the
/// target's own value.** Asking for a level costs one coach plan (about 25 renders); the same patch gives the same
/// answer at every worker count.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestOut {
    /// `enth.suggest/1`.
    pub schema: String,
    /// The patch file that was judged.
    pub patch: String,
    /// The challenge's code, when it has one (a `--challenge` file made from a custom spec has a long one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The reveal level asked for, 1 to `levels`.
    pub level: u8,
    /// How many levels there are (4): 1 which aspect, 2 which module, 3 which parameter and which way, 4 a range.
    pub levels: u8,
    /// True when nothing measured is off: every level then says so.
    pub close: bool,
    /// True when the hint comes from a verified render of the solver's fix; false for a named structural
    /// difference (a waveform, a mode, a module or a route) or general advice.
    pub measured: bool,
    /// Level 1 on: the aspect that is off (`pitch`, `brightness`, `attack`, `decay`, `sustain`, `release`,
    /// `noisiness`, `beating`, `tremolo` or `sweep`); absent when `close`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    /// Level 1 on: how your sound differs: `higher`, `lower`, `missing` (the target has it, yours does not) or
    /// `extra`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difference: Option<String>,
    /// Level 2 on: the module to look at (`filter1`); absent for a region such as the voice settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Level 3 on: the parameter to change (`filter1.cutoff`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    /// Level 3 on: `raise` or `lower`; absent for a categorical parameter, which is changed, not turned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    /// Level 4: the values the parameter should land between, `[low, high]` as unit strings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Vec<String>>,
    /// The hint in plain language, complete at its level.
    pub text: String,
    /// Renders the coach used for this answer (the target's sensitivity and your patch's probes).
    pub renders: usize,
    /// The next command to try.
    pub hint: String,
}

// ---------------------------------------------------------------------------------------------------------------
// scores (shared with compare)
// ---------------------------------------------------------------------------------------------------------------

/// The three percentages of a round's score and its pass decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScoreOut {
    /// Sound %, 0 to 100: how close it sounds. It alone decides `passed`.
    pub sound: f64,
    /// Structure %, 0 to 100: how close the randomized parameters are.
    pub structure: f64,
    /// Combined %: the challenge's weighted mix of the two.
    pub combined: f64,
    /// The pass mark, in sound %.
    pub pass_mark: u8,
    /// Whether sound % is at or above the pass mark.
    pub passed: bool,
}

// ---------------------------------------------------------------------------------------------------------------
// match
// ---------------------------------------------------------------------------------------------------------------

/// What was matched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatchTargetOut {
    /// `code` (a challenge's target), `patch` or `wav`.
    pub kind: String,
    /// The code, or the file.
    pub source: String,
    /// For a code: the target's fingerprint (64 hex digits), so a log can tell drift from another target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// The parameters the search could move.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FreeOut {
    /// How the set was chosen: `listed` (`--free` addresses and modules), `observable`, `drawn` (the parameters the
    /// randomizer drew; a code target only) or `level N`.
    pub how: String,
    /// How many parameters.
    pub count: usize,
    /// How many are continuous axes (searched by CMA-ES).
    pub continuous: usize,
    /// How many are categorical (a waveform, a mode, a switch: chosen per run).
    pub categorical: usize,
    /// The addresses, in address order.
    pub addrs: Vec<String>,
}

/// The settings of a search that shape its result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatchSettingsOut {
    /// The most renders, the probe included. Hard.
    pub budget: usize,
    /// The search seed.
    pub seed: u64,
    /// Stop as soon as the distance is at or below this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_at: Option<f64>,
    /// A wall-clock cap in seconds. With a cap the result depends on the machine's speed; without one it is
    /// identical for any `--workers`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_seconds: Option<f64>,
}

/// One point of the search's progress.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceStepOut {
    /// Renders used so far.
    pub renders: usize,
    /// `probe`, or `run N` (a CMA-ES run, counted from 0).
    pub stage: String,
    /// The best distance so far.
    pub best: f64,
}

/// How the search went: the points where the best distance improved (thinned to a dozen), or every batch with
/// `--full`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceOut {
    /// How many batches were evaluated.
    pub batches: usize,
    /// The points, in order, ending at the last improvement.
    pub steps: Vec<TraceStepOut>,
}

/// `enth.match/1`: the best patch a search found for a target.
///
/// `distance` is metric v1 (`logmel-l1/1`, ADR-0007) between the target and the best patch, both played with the
/// target's performance: 0 is identical, around 0.06 is a pitch a few cents off, over 1 is a different sound.
/// `score` is the game's own score of the best patch for a **code** target (the scorers the challenge names); for a
/// patch or WAV target only the distance is defined. The output is identical for any `--workers`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatchOut {
    /// `enth.match/1`.
    pub schema: String,
    /// What was matched.
    pub target: MatchTargetOut,
    /// Where the search started: `challenge start`, `init`, or the `--start` file.
    pub start: String,
    /// The parameters that could move.
    pub free: FreeOut,
    /// The best patch's distance to the target.
    pub distance: f64,
    /// The start patch's own distance (absent when the budget ended before it was measured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_distance: Option<f64>,
    /// The distance of the heuristic start (the descriptors' guess for envelope times, pitch and cutoff), when it
    /// changed anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heuristic_distance: Option<f64>,
    /// The game's score of the best patch (a code target only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<ScoreOut>,
    /// The game's score of the start patch (a code target only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_score: Option<ScoreOut>,
    /// Renders used, the probe included (at most the budget).
    pub renders: usize,
    /// CMA-ES runs begun.
    pub runs: usize,
    /// Why it ended: `goal` (`--stop-at` reached), `budget`, `time` (`--max-seconds`) or `exhausted` (nothing more
    /// to try).
    pub stop: String,
    /// The settings that shaped the result.
    pub settings: MatchSettingsOut,
    /// What the best patch changes from the start, in real units.
    pub changes: Vec<ChangeOut>,
    /// The same changes as exact ops: `enth apply <start> --ops '<ops>'` writes the best patch.
    pub ops: Vec<serde_json::Value>,
    /// The file the best patch was written to (`--out`); null when none was.
    pub wrote: Option<String>,
    /// How the search went.
    pub trace: TraceOut,
    /// The next command to try.
    pub hint: String,
}

// ---------------------------------------------------------------------------------------------------------------
// explain
// ---------------------------------------------------------------------------------------------------------------

/// A descriptor a parameter moves, with what it means.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MovesOut {
    /// The descriptor (`spectrum.centroid_hz`), as `describe` and `check` name it.
    pub feature: String,
    /// What that descriptor measures, in a sentence.
    pub meaning: String,
}

/// The catalogue's facts about one parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParamFactsOut {
    /// The key used in addresses (`cutoff`).
    pub key: String,
    /// The unit suffix values are stored in (`Hz`, `ms`, `dB`, `%`, `ct`, `st`); empty for plain numbers, enums and
    /// toggles.
    pub unit: String,
    /// The lowest value (null for enums and toggles).
    pub min: Option<f64>,
    /// The highest value (null for enums and toggles).
    pub max: Option<f64>,
    /// The default: a number in `unit`, an enum key, or a boolean for a toggle.
    pub default: Reading,
    /// `linear`, `log`, `power`, `int`, `enum` or `toggle`.
    pub scale: String,
    /// The enum keys, for enums.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// Example spellings a value may take (`800Hz`, `1.2kHz`).
    pub accepts: Vec<String>,
    /// `basic`, `intermediate` or `advanced`.
    pub tier: String,
    /// Routes may modulate it.
    pub modulatable: bool,
    /// Host automation may drive it.
    pub automatable: bool,
    /// The descriptor it mainly moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves: Option<MovesOut>,
    /// How the value shows in that descriptor, where the two differ.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads_as: Option<String>,
}

/// One parameter of a module, in short.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParamBriefOut {
    /// The key.
    pub key: String,
    /// The display name.
    pub name: String,
    /// The range in real units (`20Hz..20kHz`), or the values (`lp|bp|hp|notch`).
    pub range: String,
    /// The default, in real units.
    pub default: String,
    /// What it does, in one sentence.
    pub effect: String,
    /// The descriptor it mainly moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves: Option<String>,
}

/// What one descriptor did when a parameter moved one just-noticeable step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectOut {
    /// The descriptor.
    pub feature: String,
    /// `higher`, `lower`, `appears` or `vanishes`.
    pub change: String,
    /// Its reading before, in the descriptor's unit (absent when it appears).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Reading>,
    /// Its reading after (absent when it vanishes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Reading>,
}

/// What a probe of one parameter showed in the patch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParamEffectOut {
    /// The address (`filter1.cutoff`).
    pub param: String,
    /// The value before, in real units.
    pub from: String,
    /// The value probed: one just-noticeable step, toward the roomier side of the range.
    pub to: String,
    /// How far the most affected descriptor moved, in units of the smallest difference the game's structure score
    /// tolerates in it: 1 is one tolerance, so a parameter should move several such steps to be heard as a change.
    pub reach: f64,
    /// True when `reach` is under 0.25: no descriptor notices the step in this patch (the parameter does nothing
    /// here, and the game does not compare it).
    pub inert: bool,
    /// The descriptors that moved, most affected first (at most four).
    pub effects: Vec<EffectOut>,
}

/// Measured with the engine on a patch (`--patch`): the response of the descriptors to one step of each parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeasuredOut {
    /// The patch that was probed.
    pub patch: String,
    /// The performance that was played.
    pub perf: PerfOut,
    /// Renders used, the patch's own included.
    pub renders: usize,
    /// One entry per continuous parameter probed (a parameter query: one).
    pub params: Vec<ParamEffectOut>,
    /// Parameters left out: categorical ones, which a step does not describe, and ones the render budget skipped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
}

/// `enth.explain/1`: what a parameter, a module or a module kind does, from the catalogue; with `--patch`, also
/// what it does in that patch, measured.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExplainOut {
    /// `enth.explain/1`.
    pub schema: String,
    /// What was asked: `param`, `module` (a module id in the patch), `kind` or `builtin` (`voice`, `master`).
    pub what: String,
    /// The address, id or kind as resolved (`filter1.cutoff`, `filter1`, `svf`, `voice`).
    pub subject: String,
    /// The module kind (`svf`), or `voice` / `master`.
    pub kind: String,
    /// The display name of the parameter, or of the module kind.
    pub name: String,
    /// The catalogue's sentence for it.
    pub summary: String,
    /// For a parameter: its facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<ParamFactsOut>,
    /// For a module, kind or built-in: its parameters, in short.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamBriefOut>,
    /// The explanation in plain language (what a terminal prints).
    pub text: String,
    /// What the engine measured on `--patch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured: Option<MeasuredOut>,
    /// The next command to try.
    pub hint: String,
}

// ---------------------------------------------------------------------------------------------------------------
// compare
// ---------------------------------------------------------------------------------------------------------------

/// What a patch was compared with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareTargetOut {
    /// `code` (`--code`), `challenge` (`--challenge FILE`), `patch` or `wav` (`--target`).
    pub kind: String,
    /// The code, or the file path.
    pub source: String,
    /// The challenge's code (a challenge file without one: null; null for a patch or a WAV).
    pub code: Option<String>,
    /// The regenerated target's fingerprint (64 hex digits), for a challenge.
    pub fingerprint: Option<String>,
    /// The phrase both were heard through, for a challenge or a patch reference.
    pub phrase: Option<PhraseRefOut>,
    /// The performance both were played with, for a patch or WAV reference (a challenge's is its phrase).
    pub perf: Option<PerfOut>,
}

/// A phrase in the table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhraseRefOut {
    /// Its id (`--phrase 1`).
    pub id: u16,
    /// Its name (`held`, `octave_pair`, `legato_pair`).
    pub name: String,
}

/// One descriptor that differs between the patch and the reference by more than the game tolerates in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DifferenceOut {
    /// The descriptor (`spectrum.centroid_hz`).
    pub feature: String,
    /// The aspect it belongs to (`brightness`, `pitch`, `attack`, `decay`, `sustain`, `release`, `noisiness`,
    /// `beating`, `tremolo`, `sweep`): the coach's words.
    pub aspect: String,
    /// The patch relative to the reference: `higher`, `lower`, `missing` (the reference has it, the patch does not)
    /// or `extra` (the patch has it, the reference does not).
    pub direction: String,
    /// The patch's reading, in the descriptor's unit (null when missing).
    pub ours: Option<Reading>,
    /// The reference's reading (null when extra).
    pub target: Option<Reading>,
    /// How far apart they are, in units of the smallest difference the game tolerates in this descriptor (null for
    /// missing and extra). Only differences of 0.5 or more are listed.
    pub units: Option<f64>,
    /// `high` (4 units or more, or missing or extra), `medium` (2 or more) or `low`.
    pub severity: String,
    /// In words: "ours is brighter than the target".
    pub perceptual: String,
}

/// `enth.compare/2`: a patch against a challenge's target, a reference patch or a reference WAV.
///
/// Version 2 (lane CL) adds `mode`, `target` (which replaces `code` and `fingerprint`), `distance`, `metric` and
/// `differences`, and makes the game's scores optional: a WAV has no nuisance floor to scale them by, so a WAV
/// reference gets a `distance` and the `differences`, and `sound`, `structure`, `combined`, `pass_mark`, `passed`,
/// `margin`, `scorers` and `report` are null or empty. A challenge's `report` no longer holds the target's readings
/// (`--reveal` brings them back): the game never shows them during a round.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareOut {
    /// `enth.compare/2`.
    pub schema: String,
    /// `challenge` (a code or a challenge file), `patch` or `wav`.
    pub mode: String,
    /// The patch scored.
    pub patch: String,
    /// What it was compared with.
    pub target: CompareTargetOut,
    /// Metric v1 (`logmel-l1/1`, the distance `enth match` minimises) between the patch's render and the target's:
    /// 0 is identical, over 1 is a different sound.
    pub distance: f64,
    /// The metric's id.
    pub metric: String,
    /// Sound %, 0 to 100 (decides the pass); null for a WAV reference.
    pub sound: Option<f64>,
    /// Structure %, 0 to 100; null for a WAV reference.
    pub structure: Option<f64>,
    /// Combined %, 0 to 100; null for a WAV reference.
    pub combined: Option<f64>,
    /// The pass mark (sound %); null for a WAV reference.
    pub pass_mark: Option<u8>,
    /// Sound % at or above the pass mark; null for a WAV reference.
    pub passed: Option<bool>,
    /// Sound % minus the pass mark; null for a WAV reference.
    pub margin: Option<f64>,
    /// The scorers that scored it: sound, structure, combined (empty for a WAV reference).
    pub scorers: Vec<String>,
    /// The full score report (`enth.score-report/1`): components, the per-parameter verdicts; null for a WAV
    /// reference. For a challenge it holds no target readings unless `--reveal`.
    pub report: Option<serde_json::Value>,
    /// The descriptors that differ, the most different first (at most eight). Empty for a challenge: the game
    /// shows the direction and closeness in the report, never the target's readings.
    pub differences: Vec<DifferenceOut>,
    /// What it means and what to do next.
    pub hint: String,
}

// ---------------------------------------------------------------------------------------------------------------
// challenge decode
// ---------------------------------------------------------------------------------------------------------------

/// The preset a spec is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresetOut {
    /// Its id in code v1's table (1 to 5: the levels).
    pub id: u8,
    /// Its name (`Level 2`).
    pub name: String,
}

/// `enth.challenge-decode/1`: what a code says, read without regenerating its target (no generation, no render).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChallengeDecodeOut {
    /// `enth.challenge-decode/1`.
    pub schema: String,
    /// The code in its canonical text (upper case, hyphens every four symbols).
    pub code: String,
    /// `short` (a preset and a 40-bit seed) or `long` (any settings, with the engine and a fingerprint prefix).
    pub form: String,
    /// The preset the settings are, or null for custom settings (only a long code carries those).
    pub preset: Option<PresetOut>,
    /// The settings: generator, seed, difficulty, amounts per category, musical, template.
    pub spec: SpecOut,
    /// The categories the amounts are indexed by, in order.
    pub categories: Vec<String>,
    /// The engine a long code was made with (null for a short code).
    pub engine: Option<String>,
    /// Whether that is the engine of this build (null for a short code, which names none).
    pub engine_current: Option<bool>,
    /// A long code's target fingerprint prefix: 8 hex digits that `enth challenge show` checks against the
    /// regenerated target (null for a short code).
    pub target_prefix: Option<String>,
    /// What to do next.
    pub hint: String,
}
