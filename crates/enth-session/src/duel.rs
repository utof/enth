//! **Owner: lane SE** (Session, freeze point F1e; the F1e judge reviews the API with the other R4 and R5 APIs).
//! Pre-declared by lane S0e (R4/R5, freeze point F0e); written by lane SE.
//!
//! The pure duel state (docs/duels.md, "The flow"; docs/session.md, "Duel"). It does no I/O, spawns no threads and
//! reads no clocks and **has no networking: the clipboard is the shell's**, written on a click and read only on an
//! explicit Paste. Background work runs as jobs on the shell's game worker.
//!
//! A duel is a challenge code the player and a friend both play, plus what the friend sent back. The player plays it
//! as an ordinary Recreate round ([`game`](crate::game)); this module is everything around the round.
//!
//! - **The player's result** ([`DuelOwn`], [`DuelResult`]) is built from the round's **first passing submit**
//!   (otherwise the best submit, marked "not passed"): its scores, the time to pass, the submit count, the assists, and
//!   its patch. `game` keeps a pass in the saved round as a diff from the start
//!   ([`ComparedSubmit`](crate::game::ComparedSubmit), within `ROUND_BYTES`, so it survives a reload); the best submit of
//!   a round that has not passed is held in memory only, since it can still change. It is read from the live round, and **kept when the round ends** while a duel is open on
//!   its code ([`Session::duel_note_close`]), so the answer can still be shared after the player ended the round. It
//!   is `settled` once the compared submit can no longer change (a pass, a give-up, the round's end), and only a
//!   settled result with no reveal can be shared: **a revealed target gives no scorecard.** The first played attempt
//!   at a duel's code stands (a rematch is a new duel).
//! - **The friend's scorecard and answer arrive as data** ([`FriendScorecard`], [`FriendAnswer`]): the shell decodes
//!   them with `enth-challenge`'s paste-anything and result codes and hands over the fields the view shows plus the
//!   code text, which the session never parses beyond the challenge code. The scorecard is shown at once, as
//!   **reported** by their app. **The answer stays sealed until the player has submitted or given up**
//!   ([`AnswerState::Sealed`]): a session guard so the view, the verify job, "use" and audition refuse until then, not
//!   cryptography (the source is open and the code decodes).
//! - **The Verify job** ([`DuelRequest::Verify`]) goes out to the shell, which runs `enth-challenge`'s `verify` on the
//!   game worker and posts back ([`Session::duel_job_done`]): the receiver's own re-score of both answers, whether
//!   the friend's answer matches its commitment and their claim, and their patch. **The receiver's scores decide;**
//!   a claimed number never does, and the session only stores what the worker measured.
//! - **Loading their patch** ([`DuelEdit::duel_use_answer`]) is one undo step with no host writes (the session's
//!   preset load, as "Use this" in Create), refused while the answer is sealed, before it is checked, and while a
//!   round is live. Their patch can also be auditioned ([`DuelRequest::Audition`]).
//! - **Nothing is saved in the project.** Duels live in the per-user history the shell owns (lane DUI): this module
//!   adds no field to the `enth` state blob, `enth.plugin-state/1` is untouched (the round record gains one optional
//!   field, [`RoundRecord::compared`](crate::game::RoundRecord::compared)), and a host load leaves a duel alone.
//! - **The salt is the shell's.** The scorecard's commitment is salted; a core crate has no randomness, so the shell
//!   draws the salt once per duel, keeps it with the result in the history and gives it to `enth-challenge`'s code
//!   builders. The session builds no code.
//!
//! The session takes no type from `enth-challenge`'s `result`, `answer` or `verify` (lane RC builds them in
//! parallel): the seam is plain data, in the units the session uses (scores in hundredths of a percent, as
//! [`ScoreSummary`]; the claim in tenths, as the scorecard carries it).
//!
//! After F2d the session's duel types change only additively (docs/phase-4.md, "Freezes"): the enums and the structs
//! the shell builds are `non_exhaustive`, with constructors.

use enth_challenge::{AssistsUsed, ChallengeCode, CodeError, EngineVersion, Outcome, ScoringSpec};
use enth_patch::Patch;

use crate::game::{RoundOrigin, RoundRecord, ScoreSummary, SubmitRecord, compared_number};
use crate::model::{Session, SessionError};

/// The most text a pasted code may be (a deflated whole patch is about 1,100 symbols; this is far above any code and
/// far below anything that could make the view heavy).
pub const MAX_CODE_TEXT: usize = 8 * 1024;

/// A duel's id: a job's result carries its job id; this names the duel in the view and in the audition request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DuelId(pub u64);

/// A verify job's id: its result carries it; a result for a job the duel no longer waits for is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DuelJobId(pub u64);

/// The player's own result, without the patch: what the duel card shows and the shell builds the scorecard from
/// (with [`DuelResult::answer`] for the answer code). Scores are hundredths of a percent.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DuelOwn {
    /// The challenge code (its canonical text).
    pub code: String,
    /// How the round stands: passed, not passed, or gave up.
    pub outcome: Outcome,
    /// Which scored submit is compared (1 = the first): the first passing one, else the best. `None` for a round given
    /// up before any submit.
    pub number: Option<u32>,
    /// The compared submit's scores; `passed` is false for a best-of-a-failed-round ("not passed").
    pub score: Option<ScoreSummary>,
    /// Scored submits in the round.
    pub submits: u32,
    /// The time to pass, in editor-visible milliseconds (**reported**: the shell's clock, not checkable). `None`
    /// without a pass.
    pub time_ms: Option<u64>,
    /// Active time of the whole round.
    pub active_ms: u64,
    /// The help used (hints, reveal, the dim assist, "How close am I?").
    pub assists: AssistsUsed,
    /// The engine that scored it (this build's).
    pub engine: EngineVersion,
    /// The scorer ids the round was judged with, once its challenge was prepared (the scorecard names them as a set).
    pub scoring: Option<ScoringSpec>,
    /// The compared submit can no longer change: it passed, the target was revealed, or the round ended.
    pub settled: bool,
    /// The compared submit's patch is kept, so an answer code can be made. False while the target is regenerated after
    /// a reload, when the diff did not fit [`COMPARED_BYTES`](crate::game::COMPARED_BYTES), and for the best submit of a
    /// round that has not passed once a reload has forgotten it (the next best submit brings it back).
    pub answer_kept: bool,
}

