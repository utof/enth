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
//! stash). Hints ([`GameEdit::game_hint`]) and a reveal mark the round assisted. The agreement vote
//! ([`GameEdit::game_vote`]) and a dispute ([`GameEdit::game_dispute`]) need a scored submit. Each round that was
//! played ends with one [`RoundLogEntry`] in the outbox ([`GameRequest::Log`]); lane GK writes the exporter.
//!
//! # What is saved
//!
//! The `enth` field's `game` object ([`GameState`]) holds the [`RoundRecord`]: the challenge code (the target is
//! never persisted: it is regenerated from the code and checked against [`RoundRecord::target_hash`]), the stash
//! (the player's own patch, as patch-file text), the active time, the scored submits as integers, hints, the vote
//! and the edit log. It is absent while no round is live, so a state without a round keeps its bytes, and it stays
//! within `enth.plugin-state/1` (optional fields with defaults; docs/session.md, "The saved state").
//!
//! # F1r swaps (placeholders for lane CT's types)
//!
//! [`Challenge`], [`ScoreReport`] and the code decoder are local stand-ins in [`placeholder`] until lane CT's
//! `enth_challenge::model::Challenge`, `enth_challenge::score::ScoreReport` and `enth_challenge::code::decode` land.
//! The session reads them only through [`placeholder::summary`], [`placeholder::knobs`] and [`Challenge::code`], so
//! the swap touches the two type aliases, those helpers and the tests' constructors.

use std::sync::Arc;

use enth_patch::canon::canonical_hash;
use enth_patch::history::{GestureId, History};
use enth_patch::{MacroIndex, Op, ParamAddr, Patch, RouteAddr};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bridge::host_layout_changed;
use crate::model::{Session, ShellRequest};
use crate::state::{StateError, decode_patch};

/// The challenge a round plays. **F1r swap:** `enth_challenge::model::Challenge` (lane CT).
pub type Challenge = placeholder::Challenge;

/// What the scorer returns for one submit. **F1r swap:** `enth_challenge::score::ScoreReport` (lane CT).
pub type ScoreReport = placeholder::ScoreReport;

/// After the last edit, how long the patch must stay unchanged before the session asks for Mine's loudness (A/B
/// gains) and, after the first submit, the live rings (report 01 §2.6: about 300 ms).
pub const SETTLE_MS: u64 = 300;

/// How many hint levels a round offers: the worst descriptor, the module, the direction, then the move itself
/// (report 02 §2.8).
pub const HINT_LEVELS: u8 = 4;

/// The most scored submits one round records (a bound on the saved state, not a game rule anyone should reach).
pub const MAX_SUBMITS: usize = 255;

/// The most bytes the edit log keeps (its ops' JSON); later entries are counted in
/// [`RoundRecord::edits_dropped`] instead. The log also shrinks so the whole round stays under [`ROUND_BYTES`].
pub const EDIT_LOG_BYTES: usize = 256 * 1024;

/// The budget for the whole saved round (stash, last submitted patch and edit log), well under the `enth` field's
/// [`MAX_FIELD_BYTES`](crate::state::MAX_FIELD_BYTES): a round never makes the saved state unloadable.
pub const ROUND_BYTES: usize = 768 * 1024;

/// The longest submitted patch the record keeps for the round log (a realistic patch is about 20 KB).
pub const MAX_SUBMITTED_PATCH_BYTES: usize = 128 * 1024;

/// The longest dispute note kept, in characters.
pub const DISPUTE_CHARS: usize = 2000;

/// The schema of a [`RoundLogEntry`] line.
pub const ROUND_LOG_SCHEMA: &str = "enth.round-log/1";

// ---------------------------------------------------------------------------------------------------------------
// Placeholders for lane CT's types
// ---------------------------------------------------------------------------------------------------------------

/// Minimal stand-ins for lane CT's `Challenge`, `ScoreReport` and code decoder, so the session game API builds and
/// is tested before F1r. **Every item here is swapped at F1r**; nothing outside this module and the two aliases
/// above names them.
pub mod placeholder {
    use serde::{Deserialize, Serialize};

    use super::{KnobScore, ScoreSummary};

