//! **Owner: lane SG** (the session game API, freeze point F1r). Pre-declared by lane S0r (R1/S1).
//!
//! The pure Recreate round (docs/session.md, "The Recreate round"; design.md §9, "The round: A/B and feedback";
//! report 01 §2.6). Like the rest of the session it does no I/O, spawns no threads and reads no clocks: **time comes
//! from timestamps the shell passes in** ([`Session::game_tick`]), and **background work runs as [`Job`]s the shell
//! executes** on its own worker and hands back ([`Session::game_job_done`]). The audio thread never touches any of it.
//!
//! # The lifecycle
//!
//! | Phase | Entered by | The patch | Leaves by |
//! | --- | --- | --- | --- |
//! | (no round) | — | the player's own | [`GameEdit::game_start`] |
//! | [`Phase::Preparing`] | `game_start` (a [`JobKind::Prepare`] runs) | unchanged | [`JobResult::Prepared`]: the player's patch and history are stashed, the start patch loads |
//! | [`Phase::Playing`] | the target is ready | the start, then the player's edits | `game_submit` and its [`JobResult::Scored`] |
//! | [`Phase::Submitted`] | a scored submit that did not pass | the player's | a passing submit |
//! | [`Phase::Passed`] | the first passing submit (it sets the time to pass) | the player's | — (keep tweaking, vote, end) |
//! | [`Phase::Revealed`] | `game_reveal` (gives up; assisted) | the target | — (vote, end) |
//! | [`Phase::Broken`] | the target could not be prepared or regenerated | unchanged | `game_end` only |
//!
//! From any phase: [`GameEdit::game_end`] ([`EndChoice::Restore`] brings back the stashed patch and its undo
//! history, [`EndChoice::Keep`] keeps the round's patch), or `game_start` again (a new round keeps the original
//! stash). Hints ([`GameEdit::game_hint`]), a reveal and the editor's dim assist ([`GameEdit::game_dim_unused`]) mark
//! the round assisted. The agreement vote
//! ([`GameEdit::game_vote`]) and a dispute ([`GameEdit::game_dispute`]) need a scored submit. Each round that was
//! played ends with one [`RoundLogEntry`] in the outbox ([`GameRequest::Log`]); lane GK writes the exporter.
//!
//! # What is saved
//!
//! The `enth` field's `game` object ([`GameState`]) holds the [`RoundRecord`]: the challenge code (the target is
//! never persisted: it is regenerated from the code and checked against [`RoundRecord::target_hash`], the target's
//! fingerprint), the stash (the player's own patch, as patch-file text), the active time, the scored submits as
//! integers, hints, the blind rating, the vote and the edit log. It is absent while no round is live, so a state
//! without a round keeps its bytes, and it stays within `enth.plugin-state/1` (optional fields with defaults;
//! docs/session.md, "The saved state"). It is read tolerantly: an unknown enum value reads as `unknown`, and a
//! `game` object this build cannot read at all keeps its round's stash ([`GameState::lenient`]), so a newer build's
//! round fails only itself, never the editor preferences or the player's patch. Every part is budgeted in escaped
//! bytes ([`ROUND_BYTES`]), so a round never makes the `enth` field unloadable.
//!
//! # Lane CT's types
//!
//! A round starts from settings or a code ([`RoundSource`]) and keeps the **code text** it started from, so a long
//! code keeps its maker's engine version (docs/challenge.md). The [`Challenge`] itself comes back from the prepare
//! job, because the phrase is the generator's pick (`Challenge::recreate(spec, phrase)`); the shell's worker builds
//! the job's [`Prepared`] from CT's `prepare_round` ([`Prepared::from_round`]). The session checks the regenerated
//! target itself, by its fingerprint ([`TargetFingerprint`], schema-independent): [`ChallengeCode::verify`] (a long
//! code's prefix refuses, another engine version warns) and, on a resumed round, the full fingerprint it saved. It
//! checks each worker report against the challenge too (scorer ids, pass mark, the pass recomputed from the sound
//! %). Scores are CT's [`ScoreReport`]; the round log carries CT's [`RoundResult`] (`enth.round-result/1`).
//!
//! # Lesson rounds (lane SL, R2)
//!
//! A round may also start from a given [`Challenge`] ([`RoundSource::Challenge`], additive), which the worker
//! prepares with `enth_challenge::prepare_challenge` ([`JobKind::PrepareChallenge`]). A lesson's Recreate step starts
//! one through the session's `learn` module: its record and its log line carry [`RoundOrigin::Lesson`] (`"origin":
//! "lesson"`, omitted for a game round, so `gr1-report` can leave lesson rounds out), and it **never stashes**: the
//! `learn` object holds the only stash, so a lesson and its round fit the `enth` field together. A lesson round has
//! no code unless its challenge is a seed challenge; after a reload it waits for the lesson to reopen and hand its
//! challenge back. While a lesson is live, the editor's `game_start` is refused ([`GameError::LessonLive`]), and so is
//! `game_end` on the lesson's round ([`GameError::LessonRound`]): the lesson's Next, Back and End move it.

use std::sync::Arc;

use enth_challenge::model::RATING_MAX;
pub use enth_challenge::{
    Agreement, AssistsUsed, Challenge, ChallengeCode, CodeError, CodeWarning, Direction,
    EngineVersion, Outcome, PreparedRound, RandomSpec, RoundResult, ScoreReport, SpecError,
    StructureScore, TargetFingerprint, Verdict,
};
use enth_patch::history::{GestureId, History};
use enth_patch::{MacroIndex, ModuleId, Op, ParamAddr, Patch, RouteAddr};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bridge::host_layout_changed;
use crate::model::{Session, ShellRequest};
use crate::state::{StateError, decode_patch};

/// After the last edit, how long the patch must stay unchanged before the session asks for Mine's loudness (A/B
/// gains) and, after the first submit, the live rings (report 01 §2.6: about 300 ms).
pub const SETTLE_MS: u64 = 300;

/// How many hint levels a round offers: the worst descriptor, the module, the direction, then the move itself
/// (report 02 §2.8).
pub const HINT_LEVELS: u8 = 4;

/// The most scored submits one round records (a bound on the saved state, not a game rule anyone should reach).
pub const MAX_SUBMITS: usize = 255;

// The saved round's budget (F1r review S8). Every part that is text inside the record is counted as it is saved:
// escaped, inside the `enth` field's JSON (a patch's quotes double it). The parts add up to less than ROUND_BYTES,
// which leaves the rest of the `enth` field room under MAX_FIELD_BYTES (checked below and by test).

/// The most bytes the edit log keeps (its entries' JSON); later entries are counted in
/// [`RoundRecord::edits_dropped`] instead, and the whole log goes to the round's `end` line
/// ([`RoundLogEntry::edit_log`]). Every save and every host undo point carries it, so it stays small.
pub const EDIT_LOG_BYTES: usize = 64 * 1024;

/// The largest stash, escaped as saved: the player's own patch text (a realistic patch is about 20 KB, about 30 KB
/// escaped). A round whose stash would not fit is refused at its start ([`GameError::TooLarge`]), so the player's
/// patch is never lost to an unloadable save.
pub const STASH_BYTES: usize = 384 * 1024;

/// The longest submitted patch the record keeps for the round log, escaped as saved (omitted above it).
pub const MAX_SUBMITTED_PATCH_BYTES: usize = 128 * 1024;

/// The longest first report the record keeps, escaped as saved (omitted above it; a report is a few KB).
pub const FIRST_REPORT_BYTES: usize = 64 * 1024;

/// The longest dispute note kept, in characters (at most 6 bytes each once escaped).
pub const DISPUTE_CHARS: usize = 2000;

/// The budget for the whole saved round: the parts above plus the submits and the record's own fields. Well under
/// the `enth` field's [`MAX_FIELD_BYTES`](crate::state::MAX_FIELD_BYTES): a round never makes the saved state
/// unloadable (the largest round is a test).
pub const ROUND_BYTES: usize = 768 * 1024;

/// The record's fields besides the budgeted texts and the submits: id, code, phase, fingerprint, times, flags.
const RECORD_FIXED_BYTES: usize = 2 * 1024;

/// One submit's JSON at its largest.
const SUBMIT_BYTES: usize = 128;

const _: () = assert!(
    STASH_BYTES
        + MAX_SUBMITTED_PATCH_BYTES
        + FIRST_REPORT_BYTES
        + EDIT_LOG_BYTES
        + 6 * DISPUTE_CHARS
        + MAX_SUBMITS * SUBMIT_BYTES
        + RECORD_FIXED_BYTES
        <= ROUND_BYTES
);
const _: () = assert!(ROUND_BYTES + 64 * 1024 < crate::state::MAX_FIELD_BYTES);

/// The bytes `text` takes saved as a JSON string: its escaped form with the quotes.
pub fn escaped_len(text: &str) -> usize {
    serde_json::to_string(text).map_or(usize::MAX, |t| t.len())
}

/// The schema of a [`RoundLogEntry`] line.
pub const ROUND_LOG_SCHEMA: &str = "enth.round-log/1";

// ---------------------------------------------------------------------------------------------------------------
// Reading lane CT's results
// ---------------------------------------------------------------------------------------------------------------

/// The integer summary the round saves of a report (the only place the session reads its scores).
pub fn summary(report: &ScoreReport) -> ScoreSummary {
    ScoreSummary::from_percent(
        report.passed,
        report.sound.percent,
        report.structure.percent,
        report.combined.percent,
    )
}

/// The rings a structure score gives: one per row, in its order (the randomized parameters first), on the
/// **player's** control (`answer_addr`: once modules are added and removed, the player's `osc2` may be the target's
/// `osc1`). A row with no match in the player's patch (`direction: missing`), an address this build cannot parse (a
/// report from a newer catalogue) or a verdict from a newer build has no knob to ring and is skipped. A report
/// without answer addresses (written before they existed) rings the target's address.
pub fn knobs(structure: &StructureScore) -> Vec<KnobScore> {
    structure
        .params
        .iter()
        .filter(|p| p.verdict != Verdict::Unknown)
        .filter_map(|p| {
            let addr = match &p.answer_addr {
                Some(text) => text.parse().ok()?,
                None if p.direction == enth_challenge::Direction::Missing => return None,
                None => p.param_addr().ok()?,
            };
            Some(KnobScore {
                addr,
                verdict: p.verdict,
                randomized: p.randomized,
            })
        })
        .collect()
}

/// Every ring a structure score gives (lane RUI, additive beside [`knobs`]): one per row with a control in the
/// player's patch, in its order, **including the route and enable-flag rows** that [`knobs`] skips. A row's
/// `answer_addr` (the player's control) is read as lane ST writes it: a route as [`RouteAddr`]'s text
/// (`lfo1 -> filter1.cutoff`), an enable flag as `<id>.enabled`, anything else as a [`ParamAddr`]. A row with no
/// control in the player's patch (`answer_addr` absent: a missing route or module), text this build cannot parse or
/// a verdict from a newer build is skipped. A report without answer addresses rings the target's address, as
/// [`knobs`] does.
pub fn rings(structure: &StructureScore) -> Vec<Ring> {
    structure
        .params
        .iter()
        .filter(|p| p.verdict != Verdict::Unknown)
        .filter_map(|p| {
            let text = match &p.answer_addr {
                Some(text) => text.as_str(),
                None if p.direction == enth_challenge::Direction::Missing => return None,
                None => p.addr.as_str(),
            };
            Some(Ring {
                at: RingAt::parse(text)?,
                verdict: p.verdict,
                randomized: p.randomized,
            })
        })
        .collect()
}

/// Where a round starts from ([`GameEdit::game_start`]). Not `Copy` since lane SL added
/// [`RoundSource::Challenge`] (a challenge owns patches).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoundSource {
    /// The settings page: the shell drew the seed (a short code holds 40 bits of it). A preset's spec has its short
    /// code at once; a custom spec gets its long code when the target exists (it carries the target's fingerprint prefix).
    Settings(RandomSpec),
    /// A code the player pasted, or a daily's.
    Code(ChallengeCode),
    /// A given challenge (lane SL, R2; additive): a lesson's Recreate step, or a challenge file. The worker prepares it
    /// with `enth_challenge::prepare_challenge` ([`JobKind::PrepareChallenge`]); the session checks that the prepared
    /// challenge is this one. A seed challenge gets its code when the target exists; any other target has none, so
    /// such a round outside a lesson does not survive a project reload (it restores [`Phase::Broken`], its stash kept;
    /// a lesson resumes its round from the lesson instead).
    Challenge(Box<Challenge>),
}

/// Who started a round (lane SL, R2). Saved in the record and written on the round-log line only when it is not
/// [`RoundOrigin::Game`], so a game round keeps its bytes and older lines read as game rounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundOrigin {
    /// The Recreate game: from settings, a code or a challenge file. Counts for GR1.
    #[default]
    Game,
    /// A lesson's Recreate step (the session's `learn` module). Never stashes; `gr1-report` leaves it out.
    Lesson,
    /// A Review item (lane REV, R2; additive): a seeded sibling of a passed lesson's Recreate step, started from the
    /// lesson browser (`crate::review`). It stashes like a game round, gives no hint before its first submit, and
    /// `gr1-report` leaves it out (its origin is not `game`).
    Review,
    /// An origin from a newer build: not a game round either.
    #[serde(other)]
    Other,
}

impl RoundOrigin {
    /// The default origin, which the saved record and the log line omit.
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes the field by reference"
    )]
    pub fn is_game(&self) -> bool {
        *self == RoundOrigin::Game
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The saved state
// ---------------------------------------------------------------------------------------------------------------

/// The instance's game data, as the `enth` field saves it ([`InstanceState::game`](crate::InstanceState::game)).
///
/// The session keeps the field `None` while it is empty (no round is live), so a state without a round saves
/// exactly the bytes it saved before R1. `GameState::default()` (no round) serialises as `{}`; later optional fields
/// (the randomizer's last settings, an auto-difficulty staircase, a seed history) can join it within version 1, and
/// the round's entries touch only [`round`](GameState::round), never them (F1r review N3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameState {
    /// The round in progress, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<RoundRecord>,
}