impl DuelOwn {
    /// The scorecard may be sent: the result is settled, has a score and the target was not revealed (**a revealed
    /// target gives no scorecard**).
    #[must_use]
    pub fn can_share(&self) -> bool {
        self.settled && self.score.is_some() && !self.assists.revealed
    }

    /// The answer code may be sent after both have played: [`can_share`](Self::can_share) and the patch is kept.
    #[must_use]
    pub fn can_share_answer(&self) -> bool {
        self.can_share() && self.answer_kept
    }
}

/// The player's own result with its patch ([`DuelEdit::duel_own`]): what the shell keeps in the duel history and
/// builds the codes from. Clone-heavy (the patch), so ask on a click, not every frame.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DuelResult {
    /// The result.
    pub own: DuelOwn,
    /// The compared submit's patch (the round's start with the kept diff applied); `None` unless
    /// [`DuelOwn::answer_kept`].
    pub answer: Option<Box<Patch>>,
}

impl DuelResult {
    /// A result restored from the shell's history ([`DuelEdit::duel_set_own`]).
    #[must_use]
    pub fn new(own: DuelOwn, answer: Option<Patch>) -> Self {
        let mut own = own;
        own.answer_kept = answer.is_some();
        DuelResult {
            own,
            answer: answer.map(Box::new),
        }
    }
}

impl DuelOwn {
    /// A result for the shell to restore (a duel history entry): the challenge `code`, how the round `outcome` stood,
    /// its `submits` and `active_ms`. The shell sets the rest with the builders; the result is settled.
    #[must_use]
    pub fn new(code: impl Into<String>, outcome: Outcome, submits: u32, active_ms: u64) -> Self {
        DuelOwn {
            code: code.into(),
            outcome,
            number: None,
            score: None,
            submits,
            time_ms: None,
            active_ms,
            assists: AssistsUsed::default(),
            engine: EngineVersion::CURRENT,
            scoring: None,
            settled: true,
            answer_kept: false,
        }
    }

    /// With the compared submit: its number among the round's submits and its scores.
    #[must_use]
    pub fn with_score(mut self, number: u32, score: ScoreSummary) -> Self {
        self.number = Some(number);
        self.score = Some(score);
        self
    }

    /// With the time to pass (reported).
    #[must_use]
    pub fn with_time(mut self, time_ms: u64) -> Self {
        self.time_ms = Some(time_ms);
        self
    }

    /// With the help used.
    #[must_use]
    pub fn with_assists(mut self, assists: AssistsUsed) -> Self {
        self.assists = assists;
        self
    }

    /// With the engine that scored it.
    #[must_use]
    pub fn with_engine(mut self, engine: EngineVersion) -> Self {
        self.engine = engine;
        self
    }

    /// With the scorers it was judged with.
    #[must_use]
    pub fn with_scoring(mut self, scoring: ScoringSpec) -> Self {
        self.scoring = Some(scoring);
        self
    }
}

/// A friend's scorecard as their app **reported** it: the shell decodes the scorecard code (kind 1 behind version 15)
/// and hands over what the card shows, with the code's text for `verify` (which checks the commitment against the
/// answer's). The session takes nothing on trust: the receiver's re-score decides.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FriendScorecard {
    /// The challenge code the card is for (its text; it must decode).
    pub code: String,
    /// The scorecard code as pasted, normalised by paste-anything.
    pub text: String,
    /// The engine their app ran.
    pub engine: EngineVersion,
    /// Their sound %, in tenths of a percent (0 to 1000), as the card carries it.
    pub sound_tenths: u16,
    /// Their combined %, in tenths of a percent.
    pub combined_tenths: u16,
    /// Their compared submit passed.
    pub passed: bool,
    /// Their submit count (reported).
    pub submits: u32,
    /// Their time to pass, in whole seconds (reported); `None` without a pass.
    pub time_s: Option<u32>,
    /// Their assists (reported).
    pub assists: AssistsUsed,
}

impl FriendScorecard {
    /// A scorecard from its decoded fields; the rest defaults and the shell sets it with the builders.
    #[must_use]
    pub fn new(
        code: impl Into<String>,
        text: impl Into<String>,
        engine: EngineVersion,
        sound_tenths: u16,
        combined_tenths: u16,
        passed: bool,
    ) -> Self {
        FriendScorecard {
            code: code.into(),
            text: text.into(),
            engine,
            sound_tenths,
            combined_tenths,
            passed,
            submits: 0,
            time_s: None,
            assists: AssistsUsed::default(),
        }
    }

    /// With their submit count and time to pass.
    #[must_use]
    pub fn with_play(mut self, submits: u32, time_s: Option<u32>) -> Self {
        self.submits = submits;
        self.time_s = time_s;
        self
    }

    /// With their assists.
    #[must_use]
    pub fn with_assists(mut self, assists: AssistsUsed) -> Self {
        self.assists = assists;
        self
    }

    fn check(&self) -> Result<(), DuelError> {
        if self.text.len() > MAX_CODE_TEXT {
            return Err(DuelError::BadImport("the scorecard text is too long"));
        }
        if self.sound_tenths > 1000 || self.combined_tenths > 1000 {
            return Err(DuelError::BadImport("a score is above 100 %"));
        }
        Ok(())
    }
}