    /// Stand-in for `enth_challenge::model::Challenge`. The session needs only its code (persisted, to regenerate
    /// the target) and hands the whole value to the shell's jobs.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Challenge {
        /// The challenge code (Crockford base32 in CT's code v1).
        pub code: String,
    }

    impl Challenge {
        /// A challenge with this code.
        pub fn new(code: impl Into<String>) -> Self {
            Challenge { code: code.into() }
        }

        /// The code a round saves to regenerate the target.
        pub fn code(&self) -> &str {
            &self.code
        }
    }

    /// Stand-in for `enth_challenge::code::decode`: rebuilds the challenge from a saved code when a round is
    /// restored. Refuses an empty code or one longer than 64 characters (CT's long form is about 32).
    pub fn decode(code: &str) -> Result<Challenge, String> {
        if code.is_empty() || code.chars().count() > 64 {
            return Err(format!("not a challenge code: {code:?}"));
        }
        Ok(Challenge::new(code))
    }

    /// Stand-in for `enth_challenge::score::ScoreReport`: the three scores (0–100), pass (sound alone decides it)
    /// and the per-knob verdicts for the rings. CT's report also carries the descriptor components.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct ScoreReport {
        /// Sound decided a pass.
        pub passed: bool,
        /// Sound %, 0–100.
        pub sound: f32,
        /// Structure %, 0–100.
        pub structure: f32,
        /// Combined %, 0–100.
        pub combined: f32,
        /// One verdict per randomized, observable knob.
        pub knobs: Vec<KnobScore>,
    }

    /// The integer summary the round saves (the only place the session reads the report's scores).
    pub fn summary(report: &ScoreReport) -> ScoreSummary {
        ScoreSummary::from_percent(
            report.passed,
            report.sound,
            report.structure,
            report.combined,
        )
    }

    /// The rings a report gives (the only place the session reads its knobs).
    pub fn knobs(report: &ScoreReport) -> Vec<KnobScore> {
        report.knobs.clone()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The saved state
// ---------------------------------------------------------------------------------------------------------------

/// The instance's game data, as the `enth` field saves it ([`InstanceState::game`](crate::InstanceState::game)).
///
/// The session keeps the field `None` while no round is live, so a state without a round saves exactly the bytes
/// it saved before R1. `GameState::default()` (no round) serialises as `{}`; later optional fields (the
/// randomizer's last settings, an auto-difficulty staircase, a seed history) can join it within version 1.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameState {
    /// The round in progress, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<RoundRecord>,
}

/// One round as it is saved: everything needed to resume it after the project is reopened, except the target
/// (regenerated from [`code`](RoundRecord::code)) and what the shell's jobs recompute (the target's audio, Mine's
/// loudness, the live rings, a hint's text, the full score report).
///
/// Integers and text only, so its bytes are deterministic and it is `Eq`. Every field has a default and empty ones
/// are omitted, so a newer build's extra fields are ignored and missing ones default (state version 1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoundRecord {
    /// Which round this is: a host load of the same round keeps the live progress.
    pub id: RoundId,
    /// The challenge code. The target is regenerated from it, never persisted (design.md §9).
    pub code: String,
    /// Where the round is.
    pub phase: Phase,
    /// The player's own patch from before the first round, as patch-file text (exactly what the `patch` field
    /// held, so a patch this build could not read survives too). `None` until the first round's target is ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash: Option<String>,
    /// The target's canonical hash (64 hex digits), set when the target is first prepared; a regenerated target
    /// must match it, or the round is [`Phase::Broken`].
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
    /// The agreement vote, once given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vote: Option<Vote>,
    /// The player's dispute note, once given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispute: Option<String>,
    /// The patch of the last scored submit, as patch-file text, for the round log (omitted when larger than
    /// [`MAX_SUBMITTED_PATCH_BYTES`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted_patch: Option<String>,
    /// The edit log: every change to the round's patch, stamped with active time.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<EditRecord>,
    /// Log entries not kept because the log reached its byte budget.
    pub edits_dropped: u32,
}

impl RoundRecord {
    /// Hints or a reveal were used.
    pub fn assisted(&self) -> bool {
        self.hints > 0 || self.revealed
    }