impl GameState {
    /// Nothing to save: every field is at its default.
    pub fn is_empty(&self) -> bool {
        *self == GameState::default()
    }

    /// Read a saved `game` object without ever failing (F1r review M3): the state if it decodes; otherwise, when it
    /// holds a round whose stash is text, that round restored [`Phase::Broken`] with its id, code and stash (so
    /// ending it brings the player's patch back); otherwise nothing. A newer build's round thus fails only itself,
    /// never the `enth` field and the editor preferences in it.
    pub fn lenient(value: &Value) -> Option<GameState> {
        if let Ok(state) = GameState::deserialize(value) {
            return Some(state);
        }
        let round = value.get("round")?;
        let stash = round.get("stash")?.as_str()?.to_owned();
        let id = round
            .get("id")
            .and_then(|v| serde_json::from_value::<RoundId>(v.clone()).ok())
            .unwrap_or_default();
        let code = round
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Some(GameState {
            round: Some(RoundRecord {
                id,
                code,
                phase: Phase::Broken,
                stash: Some(stash),
                ..RoundRecord::default()
            }),
        })
    }
}

/// The `enth` field's `game` reader ([`GameState::lenient`]): never fails, so the rest of the field always loads.
pub(crate) fn lenient_game<'de, D>(deserializer: D) -> Result<Option<GameState>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.as_ref().and_then(GameState::lenient))
}

/// One round as it is saved: everything needed to resume it after the project is reopened, except the target
/// (regenerated from [`code`](RoundRecord::code)) and what the shell's jobs recompute (the target's audio, Mine's
/// loudness, the live rings, a hint's text, the full score report).
///
/// Integers and text only, so its bytes are deterministic and it is `Eq`. Every field has a default and empty ones
/// are omitted, so a newer build's extra fields are ignored and missing ones default (state version 1). Every enum
/// in it reads a newer build's value as `unknown` (`Phase::Unknown`, `EditSource::Other`, `Agreement::Unknown`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoundRecord {
    /// Which round this is: a host load of the same round keeps the live progress.
    pub id: RoundId,
    /// The text of the code the round started from (a long code keeps its maker's engine version). The target is
    /// regenerated from it, never persisted (design.md §9). Empty only while a round started from custom settings
    /// is still preparing (its long code carries the target's fingerprint prefix, so it exists with the target).
    pub code: String,
    /// Where the round is.
    pub phase: Phase,
    /// The player's own patch from before the first round, as patch-file text (exactly what the `patch` field
    /// held, so a patch this build could not read survives too). `None` until the first round's target is ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash: Option<String>,
    /// The target's fingerprint (64 hex digits, [`TargetFingerprint`]: schema-independent, so a patch schema bump
    /// never breaks a saved round), set when the target is first prepared; a regenerated target must match it, or
    /// the round is [`Phase::Broken`] (`target_drift`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_hash: Option<String>,
    /// Active time in the round, in milliseconds: counted only while the editor is visible and the target is ready.
    pub active_ms: u64,
    /// Every scored submit, oldest first. The first is the recorded score.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub submits: Vec<SubmitRecord>,
    /// The active time of the first passing submit (the time to pass).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_pass_ms: Option<u64>,
    /// Hint levels used (0 = none). Any hint marks the round assisted.
    pub hints: u8,
    /// The target was revealed (gave up). Marks the round assisted.
    pub revealed: bool,
    /// The editor's `dim_unused` assist (Challenge v1's: the modules the target leaves alone are dimmed) was on at
    /// some time while the round could be played ([`GameEdit::game_dim_unused`]; lane HINT, FU1). Marks the round
    /// assisted. Omitted when false, so a round without it keeps its bytes and an older record reads with it off.
    #[serde(skip_serializing_if = "is_false")]
    pub dim_unused: bool,
    /// The blind rating (1–5) of the first submit, given before its score showed (F1r review S9).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    /// The agreement vote, once given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agreement: Option<Agreement>,
    /// The player's dispute note, once given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispute: Option<String>,
    /// The first submit's full [`ScoreReport`] (the recorded score), as its own versioned JSON text
    /// (`enth.score-report/1`), for the round's [`RoundResult`]. Text, so the record holds no float and a report
    /// schema this build cannot read never makes the `enth` field unloadable. Omitted above
    /// [`FIRST_REPORT_BYTES`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_report: Option<String>,
    /// The patch of the last scored submit, as patch-file text, for the round log (omitted when larger than
    /// [`MAX_SUBMITTED_PATCH_BYTES`], escaped).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted_patch: Option<String>,
    /// The edit log: every change to the round's patch, stamped with active time.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<EditRecord>,
    /// Log entries not kept because the log reached its byte budget.
    pub edits_dropped: u32,
    /// Who started the round (lane SL, R2): omitted for a game round, so a game round keeps its bytes. A lesson round
    /// ([`RoundOrigin::Lesson`]) never stashes, and after a reload it waits for its lesson to hand the challenge back.
    #[serde(skip_serializing_if = "RoundOrigin::is_game")]
    pub origin: RoundOrigin,
}

/// `skip_serializing_if` for a flag the record writes only when set.
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl RoundRecord {
    /// The help used, in CT's terms (R1 has no live meter yet).
    pub fn assists(&self) -> AssistsUsed {
        AssistsUsed {
            hints: self.hints,
            live_meter: false,
            revealed: self.revealed,
            dim_unused: self.dim_unused,
        }
    }

    /// Hints, a reveal or the dim assist were used.
    pub fn assisted(&self) -> bool {
        self.assists().any()
    }

    /// The target was ready at least once: the player has played this round (it gets a log entry when it ends).
    pub fn played(&self) -> bool {
        self.target_hash.is_some()
    }

    /// How the round stands or ended, in CT's terms: passed, gave up (revealed before a pass) or not passed.
    pub fn outcome(&self) -> Outcome {
        if self.time_to_pass_ms.is_some() {
            Outcome::Passed
        } else if self.revealed {
            Outcome::GaveUp
        } else {
            Outcome::NotPassed
        }
    }

    /// The round's [`RoundResult`] so far. `None` when its code does not decode (a broken round restored from a
    /// newer build). The first report is read from its saved text; if that is unreadable too, the result counts no
    /// submit, so it stays a valid `enth.round-result/1`, and the log line's own summaries keep the scores.
    pub fn result(&self) -> Option<RoundResult> {
        let code = ChallengeCode::decode(&self.code).ok()?;
        let first_submit = self
            .first_report
            .as_deref()
            .and_then(|text| serde_json::from_str::<ScoreReport>(text).ok());
        let submits = if first_submit.is_some() {
            self.submits.len() as u32
        } else {
            0
        };
        Some(RoundResult {
            code,
            outcome: self.outcome(),
            first_submit,
            submits,
            time_to_pass_ms: self.time_to_pass_ms,
            active_ms: self.active_ms,
            assists: self.assists(),
            agreement: self.agreement,
            rating: self.rating,
        })
    }
}

/// A round's identity within an instance: a serial the session counts up and the shell's timestamp at the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct RoundId {
    /// Rounds started in this instance (restored rounds keep theirs; the next round counts on from the highest).
    pub serial: u32,
    /// The shell's timestamp when the round started.
    pub nonce: u64,
}

/// Where a round is ([module docs](self)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The target is being generated and rendered ([`JobKind::Prepare`]); the player's patch still plays.
    #[default]
    Preparing,
    /// The target is ready and the start patch loaded; no scored submit yet.
    Playing,
    /// At least one scored submit, none passed; the rings update live.
    Submitted,
    /// A submit passed; the time to pass is recorded.
    Passed,
    /// The target was revealed.
    Revealed,
    /// The target could not be prepared or regenerated (a failed job, a bad code, a different hash). Only
    /// [`GameEdit::game_end`] is left; the stash is kept.
    Broken,
    /// A phase from a newer build: restored as [`Phase::Broken`].
    #[serde(other)]
    Unknown,
}

impl Phase {
    /// The player can edit, submit and ask for hints (`Playing`, `Submitted`, `Passed`).
    pub fn is_open(self) -> bool {
        matches!(self, Phase::Playing | Phase::Submitted | Phase::Passed)
    }

    /// The round's patch is in play (every phase but `Preparing`, `Broken` and `Unknown`).
    pub fn is_live(self) -> bool {
        self.is_open() || self == Phase::Revealed
    }
}

/// One scored submit, as integers: scores in hundredths of a percent (0–10000), times in active milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct SubmitRecord {
    /// The active time when the player pressed Submit (not when the score arrived).
    pub at_ms: u64,
    /// The scores.
    #[serde(flatten)]
    pub score: ScoreSummary,
}

/// The three scores and the pass flag of one submit, in hundredths of a percent (0–10000), so the saved state
/// never holds a float.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ScoreSummary {
    /// Sound decided a pass.
    pub passed: bool,
    /// Sound %, × 100.
    pub sound: u16,
    /// Structure %, × 100.
    pub structure: u16,
    /// Combined %, × 100.
    pub combined: u16,
}

impl ScoreSummary {
    /// From percentages (0–100; clamped, a NaN counts as 0), rounded to hundredths.
    pub fn from_percent(passed: bool, sound: f64, structure: f64, combined: f64) -> Self {
        fn hundredths(pct: f64) -> u16 {
            if pct.is_nan() {
                return 0;
            }
            (pct.clamp(0.0, 100.0) * 100.0).round() as u16
        }
        ScoreSummary {
            passed,
            sound: hundredths(sound),
            structure: hundredths(structure),
            combined: hundredths(combined),
        }
    }
}

/// One entry of the edit log.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditRecord {
    /// The round's active time of the change (a drag merged into one entry has the time of its last move).
    pub t_ms: u64,
    /// What changed the patch.
    pub source: EditSource,
    /// The ops in their resolved wire form (`enth_patch::wire::to_json`), so replaying the log from the start patch
    /// with `enth_patch::wire::resolve` gives the same patches. Empty for a whole-patch change (a reveal, a host
    /// load): replay continues from the target, or stops.
    pub ops: Value,
}

/// What changed the round's patch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditSource {
    /// An editor edit, undo or redo (anything delivered through the session's edit path).
    #[default]
    Editor,
    /// Host automation folded into the patch (only slotted parameters and macros).
    Host,
    /// The host loaded a patch (for example REAPER's own Undo); `ops` is empty.
    Load,
    /// The player revealed the target; `ops` is empty.
    Reveal,
    /// A source from a newer build.
    #[serde(other)]
    Other,
}

// ---------------------------------------------------------------------------------------------------------------
// Feedback types
// ---------------------------------------------------------------------------------------------------------------

/// One knob's ring after the first submit (CT's [`Verdict`]: Good / Near / Bad, Syntorial-style; the editor's
/// `Overlay` draws it).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KnobScore {
    /// The player's control.
    pub addr: ParamAddr,
    /// Its verdict.
    pub verdict: Verdict,
    /// The target randomized it; `false` for a control the player moved that the target left alone.
    pub randomized: bool,
}

/// Where a ring goes in the player's patch ([`rings`]; lane RUI, additive).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RingAt {
    /// A parameter's control (a knob, a switch, a segmented control).
    Param(ParamAddr),
    /// A modulation route (`lfo1 -> filter1.cutoff`): its matrix cell or route-list row.
    Route(RouteAddr),
    /// A module's enable toggle (a row `<id>.enabled`).
    Enable(ModuleId),
}

impl RingAt {
    /// Reads a structure row's address as lane ST writes it: a route (`src -> dst`), an enable flag
    /// (`<id>.enabled`) or a parameter. `None` for text this build cannot parse.
    pub fn parse(text: &str) -> Option<RingAt> {
        if text.contains("->") {
            return text.parse().ok().map(RingAt::Route);
        }
        if let Some(id) = text.strip_suffix(".enabled") {
            return ModuleId::new(id).ok().map(RingAt::Enable);
        }
        text.parse().ok().map(RingAt::Param)
    }
}

/// One ring after the first submit, on a parameter, a route or a module's enable toggle ([`rings`]; lane RUI,
/// additive beside [`KnobScore`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ring {
    /// The player's control.
    pub at: RingAt,
    /// Its verdict.
    pub verdict: Verdict,
    /// The target randomized it; `false` for a control the player moved that the target left alone.
    pub randomized: bool,
}

/// A coach hint, as the shell's hint job returns it (lane SN fills it from `enth-solver`'s coach).
///
/// Besides the text, the structured fields say what the hint is about, so the editor can mark the control (lane HINT,
/// FU1). They follow the coach's level ladder (docs/dsp/hints.md, "The coach"): each is filled only from the level
/// whose text already says it, and **none ever holds a value** (no target value, no fix, no range: the plan's verified
/// fix stays in the worker). All are additive and serde-default, so a hint without them reads as before.
///
/// | Field | From level | What |
/// | --- | --- | --- |
/// | [`aspect`](Self::aspect), [`difference`](Self::difference) | 1 | the aspect that is off (the coach's aspect code: `brightness`, `attack`…) and how the player's sound differs |
/// | [`module`](Self::module) | 2 | the module to look at (none for a region: the voice settings, the routes, advice) |
/// | [`param`](Self::param), [`turn`](Self::turn) | 3 | the parameter, and which way to turn it (no turn for a categorical parameter or a route) |
/// | [`route`](Self::route) | 4 | a route hint's route, source and destination (level 3 names only its destination, as `param`) |
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CoachHint {
    /// The level it answers (1 = the worst descriptor … [`HINT_LEVELS`] = the move itself).
    pub level: u8,
    /// What to show.
    pub text: String,
    /// Level 1 on: the aspect that is off, as the coach's stable code (`enth_solver::coach::Aspect::code`). Text, so a
    /// newer build's aspect never fails the hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    /// Level 1 on: how the player's sound differs from the target's in that aspect (`higher`, `lower`, `missing`,
    /// `extra`; a newer build's reads as `unknown`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difference: Option<Direction>,
    /// Level 2 on: the module to look at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<ModuleId>,
    /// Level 3 on: the parameter to change (a route hint's destination).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<ParamAddr>,
    /// Level 3 on: which way to turn [`param`](Self::param), when the text says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<HintTurn>,
    /// Level 4 of a route hint: the route to add, remove or re-weigh (never its depth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteAddr>,
}