/// A friend's answer code (kind 2 behind version 15): the challenge it is for and the code text. The session holds it
/// **sealed** until the player has played, and relays the text to the Verify job, which decodes it
/// (`answer_too_new`, `bad_check` and a hash mismatch are `verify`'s refusals).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FriendAnswer {
    /// The challenge code the answer is for (its text; it must decode).
    pub code: String,
    /// The answer code as pasted, normalised by paste-anything.
    pub text: String,
}

impl FriendAnswer {
    /// An answer for the challenge `code`.
    #[must_use]
    pub fn new(code: impl Into<String>, text: impl Into<String>) -> Self {
        FriendAnswer {
            code: code.into(),
            text: text.into(),
        }
    }

    fn check(&self) -> Result<(), DuelError> {
        if self.text.is_empty() {
            return Err(DuelError::BadImport("the answer code is empty"));
        }
        if self.text.len() > MAX_CODE_TEXT {
            return Err(DuelError::BadImport("the answer text is too long"));
        }
        Ok(())
    }
}

/// A score the receiver's build measured, in hundredths of a percent ([`ScoreSummary`]'s unit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rescored {
    /// Sound %, × 100.
    pub sound: u16,
    /// Combined %, × 100.
    pub combined: u16,
    /// Sound is at or above the challenge's pass mark.
    pub passed: bool,
}

impl Rescored {
    /// A re-score from the worker's percentages' hundredths.
    #[must_use]
    pub fn new(sound: u16, combined: u16, passed: bool) -> Self {
        Rescored {
            sound,
            combined,
            passed,
        }
    }
}

/// The Verify job: the shell runs `enth-challenge`'s `verify` for `answer` (decode, regenerate `code`'s target, apply
/// the ops to the start and check the hash, check the scorecard's commitment when `card` is at hand, score with the
/// named scorer set, compare with the claim) and, with the same evaluator, re-scores `own`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DuelVerify {
    /// Its id (the result carries it back).
    pub job: DuelJobId,
    /// The duel.
    pub duel: DuelId,
    /// The challenge code.
    pub code: String,
    /// The player's own compared patch, to re-score beside theirs; `None` when the player gave up with nothing to
    /// score.
    pub own: Option<Box<Patch>>,
    /// Their scorecard's code text, when one was imported.
    pub card: Option<String>,
    /// Their answer code's text.
    pub answer: String,
}

/// What the Verify job measured.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DuelVerified {
    /// The player's own patch, re-scored here. Present exactly when the job carried one.
    pub own: Option<Rescored>,
    /// Their patch, re-scored here.
    pub theirs: Rescored,
    /// Their patch (the start with their ops applied, the hash checked): what "use" loads and audition plays.
    pub theirs_patch: Option<Box<Patch>>,
    /// Their answer matches the scorecard's commitment; `None` without a scorecard.
    pub commitment_ok: Option<bool>,
    /// Their re-score matches the scorecard's claim (compared at the claim's precision, by `verify`); `None` without
    /// a scorecard. A mismatch on the same engine is a flag, not a verdict: the receiver's scores decide.
    pub matches_claim: Option<bool>,
}

impl DuelVerified {
    /// A verified answer with its re-scores; the checks default to none.
    #[must_use]
    pub fn new(own: Option<Rescored>, theirs: Rescored) -> Self {
        DuelVerified {
            own,
            theirs,
            theirs_patch: None,
            commitment_ok: None,
            matches_claim: None,
        }
    }

    /// With their patch.
    #[must_use]
    pub fn with_patch(mut self, patch: Patch) -> Self {
        self.theirs_patch = Some(Box::new(patch));
        self
    }

    /// With the commitment and claim checks.
    #[must_use]
    pub fn with_checks(mut self, commitment_ok: Option<bool>, matches_claim: Option<bool>) -> Self {
        self.commitment_ok = commitment_ok;
        self.matches_claim = matches_claim;
        self
    }

    fn check(&self, had_own: bool) -> Result<(), DuelError> {
        let in_range = |r: &Rescored| r.sound <= 10_000 && r.combined <= 10_000;
        if !in_range(&self.theirs) || self.own.as_ref().is_some_and(|o| !in_range(o)) {
            return Err(DuelError::BadResult("a score is above 100 %"));
        }
        if self.own.is_some() != had_own {
            return Err(DuelError::BadResult(
                "the own answer was re-scored or left out against the job",
            ));
        }
        Ok(())
    }
}

/// A job's answer ([`Session::duel_job_done`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DuelJobResult {
    /// For [`DuelRequest::Verify`].
    Verified(Box<DuelVerified>),
    /// The job could not finish: `code_too_new`, `scorer_too_new`, `answer_too_new`, `bad_check`, a generator or hash
    /// mismatch (refused: the round must never be judged against a different target or a damaged answer).
    Failed {
        /// A stable snake-case code (`enth-challenge`'s).
        code: String,
        /// What happened.
        message: String,
        /// What to do about it.
        hint: String,
    },
}

/// The duel outbox the shell drains ([`Session::drain_duel_requests`]) and acts on after unlocking.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DuelRequest {
    /// Run `verify` on the game worker and post the result with [`Session::duel_job_done`].
    Verify(DuelVerify),
    /// The job is no longer wanted (the answer was replaced, the duel closed): drop it if it has not run; a result that
    /// arrives anyway is stale.
    Cancel(DuelJobId),
    /// Render `patch` with the duel's challenge phrase at the host's rate and play it once.
    Audition {
        /// The duel.
        duel: DuelId,
        /// The challenge code (the phrase to play it with).
        code: String,
        /// Their checked patch.
        patch: Box<Patch>,
    },
    /// Stop an audition.
    StopAudition,
}

/// Whether the friend's answer may be seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnswerState {
    /// No answer imported.
    None,
    /// Imported and **sealed** until the player has submitted or given up: nothing of it is in the view, and the verify,
    /// use and audition intents refuse.
    Sealed,
    /// Imported and open: the player has played.
    Open,
}