    /// The target was ready at least once: the player has played this round (it gets a log entry when it ends).
    pub fn played(&self) -> bool {
        self.target_hash.is_some()
    }

    /// How the round stands, for the log.
    pub fn outcome(&self) -> Outcome {
        if self.time_to_pass_ms.is_some() {
            Outcome::Passed
        } else if self.revealed {
            Outcome::Revealed
        } else if self.phase == Phase::Broken {
            Outcome::Broken
        } else {
            Outcome::Abandoned
        }
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
    pub fn from_percent(passed: bool, sound: f32, structure: f32, combined: f32) -> Self {
        fn hundredths(pct: f32) -> u16 {
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

/// The three-way agreement vote (report 05: GR1's "mostly agree").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vote {
    /// "Felt too low."
    TooLow,
    /// "About right."
    Right,
    /// "Felt too high."
    TooHigh,
}

// ---------------------------------------------------------------------------------------------------------------
// Feedback types
// ---------------------------------------------------------------------------------------------------------------

/// A knob's ring after the first submit (Syntorial-style right / near / wrong; the editor's `Overlay` draws it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Within its tolerance.
    Good,
    /// Close.
    Near,
    /// Off.
    Bad,
}

/// One knob's verdict.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KnobScore {
    /// The parameter.
    pub addr: ParamAddr,
    /// Its verdict.
    pub verdict: Verdict,
}

/// A coach hint, as the shell's hint job returns it (lane SN fills it from `enth-solver`'s coach).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CoachHint {
    /// The level it answers (1 = the worst descriptor … [`HINT_LEVELS`] = the move itself).
    pub level: u8,
    /// What to show.
    pub text: String,
}

impl CoachHint {
    /// A hint at `level` saying `text`.
    pub fn new(level: u8, text: impl Into<String>) -> Self {
        CoachHint {
            level,
            text: text.into(),
        }
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
/// (`None`, or not finite) leaves both at 0 dB. **Swap when lane SS lands:** P11 asks the A/B level match and the
/// scorer to share one gain function (`enth_challenge::loudness`); both already read the same loudness measurement.
pub fn ab_gains(target_lufs: Option<f32>, mine_lufs: Option<f32>) -> (f32, f32) {
    match (target_lufs, mine_lufs) {
        (Some(t), Some(m)) if t.is_finite() && m.is_finite() => {
            if t > m {
                (m - t, 0.0)
            } else {
                (0.0, t - m)
            }
        }
        _ => (0.0, 0.0),
    }
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
    /// What to do.
    pub kind: JobKind,
}

/// What a [`Job`] does. Every job is a pure function of its payload (scorers and the generator are deterministic).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum JobKind {
    /// Generate the challenge's start and target patches (lane RG), render the target's loop of the scored phrase
    /// and keep it for Target ▶ (the shell owns the buffer; the session never sees audio), measure its loudness,
    /// and calibrate the scorer (lane SS). Answer [`JobResult::Prepared`].
    Prepare {
        /// The challenge.
        challenge: Arc<Challenge>,
    },
    /// Score `patch` against the target (lanes SS, ST). Answer [`JobResult::Scored`].
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
    /// The per-knob rings for `patch` (structure only, no render needed). Answer [`JobResult::Rings`].
    Rings {
        /// The challenge.
        challenge: Arc<Challenge>,
        /// The target.
        target: Arc<Patch>,
        /// The player's patch after the last settled edit.
        patch: Box<Patch>,
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
            JobKind::Score { .. } => "score",
            JobKind::Rings { .. } => "rings",
            JobKind::Loudness { .. } => "loudness",
            JobKind::Hint { .. } => "hint",
        }
    }
}

/// A job's answer.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum JobResult {
    /// For [`JobKind::Prepare`]: the start and target patches and the target's loudness (LUFS; `None` if silent).
    Prepared {
        /// The patch the player starts from.
        start: Box<Patch>,
        /// The target (kept in memory only; never saved).
        target: Box<Patch>,
        /// The target loop's integrated loudness.
        target_lufs: Option<f32>,
    },
    /// For [`JobKind::Score`].
    Scored(ScoreReport),
    /// For [`JobKind::Rings`].
    Rings(Vec<KnobScore>),
    /// For [`JobKind::Loudness`]: Mine's loudness (LUFS; `None` if silent).
    Loudness {
        /// The phrase's integrated loudness through the player's patch.
        lufs: Option<f32>,
    },
    /// For [`JobKind::Hint`].
    Hint(CoachHint),
    /// Any job that could not finish.
    Failed {
        /// Why, for the status line.
        message: String,
    },
}