/// Which way a hint says to turn a control ([`CoachHint::turn`]; lane HINT, FU1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HintTurn {
    /// Up: a larger plain value ("raise").
    Raise,
    /// Down: a smaller plain value ("lower").
    Lower,
    /// A turn from a newer build (read tolerantly; never written by this one).
    #[serde(other)]
    Unknown,
}

impl CoachHint {
    /// A hint at `level` saying `text`, with no structured field.
    pub fn new(level: u8, text: impl Into<String>) -> Self {
        CoachHint {
            level,
            text: text.into(),
            ..CoachHint::default()
        }
    }

    /// With the aspect that is off (its code) and how the player's sound differs (level 1 on).
    #[must_use]
    pub fn with_aspect(mut self, aspect: impl Into<String>, difference: Option<Direction>) -> Self {
        self.aspect = Some(aspect.into());
        self.difference = difference;
        self
    }

    /// With the module to look at (level 2 on).
    #[must_use]
    pub fn with_module(mut self, module: ModuleId) -> Self {
        self.module = Some(module);
        self
    }

    /// With the parameter to change, and which way when the text says so (level 3 on).
    #[must_use]
    pub fn with_param(mut self, param: ParamAddr, turn: Option<HintTurn>) -> Self {
        self.param = Some(param);
        self.turn = turn;
        self
    }

    /// With a route hint's route (level 4).
    #[must_use]
    pub fn with_route(mut self, route: RouteAddr) -> Self {
        self.route = Some(route);
        self
    }

    /// The hint with every structured field its level does not reveal yet dropped (the coach's ladder, the table
    /// above): the session keeps hints this way, so a shell that filled too much can never make the editor mark a
    /// control before the text names it.
    #[must_use]
    pub fn laddered(mut self) -> Self {
        if self.level < 1 {
            self.aspect = None;
            self.difference = None;
        }
        if self.level < 2 {
            self.module = None;
        }
        if self.level < 3 {
            self.param = None;
            self.turn = None;
        }
        if self.level < HINT_LEVELS {
            self.route = None;
        }
        self
    }

    /// Whether it points at anything the editor can mark (a module, a parameter or a route).
    pub fn points(&self) -> bool {
        self.module.is_some() || self.param.is_some() || self.route.is_some()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// A/B
// ---------------------------------------------------------------------------------------------------------------

/// What the A/B switch plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Listen {
    /// Neither: the keyboard plays Mine as usual.
    #[default]
    Off,
    /// **Target ▶**: the pre-rendered, loudness-matched loop of the scored phrase (the shell's buffer).
    Target,
    /// **Mine ▶**: the same phrase played live through the player's engine, locked to the loop's position.
    Mine,
}

impl Listen {
    /// The other side of an A/B flip (`Off` flips to `Target`).
    #[must_use]
    pub fn flipped(self) -> Listen {
        match self {
            Listen::Target => Listen::Mine,
            Listen::Mine | Listen::Off => Listen::Target,
        }
    }
}

/// The A/B state the shell's audio side follows: which side plays and the loudness-match gains. The louder side is
/// attenuated, never the quieter one boosted (design.md §9), so both gains are at most 0 dB.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AbState {
    /// The switch.
    pub listen: Listen,
    /// The target's loop is rendered (the session has the prepared target).
    pub target_ready: bool,
    /// The gain on Target ▶, in dB (≤ 0).
    pub target_gain_db: f32,
    /// The gain on Mine ▶, in dB (≤ 0).
    pub mine_gain_db: f32,
}

/// The A/B level match: attenuate the louder side by the loudness difference. Either side unmeasured or silent
/// (`None`, or not finite) leaves both at 0 dB. P11: it is lane SS's shared gain function,
/// `enth_challenge::loudness::ab_gains_db`, the one the scorer's −23 LUFS gain uses; the worker measures both sides
/// with `enth_challenge::loudness::integrated_lufs` on the 48 kHz mid channel, as the scorer does.
pub fn ab_gains(target_lufs: Option<f32>, mine_lufs: Option<f32>) -> (f32, f32) {
    let (t, m) =
        enth_challenge::loudness::ab_gains_db(target_lufs.map(f64::from), mine_lufs.map(f64::from));
    (t as f32, m as f32)
}

// ---------------------------------------------------------------------------------------------------------------
// Jobs, requests, the round log
// ---------------------------------------------------------------------------------------------------------------

/// Names one job the session asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(pub u64);

/// Background work for the shell's worker. Each job carries everything it needs (owned or shared), so the worker
/// never locks the session; it posts a [`JobResult`] that the GUI thread hands back with
/// [`Session::game_job_done`].
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// Its id, to hand back with the result.
    pub id: JobId,
    /// The round it is for: the worker may keep per-round state under it (the target's buffer, the scorers'
    /// `prepare` output) and drop it when another round starts or `Ab.target_ready` turns false.
    pub round: RoundId,
    /// What to do.
    pub kind: JobKind,
}

/// What a [`Job`] does. Every job is a pure function of its payload (scorers and the generator are deterministic).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum JobKind {
    /// Run `enth_challenge::prepare_round(&spec)` (lane RG's generator, `Challenge::recreate(spec, phrase)` and every
    /// scorer it names, prepared) and keep the `PreparedRound` under the job's round for the score, rings and hint
    /// jobs; render the target's loop of the scored phrase and keep it for Target ▶ (the shell owns the buffer; the
    /// session never sees audio); measure its loudness. Answer [`JobResult::Prepared`] with
    /// [`Prepared::from_round`]. The session checks the code against the target's fingerprint.
    Prepare {
        /// The settings (from the code, for a pasted or a restored round).
        spec: RandomSpec,
    },
    /// Score `patch` against the target: `enth_challenge::score_round` on the round's `PreparedRound`. Answer
    /// [`JobResult::Scored`]; the session checks the report against the challenge.
    Score {
        /// The challenge.
        challenge: Arc<Challenge>,
        /// The target the prepare job returned.
        target: Arc<Patch>,
        /// The player's patch at the moment they pressed Submit.
        patch: Box<Patch>,
        /// Which submit this is (1 = the first).
        submit: u32,
    },
    /// The per-knob rings for `patch` (structure only, no render needed: `enth_challenge::score_structure`). Answer
    /// [`JobResult::Rings`].
    Rings {
        /// The challenge.
        challenge: Arc<Challenge>,
        /// The target.
        target: Arc<Patch>,
        /// The player's patch after the last settled edit.
        patch: Box<Patch>,
    },
    /// Prepare a given challenge (lane SL, R2; [`RoundSource::Challenge`]): `enth_challenge::prepare_challenge` (seed,
    /// patch and draw targets: the target, start, randomized parameters and fingerprint, every scorer it names
    /// prepared), kept under the job's round like [`JobKind::Prepare`]'s; render and keep the target's loop the same
    /// way. Answer [`JobResult::Prepared`] with [`Prepared::from_round`]; the session checks that the prepared
    /// challenge is the one it asked for, and a resumed round's fingerprint.
    PrepareChallenge {
        /// The challenge.
        challenge: Arc<Challenge>,
    },
    /// Render the scored phrase through `patch` and measure its loudness, for Mine ▶'s gain. Answer
    /// [`JobResult::Loudness`].
    Loudness {
        /// The challenge (its phrase).
        challenge: Arc<Challenge>,
        /// The player's patch after the last settled edit.
        patch: Box<Patch>,
    },
    /// A coach hint at `level` for `patch` (lane SN). Answer [`JobResult::Hint`].
    Hint {
        /// The challenge.
        challenge: Arc<Challenge>,
        /// The target.
        target: Arc<Patch>,
        /// The player's patch when they asked.
        patch: Box<Patch>,
        /// 1 ..= [`HINT_LEVELS`].
        level: u8,
    },
}

impl JobKind {
    /// The kind's name, for errors.
    pub fn name(&self) -> &'static str {
        match self {
            JobKind::Prepare { .. } => "prepare",
            JobKind::PrepareChallenge { .. } => "prepare_challenge",
            JobKind::Score { .. } => "score",
            JobKind::Rings { .. } => "rings",
            JobKind::Loudness { .. } => "loudness",
            JobKind::Hint { .. } => "hint",
        }
    }
}

/// What a prepare job hands back: the challenge, the start and target patches, the randomized parameters, the
/// target's fingerprint and the target loop's loudness. `non_exhaustive`, so a field added later never breaks a
/// shell: build it with [`Prepared::from_round`] (what the worker does) or [`Prepared::new`] (tests and fakes).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Prepared {
    /// `Challenge::recreate(spec, phrase)`, with the generator's phrase and the scorers it names.
    pub challenge: Challenge,
    /// The patch the player starts from.
    pub start: Patch,
    /// The target (kept in memory only; never saved).
    pub target: Patch,
    /// The randomized parameters, in address order (lane RG's `Generated::params`): the rings and Challenge v1's
    /// `dim_unused` assist (F1r review S7).
    pub params: Vec<ParamAddr>,
    /// The target's fingerprint, which the session checks against the code and a saved round.
    pub fingerprint: TargetFingerprint,
    /// The target loop's integrated loudness (LUFS; `None` if silent).
    pub target_lufs: Option<f32>,
}

impl Prepared {
    /// A prepared target from its parts.
    pub fn new(
        challenge: Challenge,
        start: Patch,
        target: Patch,
        params: Vec<ParamAddr>,
        fingerprint: TargetFingerprint,
        target_lufs: Option<f32>,
    ) -> Prepared {
        Prepared {
            challenge,
            start,
            target,
            params,
            fingerprint,
            target_lufs,
        }
    }

    /// What the worker answers for a `PreparedRound` from `enth_challenge::prepare_round`, with the loudness it
    /// measured on the target's loop. Every part comes from the one prepared round, so they belong together.
    pub fn from_round(round: &PreparedRound, target_lufs: Option<f32>) -> Prepared {
        Prepared {
            challenge: round.challenge().clone(),
            start: round.start().clone(),
            target: round.target().clone(),
            params: round.params().to_vec(),
            fingerprint: round.fingerprint(),
            target_lufs,
        }
    }
}

/// A job's answer.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum JobResult {
    /// For [`JobKind::Prepare`].
    Prepared(Box<Prepared>),
    /// For [`JobKind::Score`].
    Scored(Box<ScoreReport>),
    /// For [`JobKind::Rings`]: lane ST's structure score (its `params` carry the verdicts).
    Rings(Box<StructureScore>),
    /// For [`JobKind::Loudness`]: Mine's loudness (LUFS; `None` if silent).
    Loudness {
        /// The phrase's integrated loudness through the player's patch.
        lufs: Option<f32>,
    },
    /// For [`JobKind::Hint`] (boxed since lane HINT gave the hint its structured fields).
    Hint(Box<CoachHint>),
    /// Any job that could not finish.
    Failed {
        /// Why, for the status line.
        message: String,
    },
    /// For [`JobKind::Prepare`]: `prepare_round` refused the target as silent (`enth_challenge::RoundError::
    /// SilentTarget`): any silent answer would score 100 %. The round breaks with [`GameError::SilentTarget`] before
    /// any patch loads and is never logged (it had no target); the editor deals another seed for a round it dealt
    /// (R1/S1 lane INT).
    SilentTarget,
}

impl JobResult {
    /// The result's name, for errors.
    pub fn name(&self) -> &'static str {
        match self {
            JobResult::Prepared(_) => "prepared",
            JobResult::Scored(_) => "scored",
            JobResult::Rings(_) => "rings",
            JobResult::Loudness { .. } => "loudness",
            JobResult::Hint(_) => "hint",
            JobResult::Failed { .. } => "failed",
            JobResult::SilentTarget => "silent_target",
        }
    }
}

/// Something the shell must do for the round, drained with [`Session::drain_game_requests`] under the lock and
/// acted on **after unlocking** (like [`ShellRequest`]s).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GameRequest {
    /// Run this job on the game worker; hand the result back with [`Session::game_job_done`].
    Run(Job),
    /// The round no longer needs this job: drop it if it has not run (a result that arrives anyway is refused as
    /// stale, harmlessly).
    Cancel(JobId),
    /// The A/B state changed: store it where `process()` reads it (atomics), and drop the target buffer when
    /// `target_ready` is false.
    Ab(AbState),
    /// Append this line to the local round log (JSONL; lane GK's exporter). File I/O: never on the audio thread.
    Log(Box<RoundLogEntry>),
    /// The round changed what the `enth` field saves (a start, a scored submit, a hint, a reveal, a vote, a
    /// dispute, an end): mark the host project dirty, coalesced with [`ShellRequest::PatchEdited`].
    Dirty,
}

/// Why a [`RoundLogEntry`] was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEvent {
    /// The round ended (one per played round).
    End,
    /// The player pressed Disagree (written at once, so a crash cannot lose it; the round's `end` line follows).
    Dispute,
}

/// What ended a round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndedBy {
    /// [`EndChoice::Restore`].
    Restore,
    /// [`EndChoice::Keep`].
    Keep,
    /// Another round started ("another round?" answered yes: GR1's "want more").
    NewRound,
    /// A host state load replaced or removed the round.
    Host,
}