/// An error the Verify job posted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuelFailure {
    /// A stable snake-case code.
    pub code: String,
    /// What happened.
    pub message: String,
    /// What to do about it.
    pub hint: String,
}

/// Where the verification stands.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VerifyState {
    /// Not asked for, or asked for before and the inputs changed since.
    Idle,
    /// The job runs in the shell.
    Running,
    /// Done ([`DuelView::checked`]).
    Done,
    /// Refused or failed; ask again after fixing the input.
    Failed(DuelFailure),
}

/// What the receiver's own build measured.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DuelChecked {
    /// The player's own compared patch, re-scored beside theirs.
    pub own: Option<Rescored>,
    /// Their patch, re-scored.
    pub theirs: Rescored,
    /// Their answer matches the scorecard's commitment; `None` without a scorecard.
    pub commitment_ok: Option<bool>,
    /// Their re-score matches the scorecard's claim; `None` without a scorecard.
    pub matches_claim: Option<bool>,
    /// Their patch is at hand, so "use" and audition work.
    pub has_patch: bool,
    /// The player's compared submit is still the one that was re-scored. False when the round went on to a better
    /// submit since the check (a round that has not passed yet), so the view marks the own score as out of date.
    pub own_current: bool,
}

/// The duel card's read model ([`DuelEdit::duel`]), rebuilt on every call. It holds no patch, and **nothing of a sealed
/// answer**.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DuelView {
    /// The duel.
    pub id: DuelId,
    /// The challenge code (canonical text).
    pub code: String,
    /// The player's own result, once they have submitted or given up.
    pub own: Option<DuelOwn>,
    /// Their scorecard as reported (shown at once: it spoils nothing).
    pub card: Option<FriendScorecard>,
    /// Their scorecard comes from another engine version than this build's: the view shows both numbers.
    pub engine_differs: bool,
    /// Whether their answer is held, sealed or open.
    pub answer: AnswerState,
    /// The verification.
    pub verify: VerifyState,
    /// What it measured.
    pub checked: Option<DuelChecked>,
    /// Their patch was loaded as one undo step.
    pub used: bool,
    /// Their patch was asked to play last.
    pub auditioning: bool,
}

/// Why the session refused a duel intent or post.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DuelError {
    /// No duel is open (open one, or import a scorecard).
    NoDuel,
    /// The challenge code does not decode (`code_too_new`, `bad_check`, …): the inner code passes through.
    Code(CodeError),
    /// The import is for another challenge than the open duel's.
    OtherChallenge {
        /// The open duel's code.
        expected: String,
        /// The imported code.
        found: String,
    },
    /// An import that does not fit (a score above 100 %, an empty or oversized code text).
    BadImport(&'static str),
    /// Their answer is sealed: submit or give up first.
    Sealed,
    /// No answer was imported.
    NoAnswer,
    /// A verify job is running.
    Busy,
    /// A result for a job the duel no longer waits for.
    Stale(DuelJobId),
    /// The worker's result does not fit the job (a shell bug).
    BadResult(&'static str),
    /// Their patch is not at hand: verify first.
    NotChecked,
    /// A round is live: end it (Restore or Keep) before loading their patch.
    RoundLive,
    /// The duel holds the player's result already: the first played attempt stands.
    AlreadyPlayed,
    /// The result restored has no submit and no give-up.
    NotPlayed,
    /// The load was refused (a lesson is live, or the patch does not apply).
    Session(SessionError),
}

impl DuelError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            DuelError::NoDuel => "no_duel",
            DuelError::Code(e) => e.code(),
            DuelError::OtherChallenge { .. } => "other_challenge",
            DuelError::BadImport(_) => "bad_import",
            DuelError::Sealed => "answer_sealed",
            DuelError::NoAnswer => "no_answer",
            DuelError::Busy => "busy",
            DuelError::Stale(_) => "stale_job",
            DuelError::BadResult(_) => "wrong_result",
            DuelError::NotChecked => "not_checked",
            DuelError::RoundLive => "round_live",
            DuelError::AlreadyPlayed => "already_played",
            DuelError::NotPlayed => "not_played",
            DuelError::Session(e) => e.code(),
        }
    }

    /// What to do about it.
    #[must_use]
    pub fn hint(&self) -> String {
        match self {
            DuelError::NoDuel => "open a duel with a challenge code, or paste your friend's scorecard".to_owned(),
            DuelError::Code(e) => e.hint(),
            DuelError::OtherChallenge { .. } => {
                "this is for another challenge: close this duel first, or play that one".to_owned()
            }
            DuelError::BadImport(what) => format!("paste the code again: {what}"),
            DuelError::Sealed => {
                "their answer opens once you have submitted or given up".to_owned()
            }
            DuelError::NoAnswer => "paste your friend's answer code first".to_owned(),
            DuelError::Busy => "a check is already running: wait for it".to_owned(),
            DuelError::Stale(_) => "a result for a check that is gone: nothing to do".to_owned(),
            DuelError::BadResult(_) => {
                "report this: the shell answered a duel job with a result that does not fit it".to_owned()
            }
            DuelError::NotChecked => "check their answer first: their patch is read from it".to_owned(),
            DuelError::RoundLive => {
                "a round is in progress: end it (Restore brings your own patch back), then load their patch".to_owned()
            }
            DuelError::AlreadyPlayed => {
                "your first attempt stands: open the code as a new duel to play it again".to_owned()
            }
            DuelError::NotPlayed => "submit once, or give up, before a result counts".to_owned(),
            DuelError::Session(e) => e.hint(),
        }
    }
}

