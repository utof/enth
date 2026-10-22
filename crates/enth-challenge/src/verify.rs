//! **Owner: lane RC** (Result codes, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/duels.md, "Verification".
//!
//! **The receiver's own re-score decides.** [`verify`] takes an answer code (already decoded, which rebuilt the answer
//! and checked its tail: [`AnswerCode::decode`]) and, when at hand, the scorecard it was sent after, then:
//!
//! 1. regenerates the challenge (a long challenge's fingerprint prefix checked: `target_mismatch`);
//! 2. checks the scorecard against the answer: the same challenge (`not_the_same_challenge`), and whether the answer is
//!    the one its commitment fixed ([`ClaimCheck::commitment_matches`]);
//! 3. scores the canonical answer with the scorecard's scorer set (an unknown one was refused when the scorecard was
//!    read: `scorer_too_new`), or this build's set without a scorecard;
//! 4. compares the re-score with the claim ([`ClaimCheck`]): another engine version is flagged, never refused, so the UI
//!    shows both numbers ("claimed 93.4 on 0.3.0, 92.9 here").
//!
//! Only a generator or hash mismatch refuses an answer (`target_mismatch`, `answer_mismatch`, both at decode): the
//! round is never judged against another target or a damaged answer. A commitment that does not match is reported,
//! not refused: the answer still scores what it scores, but it is not the one the scorecard fixed.
//!
//! Preparing a round renders about a dozen times and scoring once more: under about a second. A duel scores both
//! answers on one prepared round ([`prepare`], then [`verify_prepared`] twice), so the two numbers come from one
//! machine and one scorer set. Core crates spawn no threads: the shell runs this on its game worker.
//!
//! An ear result needs no render: its answers are replayed against the ear generator's keys. That waits for lanes MUS
//! and CV3's types (an open item for F1E); [`crate::result::EarResult`] already carries what the replay needs.

use core::fmt;

use crate::answer::AnswerCode;
use crate::code::{CodeWarning, EngineVersion};
use crate::model::{Challenge, Target};
use crate::result::{ResultError, Scorecard, ScorerSet, TENTHS_MAX};
use crate::score::round::{PreparedRound, RoundError, prepare_challenge, score_round};
use crate::score::{ScoreError, ScoreReport};

/// What a verification found. The [`report`](Verified::report) is this machine's score of the answer, and it decides.
#[derive(Clone, Debug, PartialEq)]
pub struct Verified {
    /// This build's score of the canonical answer: "checked on your computer".
    pub report: ScoreReport,
    /// The scorer set it was scored with.
    pub scorers: ScorerSet,
    /// The claim compared, when a scorecard was given.
    pub claim: Option<ClaimCheck>,
    /// Differences that do not stop a verification (a long challenge made on another engine).
    pub warnings: Vec<CodeWarning>,
}

/// A scorecard's claim next to this machine's re-score. Percentages are in tenths, as a scorecard carries them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClaimCheck {
    /// The claimed sound %, in tenths.
    pub claimed_sound_tenths: u16,
    /// This machine's sound %, in tenths (rounded as the claim was).
    pub sound_tenths: u16,
    /// The claimed combined %, in tenths.
    pub claimed_combined_tenths: u16,
    /// This machine's combined %, in tenths.
    pub combined_tenths: u16,
    /// Whether the claim says it passed.
    pub claimed_passed: bool,
    /// Whether it passes here.
    pub passed: bool,
    /// The engine the claim was scored on.
    pub claimed_engine: EngineVersion,
    /// This build's engine.
    pub engine: EngineVersion,
    /// Whether the answer is the one the scorecard's salted commitment fixed: false means it was changed after the
    /// scorecard was sent (or belongs to another round of the same challenge).
    pub commitment_matches: bool,
}

impl ClaimCheck {
    /// Whether this machine gives the claimed numbers and pass: "as reported".
    pub fn as_reported(&self) -> bool {
        self.sound_tenths == self.claimed_sound_tenths
            && self.combined_tenths == self.claimed_combined_tenths
            && self.passed == self.claimed_passed
    }

    /// Whether the claim was scored on another engine version: a difference is then expected to be small and is
    /// shown with both numbers.
    pub fn engine_differs(&self) -> bool {
        self.claimed_engine != self.engine
    }
}