/// One line of the local round log, schema [`ROUND_LOG_SCHEMA`] (summary "GR1 data": code, patch, scores, time,
/// the blind rating, the agreement vote). Lane GK writes the exporter and `cargo xtask gr1-report`. Nothing leaves
/// the machine unless the player sends the file. Read tolerantly: unknown fields are ignored, and the fields added
/// at the F1r review default when missing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoundLogEntry {
    /// [`ROUND_LOG_SCHEMA`].
    pub schema: String,
    /// Why this line was written.
    pub event: LogEvent,
    /// The round (lines of one round share it; a round that a host load restored after its `end` line may log
    /// again: keep the last `end`).
    pub round: RoundId,
    /// The code text (also in `result`, when it decodes).
    pub code: String,
    /// What ended it (`None` on a dispute line).
    pub ended_by: Option<EndedBy>,
    /// The round could not continue ([`Phase::Broken`]).
    pub broken: bool,
    /// Lane CT's `enth.round-result/1`: outcome, the first submit's report (the recorded score), the submit count,
    /// the time to pass, active time, assists and the agreement vote. `None` only when the code does not decode.
    pub result: Option<RoundResult>,
    /// Every scored submit as integers (the result holds only the first report and a count).
    pub submits: Vec<SubmitRecord>,
    /// The dispute note.
    pub dispute: Option<String>,
    /// Edits made (log entries plus those over the log's budget).
    pub edits: u32,
    /// The last scored submit's patch, as patch-file text.
    pub patch: Option<String>,
    /// The last scored submit's full report, while the session still holds it (not after a project reload).
    pub report: Option<ScoreReport>,
    /// The target's fingerprint (64 hex digits), once the target was ready (F1r review S3): a replay of a logged
    /// short code (which carries no prefix) can tell whether it regenerated the same target.
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// The engine of the build that played the round (F1r review S3): an engine change can move a sound score.
    #[serde(default)]
    pub engine: Option<EngineVersion>,
    /// The edit log, on the `end` line (empty on a `dispute` line): every change to the round's patch with its
    /// active time, up to [`EDIT_LOG_BYTES`] (F1r review S8).
    #[serde(default)]
    pub edit_log: Vec<EditRecord>,
    /// Who started the round (lane SL, R2): `"origin":"lesson"` for a lesson's Recreate step, which `gr1-report`
    /// leaves out. Omitted for a game round, so game lines keep their bytes, and a line without it reads as one.
    #[serde(default, skip_serializing_if = "RoundOrigin::is_game")]
    pub origin: RoundOrigin,
    /// A lesson round's lesson and step (`enth_learn::LessonRef`, F1l review should-fix 4), so `gr2-report` can tell
    /// which step a line belongs to: a lesson round on a patch or draw target has no code. Omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lesson: Option<enth_learn::LessonRef>,
    /// How the round ended, when [`RoundLogEntry::result`] is `None` (a round with no code, such as a lesson round on
    /// a patch or draw target, whose `enth.round-result/1` cannot be written without one): passed, gave up or not
    /// passed. Omitted when the result says it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
}

// ---------------------------------------------------------------------------------------------------------------
// The editor's side
// ---------------------------------------------------------------------------------------------------------------

/// What the editor's view of the editor is, for the round's clock ([`Session::game_tick`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorView {
    /// The editor was just built (every `build()` is a fresh open): the clock restarts from now, and the time since
    /// the last tick never counts, even if the close was never reported.
    Opened,
    /// An editor frame: the time since the last tick counts if the editor was visible then.
    Visible,
    /// The editor closed: the time up to now counts, then the clock stops until the next `Opened`.
    Hidden,
}

/// How a round ends ([`GameEdit::game_end`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndChoice {
    /// Bring back the player's own patch and undo history from before the round.
    Restore,
    /// Keep the round's patch (and the round's undo history); the stash is dropped.
    Keep,
}

/// What the round is waiting for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags the challenge bar shows, not a state machine"
)]
pub struct Pending {
    /// The target is being prepared (or regenerated after a reload).
    pub preparing: bool,
    /// A submit is being scored.
    pub scoring: bool,
    /// A hint is being computed.
    pub hinting: bool,
}

/// The editor's read model of the round, rebuilt every frame ([`GameEdit::round`]). Build one with `Default` and
/// set fields (it is `non_exhaustive`): a kittest fake can.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a view model: independent facts the challenge bar and results drawer show"
)]
pub struct RoundView {
    /// The challenge code (Copy code).
    pub code: String,
    /// Where the round is.
    pub phase: Phase,
    /// What it waits for.
    pub pending: Pending,
    /// Active time, in milliseconds (the timer).
    pub active_ms: u64,
    /// The time to pass.
    pub time_to_pass_ms: Option<u64>,
    /// The recorded (first) score.
    pub first: Option<SubmitRecord>,
    /// The latest score.
    pub last: Option<SubmitRecord>,
    /// Scored submits.
    pub submits: u32,
    /// The latest score's full report (its components), while in memory.
    pub report: Option<Arc<ScoreReport>>,
    /// The rings (after the first submit; live).
    pub knobs: Vec<KnobScore>,
    /// Every ring (after the first submit; live), routes and enable flags included ([`rings`]; lane RUI,
    /// additive): `knobs` holds the parameter rings only.
    pub rings: Vec<Ring>,
    /// The A/B state.
    pub ab: AbState,
    /// Hint levels used.
    pub hints: u8,
    /// The latest hint.
    pub hint: Option<CoachHint>,
    /// Hints, a reveal or the dim assist were used.
    pub assisted: bool,
    /// The target was revealed.
    pub revealed: bool,
    /// The `dim_unused` assist is recorded for this round ([`GameEdit::game_dim_unused`]; lane HINT, FU1): the editor
    /// reports it once, when the dimming first shows.
    pub dim_unused: bool,
    /// The blind rating (1–5), once given.
    pub rating: Option<u8>,
    /// The agreement vote.
    pub agreement: Option<Agreement>,
    /// The difficulty, 0–100 (the challenge bar's dots).
    pub difficulty: Option<u8>,
    /// The randomized parameters, in address order, once the target is ready (F1r review S7): Challenge v1's
    /// `dim_unused` assist dims every other control.
    pub params: Vec<ParamAddr>,
    /// When the round next needs a frame, in the shell's milliseconds (the clock `game_tick` gets), even if
    /// nothing else changes (F1r review S11): a settled patch's jobs are due, or the running timer shows its next
    /// second. The editor draws nothing while idle, so it asks for a repaint at this time. `None`: no deadline.
    pub wake_at_ms: Option<u64>,
    /// What the code check reported without stopping the round (`engine_differs`: the code was made with another
    /// engine, so the target may sound slightly different from its maker's).
    pub warnings: Vec<CodeWarning>,
    /// The last thing that went wrong in the round.
    pub error: Option<GameError>,
    /// The round is a lesson's Recreate step (lane SL, R2): the lesson's Next, Back and End move it, so the editor
    /// leaves out the round's own end and new-round controls.
    pub lesson: bool,
}

/// The editor's round intents (lane RUI). [`Session`] implements it; a kittest fake may too. Object-safe. Every
/// call is made on the GUI thread under the session lock, at the end of the frame like `SessionEdit`'s.
pub trait GameEdit {
    /// The round, if one is live.
    fn round(&self) -> Option<RoundView>;
    /// Start a round from settings, a code or a challenge (a round already live ends first, logged; its stash carries
    /// over). Refused settings change nothing, and so does a start while a lesson is live
    /// ([`GameError::LessonLive`]: the lesson starts its own rounds). Asks for a [`JobKind::Prepare`] (or a
    /// [`JobKind::PrepareChallenge`]); the patch changes only when it is done.
    fn game_start(&mut self, source: RoundSource) -> Result<(), GameError>;
    /// Set the A/B switch. `Target` needs the target ready; `Target` and `Mine` need a live round.
    fn game_listen(&mut self, listen: Listen) -> Result<(), GameError>;
    /// Submit the current patch for scoring. Returns the submit's number (1 = first).
    fn game_submit(&mut self) -> Result<u32, GameError>;
    /// Ask for the next hint level (marks the round assisted). Returns the level asked for.
    fn game_hint(&mut self) -> Result<u8, GameError>;
    /// Give up: load the target (marks the round assisted; no more submits).
    fn game_reveal(&mut self) -> Result<(), GameError>;
    /// The editor's `dim_unused` assist is showing (Challenge v1's: the modules the target leaves alone are dimmed;
    /// lane HINT, FU1): marks the round assisted ([`AssistsUsed::dim_unused`]), for good. Turning the assist off later
    /// does not unmark it. Needs the target ready (the dimming reads its randomized parameters): `not_ready` while
    /// preparing, `broken` for a broken round.
    fn game_dim_unused(&mut self) -> Result<(), GameError>;
    /// The blind rating of the first submit, 1–5 ("nothing like it" to "can't tell them apart"), asked **before its
    /// score shows** so the score cannot anchor it (ADR-0007's note, review point (a); GR1 refits the curve with it).
    /// Accepted from the start of play until the first score is recorded; a later rating replaces an earlier one.
    fn game_rate(&mut self, rating: u8) -> Result<(), GameError>;
    /// The agreement vote (after a scored submit); a later vote replaces it.
    fn game_vote(&mut self, agreement: Agreement) -> Result<(), GameError>;
    /// The Disagree button (after a scored submit): keeps the note and logs a dispute line at once.
    fn game_dispute(&mut self, note: &str) -> Result<(), GameError>;
    /// End the round (logged if it was played). Refused for a lesson's round while its lesson is live
    /// ([`GameError::LessonRound`]).
    fn game_end(&mut self, choice: EndChoice) -> Result<(), GameError>;
}

/// Why a game call failed. Every variant has a stable snake-case [`code`](GameError::code) and a one-line
/// [`hint`](GameError::hint).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GameError {
    /// No round is live.
    NoRound,
    /// The target is not ready yet (being prepared, or regenerated after a reload).
    NotReady,
    /// A job of this kind is already running (`score` or `hint`).
    Busy {
        /// The kind.
        what: &'static str,
    },
    /// The target was revealed: no more submits or hints.
    Revealed,
    /// The round cannot continue; only ending it is left.
    Broken,
    /// The vote and the dispute need a scored submit.
    NotScored,
    /// Every hint level is used.
    NoMoreHints,
    /// The round reached [`MAX_SUBMITS`].
    TooManySubmits,
    /// A result for a job the round no longer waits for (ended, replaced, or cancelled): ignore it.
    StaleJob(JobId),
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
    /// The regenerated target differs from the one the round was played against (another fingerprint).
    TargetDrift {
        /// The saved fingerprint.
        expected: String,
        /// The regenerated target's fingerprint.
        found: String,
    },
    /// The code is refused: it does not decode (`code_too_new`, `bad_check`, …), or the regenerated target has
    /// another fingerprint prefix than a long code's (`target_mismatch`). The inner code passes through.
    Code(CodeError),
    /// The settings are refused (`out_of_range`, `no_generator`, `code_too_new`).
    Spec(SpecError),
    /// The round would not fit the saved state: the player's patch is too large to stash (a patch this build could
    /// not read, kept as text). Nothing changed.
    TooLarge {
        /// What does not fit (`stash`).
        what: &'static str,
        /// Its size, escaped as saved.
        bytes: usize,
        /// The most it may take.
        max: usize,
    },
    /// The worker's report does not fit the challenge (other scorer ids or pass mark, a percentage out of range, a
    /// pass that does not follow from the sound %): a shell bug. No submit is recorded.
    BadReport {
        /// What is wrong.
        reason: String,
    },
    /// The target is silent through its phrase ([`JobResult::SilentTarget`]): there is nothing to recreate. Only
    /// ending the round, or starting another, is left; nothing was stashed or loaded.
    SilentTarget,
    /// A rating outside 1–5.
    BadRating(u8),
    /// The first score is recorded: a rating now would not be blind.
    AlreadyScored,
    /// A lesson is live (lane SL, R2): a round of the game's own would stash a second patch, so end the lesson first.
    LessonLive,
    /// The round belongs to the live lesson (lane SL, R2): the lesson's Next, Back and End move it.
    LessonRound,
    /// A Review round gives no hint before its first submit (lane REV, R2; report 01 §3: "no hints on the first try").
    FirstTry,
}