impl std::fmt::Display for DuelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DuelError::NoDuel => f.write_str("no duel is open"),
            DuelError::Code(e) => write!(f, "challenge code refused: {e}"),
            DuelError::OtherChallenge { expected, found } => {
                write!(f, "the duel is on {expected}, not {found}")
            }
            DuelError::BadImport(what) => write!(f, "bad import: {what}"),
            DuelError::Sealed => f.write_str("their answer is sealed"),
            DuelError::NoAnswer => f.write_str("no answer was imported"),
            DuelError::Busy => f.write_str("a verify job is running"),
            DuelError::Stale(id) => write!(f, "duel job {} is not awaited", id.0),
            DuelError::BadResult(what) => write!(f, "the verify result does not fit: {what}"),
            DuelError::NotChecked => f.write_str("their patch is not checked yet"),
            DuelError::RoundLive => f.write_str("a round is live"),
            DuelError::AlreadyPlayed => f.write_str("the duel holds your result already"),
            DuelError::NotPlayed => f.write_str("the result has no submit and no give-up"),
            DuelError::Session(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DuelError {}

/// The editor's duel intents and read model. [`Session`] implements it; a kittest fake can. Object-safe. Every call is
/// made on the GUI thread under the session lock, at the end of the frame like `SessionEdit`'s.
pub trait DuelEdit {
    /// The open duel, if there is one.
    fn duel(&self) -> Option<DuelView>;
    /// The player's compared result with its patch. With a duel open: the duel's (the kept one once the round ended,
    /// else the live round's on the duel's code). With none: the live game round's, whatever its code, so the share
    /// line can be made right after a pass. `None` until the player has submitted or given up. Clones the patch: ask on
    /// a click.
    fn duel_own(&self) -> Option<DuelResult>;
    /// Open a duel on a challenge code (any text `ChallengeCode::decode` reads; the duel keeps its canonical form). A
    /// duel already open is closed first (a running check is cancelled). This starts no round: the editor starts the
    /// Recreate round with `game_start` as for any code.
    ///
    /// # Errors
    /// [`DuelError::Code`].
    fn duel_open(&mut self, code: &str) -> Result<DuelId, DuelError>;
    /// Restore the player's result from the shell's history (a duel reopened after the round ended or the project
    /// closed). The first played attempt stands: refused when the duel holds one.
    ///
    /// # Errors
    /// [`DuelError::NoDuel`], [`OtherChallenge`](DuelError::OtherChallenge), [`NotPlayed`](DuelError::NotPlayed),
    /// [`AlreadyPlayed`](DuelError::AlreadyPlayed), [`BadImport`](DuelError::BadImport).
    fn duel_set_own(&mut self, own: DuelResult) -> Result<(), DuelError>;
    /// Import their scorecard, as reported. With no duel open it opens one on the card's challenge; a card replaces an
    /// earlier one (a running check is cancelled, the check forgotten).
    ///
    /// # Errors
    /// [`DuelError::Code`], [`OtherChallenge`](DuelError::OtherChallenge) when another duel is open,
    /// [`BadImport`](DuelError::BadImport).
    fn duel_import_scorecard(&mut self, card: FriendScorecard) -> Result<DuelId, DuelError>;
    /// Import their answer code. It is held **sealed** until the player has submitted or given up. With no duel open
    /// it opens one on the answer's challenge; an answer replaces an earlier one (a running check is cancelled).
    ///
    /// # Errors
    /// [`DuelError::Code`], [`OtherChallenge`](DuelError::OtherChallenge), [`BadImport`](DuelError::BadImport).
    fn duel_import_answer(&mut self, answer: FriendAnswer) -> Result<DuelId, DuelError>;
    /// Check their answer: a [`DuelRequest::Verify`] goes out. At most one runs.
    ///
    /// # Errors
    /// [`DuelError::NoDuel`], [`NoAnswer`](DuelError::NoAnswer), [`Sealed`](DuelError::Sealed),
    /// [`Busy`](DuelError::Busy).
    fn duel_verify(&mut self) -> Result<DuelJobId, DuelError>;
    /// Load their checked patch as **one undo step** with no host writes, named `name` (empty keeps the patch's own).
    ///
    /// # Errors
    /// [`DuelError::NoDuel`], [`Sealed`](DuelError::Sealed), [`NotChecked`](DuelError::NotChecked),
    /// [`RoundLive`](DuelError::RoundLive), [`Session`](DuelError::Session) (a lesson is live).
    fn duel_use_answer(&mut self, name: &str) -> Result<(), DuelError>;
    /// Play their checked patch once, rendered by the shell with the challenge's phrase.
    ///
    /// # Errors
    /// [`DuelError::NoDuel`], [`Sealed`](DuelError::Sealed), [`NotChecked`](DuelError::NotChecked).
    fn duel_audition(&mut self) -> Result<(), DuelError>;
    /// Stop an audition.
    fn duel_stop_audition(&mut self);
    /// Close the duel: a running check is cancelled and its result is stale.
    fn duel_close(&mut self);
}

/// The open duel.
#[derive(Debug)]
struct DuelRun {
    id: DuelId,
    code: String,
    /// The player's result, kept when the round ended or restored by the shell (the first attempt stands).
    own: Option<DuelResult>,
    card: Option<FriendScorecard>,
    answer: Option<FriendAnswer>,
    verify: VerifyLive,
    /// The compared submit's number when the last check was asked for: a check made while the round ran stands when the
    /// round ends on the same compared submit, and is forgotten when it moved on.
    check_number: Option<u32>,
    checked: Option<DuelChecked>,
    theirs_patch: Option<Box<Patch>>,
    used: bool,
    auditioning: bool,
}

#[derive(Debug)]
enum VerifyLive {
    Idle,
    Running { job: DuelJobId, had_own: bool },
    Done,
    Failed(DuelFailure),
}

/// The duel state the session keeps between calls (never saved).
#[derive(Debug, Default)]
pub(crate) struct DuelLive {
    next_id: u64,
    next_job: u64,
    run: Option<DuelRun>,
    requests: Vec<DuelRequest>,
}

impl DuelLive {
    /// Close the open duel for good: cancel its job and its audition.
    fn drop_run(&mut self) {
        if let Some(run) = self.run.take() {
            if let VerifyLive::Running { job, .. } = run.verify {
                self.requests.push(DuelRequest::Cancel(job));
            }
            if run.auditioning {
                self.requests.push(DuelRequest::StopAudition);
            }
        }
    }
}

impl DuelRun {
    /// The inputs of a check changed: forget it, and cancel a running one.
    fn forget_check(&mut self, requests: &mut Vec<DuelRequest>) {
        if let VerifyLive::Running { job, .. } = self.verify {
            requests.push(DuelRequest::Cancel(job));
        }
        self.verify = VerifyLive::Idle;
        self.checked = None;
        self.theirs_patch = None;
        self.used = false;
        if self.auditioning {
            self.auditioning = false;
            requests.push(DuelRequest::StopAudition);
        }
    }
}

/// The canonical text of a challenge code, or why it is refused.
fn canonical(code: &str) -> Result<String, DuelError> {
    ChallengeCode::decode(code)
        .map(|c| c.to_string())
        .map_err(DuelError::Code)
}

impl Session {
    /// The duel outbox: [`DuelRequest`]s for the shell to act on after unlocking (wherever `drain_requests` is
    /// drained).
    pub fn drain_duel_requests(&mut self) -> std::vec::Drain<'_, DuelRequest> {
        self.duel.requests.drain(..)
    }

    /// Hand back the result of job `job` (under the lock, on the GUI or host main thread). A result for a job the duel
    /// no longer waits for (the duel closed, the answer replaced, another check since) is [`DuelError::Stale`] and
    /// changes nothing.
    ///
    /// # Errors
    /// [`DuelError::Stale`]; [`DuelError::BadResult`] for a result that does not fit the job (the check fails and can
    /// be asked again).
    pub fn duel_job_done(
        &mut self,
        job: DuelJobId,
        result: DuelJobResult,
    ) -> Result<(), DuelError> {
        let Some(run) = self.duel.run.as_mut() else {
            return Err(DuelError::Stale(job));
        };
        let had_own = match run.verify {
            VerifyLive::Running {
                job: waiting,
                had_own,
            } if waiting == job => had_own,
            _ => return Err(DuelError::Stale(job)),
        };
        match result {
            DuelJobResult::Failed {
                code,
                message,
                hint,
            } => {
                run.verify = VerifyLive::Failed(DuelFailure {
                    code,
                    message,
                    hint,
                });
                Ok(())
            }
            DuelJobResult::Verified(verified) => {
                if let Err(e) = verified.check(had_own) {
                    run.verify = VerifyLive::Failed(DuelFailure {
                        code: e.code().to_owned(),
                        message: e.to_string(),
                        hint: e.hint(),
                    });
                    return Err(e);
                }
                let DuelVerified {
                    own,
                    theirs,
                    theirs_patch,
                    commitment_ok,
                    matches_claim,
                } = *verified;
                run.checked = Some(DuelChecked {
                    own,
                    theirs,
                    commitment_ok,
                    matches_claim,
                    has_patch: theirs_patch.is_some(),
                    own_current: true,
                });
                run.theirs_patch = theirs_patch;
                run.verify = VerifyLive::Done;
                Ok(())
            }
        }
    }

    /// A round just closed (called from [`Session::game_close`] with the closed record, while the round's challenge
    /// and start are still held): an open duel on the round's code keeps the player's result, if they submitted or
    /// gave up and the duel holds no attempt yet. A round that was never played, or played for another code, changes
    /// nothing.
    pub(crate) fn duel_note_close(&mut self, record: &RoundRecord) {
        let wanted = self
            .duel
            .run
            .as_ref()
            .is_some_and(|run| run.own.is_none() && run.code == record.code);
        if !wanted {
            return;
        }
        let Some(summary) = self.duel_summary(record, true) else {
            return;
        };
        let answer = self.duel_answer(record);
        let mut own = summary;
        own.answer_kept = answer.is_some();
        if let Some(run) = self.duel.run.as_mut() {
            // A check made while the round ran stands when the compared submit is still the one it was made for.
            let same = run.check_number == own.number;
            run.own = Some(DuelResult {
                own,
                answer: answer.map(Box::new),
            });
            if !same {
                run.forget_check(&mut self.duel.requests);
            }
        }
    }

    /// The player's result for the live game round, if it has one: a game round (not a lesson's or Review's) whose
    /// target was ready and that has a scored submit or was revealed. `settled` when the compared submit can no longer
    /// change; `force_settled` (the round is ending) settles it.
    fn duel_summary(&self, record: &RoundRecord, force_settled: bool) -> Option<DuelOwn> {
        if record.origin != RoundOrigin::Game
            || !record.played()
            || (record.submits.is_empty() && !record.revealed)
        {
            return None;
        }
        let number = compared_number(&record.submits);
        let compared: Option<&SubmitRecord> =
            number.and_then(|n| record.submits.get(n as usize - 1));
        let outcome = record.outcome();
        let kept = number.is_some_and(|n| {
            let saved = record
                .compared
                .as_ref()
                .is_some_and(|c| c.submit == n && c.ops.is_some())
                && self.game_start_patch().is_some();
            saved || self.game_best_patch(n).is_some()
        });
        Some(DuelOwn {
            code: record.code.clone(),
            outcome,
            number,
            score: compared.map(|s| s.score),
            submits: record.submits.len() as u32,
            time_ms: record.time_to_pass_ms,
            active_ms: record.active_ms,
            assists: record.assists(),
            engine: EngineVersion::CURRENT,
            scoring: self.game_challenge().map(|c| c.scoring.clone()),
            settled: force_settled || outcome == Outcome::Passed || record.revealed,
            answer_kept: kept,
        })
    }

    /// The compared submit's patch: the round's start with the kept diff applied. `None` when the diff was not kept or
    /// the start is not here yet (the target is being regenerated after a reload).
    fn duel_answer(&self, record: &RoundRecord) -> Option<Patch> {
        let number = compared_number(&record.submits)?;
        if let Some(best) = self.game_best_patch(number) {
            return Some(Patch::clone(best));
        }
        let compared = record.compared.as_ref().filter(|c| c.submit == number)?;
        let ops = compared.ops.as_ref()?;
        let start = self.game_start_patch()?;
        let ops = enth_patch::wire::resolve(start, ops).ok()?;
        let mut patch = Patch::clone(start);
        enth_patch::apply_all(&mut patch, &ops).ok()?;
        Some(patch)
    }

    /// The live game round's record, when the open duel is on its code (or, with none open, whatever its code).
    fn duel_live_record(&self) -> Option<&RoundRecord> {
        let record = self.game_record()?;
        match &self.duel.run {
            Some(run) if run.code != record.code => None,
            _ => Some(record),
        }
    }

    /// The duel's own result without the patch: the kept one, else the live round's.
    fn duel_own_summary(&self) -> Option<DuelOwn> {
        if let Some(own) = self.duel.run.as_ref().and_then(|run| run.own.as_ref()) {
            return Some(own.own.clone());
        }
        let record = self.duel_live_record()?;
        self.duel_summary(record, false)
    }

    /// The player has submitted or given up on the duel's code: their answer opens.
    fn duel_played(&self) -> bool {
        self.duel_own_summary().is_some()
    }

    /// The state a duel intent works on, or [`DuelError::NoDuel`].
    fn duel_run(&self) -> Result<&DuelRun, DuelError> {
        self.duel.run.as_ref().ok_or(DuelError::NoDuel)
    }

    /// Open a duel on a canonical `code` if none is open; otherwise the open one must be on it.
    fn duel_for(&mut self, code: &str) -> Result<DuelId, DuelError> {
        let code = canonical(code)?;
        match &self.duel.run {
            Some(run) if run.code == code => Ok(run.id),
            Some(run) => Err(DuelError::OtherChallenge {
                expected: run.code.clone(),
                found: code,
            }),
            None => Ok(self.duel_begin(code)),
        }
    }

    fn duel_begin(&mut self, code: String) -> DuelId {
        self.duel.drop_run();
        self.duel.next_id += 1;
        let id = DuelId(self.duel.next_id);
        self.duel.run = Some(DuelRun {
            id,
            code,
            own: None,
            card: None,
            answer: None,
            verify: VerifyLive::Idle,
            check_number: None,
            checked: None,
            theirs_patch: None,
            used: false,
            auditioning: false,
        });
        id
    }
}

