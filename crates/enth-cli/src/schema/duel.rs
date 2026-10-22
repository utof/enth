//! **Owner: lane SKD** (freeze point F2d), then **EV4** (additive). The output types of `enth duel result`
//! (`enth.duel-result/1`) and `enth duel verify` (`enth.duel-verify/1`). Scores are "checked on your computer" (this
//! build re-scored them); times and assists are "reported" (the sender's app's, never checked).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ScoreOut;

/// `enth.duel-result/1`: a played patch's two codes. The **scorecard** spoils nothing and is sent when you finish; the
/// **answer code** gives the patch away and is sent once your friend has played.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuelResultOut {
    /// `enth.duel-result/1`.
    pub schema: String,
    /// The challenge code (canonical text).
    pub code: String,
    /// The patch file played.
    pub patch: String,
    /// The answer's score on this build: the canonical answer (the patch without its `meta`, labels, macro names and
    /// slots), which is what a receiver re-scores. Checked on your computer.
    pub score: ScoreOut,
    /// The share message, one line each: the two human lines, then the scorecard.
    pub message: Vec<String>,
    /// The scorecard (kind 1): send it now.
    pub scorecard: String,
    /// The answer code (kind 2): send it after your friend has played.
    pub answer: String,
    /// Symbols in each code (code v1's check symbol included, hyphens not).
    pub symbols: DuelSymbolsOut,
    /// The answer code's salt, 16 hex digits: 64 bits of OS randomness, or what `--salt` gave. Keep it with the codes:
    /// `--salt` makes the same answer code again, the one the scorecard's commitment fixed.
    pub salt: String,
    /// What the scorecard reports and nobody can check: the submits, the time, the hints.
    pub reported: DuelReportedOut,
    /// Differences that did not stop the codes (another engine than a long code's).
    pub warnings: Vec<String>,
    /// What to do next.
    pub next: String,
}

/// Symbols in a code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuelSymbolsOut {
    /// The scorecard (51 with a short challenge).
    pub scorecard: usize,
    /// The answer code.
    pub answer: usize,
}

/// What a scorecard reports: the sender's app's numbers, never checked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuelReportedOut {
    /// Submits up to the compared one (1 to 127; 127 means "127 or more").
    pub submits: u8,
    /// Seconds: the time to pass, or the round's active time without a pass (tenths; at most 6553.5, "or more").
    pub seconds: f64,
    /// Hints used (15 means "15 or more").
    pub hints: u8,
    /// The live meter, "dim unused", "How close am I?", a reveal after the pass.
    pub assists: Vec<String>,
}

/// `enth.duel-verify/1`: every code in the text, and each answer code re-scored on this build against the scorecard
/// sent before it. **This computer's scores decide**; a claim is only compared.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuelVerifyOut {
    /// `enth.duel-verify/1`.
    pub schema: String,
    /// Every code that decoded, in order.
    pub found: Vec<DuelFoundOut>,
    /// Code-looking tokens that did not decode, with why.
    pub refused: Vec<DuelRefusedOut>,
    /// The scorecards, as reported by their app.
    pub cards: Vec<DuelCardOut>,
    /// Each answer code, checked on this computer.
    pub checks: Vec<DuelCheckOut>,
    /// Where `--out` wrote the answer's patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrote: Option<String>,
    /// What to do next.
    pub next: String,
}

/// A code found in the text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuelFoundOut {
    /// `challenge`, `scorecard`, `answer`, `ear_challenge` or `ear_result`.
    pub kind: String,
    /// Its canonical text.
    pub code: String,
}

/// A token that looked like a code and did not decode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuelRefusedOut {
    /// The token, as read.
    pub text: String,
    /// The stable error code (`bad_check`, `code_too_new`, `answer_too_new`, …).
    pub error: String,
    /// What to do about it.
    pub hint: String,
}

/// A scorecard, as reported by the sender's app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuelCardOut {
    /// The challenge played.
    pub challenge: String,
    /// The two human lines.
    pub lines: Vec<String>,
    /// The claimed sound %, as reported.
    pub sound: f64,
    /// The claimed combined %.
    pub combined: f64,
    /// Whether it claims a pass.
    pub passed: bool,
    /// The engine the claim was scored on.
    pub engine: String,
    /// The engine this build runs.
    pub engine_here: String,
    /// The claim's scorer set, by its index.
    pub scorers: u8,
    /// Whether this build has that scorer set.
    pub scorers_known: bool,
    /// The reported submits, time and assists.
    pub reported: DuelReportedOut,
}

/// One answer code, checked on this computer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuelCheckOut {
    /// The challenge it was played on.
    pub challenge: String,
    /// Symbols in the answer code.
    pub symbols: usize,
    /// The ops it carries.
    pub ops: usize,
    /// Their answer's score here: checked on your computer.
    pub theirs: ScoreOut,
    /// The scorer set it was scored with (its index).
    pub scorers: u8,
    /// The scorecard's claim beside it, when a scorecard for the same challenge was in the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<DuelClaimOut>,
    /// Your patch (`--mine`) scored as its canonical answer on the same prepared round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<ScoreOut>,
    /// Differences that did not stop the check (a long challenge made on another engine).
    pub warnings: Vec<String>,
}

/// A claim beside this computer's re-score. Percentages are in tenths, rounded as the scorecard rounds them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuelClaimOut {
    /// The claimed sound %.
    pub claimed_sound: f64,
    /// The sound % here, rounded to tenths.
    pub sound: f64,
    /// The claimed combined %.
    pub claimed_combined: f64,
    /// The combined % here, rounded to tenths.
    pub combined: f64,
    /// Whether it claims a pass.
    pub claimed_passed: bool,
    /// Whether it passes here.
    pub passed: bool,
    /// The same numbers and pass with the claim's own scorers: "as reported".
    pub as_reported: bool,
    /// The claim was scored on another engine version: both numbers are shown.
    pub engine_differs: bool,
    /// The claim was scored with a scorer set this build lacks: checked with this build's, both numbers shown.
    pub scorers_differ: bool,
    /// The answer is the one the scorecard's salted commitment fixed. False: it was changed after the scorecard
    /// (flagged, never refused).
    pub commitment_matches: bool,
}