impl GameError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            GameError::NoRound => "no_round",
            GameError::NotReady => "not_ready",
            GameError::Busy { .. } => "busy",
            GameError::Revealed => "revealed",
            GameError::Broken => "round_broken",
            GameError::NotScored => "not_scored",
            GameError::NoMoreHints => "no_more_hints",
            GameError::TooManySubmits => "too_many_submits",
            GameError::StaleJob(_) => "stale_job",
            GameError::WrongResult { .. } | GameError::BadReport { .. } => "wrong_result",
            GameError::JobFailed { .. } => "job_failed",
            GameError::TargetDrift { .. } => "target_drift",
            GameError::Code(e) => e.code(),
            GameError::Spec(e) => e.code(),
            GameError::TooLarge { .. } => "too_large",
            GameError::SilentTarget => "silent_target",
            GameError::BadRating(_) => "bad_rating",
            GameError::AlreadyScored => "already_scored",
            GameError::LessonLive => "lesson_live",
            GameError::LessonRound => "lesson_round",
            GameError::FirstTry => "first_try",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            GameError::NoRound => "start a round first".to_owned(),
            GameError::NotReady => "the target is still being prepared: try again in a moment".to_owned(),
            GameError::Busy { what } => format!("the {what} is still being computed"),
            GameError::Revealed => "the target was revealed: start a new round".to_owned(),
            GameError::Broken => {
                "this round cannot continue: end it (your own patch comes back)".to_owned()
            }
            GameError::Code(e) => e.hint(),
            GameError::Spec(e) => e.hint(),
            GameError::NotScored => "submit first: the vote is about your score".to_owned(),
            GameError::NoMoreHints => "every hint is used: reveal the target, or keep going".to_owned(),
            GameError::TooManySubmits => "this round has as many submits as it records".to_owned(),
            GameError::StaleJob(_) => "nothing to do: the round no longer waits for this job".to_owned(),
            GameError::WrongResult { .. } => "report this: the shell answered a job with the wrong result".to_owned(),
            GameError::JobFailed { .. } => "try again; report it if it keeps failing".to_owned(),
            GameError::TargetDrift { .. } => {
                "this build generates a different target for the code: end the round (report it with the code)"
                    .to_owned()
            }
            GameError::TooLarge { .. } => {
                "your patch is too large to keep during a round: save it as a preset, then start from Init".to_owned()
            }
            GameError::BadReport { .. } => {
                "report this: the shell scored with other rules than the challenge's".to_owned()
            }
            GameError::SilentTarget => {
                "nobody can hear this target, so there is nothing to recreate: start another round (a new seed), or \
                 change the settings or the code"
                    .to_owned()
            }
            GameError::BadRating(_) => "rate from 1 (nothing like it) to 5 (can't tell them apart)".to_owned(),
            GameError::AlreadyScored => {
                "the rating is asked before the score shows, so it cannot change now".to_owned()
            }
            GameError::LessonLive => {
                "a lesson is in progress: end it first (Restore brings your own patch back)".to_owned()
            }
            GameError::LessonRound => {
                "this round is the lesson's step: use Next, Back or End lesson".to_owned()
            }
            GameError::FirstTry => {
                "Review gives no hint on the first try: submit your answer, then hints open".to_owned()
            }
        }
    }
}

impl std::fmt::Display for GameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GameError::NoRound => f.write_str("no round is live"),
            GameError::NotReady => f.write_str("the target is not ready"),
            GameError::Busy { what } => write!(f, "a {what} job is already running"),
            GameError::Revealed => f.write_str("the target was revealed"),
            GameError::Broken => f.write_str("the round cannot continue"),
            GameError::NotScored => f.write_str("no submit has been scored"),
            GameError::NoMoreHints => f.write_str("no hint levels left"),
            GameError::TooManySubmits => write!(f, "more than {MAX_SUBMITS} submits"),
            GameError::StaleJob(id) => write!(f, "job {} is not awaited", id.0),
            GameError::WrongResult { job, result } => {
                write!(f, "a {job} job answered with a {result} result")
            }
            GameError::JobFailed { job, message } => write!(f, "the {job} job failed: {message}"),
            GameError::TargetDrift { expected, found } => write!(
                f,
                "the regenerated target's fingerprint is {found}, not {expected}"
            ),
            GameError::Code(e) => write!(f, "challenge code refused: {e}"),
            GameError::Spec(e) => write!(f, "settings refused: {e}"),
            GameError::TooLarge { what, bytes, max } => {
                write!(
                    f,
                    "the {what} takes {bytes} bytes, over the {max} a round may save"
                )
            }
            GameError::BadReport { reason } => {
                write!(f, "the score report does not fit the challenge: {reason}")
            }
            GameError::SilentTarget => f.write_str("the target is silent"),
            GameError::BadRating(r) => write!(f, "rating {r} is not 1 to {RATING_MAX}"),
            GameError::AlreadyScored => f.write_str("the first score is already recorded"),
            GameError::LessonLive => f.write_str("a lesson is live"),
            GameError::LessonRound => f.write_str("the round belongs to the live lesson"),
            GameError::FirstTry => f.write_str("no hint before the first submit in Review"),
        }
    }
}

impl std::error::Error for GameError {}

// ---------------------------------------------------------------------------------------------------------------
// The session's transient round state
// ---------------------------------------------------------------------------------------------------------------

/// The scored submit in flight.
#[derive(Debug)]
struct PendingScore {
    id: JobId,
    /// The active time when Submit was pressed.
    at_ms: u64,
    /// The submitted patch, as text, for the record.
    patch: String,
}

/// The jobs the round waits for, at most one per kind.
#[derive(Debug, Default)]
struct Jobs {
    next: u64,
    prepare: Option<JobId>,
    score: Option<PendingScore>,
    /// With the patch revision it measured.
    rings: Option<(JobId, u64)>,
    /// With the patch revision it measured.
    loudness: Option<(JobId, u64)>,
    hint: Option<JobId>,
}

/// One value op's target, for merging a drag's log entries (the history merges the same way).
#[derive(Clone, Debug, PartialEq, Eq)]
enum MergeKey {
    Param(ParamAddr),
    Depth(RouteAddr),
    Macro(MacroIndex),
}

impl MergeKey {
    fn of(ops: &[Op]) -> Option<MergeKey> {
        match ops {
            [Op::SetParam { addr, .. }] => Some(MergeKey::Param(addr.clone())),
            [Op::SetDepth { route, .. }] => Some(MergeKey::Depth(route.clone())),
            [Op::SetMacro { index, .. }] => Some(MergeKey::Macro(*index)),
            _ => None,
        }
    }
}

/// The round's state that is not saved: the challenge and the prepared target, the player's stashed undo history,
/// jobs in flight, the clock, live feedback and the outbox. Lives in [`Session`] beside the saved
/// [`RoundRecord`] (in `InstanceState::game`).
#[derive(Default)]
pub(crate) struct GameLive {
    /// The round's settings (from its code, or the settings page; a seed challenge's spec).
    spec: Option<RandomSpec>,
    /// The challenge a [`RoundSource::Challenge`] round was started from (lane SL): the prepared one must equal it.
    source: Option<Arc<Challenge>>,
    /// The challenge the prepare job built (with the generator's phrase).
    challenge: Option<Arc<Challenge>>,
    /// What the code check reported without stopping the round.
    warnings: Vec<CodeWarning>,
    target: Option<Arc<Patch>>,
    /// The randomized parameters, in address order.
    params: Vec<ParamAddr>,
    target_lufs: Option<f32>,
    mine_lufs: Option<f32>,
    /// The player's own undo history, while the round runs in this process (not saved: no history is).
    stash_history: Option<History>,
    listen: Listen,
    /// The A/B state last pushed to the shell.
    ab_sent: Option<AbState>,
    jobs: Jobs,
    /// Counts changes to the round's patch, so settled jobs know whether they measured the latest one.
    revision: u64,
    /// When (shell time) the settled jobs are due.
    settle_at: Option<u64>,
    /// The latest shell timestamp.
    now: u64,
    /// The clock: the last tick's time and whether the editor was visible since.
    clock_last: Option<u64>,
    visible: bool,
    knobs: Vec<KnobScore>,
    rings: Vec<Ring>,
    report: Option<Arc<ScoreReport>>,
    hint: Option<CoachHint>,
    error: Option<GameError>,
    requests: Vec<GameRequest>,
    /// The open gesture and the target of the log's last entry, while a drag can still merge into it.
    merge: Option<(GestureId, MergeKey)>,
    /// The edit log's size, in bytes of its ops' JSON.
    edit_bytes: usize,
    /// The highest round serial this instance has used.
    serial: u32,
}

impl std::fmt::Debug for GameLive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameLive")
            .field("target_ready", &self.target.is_some())
            .field("listen", &self.listen)
            .field("jobs", &self.jobs)
            .field("revision", &self.revision)
            .field("requests", &self.requests.len())
            .finish_non_exhaustive()
    }
}

impl GameLive {
    /// Forget everything about the round (keeps the counters, the clock and the outbox).
    fn reset(&mut self) {
        let keep = GameLive {
            jobs: Jobs {
                next: self.jobs.next,
                ..Jobs::default()
            },
            now: self.now,
            clock_last: self.clock_last,
            visible: self.visible,
            requests: std::mem::take(&mut self.requests),
            ab_sent: self.ab_sent,
            serial: self.serial,
            ..GameLive::default()
        };
        *self = keep;
    }

    fn job(&mut self, round: RoundId, kind: JobKind) -> JobId {
        self.jobs.next += 1;
        let id = JobId(self.jobs.next);
        self.requests
            .push(GameRequest::Run(Job { id, round, kind }));
        id
    }

    /// Cancel every job in flight.
    fn cancel_all(&mut self) {
        let jobs = std::mem::take(&mut self.jobs);
        let ids = [
            jobs.prepare,
            jobs.score.map(|s| s.id),
            jobs.rings.map(|r| r.0),
            jobs.loudness.map(|l| l.0),
            jobs.hint,
        ];
        self.requests
            .extend(ids.into_iter().flatten().map(GameRequest::Cancel));
        self.jobs.next = jobs.next;
    }

    fn dirty(&mut self) {
        if !self.requests.contains(&GameRequest::Dirty) {
            self.requests.push(GameRequest::Dirty);
        }
    }
}

/// One log entry's size in the budget: its ops' JSON plus the entry's own keys and time at their longest.
fn entry_bytes(ops: &Value) -> usize {
    ops.to_string().len() + 64
}

// ---------------------------------------------------------------------------------------------------------------
// The session's game entries
// ---------------------------------------------------------------------------------------------------------------

impl Session {
    /// The saved round, if one is live (what the `enth` field would save; tests and the round log read it).
    pub fn game_record(&self) -> Option<&RoundRecord> {
        self.instance.game.as_ref()?.round.as_ref()
    }

    fn record_mut(&mut self) -> Option<&mut RoundRecord> {
        self.instance.game.as_mut()?.round.as_mut()
    }

    /// The latest shell timestamp ([`Session::game_tick`]'s clock, which the lesson shares; lane SL).
    pub(crate) fn game_now(&self) -> u64 {
        self.game.now
    }

    /// The live round is a lesson's (lane SL).
    pub(crate) fn lesson_round_live(&self) -> bool {
        self.game_record()
            .is_some_and(|r| r.origin == RoundOrigin::Lesson)
    }

    /// A lesson round that can no longer be resumed (its lesson did not come back after a reload): only ending it is
    /// left. It never stashed, so ending it changes nothing (lane SL).
    pub(crate) fn game_break_lesson_round(&mut self) {
        if let Some(record) = self.record_mut()
            && record.origin == RoundOrigin::Lesson
            && record.phase != Phase::Broken
        {
            record.phase = Phase::Broken;
            self.game.jobs.prepare = None;
            self.game.listen = Listen::Off;
        }
    }

    /// A restored lesson round waits for its lesson to hand the challenge back: nothing prepares it, and its target
    /// is not here (lane SL).
    pub(crate) fn lesson_round_waiting(&self) -> bool {
        self.game_record()
            .is_some_and(|r| r.origin == RoundOrigin::Lesson && r.phase != Phase::Broken)
            && self.game.jobs.prepare.is_none()
            && self.game.target.is_none()
    }

    /// Resume a lesson round restored from the saved state (lane SL): its lesson reopened and handed its challenge
    /// back, so ask for it to be prepared again. The prepared target must match the saved fingerprint, and the patch
    /// is not touched (the `patch` field holds the round's). Returns `false` if no restored lesson round waits.
    pub(crate) fn game_resume_lesson_round(&mut self, challenge: Challenge) -> bool {
        if !self.lesson_round_waiting() {
            return false;
        }
        let Some(id) = self.game_record().map(|r| r.id) else {
            return false;
        };
        let challenge = Arc::new(challenge);
        self.game.spec = challenge.spec().copied();
        self.game.source = Some(Arc::clone(&challenge));
        let job = self.game.job(id, JobKind::PrepareChallenge { challenge });
        self.game.jobs.prepare = Some(job);
        true
    }

    /// The A/B state the shell's audio side follows.
    pub fn game_ab(&self) -> AbState {
        let (target_gain_db, mine_gain_db) = ab_gains(self.game.target_lufs, self.game.mine_lufs);
        AbState {
            listen: self.game.listen,
            target_ready: self.game.target.is_some(),
            target_gain_db,
            mine_gain_db,
        }
    }