impl DuelEdit for Session {
    fn duel(&self) -> Option<DuelView> {
        let run = self.duel.run.as_ref()?;
        let own = self.duel_own_summary();
        let answer = match (&run.answer, own.is_some()) {
            (None, _) => AnswerState::None,
            (Some(_), false) => AnswerState::Sealed,
            (Some(_), true) => AnswerState::Open,
        };
        let sealed = answer == AnswerState::Sealed;
        let current = own.as_ref().and_then(|o| o.number);
        Some(DuelView {
            id: run.id,
            code: run.code.clone(),
            own,
            card: run.card.clone(),
            engine_differs: run
                .card
                .as_ref()
                .is_some_and(|c| c.engine != EngineVersion::CURRENT),
            answer,
            verify: match &run.verify {
                VerifyLive::Idle => VerifyState::Idle,
                VerifyLive::Running { .. } => VerifyState::Running,
                VerifyLive::Done => VerifyState::Done,
                VerifyLive::Failed(f) => VerifyState::Failed(f.clone()),
            },
            // The check of a sealed answer would show their score before the player has played: nothing is shown.
            checked: run.checked.clone().filter(|_| !sealed).map(|mut c| {
                c.own_current = run.check_number == current;
                c
            }),
            used: run.used,
            auditioning: run.auditioning,
        })
    }