/// Why an answer could not be verified.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum VerifyError {
    /// The scorecard and the answer name different challenges (`not_the_same_challenge`).
    NotTheSameChallenge,
    /// The prepared round is another challenge's, or scores with another set than asked (`round_mismatch`).
    RoundMismatch,
    /// The answer or the scorecard is refused (their own codes).
    Code(ResultError),
    /// The round could not be prepared (its code: `silent_target` never, since a duel's round is scored as given).
    Round(RoundError),
    /// The answer could not be scored.
    Score(ScoreError),
}

impl VerifyError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            VerifyError::NotTheSameChallenge => "not_the_same_challenge",
            VerifyError::RoundMismatch => "round_mismatch",
            VerifyError::Code(e) => e.code(),
            VerifyError::Round(e) => e.code(),
            VerifyError::Score(e) => e.code(),
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            VerifyError::NotTheSameChallenge => {
                "this scorecard and this answer code are from different challenges".to_owned()
            }
            VerifyError::RoundMismatch => {
                "the prepared round is not this answer's challenge and scorer set".to_owned()
            }
            VerifyError::Code(e) => e.hint(),
            VerifyError::Round(e) => e.hint(),
            VerifyError::Score(e) => e.hint(),
        }
    }
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifyError::NotTheSameChallenge => {
                f.write_str("the scorecard and the answer name different challenges")
            }
            VerifyError::RoundMismatch => f.write_str("the prepared round is another one"),
            VerifyError::Code(e) => write!(f, "{e}"),
            VerifyError::Round(e) => write!(f, "{e}"),
            VerifyError::Score(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for VerifyError {}

impl From<ResultError> for VerifyError {
    fn from(e: ResultError) -> Self {
        VerifyError::Code(e)
    }
}

impl From<RoundError> for VerifyError {
    fn from(e: RoundError) -> Self {
        VerifyError::Round(e)
    }
}

impl From<ScoreError> for VerifyError {
    fn from(e: ScoreError) -> Self {
        VerifyError::Score(e)
    }
}

/// The scorer set a verification uses: the scorecard's, or this build's without one.
pub fn scorer_set(scorecard: Option<&Scorecard>) -> ScorerSet {
    scorecard.map_or_else(ScorerSet::current, |c| c.fields().scorers)
}

/// The challenge an answer code was played on, scored with `scorers`: the Recreate challenge of its spec and phrase.
pub fn challenge_of(answer: &AnswerCode, scorers: ScorerSet) -> Challenge {
    let spec = *answer.challenge().spec();
    let mut challenge = Challenge::recreate(spec, answer.phrase());
    challenge.scoring = scorers.scoring(spec.difficulty);
    challenge
}

/// Prepare the round an answer code was played on, scored with `scorers` (what [`verify_prepared`] takes). A duel
/// prepares once and verifies both answers on it.
pub fn prepare(answer: &AnswerCode, scorers: ScorerSet) -> Result<PreparedRound, VerifyError> {
    Ok(prepare_challenge(challenge_of(answer, scorers))?)
}

/// Verify `answer`, with the `scorecard` it was sent after when at hand: prepare its round with the scorecard's scorer
/// set (or this build's), score the canonical answer and compare with the claim. See the [module docs](self).
pub fn verify(answer: &AnswerCode, scorecard: Option<&Scorecard>) -> Result<Verified, VerifyError> {
    let round = prepare(answer, scorer_set(scorecard))?;
    verify_prepared(&round, answer, scorecard)
}

/// [`verify`] on a round prepared by [`prepare`] (or any round of the same challenge with the same scorers):
/// `round_mismatch` when it is another challenge, target or scorer set.
pub fn verify_prepared(
    round: &PreparedRound,
    answer: &AnswerCode,
    scorecard: Option<&Scorecard>,
) -> Result<Verified, VerifyError> {
    if let Some(card) = scorecard
        && card.challenge() != answer.challenge()
    {
        return Err(VerifyError::NotTheSameChallenge);
    }
    let scorers = scorer_set(scorecard);
    let spec = *answer.challenge().spec();
    let same_round = round.challenge().target == Target::Seed(spec)
        && round.fingerprint() == answer.target()
        && round.challenge().scoring == scorers.scoring(spec.difficulty);
    if !same_round {
        return Err(VerifyError::RoundMismatch);
    }
    let warnings = answer
        .challenge()
        .verify(&round.fingerprint(), EngineVersion::CURRENT)
        .map_err(ResultError::from)?;
    let report = score_round(round, answer.answer())?;
    let claim = scorecard.map(|card| {
        let f = card.fields();
        ClaimCheck {
            claimed_sound_tenths: f.sound_tenths,
            sound_tenths: tenths(report.sound.percent),
            claimed_combined_tenths: f.combined_tenths,
            combined_tenths: tenths(report.combined.percent),
            claimed_passed: f.passed,
            passed: report.passed,
            claimed_engine: f.engine,
            engine: report.engine.unwrap_or(EngineVersion::CURRENT),
            commitment_matches: f.commitment == answer.commitment(),
        }
    });
    Ok(Verified {
        report,
        scorers,
        claim,
        warnings,
    })
}

/// A percentage in tenths, rounded as a scorecard rounds its claim.
fn tenths(percent: f64) -> u16 {
    if percent.is_finite() {
        (percent * 10.0).round().clamp(0.0, f64::from(TENTHS_MAX)) as u16
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer::AnswerCode;
    use crate::code::ChallengeCode;
    use crate::model::AssistsUsed;
    use crate::result::Scorecard;
    use crate::score::round::prepare_round;
    use crate::spec::SpecPreset;

    fn code(level: u8, seed: u64) -> ChallengeCode {
        ChallengeCode::short(SpecPreset::by_id(level).unwrap().spec(1, seed)).unwrap()
    }

    /// The flow of docs/duels.md on one machine: play, score, make the codes, verify both ways.
    #[test]
    fn a_scorecard_and_its_answer_verify_as_reported() {
        let challenge = code(2, 42);
        let round = prepare_round(challenge.spec()).unwrap();
        // A passing answer: the target itself, with a name in its meta (stripped by the code).
        let mut answer = round.target().clone();
        answer.meta.author = "Somebody Real".to_owned();
        let report = score_round(&round, &answer).unwrap();
        let code = AnswerCode::new(challenge, 0x005E_ED0F_5A17, &answer).unwrap();
        let card = Scorecard::new(
            challenge,
            &report,
            &round.challenge().scoring,
            3,
            161_000,
            &AssistsUsed::default(),
            code.commitment(),
        )
        .unwrap();
        let card = Scorecard::decode(&card.encode()).unwrap();
        let code = AnswerCode::decode(&code.encode()).unwrap();
        let v = verify(&code, Some(&card)).unwrap();
        let claim = v.claim.unwrap();
        assert!(claim.as_reported(), "{claim:?}");
        assert!(claim.commitment_matches);
        assert!(!claim.engine_differs());
        assert_eq!(
            v.report.sound.percent.to_bits(),
            report.sound.percent.to_bits()
        );
        assert_eq!(v.scorers, ScorerSet::V1);
        // The same round prepared once verifies it too, and without the scorecard.
        let prepared = prepare(&code, ScorerSet::current()).unwrap();
        let alone = verify_prepared(&prepared, &code, None).unwrap();
        assert_eq!(alone.report, v.report);
        assert_eq!(alone.claim, None);
    }

    #[test]
    fn another_answer_than_the_committed_one_is_flagged_and_another_challenge_refused() {
        let challenge = code(1, 7);
        let round = prepare_round(challenge.spec()).unwrap();
        let answer = round.start().clone();
        let report = score_round(&round, &answer).unwrap();
        let committed = AnswerCode::new(challenge, 1, round.target()).unwrap();
        let sent = AnswerCode::new(challenge, 2, &answer).unwrap();
        let card = Scorecard::new(
            challenge,
            &report,
            &round.challenge().scoring,
            1,
            9_000,
            &AssistsUsed::default(),
            committed.commitment(),
        )
        .unwrap();
        let v = verify(&sent, Some(&card)).unwrap();
        assert!(!v.claim.unwrap().commitment_matches);
        let other = AnswerCode::new(code(1, 8), 3, &answer).unwrap();
        assert_eq!(
            verify(&other, Some(&card)).unwrap_err(),
            VerifyError::NotTheSameChallenge
        );
        let prepared = prepare(&sent, ScorerSet::V1).unwrap();
        assert_eq!(
            verify_prepared(&prepared, &other, None).unwrap_err().code(),
            "round_mismatch"
        );
    }
}