    /// Take everything the shell must do for the round, oldest first: jobs to run or cancel, A/B changes, round-log
    /// lines, dirty marks. Drain it with [`Session::drain_requests`], under the lock, and act **after unlocking**.
    pub fn drain_game_requests(&mut self) -> std::vec::Drain<'_, GameRequest> {
        let ab = self.game_ab();
        if self.game.ab_sent != Some(ab)
            && (self.game.ab_sent.is_some() || ab != AbState::default())
        {
            self.game.ab_sent = Some(ab);
            self.game.requests.push(GameRequest::Ab(ab));
        }
        self.game.requests.drain(..)
    }

    /// The round's clock and its debounced jobs. The editor glue calls it **once per editor frame**, after `pump`
    /// and `fold_host` and before the editor's reads and edits, with the shell's monotonic time in milliseconds
    /// (any origin; a timestamp older than the last is treated as the last), and with [`EditorView::Opened`] at
    /// every `build()` and [`EditorView::Hidden`] when the editor closes. Active time counts only between visible
    /// ticks while the target is ready, so it pauses while the editor is closed. Every other game entry uses the
    /// latest tick's time. Cheap, and a no-op beyond the clock when no round is live.
    pub fn game_tick(&mut self, now_ms: u64, view: EditorView) {
        let now = self.game.clock_last.map_or(now_ms, |last| now_ms.max(last));
        let counting =
            self.game.target.is_some() && self.game_record().is_some_and(|r| r.phase.is_live());
        if view != EditorView::Opened
            && self.game.visible
            && counting
            && let Some(last) = self.game.clock_last
            && let Some(record) = self.record_mut()
        {
            record.active_ms = record.active_ms.saturating_add(now - last);
        }
        self.game.clock_last = Some(now);
        self.game.visible = view != EditorView::Hidden;
        self.game.now = now;
        if let Some(at) = self.game.settle_at
            && now >= at
        {
            self.game_settle();
        }
        self.learn_tick(now); // the lesson's sound checks share this clock (lane SL, crate::learn)
    }

    /// The settled jobs: Mine's loudness always, the rings after the first submit; each only if none of its kind is
    /// running (a result for an older revision asks again).
    fn game_settle(&mut self) {
        let (Some(challenge), Some(target)) =
            (self.game.challenge.clone(), self.game.target.clone())
        else {
            return;
        };
        let Some(record) = self.game_record() else {
            return;
        };
        if !record.phase.is_live() {
            return;
        }
        let scored = !record.submits.is_empty();
        let round = record.id;
        self.game.settle_at = None;
        let revision = self.game.revision;
        if self.game.jobs.loudness.is_none() {
            let id = self.game.job(
                round,
                JobKind::Loudness {
                    challenge: Arc::clone(&challenge),
                    patch: Box::new(self.patch.clone()),
                },
            );
            self.game.jobs.loudness = Some((id, revision));
        }
        if scored && self.game.jobs.rings.is_none() {
            let id = self.game.job(
                round,
                JobKind::Rings {
                    challenge,
                    target,
                    // In a lesson, what the step locks counts at its step value (lane SL: host automation of a
                    // locked parameter never counts toward a check); otherwise the patch itself.
                    patch: Box::new(self.checked_patch()),
                },
            );
            self.game.jobs.rings = Some((id, revision));
        }
    }

    /// Hand back a job's result. Called on the GUI thread (or the host main thread), under the lock, **never on the
    /// worker**: a prepared target loads the start patch, which sends a plan and asks the shell to rename and mark
    /// dirty. A result for a job the round no longer waits for is [`GameError::StaleJob`] and changes nothing.
    pub fn game_job_done(&mut self, id: JobId, result: JobResult) -> Result<(), GameError> {
        let jobs = &mut self.game.jobs;
        let job = if jobs.prepare == Some(id) {
            jobs.prepare = None;
            "prepare"
        } else if jobs.score.as_ref().is_some_and(|s| s.id == id) {
            "score"
        } else if jobs.rings.is_some_and(|r| r.0 == id) {
            "rings"
        } else if jobs.loudness.is_some_and(|l| l.0 == id) {
            "loudness"
        } else if jobs.hint == Some(id) {
            jobs.hint = None;
            "hint"
        } else {
            return Err(GameError::StaleJob(id));
        };
        match (job, result) {
            (_, JobResult::Failed { message }) => {
                self.game_job_failed(job, message);
                Ok(())
            }
            ("prepare", JobResult::SilentTarget) => {
                self.game_silent_target();
                Ok(())
            }
            ("prepare", JobResult::Prepared(prepared)) => {
                self.game_prepared(*prepared);
                Ok(())
            }
            ("score", JobResult::Scored(report)) => {
                // The session never records a report the challenge could not have produced (F1r review S10).
                let check = self.game.challenge.as_ref().map_or_else(
                    || Err("the round has no challenge".to_owned()),
                    |c| report.check_against(&c.scoring),
                );
                if let Err(reason) = check {
                    self.game.jobs.score = None;
                    let err = GameError::BadReport { reason };
                    self.game.error = Some(err.clone());
                    return Err(err);
                }
                self.game_scored(*report);
                Ok(())
            }
            ("rings", JobResult::Rings(structure)) => {
                let measured = self.game.jobs.rings.take().map_or(0, |r| r.1);
                let expected = self.game.challenge.as_ref().map(|c| &c.scoring.structure);
                if expected != Some(&structure.scorer) {
                    let err = GameError::BadReport {
                        reason: format!("the rings come from {}", structure.scorer),
                    };
                    self.game.error = Some(err.clone());
                    return Err(err);
                }
                self.game.knobs = knobs(&structure);
                self.game.rings = rings(&structure);
                self.game_resettle(measured);
                Ok(())
            }
            ("loudness", JobResult::Loudness { lufs }) => {
                let measured = self.game.jobs.loudness.take().map_or(0, |l| l.1);
                self.game.mine_lufs = lufs;
                self.game_resettle(measured);
                Ok(())
            }
            ("hint", JobResult::Hint(hint)) => {
                self.game.hint = Some((*hint).laddered());
                Ok(())
            }
            (job, result) => {
                // Put the job back: it is still owed its real result.
                let err = GameError::WrongResult {
                    job,
                    result: result.name(),
                };
                match job {
                    "prepare" => self.game.jobs.prepare = Some(id),
                    "hint" => self.game.jobs.hint = Some(id),
                    _ => {}
                }
                Err(err)
            }
        }
    }

    /// A settled job measured an older revision of the patch: ask again at the next tick.
    fn game_resettle(&mut self, measured: u64) {
        if measured < self.game.revision && self.game.settle_at.is_none() {
            self.game.settle_at = Some(self.game.now);
        }
    }

    /// The prepare job found the target silent: the round breaks before anything is stashed or loaded (so ending it
    /// changes nothing, and starting another closes it unlogged: it was never played).
    fn game_silent_target(&mut self) {
        if let Some(record) = self.record_mut() {
            record.phase = Phase::Broken;
        }
        self.game.listen = Listen::Off;
        self.game.error = Some(GameError::SilentTarget);
    }

    fn game_job_failed(&mut self, job: &'static str, message: String) {
        let err = GameError::JobFailed { job, message };
        match job {
            "prepare" => {
                if let Some(record) = self.record_mut() {
                    record.phase = Phase::Broken;
                }
                self.game.listen = Listen::Off;
            }
            "score" => self.game.jobs.score = None,
            "rings" => self.game.jobs.rings = None,
            "loudness" => self.game.jobs.loudness = None,
            _ => {}
        }
        self.game.error = Some(err);
    }

    /// The prepare job is done: check the challenge and the target against the round's settings, code and saved
    /// fingerprint, then start play (a fresh round) or resume it (a restored one). Any mismatch breaks the round.
    fn game_prepared(&mut self, prepared: Prepared) {
        let Prepared {
            challenge,
            start,
            target,
            params,
            fingerprint,
            target_lufs,
        } = prepared;
        let hex = fingerprint.to_hex();
        if challenge.spec() != self.game.spec.as_ref() {
            self.game_job_failed(
                "prepare",
                "the prepared challenge is for other settings".to_owned(),
            );
            return;
        }
        // A round from a given challenge must get that challenge back (lane SL).
        if let Some(source) = &self.game.source
            && **source != challenge
        {
            self.game_job_failed(
                "prepare",
                "the prepared challenge is not the one the round asked for".to_owned(),
            );
            return;
        }
        let given = self.game.source.is_some();
        // A fresh round stashes the player's patch: it must fit the saved state (it may have grown while the target
        // was prepared, through a host load).
        let own_bytes = self.own_patch_bytes();
        let Some(record) = self.instance.game.as_mut().and_then(|g| g.round.as_mut()) else {
            return;
        };
        let checked = if let Some(expected) = &record.target_hash
            && *expected != hex
        {
            Err(GameError::TargetDrift {
                expected: expected.clone(),
                found: hex.clone(),
            })
        } else if record.phase == Phase::Preparing
            && record.stash.is_none()
            && record.origin != RoundOrigin::Lesson
            && own_bytes > STASH_BYTES
        {
            Err(GameError::TooLarge {
                what: "stash",
                bytes: own_bytes,
                max: STASH_BYTES,
            })
        } else if record.code.is_empty() {
            // Custom settings: the long code carries the target's fingerprint prefix, so it exists only now. A given
            // challenge whose target is not a seed has no code at all (lane SL), and plays without one.
            match challenge.code(fingerprint.prefix()) {
                Some(code) => {
                    record.code = code.to_string();
                    Ok(Vec::new())
                }
                None if given => Ok(Vec::new()),
                None => Err(GameError::JobFailed {
                    job: "prepare",
                    message: "this challenge has no code".to_owned(),
                }),
            }
        } else {
            ChallengeCode::decode(&record.code)
                .and_then(|code| code.verify(&fingerprint, EngineVersion::CURRENT))
                .map_err(GameError::Code)
        };
        let warnings = match checked {
            Ok(warnings) => warnings,
            Err(e) => {
                record.phase = Phase::Broken;
                self.game.listen = Listen::Off;
                self.game.error = Some(e);
                return;
            }
        };
        record.target_hash = Some(hex);
        let fresh = record.phase == Phase::Preparing;
        self.game.challenge = Some(Arc::new(challenge));
        self.game.target = Some(Arc::new(target));
        self.game.params = params;
        self.game.target_lufs = target_lufs;
        self.game.warnings = warnings;
        self.game.error = None;
        if fresh {
            self.game_begin(start);
        }
        // Time counts from now, not from the start of the preparation; Mine's loudness (and, for a resumed round
        // that was scored, the rings) are measured at the next tick.
        self.game.clock_last = Some(self.game.now);
        self.game.settle_at = Some(self.game.now);
    }

    /// The player's own patch text as a stash would save it (escaped): what [`STASH_BYTES`] bounds.
    pub(crate) fn own_patch_bytes(&self) -> usize {
        match &self.unloaded_patch {
            Some(text) => escaped_len(text),
            None => escaped_len(&self.patch.to_json()),
        }
    }

    /// Set the saved round, keeping the `game` object's other fields; an empty object is saved as none, so a state
    /// without a round keeps its bytes (F1r review N3).
    fn put_round(&mut self, round: Option<RoundRecord>) {
        let game = self.instance.game.get_or_insert_with(GameState::default);
        game.round = round;
        if game.is_empty() {
            self.instance.game = None;
        }
    }

    /// The target is ready for a fresh round: stash the player's patch and history (unless a previous round already
    /// did), give the round a history of its own, and load the start patch like a load.
    fn game_begin(&mut self, start: Patch) {
        self.close_drag();
        let fresh = History::with_cap(self.config.history_cap);
        let history = std::mem::replace(&mut self.history, fresh);
        let own = self
            .unloaded_patch
            .clone()
            .unwrap_or_else(|| self.patch.to_json());
        let Some(record) = self.instance.game.as_mut().and_then(|g| g.round.as_mut()) else {
            return;
        };
        // A lesson round never stashes: the `learn` object holds the only stash (lane SL). The step's history is
        // dropped, as a round's start always gives the round a history of its own.
        let lesson = record.origin == RoundOrigin::Lesson;
        if record.stash.is_none() && !lesson {
            record.stash = Some(own);
            self.game.stash_history = Some(history);
        }
        record.phase = Phase::Playing;
        self.game.revision += 1;
        self.game.merge = None;
        self.game.dirty();
        self.seat_patch(start, None);
        if lesson {
            self.learn_round_seated(); // the step's one plan: the lesson pins its locks and sees the patch
        }
    }

    fn game_scored(&mut self, report: ScoreReport) {
        let Some(pending) = self.game.jobs.score.take() else {
            return;
        };
        let score = summary(&report);
        let knobs = knobs(&report.structure);
        let rings = rings(&report.structure);
        let Some(record) = self.record_mut() else {
            return;
        };
        if record.submits.is_empty() {
            record.first_report = serde_json::to_string(&report)
                .ok()
                .filter(|text| escaped_len(text) <= FIRST_REPORT_BYTES);
        }
        record.submits.push(SubmitRecord {
            at_ms: pending.at_ms,
            score,
        });
        record.submitted_patch =
            (escaped_len(&pending.patch) <= MAX_SUBMITTED_PATCH_BYTES).then_some(pending.patch);
        let first_pass = score.passed && record.time_to_pass_ms.is_none();
        if first_pass {
            record.time_to_pass_ms = Some(pending.at_ms);
        }
        if record.phase != Phase::Revealed {
            record.phase = if record.time_to_pass_ms.is_some() {
                Phase::Passed
            } else {
                Phase::Submitted
            };
        }
        // A lesson's step passes when its round first passes (lane SL); the round stays live until the step moves.
        let lesson_pass = (first_pass && record.origin == RoundOrigin::Lesson).then(|| {
            (
                record.assisted(),
                record.submits.iter().map(|s| s.score.sound).max(),
            )
        });
        self.game.knobs = knobs;
        self.game.rings = rings;
        self.game.report = Some(Arc::new(report));
        self.game.error = None;
        self.game.dirty();
        if let Some((assisted, best)) = lesson_pass {
            self.learn_round_result(true, assisted, best);
        }
    }

    /// End a drag where the whole patch is about to be replaced: its history step is closed without host writes
    /// (the host gestures it opened stay open until the editor's next `end_gesture`, as on a load).
    pub(crate) fn close_drag(&mut self) {
        self.gestures.open = None;
        self.history.end_gesture(&self.patch);
    }

    /// Replace the whole patch the way a load does (the round's start, a reveal, the end's restore): forget the
    /// engine's remembered host slot values, reset the host baseline (the next fold only records), release audition
    /// notes, send the plan (pumped at once), ask for a rename when the host would see other names, and mark the
    /// project dirty. The caller has already set the history. A lesson's step loads and its end's restore seat their
    /// patches the same way (lane SL).
    pub(crate) fn seat_patch(&mut self, patch: Patch, unloaded: Option<String>) {
        let renamed = host_layout_changed(&self.patch, &patch);
        self.patch = patch;
        self.unloaded_patch = unloaded;
        self.reset_host_baseline();
        let _ = self.audition_all_off();
        if let Some(port) = &mut self.port {
            port.compile_pending = true;
            port.forget_slots = u64::MAX;
        }
        if renamed {
            self.requests.push(ShellRequest::RenameParams);
        }
        if !self.requests.contains(&ShellRequest::PatchEdited) {
            self.requests.push(ShellRequest::PatchEdited);
        }
        self.pump();
    }

    /// End the live round, if any, for `why`: log it if it was played, cancel its jobs, switch A/B off and forget
    /// its transient state. Takes only the round from the `game` object (its other fields stay). Returns the record
    /// (with the stash) and the player's stashed history.
    pub(crate) fn game_close(&mut self, why: EndedBy) -> Option<(RoundRecord, Option<History>)> {
        let record = self.instance.game.as_mut()?.round.take()?;
        self.put_round(None);
        if record.played() {
            let entry = self.log_entry(&record, LogEvent::End, Some(why));
            self.game.requests.push(GameRequest::Log(Box::new(entry)));
            // Lane REV: the first submit's rings feed the review boxes (every played round but a host load's).
            self.review_note_close(&record, why, self.game.target.clone());
        }
        self.game.cancel_all();
        let history = self.game.stash_history.take();
        self.game.reset();
        // A host load is the host's own change: marking the project dirty after it would add a host undo point
        // (and cut REAPER's redo).
        if why != EndedBy::Host {
            self.game.dirty();
        }
        Some((record, history))
    }

    fn log_entry(
        &self,
        record: &RoundRecord,
        event: LogEvent,
        ended_by: Option<EndedBy>,
    ) -> RoundLogEntry {
        let result = record.result();
        RoundLogEntry {
            schema: ROUND_LOG_SCHEMA.to_owned(),
            event,
            round: record.id,
            code: record.code.clone(),
            ended_by,
            broken: record.phase == Phase::Broken,
            submits: record.submits.clone(),
            dispute: record.dispute.clone(),
            edits: (record.edits.len() as u32).saturating_add(record.edits_dropped),
            patch: record.submitted_patch.clone(),
            report: self.game.report.as_deref().cloned(),
            fingerprint: record.target_hash.clone(),
            engine: Some(EngineVersion::CURRENT),
            edit_log: if event == LogEvent::End {
                record.edits.clone()
            } else {
                Vec::new()
            },
            origin: record.origin,
            // The step the round belongs to: the saved reference still names it while the round closes (a step
            // change, the lesson's end or a new lesson saves its new reference only after the round is closed).
            lesson: (record.origin == RoundOrigin::Lesson)
                .then(|| self.instance.learn.as_ref().and_then(|l| l.lesson.clone()))
                .flatten(),
            outcome: result.is_none().then(|| record.outcome()),
            result,
        }
    }

    /// When the round next needs a frame without input (the view's `wake_at_ms`): the settled jobs' time, or the
    /// running timer's next whole second of active time.
    fn game_wake_at(&self, record: &RoundRecord) -> Option<u64> {
        let timer = (self.game.visible
            && self.game.clock_last.is_some()
            && self.game.target.is_some()
            && record.phase.is_live())
        .then(|| self.game.now.saturating_add(1000 - record.active_ms % 1000));
        match (self.game.settle_at, timer) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The live round with an open phase and a ready target, for submit and hint.
    fn open_round(&self) -> Result<(&RoundRecord, Arc<Challenge>, Arc<Patch>), GameError> {
        let record = self.game_record().ok_or(GameError::NoRound)?;
        match record.phase {
            Phase::Revealed => return Err(GameError::Revealed),
            Phase::Broken | Phase::Unknown => return Err(GameError::Broken),
            Phase::Preparing => return Err(GameError::NotReady),
            Phase::Playing | Phase::Submitted | Phase::Passed => {}
        }
        match (&self.game.challenge, &self.game.target) {
            (Some(c), Some(t)) => Ok((record, Arc::clone(c), Arc::clone(t))),
            _ => Err(GameError::NotReady),
        }
    }

    /// Start a round from `source` for `origin`: [`GameEdit::game_start`]'s body, which the lesson calls with
    /// [`RoundOrigin::Lesson`] for its Recreate steps (lane SL). Refused settings change nothing. A game round stashes
    /// the player's patch (a live round's stash carries over); a lesson round never does.
    pub(crate) fn game_open(
        &mut self,
        source: RoundSource,
        origin: RoundOrigin,
    ) -> Result<(), GameError> {
        // Refused settings change nothing: check them before the live round ends.
        let (spec, code, kind, given) = match source {
            RoundSource::Settings(spec) => {
                spec.validate().map_err(GameError::Spec)?;
                let short = ChallengeCode::short(spec).map(|c| c.to_string());
                (
                    Some(spec),
                    short.unwrap_or_default(),
                    JobKind::Prepare { spec },
                    None,
                )
            }
            RoundSource::Code(code) => {
                let spec = *code.spec();
                (
                    Some(spec),
                    code.to_string(),
                    JobKind::Prepare { spec },
                    None,
                )
            }
            RoundSource::Challenge(challenge) => {
                if let Some(spec) = challenge.spec() {
                    spec.validate().map_err(GameError::Spec)?;
                }
                let short = challenge.short_code().map(|c| c.to_string());
                let spec = challenge.spec().copied();
                let challenge = Arc::new(*challenge);
                let kind = JobKind::PrepareChallenge {
                    challenge: Arc::clone(&challenge),
                };
                (spec, short.unwrap_or_default(), kind, Some(challenge))
            }
        };
        let lesson = origin == RoundOrigin::Lesson;
        // The player's patch must fit the saved state as a stash, unless a live round already holds the stash.
        let carried = self.game_record().is_some_and(|r| r.stash.is_some());
        let bytes = self.own_patch_bytes();
        if !lesson && !carried && bytes > STASH_BYTES {
            return Err(GameError::TooLarge {
                what: "stash",
                bytes,
                max: STASH_BYTES,
            });
        }
        let (stash, history) = match self.game_close(EndedBy::NewRound) {
            Some((record, history)) if !lesson => (record.stash, history),
            _ => (None, None),
        };
        self.game.serial = self.game.serial.saturating_add(1);
        let id = RoundId {
            serial: self.game.serial,
            nonce: self.game.now,
        };
        let record = RoundRecord {
            id,
            code,
            stash,
            origin,
            ..RoundRecord::default()
        };
        self.put_round(Some(record));
        self.unloaded_instance = None; // the user's own state now (a newer build's `enth` text is replaced)
        self.game.stash_history = history;
        self.game.spec = spec;
        self.game.source = given;
        let job = self.game.job(id, kind);
        self.game.jobs.prepare = Some(job);
        self.game.dirty();
        Ok(())
    }

    // --- hooks the rest of the session calls (each one line at its call site) ----------------------------------

    /// Whether changes to the patch belong to a round now (a live phase: not while preparing a first round, when
    /// the patch is still the player's own).
    fn game_logging(&self) -> bool {
        self.game_record().is_some_and(|r| r.phase.is_live())
    }

    /// The edit path delivered `ops` (an edit, an undo or a redo), which changed the patch from `before`. Called
    /// from [`Session::deliver`].
    pub(crate) fn game_note_edit(&mut self, before: &Patch, ops: &[Op], in_gesture: bool) {
        if !self.game_logging() {
            return;
        }
        let ops_json = enth_patch::wire::to_json(before, ops);
        let merge = in_gesture
            .then_some(self.gestures.open)
            .flatten()
            .zip(MergeKey::of(ops));
        self.game_log(EditSource::Editor, ops_json, merge);
    }

    /// A host fold wrote `op` (a `SetParam` on a slotted address, or a `SetMacro`) into the patch. Called from
    /// [`Session::fold_host`]. The wire form is taken from the patch after the op: these ops name the same spec
    /// either way.
    pub(crate) fn game_note_fold(&mut self, op: &Op) {
        if !self.game_logging() {
            return;
        }
        let ops_json = enth_patch::wire::to_json(&self.patch, std::slice::from_ref(op));
        self.game_log(EditSource::Host, ops_json, None);
    }

    /// The host loaded a patch ([`Session::load_patch`]) during a round whose target is ready (a project reopen
    /// restores the round before its target, and logs nothing).
    pub(crate) fn game_note_load(&mut self) {
        if self.game_logging() && self.game.target.is_some() {
            self.game_log(EditSource::Load, Value::Array(Vec::new()), None);
        }
    }

    /// Append to the edit log (merging a drag's moves of one target), count the change, and schedule the settled
    /// jobs.
    fn game_log(&mut self, source: EditSource, ops: Value, merge: Option<(GestureId, MergeKey)>) {
        self.game.revision += 1;
        self.game.settle_at = Some(self.game.now.saturating_add(SETTLE_MS));
        let merging = merge.is_some() && merge == self.game.merge;
        self.game.merge = merge;
        let size = entry_bytes(&ops);
        let Some(record) = self.instance.game.as_mut().and_then(|g| g.round.as_mut()) else {
            return;
        };
        let t_ms = record.active_ms;
        let budget = EDIT_LOG_BYTES;
        if merging && let Some(last) = record.edits.last_mut() {
            let old = entry_bytes(&last.ops);
            if self.game.edit_bytes - old + size <= budget {
                self.game.edit_bytes = self.game.edit_bytes - old + size;
                *last = EditRecord { t_ms, source, ops };
                return;
            }
        }
        if self.game.edit_bytes + size > budget {
            record.edits_dropped = record.edits_dropped.saturating_add(1);
            self.game.merge = None;
            return;
        }
        self.game.edit_bytes += size;
        record.edits.push(EditRecord { t_ms, source, ops });
    }

    /// A host state load set the `enth` field ([`Session::load_instance`]): `live` is the round that was live
    /// before, and `self.instance.game` holds what the host loaded. The host wins, except over the live progress of
    /// the same round: a host load of the same round (REAPER's own Undo, which restores an earlier save of this
    /// project) keeps the live record, so submits, time and the vote are never rolled back. A different round, or
    /// none, ends the live one (logged, [`EndedBy::Host`]) and restores the loaded one.
    pub(crate) fn game_reconcile_load(&mut self, live: Option<GameState>) {
        // Only the rounds are reconciled: the loaded `game` object's other fields are the host's and stay.
        let loaded = self.instance.game.as_mut().and_then(|g| g.round.take());
        let live = live.and_then(|g| g.round);
        if let (Some(l), Some(n)) = (&live, &loaded)
            && l.id == n.id
            && l.code == n.code
        {
            self.put_round(live);
            return;
        }
        if let Some(record) = live {
            self.put_round(Some(record));
            let _ = self.game_close(EndedBy::Host);
        }
        match loaded {
            Some(record) => self.game_restore(record),
            None => self.put_round(None),
        }
    }

    /// Resume a saved round: decode its code and ask for the target again (checked against the saved hash when it
    /// arrives). The patch is not touched: the `patch` field holds the round's patch.
    fn game_restore(&mut self, mut record: RoundRecord) {
        self.game.reset();
        self.game.serial = self.game.serial.max(record.id.serial);
        self.game.edit_bytes = record.edits.iter().map(|e| entry_bytes(&e.ops)).sum();
        if record.phase == Phase::Unknown {
            record.phase = Phase::Broken;
        }
        if record.origin == RoundOrigin::Lesson {
            // A lesson round waits for its lesson to reopen and hand the challenge back
            // (`game_resume_lesson_round`); if the lesson does not come back, it breaks (lane SL).
            self.put_round(Some(record));
            return;
        }
        if record.code.is_empty() {
            // Saved while a custom-settings round prepared, before its code existed. Nothing was played: without a
            // stash there is nothing to keep, and with one (a new round after a played one) only ending it is left.
            if record.stash.is_none() {
                self.put_round(None);
                return;
            }
            record.phase = Phase::Broken;
        }
        if record.phase != Phase::Broken {
            match ChallengeCode::decode(&record.code) {
                Ok(code) => {
                    let spec = *code.spec();
                    self.game.spec = Some(spec);
                    let id = self.game.job(record.id, JobKind::Prepare { spec });
                    self.game.jobs.prepare = Some(id);
                }
                Err(e) => {
                    record.phase = Phase::Broken;
                    self.game.error = Some(GameError::Code(e));
                }
            }
        }
        self.put_round(Some(record));
    }
}