    fn duel_own(&self) -> Option<DuelResult> {
        if let Some(own) = self.duel.run.as_ref().and_then(|run| run.own.as_ref()) {
            return Some(own.clone());
        }
        let record = self.duel_live_record()?;
        let mut own = self.duel_summary(record, false)?;
        let answer = self.duel_answer(record);
        own.answer_kept = answer.is_some();
        Some(DuelResult {
            own,
            answer: answer.map(Box::new),
        })
    }

    fn duel_open(&mut self, code: &str) -> Result<DuelId, DuelError> {
        let code = canonical(code)?;
        Ok(self.duel_begin(code))
    }

    fn duel_set_own(&mut self, own: DuelResult) -> Result<(), DuelError> {
        let run = self.duel_run()?;
        let code = canonical(&own.own.code)?;
        if code != run.code {
            return Err(DuelError::OtherChallenge {
                expected: run.code.clone(),
                found: code,
            });
        }
        if run.own.is_some() {
            return Err(DuelError::AlreadyPlayed);
        }
        let result = &own.own;
        if result.submits == 0 && !result.assists.revealed {
            return Err(DuelError::NotPlayed);
        }
        if result
            .score
            .is_some_and(|s| s.sound > 10_000 || s.combined > 10_000)
        {
            return Err(DuelError::BadImport("a score is above 100 %"));
        }
        let mut own = own;
        own.own.code = code;
        own.own.settled = true;
        own.own.answer_kept = own.answer.is_some();
        if let Some(run) = self.duel.run.as_mut() {
            run.own = Some(own);
            // Their answer was sealed; it opens now. A check made before ran without the player's result.
            run.forget_check(&mut self.duel.requests);
        }
        Ok(())
    }