impl JobResult {
    /// The result's name, for errors.
    pub fn name(&self) -> &'static str {
        match self {
            JobResult::Prepared { .. } => "prepared",
            JobResult::Scored(_) => "scored",
            JobResult::Rings(_) => "rings",
            JobResult::Loudness { .. } => "loudness",
            JobResult::Hint(_) => "hint",
            JobResult::Failed { .. } => "failed",
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
    Log(RoundLogEntry),
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

/// How a round stands or ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A submit passed.
    Passed,
    /// The player revealed the target without passing.
    Revealed,
    /// Ended without a pass or a reveal.
    Abandoned,
    /// The target could not be regenerated.
    Broken,
    /// Still being played (a dispute line).
    InProgress,
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
/// the agreement vote). Lane GK writes the exporter and `cargo xtask gr1-report`. Nothing leaves the machine unless
/// the player sends the file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoundLogEntry {
    /// [`ROUND_LOG_SCHEMA`].
    pub schema: String,
    /// Why this line was written.
    pub event: LogEvent,
    /// The round (lines of one round share it; a round that a host load restored after its `end` line may log
    /// again: keep the last `end`).
    pub round: RoundId,
    /// The challenge code (it carries the generator and engine versions).
    pub code: String,
    /// How the round stands or ended.
    pub outcome: Outcome,
    /// What ended it (`None` on a dispute line).
    pub ended_by: Option<EndedBy>,
    /// Active time in the round.
    pub active_ms: u64,
    /// The first passing submit's active time.
    pub time_to_pass_ms: Option<u64>,
    /// Every scored submit; the first is the recorded score.
    pub submits: Vec<SubmitRecord>,
    /// Hint levels used.
    pub hints: u8,
    /// The target was revealed.
    pub revealed: bool,
    /// Hints or a reveal were used.
    pub assisted: bool,
    /// The agreement vote.
    pub vote: Option<Vote>,
    /// The dispute note.
    pub dispute: Option<String>,
    /// Edits made (log entries plus those over the log's budget).
    pub edits: u32,
    /// The last scored submit's patch, as patch-file text.
    pub patch: Option<String>,
    /// The last scored submit's full report, while the session still holds it (not after a project reload).
    pub report: Option<ScoreReport>,
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
    /// The A/B state.
    pub ab: AbState,
    /// Hint levels used.
    pub hints: u8,
    /// The latest hint.
    pub hint: Option<CoachHint>,
    /// Hints or a reveal were used.
    pub assisted: bool,
    /// The target was revealed.
    pub revealed: bool,
    /// The agreement vote.
    pub vote: Option<Vote>,
    /// The last thing that went wrong in the round.
    pub error: Option<GameError>,
}

/// The editor's round intents (lane RUI). [`Session`] implements it; a kittest fake may too. Object-safe. Every
/// call is made on the GUI thread under the session lock, at the end of the frame like `SessionEdit`'s.
pub trait GameEdit {
    /// The round, if one is live.
    fn round(&self) -> Option<RoundView>;
    /// Start a round on `challenge` (a round already live ends first, logged; its stash carries over). Asks for a
    /// [`JobKind::Prepare`]; the patch changes only when it is done.
    fn game_start(&mut self, challenge: Challenge) -> Result<(), GameError>;
    /// Set the A/B switch. `Target` needs the target ready; `Target` and `Mine` need a live round.
    fn game_listen(&mut self, listen: Listen) -> Result<(), GameError>;
    /// Submit the current patch for scoring. Returns the submit's number (1 = first).
    fn game_submit(&mut self) -> Result<u32, GameError>;
    /// Ask for the next hint level (marks the round assisted). Returns the level asked for.
    fn game_hint(&mut self) -> Result<u8, GameError>;
    /// Give up: load the target (marks the round assisted; no more submits).
    fn game_reveal(&mut self) -> Result<(), GameError>;
    /// The agreement vote (after a scored submit); a later vote replaces it.
    fn game_vote(&mut self, vote: Vote) -> Result<(), GameError>;
    /// The Disagree button (after a scored submit): keeps the note and logs a dispute line at once.
    fn game_dispute(&mut self, note: &str) -> Result<(), GameError>;
    /// End the round (logged if it was played).
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
    /// The regenerated target differs from the one the round was played against.
    TargetDrift {
        /// The saved hash.
        expected: String,
        /// The regenerated target's hash.
        found: String,
    },
    /// A restored round's code does not decode.
    BadCode {
        /// Why.
        message: String,
    },
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
            GameError::WrongResult { .. } => "wrong_result",
            GameError::JobFailed { .. } => "job_failed",
            GameError::TargetDrift { .. } => "target_drift",
            GameError::BadCode { .. } => "bad_code",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            GameError::NoRound => "start a round first".to_owned(),
            GameError::NotReady => "the target is still being prepared: try again in a moment".to_owned(),
            GameError::Busy { what } => format!("the {what} is still being computed"),
            GameError::Revealed => "the target was revealed: start a new round".to_owned(),
            GameError::Broken | GameError::BadCode { .. } => {
                "this round cannot continue: end it (your own patch comes back)".to_owned()
            }
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
                "the regenerated target hashes to {found}, not {expected}"
            ),
            GameError::BadCode { message } => write!(f, "bad challenge code: {message}"),
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
    challenge: Option<Arc<Challenge>>,
    target: Option<Arc<Patch>>,
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