impl GameEdit for Session {
    fn round(&self) -> Option<RoundView> {
        let record = self.game_record()?;
        Some(RoundView {
            code: record.code.clone(),
            phase: record.phase,
            pending: Pending {
                preparing: self.game.jobs.prepare.is_some(),
                scoring: self.game.jobs.score.is_some(),
                hinting: self.game.jobs.hint.is_some(),
            },
            active_ms: record.active_ms,
            time_to_pass_ms: record.time_to_pass_ms,
            first: record.submits.first().copied(),
            last: record.submits.last().copied(),
            submits: record.submits.len() as u32,
            report: self.game.report.clone(),
            knobs: self.game.knobs.clone(),
            rings: self.game.rings.clone(),
            ab: self.game_ab(),
            hints: record.hints,
            hint: self.game.hint.clone(),
            assisted: record.assisted(),
            revealed: record.revealed,
            dim_unused: record.dim_unused,
            rating: record.rating,
            agreement: record.agreement,
            difficulty: self.game.spec.map(|s| s.difficulty),
            params: self.game.params.clone(),
            wake_at_ms: self.game_wake_at(record),
            warnings: self.game.warnings.clone(),
            error: self.game.error.clone(),
            lesson: record.origin == RoundOrigin::Lesson,
        })
    }

    fn game_start(&mut self, source: RoundSource) -> Result<(), GameError> {
        // A lesson starts its own rounds; one of the game's would stash a second patch (lane SL).
        if self.learn_live() {
            return Err(GameError::LessonLive);
        }
        self.game_open(source, RoundOrigin::Game)
    }

    fn game_listen(&mut self, listen: Listen) -> Result<(), GameError> {
        if listen != Listen::Off {
            let record = self.game_record().ok_or(GameError::NoRound)?;
            match record.phase {
                Phase::Preparing => return Err(GameError::NotReady),
                Phase::Broken | Phase::Unknown => return Err(GameError::Broken),
                _ => {}
            }
            if listen == Listen::Target && self.game.target.is_none() {
                return Err(GameError::NotReady);
            }
        }
        self.game.listen = listen;
        Ok(())
    }