    fn duel_import_scorecard(&mut self, card: FriendScorecard) -> Result<DuelId, DuelError> {
        card.check()?;
        let id = self.duel_for(&card.code)?;
        let canonical_code = canonical(&card.code)?;
        if let Some(run) = self.duel.run.as_mut() {
            run.forget_check(&mut self.duel.requests);
            run.card = Some(FriendScorecard {
                code: canonical_code,
                ..card
            });
        }
        Ok(id)
    }

    fn duel_import_answer(&mut self, answer: FriendAnswer) -> Result<DuelId, DuelError> {
        answer.check()?;
        let id = self.duel_for(&answer.code)?;
        let canonical_code = canonical(&answer.code)?;
        if let Some(run) = self.duel.run.as_mut() {
            run.forget_check(&mut self.duel.requests);
            run.answer = Some(FriendAnswer {
                code: canonical_code,
                ..answer
            });
        }
        Ok(id)
    }

    fn duel_verify(&mut self) -> Result<DuelJobId, DuelError> {
        let run = self.duel_run()?;
        let answer = run.answer.as_ref().ok_or(DuelError::NoAnswer)?;
        if !self.duel_played() {
            return Err(DuelError::Sealed);
        }
        if matches!(run.verify, VerifyLive::Running { .. }) {
            return Err(DuelError::Busy);
        }
        let (code, text, card) = (
            run.code.clone(),
            answer.text.clone(),
            run.card.as_ref().map(|c| c.text.clone()),
        );
        let duel = run.id;
        let own = self.duel_own().and_then(|r| r.answer);
        let number = self.duel_own_summary().and_then(|o| o.number);
        self.duel.next_job += 1;
        let job = DuelJobId(self.duel.next_job);
        let had_own = own.is_some();
        self.duel.requests.push(DuelRequest::Verify(DuelVerify {
            job,
            duel,
            code,
            own,
            card,
            answer: text,
        }));
        if let Some(run) = self.duel.run.as_mut() {
            run.verify = VerifyLive::Running { job, had_own };
            run.check_number = number;
            run.checked = None;
            run.theirs_patch = None;
            run.used = false;
        }
        Ok(job)
    }

    fn duel_use_answer(&mut self, name: &str) -> Result<(), DuelError> {
        let run = self.duel_run()?;
        if run.answer.is_some() && !self.duel_played() {
            return Err(DuelError::Sealed);
        }
        let mut patch = run
            .theirs_patch
            .as_deref()
            .cloned()
            .ok_or(DuelError::NotChecked)?;
        if self.game_record().is_some() {
            return Err(DuelError::RoundLive);
        }
        if !name.trim().is_empty() {
            name.trim().clone_into(&mut patch.meta.name);
        }
        self.load_preset(&patch, None).map_err(DuelError::Session)?;
        if let Some(run) = self.duel.run.as_mut() {
            run.used = true;
        }
        Ok(())
    }

    fn duel_audition(&mut self) -> Result<(), DuelError> {
        let run = self.duel_run()?;
        if run.answer.is_some() && !self.duel_played() {
            return Err(DuelError::Sealed);
        }
        let patch = run.theirs_patch.clone().ok_or(DuelError::NotChecked)?;
        let (duel, code) = (run.id, run.code.clone());
        self.duel
            .requests
            .push(DuelRequest::Audition { duel, code, patch });
        if let Some(run) = self.duel.run.as_mut() {
            run.auditioning = true;
        }
        Ok(())
    }

    fn duel_stop_audition(&mut self) {
        if let Some(run) = self.duel.run.as_mut()
            && run.auditioning
        {
            run.auditioning = false;
            self.duel.requests.push(DuelRequest::StopAudition);
        }
    }

    fn duel_close(&mut self) {
        self.duel.drop_run();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn the_state_and_its_messages_move_between_threads() {
        assert_send::<DuelRequest>();
        assert_send::<DuelJobResult>();
        assert_send::<DuelView>();
        assert_send::<DuelResult>();
    }

    #[test]
    fn a_revealed_result_gives_no_scorecard() {
        let score = ScoreSummary::from_percent(true, 93.4, 80.0, 90.0);
        let own = DuelOwn::new("c", Outcome::Passed, 3, 10_000).with_score(2, score);
        assert!(own.can_share());
        assert!(!own.can_share_answer(), "no patch is kept");
        let revealed = own.clone().with_assists(AssistsUsed {
            revealed: true,
            ..AssistsUsed::default()
        });
        assert!(!revealed.can_share());
        let mut unsettled = own;
        unsettled.settled = false;
        assert!(!unsettled.can_share());
    }

    #[test]
    fn imports_out_of_range_are_refused() {
        let ok = FriendScorecard::new("c", "t", EngineVersion::CURRENT, 934, 900, true);
        assert!(ok.check().is_ok());
        let high = FriendScorecard::new("c", "t", EngineVersion::CURRENT, 1001, 900, true);
        assert_eq!(high.check().map_err(|e| e.code()), Err("bad_import"));
        assert_eq!(
            FriendAnswer::new("c", "").check().map_err(|e| e.code()),
            Err("bad_import")
        );
        let long = "A".repeat(MAX_CODE_TEXT + 1);
        assert!(FriendAnswer::new("c", long).check().is_err());
    }

    #[test]
    fn a_verify_result_must_fit_its_job() {
        let theirs = Rescored::new(9342, 9000, true);
        let with_own = DuelVerified::new(Some(Rescored::new(100, 100, false)), theirs);
        assert!(with_own.check(true).is_ok());
        assert!(
            with_own.check(false).is_err(),
            "an own score for no own patch"
        );
        assert!(
            DuelVerified::new(None, theirs).check(true).is_err(),
            "the own patch was not re-scored"
        );
        let high = DuelVerified::new(None, Rescored::new(10_001, 0, true));
        assert!(high.check(false).is_err());
    }
}