    fn job(&mut self, kind: JobKind) -> JobId {
        self.jobs.next += 1;
        let id = JobId(self.jobs.next);
        self.requests.push(GameRequest::Run(Job { id, kind }));
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

/// The round's byte budget for the edit log, given the other text it saves.
fn log_budget(record: &RoundRecord) -> usize {
    let other = record.stash.as_ref().map_or(0, String::len)
        + record.submitted_patch.as_ref().map_or(0, String::len)
        + record.dispute.as_ref().map_or(0, String::len);
    ROUND_BYTES.saturating_sub(other).min(EDIT_LOG_BYTES)
}

/// One log entry's size in the budget: its ops' JSON plus the entry's own keys.
fn entry_bytes(ops: &Value) -> usize {
    ops.to_string().len() + 48
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
        self.game.settle_at = None;
        let revision = self.game.revision;
        if self.game.jobs.loudness.is_none() {
            let id = self.game.job(JobKind::Loudness {
                challenge: Arc::clone(&challenge),
                patch: Box::new(self.patch.clone()),
            });
            self.game.jobs.loudness = Some((id, revision));
        }
        if scored && self.game.jobs.rings.is_none() {
            let id = self.game.job(JobKind::Rings {
                challenge,
                target,
                patch: Box::new(self.patch.clone()),
            });
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
            (
                "prepare",
                JobResult::Prepared {
                    start,
                    target,
                    target_lufs,
                },
            ) => {
                self.game_prepared(start, target, target_lufs);
                Ok(())
            }
            ("score", JobResult::Scored(report)) => {
                self.game_scored(report);
                Ok(())
            }
            ("rings", JobResult::Rings(knobs)) => {
                let measured = self.game.jobs.rings.take().map_or(0, |r| r.1);
                self.game.knobs = knobs;
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
                self.game.hint = Some(hint);
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

    fn game_prepared(&mut self, start: Box<Patch>, target: Box<Patch>, target_lufs: Option<f32>) {
        let hash = canonical_hash(&target).to_hex();
        let Some(record) = self.record_mut() else {
            return;
        };
        if let Some(expected) = &record.target_hash
            && *expected != hash
        {
            let expected = expected.clone();
            record.phase = Phase::Broken;
            self.game.listen = Listen::Off;
            self.game.error = Some(GameError::TargetDrift {
                expected,
                found: hash,
            });
            return;
        }
        record.target_hash = Some(hash);
        let fresh = record.phase == Phase::Preparing;
        self.game.target = Some(Arc::from(target));
        self.game.target_lufs = target_lufs;
        self.game.error = None;
        if fresh {
            self.game_begin(*start);
        }
        // Time counts from now, not from the start of the preparation; Mine's loudness (and, for a resumed round
        // that was scored, the rings) are measured at the next tick.
        self.game.clock_last = Some(self.game.now);
        self.game.settle_at = Some(self.game.now);
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
        if record.stash.is_none() {
            record.stash = Some(own);
            self.game.stash_history = Some(history);
        }
        record.phase = Phase::Playing;
        self.game.revision += 1;
        self.game.merge = None;
        self.game.dirty();
        self.seat_patch(start, None);
    }

    fn game_scored(&mut self, report: ScoreReport) {
        let Some(pending) = self.game.jobs.score.take() else {
            return;
        };
        let score = placeholder::summary(&report);
        let knobs = placeholder::knobs(&report);
        let Some(record) = self.record_mut() else {
            return;
        };
        record.submits.push(SubmitRecord {
            at_ms: pending.at_ms,
            score,
        });
        record.submitted_patch =
            (pending.patch.len() <= MAX_SUBMITTED_PATCH_BYTES).then_some(pending.patch);
        if score.passed && record.time_to_pass_ms.is_none() {
            record.time_to_pass_ms = Some(pending.at_ms);
        }
        if record.phase != Phase::Revealed {
            record.phase = if record.time_to_pass_ms.is_some() {
                Phase::Passed
            } else {
                Phase::Submitted
            };
        }
        self.game.knobs = knobs;
        self.game.report = Some(Arc::new(report));
        self.game.error = None;
        self.game.dirty();
    }

    /// End a drag where the whole patch is about to be replaced: its history step is closed without host writes
    /// (the host gestures it opened stay open until the editor's next `end_gesture`, as on a load).
    fn close_drag(&mut self) {
        self.gestures.open = None;
        self.history.end_gesture(&self.patch);
    }

    /// Replace the whole patch the way a load does (the round's start, a reveal, the end's restore): forget the
    /// engine's remembered host slot values, reset the host baseline (the next fold only records), release audition
    /// notes, send the plan (pumped at once), ask for a rename when the host would see other names, and mark the
    /// project dirty. The caller has already set the history.
    fn seat_patch(&mut self, patch: Patch, unloaded: Option<String>) {
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
    /// its transient state. Returns its record (with the stash) and the player's stashed history.
    fn game_close(&mut self, why: EndedBy) -> Option<(RoundRecord, Option<History>)> {
        let record = self.instance.game.take()?.round?;
        if record.played() {
            let entry = self.log_entry(&record, LogEvent::End, Some(why));
            self.game.requests.push(GameRequest::Log(entry));
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
        RoundLogEntry {
            schema: ROUND_LOG_SCHEMA.to_owned(),
            event,
            round: record.id,
            code: record.code.clone(),
            outcome: if event == LogEvent::Dispute {
                Outcome::InProgress
            } else {
                record.outcome()
            },
            ended_by,
            active_ms: record.active_ms,
            time_to_pass_ms: record.time_to_pass_ms,
            submits: record.submits.clone(),
            hints: record.hints,
            revealed: record.revealed,
            assisted: record.assisted(),
            vote: record.vote,
            dispute: record.dispute.clone(),
            edits: (record.edits.len() as u32).saturating_add(record.edits_dropped),
            patch: record.submitted_patch.clone(),
            report: self.game.report.as_deref().cloned(),
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
        let budget = log_budget(record);
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
        let loaded = self.instance.game.take().and_then(|g| g.round);
        let live = live.and_then(|g| g.round);
        if let (Some(l), Some(n)) = (&live, &loaded)
            && l.id == n.id
            && l.code == n.code
        {
            self.instance.game = Some(GameState { round: live });
            return;
        }
        if let Some(record) = live {
            self.instance.game = Some(GameState {
                round: Some(record),
            });
            let _ = self.game_close(EndedBy::Host);
        }
        if let Some(record) = loaded {
            self.game_restore(record);
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
        match placeholder::decode(&record.code) {
            Ok(challenge) if record.phase != Phase::Broken => {
                let challenge = Arc::new(challenge);
                self.game.challenge = Some(Arc::clone(&challenge));
                let id = self.game.job(JobKind::Prepare { challenge });
                self.game.jobs.prepare = Some(id);
            }
            Ok(_) => {}
            Err(message) => {
                record.phase = Phase::Broken;
                self.game.error = Some(GameError::BadCode { message });
            }
        }
        self.instance.game = Some(GameState {
            round: Some(record),
        });
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
            ab: self.game_ab(),
            hints: record.hints,
            hint: self.game.hint.clone(),
            assisted: record.assisted(),
            revealed: record.revealed,
            vote: record.vote,
            error: self.game.error.clone(),
        })
    }

    fn game_start(&mut self, challenge: Challenge) -> Result<(), GameError> {
        let (stash, history) = match self.game_close(EndedBy::NewRound) {
            Some((record, history)) => (record.stash, history),
            None => (None, None),
        };
        self.game.serial = self.game.serial.saturating_add(1);
        let challenge = Arc::new(challenge);
        let record = RoundRecord {
            id: RoundId {
                serial: self.game.serial,
                nonce: self.game.now,
            },
            code: challenge.code().to_owned(),
            stash,
            ..RoundRecord::default()
        };
        self.instance.game = Some(GameState {
            round: Some(record),
        });
        self.unloaded_instance = None; // the user's own state now (a newer build's `enth` text is replaced)
        self.game.stash_history = history;
        self.game.challenge = Some(Arc::clone(&challenge));
        let id = self.game.job(JobKind::Prepare { challenge });
        self.game.jobs.prepare = Some(id);
        self.game.dirty();
        Ok(())
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
        let patch = Box::new(self.patch.clone());
        let text = patch.to_json();
        let id = self.game.job(JobKind::Score {
            challenge,
            target,
            patch,
            submit,
        });
        self.game.jobs.score = Some(PendingScore {
            id,
            at_ms,
            patch: text,
        });
        Ok(submit)
    }

    fn game_hint(&mut self) -> Result<u8, GameError> {
        let (record, challenge, target) = self.open_round()?;
        if self.game.jobs.hint.is_some() {
            return Err(GameError::Busy { what: "hint" });
        }
        if record.hints >= HINT_LEVELS {
            return Err(GameError::NoMoreHints);
        }
        let level = record.hints + 1;
        let patch = Box::new(self.patch.clone());
        let id = self.game.job(JobKind::Hint {
            challenge,
            target,
            patch,
            level,
        });
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
        if let Some(record) = self.record_mut() {
            record.revealed = true;
            record.phase = Phase::Revealed;
        }
        self.game_log(EditSource::Reveal, Value::Array(Vec::new()), None);
        self.game.dirty();
        Ok(())
    }

    fn game_vote(&mut self, vote: Vote) -> Result<(), GameError> {
        let record = self.record_mut().ok_or(GameError::NoRound)?;
        if record.submits.is_empty() {
            return Err(GameError::NotScored);
        }
        record.vote = Some(vote);
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
        self.game.requests.push(GameRequest::Log(entry));
        self.game.dirty();
        Ok(())
    }

    fn game_end(&mut self, choice: EndChoice) -> Result<(), GameError> {
        let why = match choice {
            EndChoice::Restore => EndedBy::Restore,
            EndChoice::Keep => EndedBy::Keep,
        };
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
        let s = ScoreSummary::from_percent(true, 87.456, -3.0, f32::NAN);
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
    fn the_log_budget_shrinks_so_the_round_fits_the_field() {
        let mut record = RoundRecord::default();
        assert_eq!(log_budget(&record), EDIT_LOG_BYTES);
        record.stash = Some("x".repeat(600 * 1024));
        assert_eq!(log_budget(&record), ROUND_BYTES - 600 * 1024);
        record.submitted_patch = Some("x".repeat(200 * 1024));
        assert_eq!(log_budget(&record), 0);
        const { assert!(ROUND_BYTES + 64 * 1024 < crate::state::MAX_FIELD_BYTES) };
    }

    #[test]
    fn listen_flips_between_target_and_mine() {
        assert_eq!(Listen::Off.flipped(), Listen::Target);
        assert_eq!(Listen::Target.flipped(), Listen::Mine);
        assert_eq!(Listen::Mine.flipped(), Listen::Target);
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