    fn game_submit(&mut self) -> Result<u32, GameError> {
        let (record, challenge, target) = self.open_round()?;
        if self.game.jobs.score.is_some() {
            return Err(GameError::Busy { what: "score" });
        }
        if record.submits.len() >= MAX_SUBMITS {
            return Err(GameError::TooManySubmits);
        }
        let submit = record.submits.len() as u32 + 1;
        let at_ms = record.active_ms;
        let round = record.id;
        // In a lesson, what the step locks is scored at its step value: host automation of a locked parameter never
        // counts toward a check (lane SL). Outside a lesson this is the patch itself.
        let patch = Box::new(self.checked_patch());
        let text = patch.to_json();
        let id = self.game.job(
            round,
            JobKind::Score {
                challenge,
                target,
                patch,
                submit,
            },
        );
        self.game.jobs.score = Some(PendingScore {
            id,
            at_ms,
            patch: text,
        });
        Ok(submit)
    }

    fn game_hint(&mut self) -> Result<u8, GameError> {
        let (record, challenge, target) = self.open_round()?;
        if crate::review::refuses_hint(record) {
            return Err(GameError::FirstTry); // lane REV: no hint on a Review round's first try
        }
        if self.game.jobs.hint.is_some() {
            return Err(GameError::Busy { what: "hint" });
        }
        // The challenge's hint budget (`None`: no limit beyond the levels; 0: no hints).
        let budget = challenge
            .assists
            .hint_budget
            .map_or(HINT_LEVELS, |b| b.min(HINT_LEVELS));
        if record.hints >= budget {
            return Err(GameError::NoMoreHints);
        }
        let level = record.hints + 1;
        let round = record.id;
        let patch = Box::new(self.patch.clone());
        let id = self.game.job(
            round,
            JobKind::Hint {
                challenge,
                target,
                patch,
                level,
            },
        );
        self.game.jobs.hint = Some(id);
        if let Some(record) = self.record_mut() {
            record.hints = level;
        }
        self.game.dirty();
        Ok(level)
    }

    fn game_reveal(&mut self) -> Result<(), GameError> {
        let (_, _, target) = self.open_round()?;
        // A hint in flight no longer matters; a submit in flight was made before the reveal and still counts.
        if let Some(id) = self.game.jobs.hint.take() {
            self.game.requests.push(GameRequest::Cancel(id));
        }
        self.close_drag();
        // A new line: the round's undo steps do not apply to the target. (An undoable reveal is lane PBC's
        // undoable load, `ops_between`: an F1r follow-up.)
        self.history = History::with_cap(self.config.history_cap);
        self.seat_patch(Patch::clone(&target), None);
        let mut lesson = None;
        if let Some(record) = self.record_mut() {
            record.revealed = true;
            record.phase = Phase::Revealed;
            if record.origin == RoundOrigin::Lesson {
                lesson = Some(record.submits.iter().map(|s| s.score.sound).max());
            }
        }
        self.game_log(EditSource::Reveal, Value::Array(Vec::new()), None);
        self.game.dirty();
        if let Some(best) = lesson {
            // The lesson sees the target loaded and the step given up (lane SL).
            self.learn_round_revealed(best);
        }
        Ok(())
    }

    fn game_dim_unused(&mut self) -> Result<(), GameError> {
        let record = self.game_record().ok_or(GameError::NoRound)?;
        match record.phase {
            Phase::Preparing => return Err(GameError::NotReady),
            Phase::Broken | Phase::Unknown => return Err(GameError::Broken),
            Phase::Playing | Phase::Submitted | Phase::Passed | Phase::Revealed => {}
        }
        // The dimming reads the target's randomized parameters: nothing is dimmed before the target is ready.
        if self.game.target.is_none() {
            return Err(GameError::NotReady);
        }
        if record.dim_unused {
            return Ok(());
        }
        if let Some(record) = self.record_mut() {
            record.dim_unused = true;
        }
        self.game.dirty();
        Ok(())
    }

    fn game_rate(&mut self, rating: u8) -> Result<(), GameError> {
        if rating == 0 || rating > RATING_MAX {
            return Err(GameError::BadRating(rating));
        }
        let record = self.game_record().ok_or(GameError::NoRound)?;
        match record.phase {
            Phase::Preparing => return Err(GameError::NotReady),
            Phase::Broken | Phase::Unknown => return Err(GameError::Broken),
            Phase::Revealed => return Err(GameError::Revealed),
            Phase::Playing | Phase::Submitted | Phase::Passed => {}
        }
        if !record.submits.is_empty() {
            return Err(GameError::AlreadyScored);
        }
        if let Some(record) = self.record_mut() {
            record.rating = Some(rating);
        }
        self.game.dirty();
        Ok(())
    }

    fn game_vote(&mut self, agreement: Agreement) -> Result<(), GameError> {
        let record = self.record_mut().ok_or(GameError::NoRound)?;
        if record.submits.is_empty() {
            return Err(GameError::NotScored);
        }
        record.agreement = Some(agreement);
        self.game.dirty();
        Ok(())
    }

    fn game_dispute(&mut self, note: &str) -> Result<(), GameError> {
        let record = self.record_mut().ok_or(GameError::NoRound)?;
        if record.submits.is_empty() {
            return Err(GameError::NotScored);
        }
        record.dispute = Some(note.chars().take(DISPUTE_CHARS).collect());
        let record = record.clone();
        let entry = self.log_entry(&record, LogEvent::Dispute, None);
        self.game.requests.push(GameRequest::Log(Box::new(entry)));
        self.game.dirty();
        Ok(())
    }

    fn game_end(&mut self, choice: EndChoice) -> Result<(), GameError> {
        let why = match choice {
            EndChoice::Restore => EndedBy::Restore,
            EndChoice::Keep => EndedBy::Keep,
        };
        // The lesson's own round moves with the lesson (lane SL). A lesson round left behind by a lesson that did
        // not come back after a reload ends here (it never stashed, so it changes nothing).
        if self.lesson_round_live() && self.learn_live() {
            return Err(GameError::LessonRound);
        }
        let (record, history) = self.game_close(why).ok_or(GameError::NoRound)?;
        if choice == EndChoice::Keep {
            return Ok(());
        }
        let Some(text) = record.stash else {
            // A first round that never started: the player's patch is still playing.
            return Ok(());
        };
        let (patch, unloaded) = match decode_patch(&text) {
            Ok(loaded) => (loaded.patch, None),
            Err(e) => {
                // The same fallback as a load: the init patch plays and a readable-elsewhere text is kept.
                let keep = !matches!(e, StateError::TooLarge { .. });
                self.last_error = Some(crate::model::SessionError::State(e));
                (Patch::init(), keep.then_some(text))
            }
        };
        self.close_drag();
        self.history = history.unwrap_or_else(|| History::with_cap(self.config.history_cap));
        self.seat_patch(patch, unloaded);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lane RUI: route and enable-flag rows ring their route and toggle; missing rows, unparsable text and newer
    /// verdicts ring nothing; `knobs` keeps only the parameter rows, as before.
    #[test]
    fn rings_include_routes_and_enable_flags_where_knobs_skip_them() {
        use enth_challenge::Direction;
        use enth_challenge::score::{ParamScore, ScorerId};
        let row = |addr: &str, answer: Option<&str>, verdict, direction| ParamScore {
            addr: addr.to_owned(),
            answer_addr: answer.map(str::to_owned),
            score: 0.5,
            verdict,
            direction,
            randomized: true,
        };
        let structure = StructureScore {
            scorer: ScorerId::STRUCTURE_V1,
            percent: 50.0,
            params: vec![
                row(
                    "osc1.wave",
                    Some("osc2.wave"),
                    Verdict::Good,
                    Direction::Same,
                ),
                row(
                    "lfo1 -> filter1.cutoff",
                    Some("lfo1 -> filter1.cutoff"),
                    Verdict::Near,
                    Direction::Lower,
                ),
                row(
                    "osc2.enabled",
                    Some("osc2.enabled"),
                    Verdict::Bad,
                    Direction::Missing,
                ),
                row(
                    "env2 -> filter1.cutoff",
                    None,
                    Verdict::Bad,
                    Direction::Missing,
                ),
                row(
                    "nope -> ",
                    Some("nope -> "),
                    Verdict::Bad,
                    Direction::Higher,
                ),
                row(
                    "filter1.cutoff",
                    Some("filter1.cutoff"),
                    Verdict::Unknown,
                    Direction::Same,
                ),
            ],
            modules: Vec::new(),
        };
        let r = rings(&structure);
        assert_eq!(
            r.iter().map(|r| r.at.clone()).collect::<Vec<_>>(),
            vec![
                RingAt::Param("osc2.wave".parse().unwrap()),
                RingAt::Route("lfo1 -> filter1.cutoff".parse().unwrap()),
                RingAt::Enable(ModuleId::new("osc2").unwrap()),
            ]
        );
        assert_eq!(
            r.iter().map(|r| r.verdict).collect::<Vec<_>>(),
            [Verdict::Good, Verdict::Near, Verdict::Bad]
        );
        assert_eq!(
            knobs(&structure).len(),
            1,
            "knobs keeps the parameter rows only"
        );
    }

    #[test]
    fn the_louder_side_is_attenuated_and_the_quieter_never_boosted() {
        assert_eq!(ab_gains(Some(-14.0), Some(-20.0)), (-6.0, 0.0));
        assert_eq!(ab_gains(Some(-20.0), Some(-14.0)), (0.0, -6.0));
        assert_eq!(ab_gains(Some(-18.0), Some(-18.0)), (0.0, 0.0));
        assert!(ab_gains(Some(-18.0), Some(-18.0)).1.is_sign_positive());
        assert_eq!(ab_gains(None, Some(-14.0)), (0.0, 0.0));
        assert_eq!(ab_gains(Some(f32::NEG_INFINITY), Some(-14.0)), (0.0, 0.0));
        assert_eq!(ab_gains(Some(-14.0), None), (0.0, 0.0));
    }

    #[test]
    fn scores_are_saved_as_clamped_hundredths() {
        let s = ScoreSummary::from_percent(true, 87.456, -3.0, f64::NAN);
        assert_eq!(
            (s.passed, s.sound, s.structure, s.combined),
            (true, 8746, 0, 0)
        );
        assert_eq!(
            ScoreSummary::from_percent(false, 250.0, 100.0, 0.004).sound,
            10000
        );
        assert_eq!(
            ScoreSummary::from_percent(false, 0.0, 100.0, 0.004).structure,
            10000
        );
    }

    #[test]
    fn a_phase_or_source_from_a_newer_build_reads_as_unknown() {
        let phase: Phase = serde_json::from_str("\"practice\"").unwrap();
        assert_eq!(phase, Phase::Unknown);
        let source: EditSource = serde_json::from_str("\"midi\"").unwrap();
        assert_eq!(source, EditSource::Other);
        assert_eq!(serde_json::to_string(&Phase::Passed).unwrap(), "\"passed\"");
    }

    #[test]
    fn escaped_lengths_count_the_quotes_and_escapes() {
        assert_eq!(escaped_len(""), 2);
        assert_eq!(escaped_len("ab"), 4);
        assert_eq!(escaped_len("\"a\""), 7);
        assert_eq!(escaped_len("\u{1}"), 8);
        assert_eq!(escaped_len("é"), 4, "UTF-8 is not escaped");
        let patch = enth_patch::Patch::init().to_json();
        assert!(escaped_len(&patch) > patch.len() + patch.matches('"').count());
    }

    #[test]
    fn listen_flips_between_target_and_mine() {
        assert_eq!(Listen::Off.flipped(), Listen::Target);
        assert_eq!(Listen::Target.flipped(), Listen::Mine);
        assert_eq!(Listen::Mine.flipped(), Listen::Target);
    }

    /// Lane HINT (FU1): the structured fields are additive. A hint from before them reads with none, a plain hint
    /// writes none of them, a structured one round-trips, and a newer build's turn or difference reads as `unknown`.
    #[test]
    fn a_coach_hints_structured_fields_are_additive() {
        let old: CoachHint =
            serde_json::from_str(r#"{"level":2,"text":"Look at the filter."}"#).unwrap();
        assert_eq!(old, CoachHint::new(2, "Look at the filter."));
        assert!(!old.points());
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"level":2,"text":"Look at the filter."}"#
        );
        let full = CoachHint::new(4, "Add a route.")
            .with_aspect("sweep", Some(Direction::Missing))
            .with_module(ModuleId::new("filter1").unwrap())
            .with_param("filter1.cutoff".parse().unwrap(), Some(HintTurn::Raise))
            .with_route("env2 -> filter1.cutoff".parse().unwrap());
        assert!(full.points());
        let text = serde_json::to_string(&full).unwrap();
        assert_eq!(
            text,
            r#"{"level":4,"text":"Add a route.","aspect":"sweep","difference":"missing","module":"filter1","param":"filter1.cutoff","turn":"raise","route":"env2 -> filter1.cutoff"}"#
        );
        assert_eq!(serde_json::from_str::<CoachHint>(&text).unwrap(), full);
        let newer = text
            .replace(r#""turn":"raise""#, r#""turn":"nudge""#)
            .replace(r#""difference":"missing""#, r#""difference":"sideways""#);
        let back: CoachHint = serde_json::from_str(&newer).unwrap();
        assert_eq!(
            (back.turn, back.difference),
            (Some(HintTurn::Unknown), Some(Direction::Unknown))
        );
    }

    #[test]
    fn the_game_types_cross_threads() {
        fn send<T: Send + 'static>() {}
        send::<Job>();
        send::<JobResult>();
        send::<GameRequest>();
        send::<RoundView>();
        send::<RoundLogEntry>();
        send::<Session>();
    }
}
