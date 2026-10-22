//! **Owner: lane RC** (Result codes, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/duels.md, "Verification".
//!
//! **The receiver's own re-score decides.** [`verify`] takes an answer code (already decoded, which rebuilt the answer
//! and checked its tail: [`AnswerCode::decode`]) and, when at hand, the scorecard it was sent after, then:
//!
//! 1. regenerates the challenge (a long challenge's fingerprint prefix checked: `target_mismatch`);
//! 2. checks the scorecard against the answer: the same challenge (`not_the_same_challenge`), and whether the answer is
//!    the one its commitment fixed ([`ClaimCheck::commitment_matches`]);
//! 3. scores the canonical answer with the scorecard's scorer set when this build has it, otherwise (a newer build's
//!    set, or no scorecard) with this build's current set ([`scorer_set`]; the F1e review's M4);
//! 4. compares the re-score with the claim ([`ClaimCheck`]): another engine version or another scorer set is flagged,
//!    never refused, so the UI shows both numbers ("claimed 93.4 on 0.3.0, 92.9 here").
//!
//! Only a generator or hash mismatch refuses an answer (`target_mismatch`, `answer_mismatch`, both at decode): the
//! round is never judged against another target or a damaged answer. A commitment that does not match is reported,
//! not refused: the answer still scores what it scores, but it is not the one the scorecard fixed.
//!
//! Preparing a round renders about a dozen times and scoring once more: under about a second. A duel scores both
//! answers on one prepared round ([`prepare`], then [`verify_prepared`] twice), so the two numbers come from one
//! machine and one scorer set. Core crates spawn no threads: the shell runs this on its game worker.
//!
//! **An ear result needs no render** ([`verify_ear`], the F1e review's M5): its answers are replayed against the keys
//! the ear generator gives for its spec, through the ear scorers ([`judge_set`]), with the result's ear rules when this
//! build has them, otherwise this build's (flagged). The keys are integers, so the replay is the same on every
//! platform and engine version.

use core::fmt;

use crate::answer::AnswerCode;
use crate::code::{CodeWarning, EngineVersion};
use crate::ear::question;
use crate::ear::scorers::{EarScore, EarScoreError, judge_set};
use crate::model::{Challenge, ChallengeError, Target};
use crate::result::{EarResult, EarRules, ResultError, Scorecard, ScorerSet, TENTHS_MAX};
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
    /// The scorer set the claim was scored with (perhaps a newer build's: [`ScorerSet::is_known`]).
    pub claimed_scorers: ScorerSet,
    /// The scorer set this machine re-scored with: the claim's when this build has it, else this build's current one.
    pub scorers: ScorerSet,
}

impl ClaimCheck {
    /// Whether this machine gives the claimed numbers and pass with the claim's own scorers: "as reported". False
    /// whenever the scorer sets differ, since then the numbers are two measures (the UI shows both).
    pub fn as_reported(&self) -> bool {
        self.sound_tenths == self.claimed_sound_tenths
            && self.combined_tenths == self.claimed_combined_tenths
            && self.passed == self.claimed_passed
            && !self.scorers_differ()
    }

    /// Whether the claim was scored with another scorer set than this machine's re-score (a newer build's set this
    /// build lacks): the UI shows both numbers, as for [`engine_differs`](Self::engine_differs) (the F1e review's M4).
    pub fn scorers_differ(&self) -> bool {
        self.claimed_scorers != self.scorers
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
    /// An ear result's challenge cannot be replayed by this build (its own code: `code_too_new` for a newer game,
    /// generator or level; `bad_answer` for a game whose answer is a patch).
    Ear(ChallengeError),
    /// An ear result's answers could not be judged against the keys (`bad_answer`: an answer of another kind than its
    /// question's, or out of its range).
    EarScore(EarScoreError),
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
            VerifyError::Ear(e) => e.code(),
            VerifyError::EarScore(e) => e.code(),
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
            VerifyError::Ear(e) => e.hint(),
            VerifyError::EarScore(e) => e.hint(),
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
            VerifyError::Ear(e) => write!(f, "{e}"),
            VerifyError::EarScore(e) => write!(f, "{e}"),
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

/// The scorer set a verification uses (the F1e review's M4): the scorecard's when this build has it, otherwise this
/// build's current set (a newer build's set, or no scorecard). Both sides of a duel across an update then score with
/// the older set, since a build keeps every set it ever had.
pub fn scorer_set(scorecard: Option<&Scorecard>) -> ScorerSet {
    scorecard
        .map(|c| c.fields().scorers)
        .filter(|s| s.is_known())
        .unwrap_or_else(ScorerSet::current)
}

/// The challenge an answer code was played on, scored with `scorers` (this build's current set when it lacks them):
/// the Recreate challenge of its spec and phrase.
pub fn challenge_of(answer: &AnswerCode, scorers: ScorerSet) -> Challenge {
    let spec = *answer.challenge().spec();
    let mut challenge = Challenge::recreate(spec, answer.phrase());
    challenge.scoring = scoring_of(scorers, spec.difficulty);
    challenge
}

/// The scoring of `scorers` at `difficulty`, or this build's current set's when this build lacks them.
fn scoring_of(scorers: ScorerSet, difficulty: u8) -> crate::model::ScoringSpec {
    scorers
        .scoring(difficulty)
        .or_else(|| ScorerSet::current().scoring(difficulty))
        .unwrap_or_else(|| crate::model::ScoringSpec::recreate(difficulty))
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
        && round.challenge().scoring == scoring_of(scorers, spec.difficulty);
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
            claimed_scorers: f.scorers,
            scorers,
        }
    });
    Ok(Verified {
        report,
        scorers,
        claim,
        warnings,
    })
}

/// What replaying an ear result found ([`verify_ear`]). The [`score`](EarVerified::score) is this machine's judgement
/// of the answers, and it decides.
#[derive(Clone, Debug, PartialEq)]
pub struct EarVerified {
    /// This build's judgement of the answers against the keys: per question, the right count, the percentage and the
    /// pass, "checked on your computer".
    pub score: EarScore,
    /// The ear rules it was judged by: the result's when this build has them, else this build's current ones.
    pub rules: EarRules,
    /// The ear rules the result names.
    pub claimed_rules: EarRules,
}

impl EarVerified {
    /// Whether the result named rules this build lacks, so it was judged by this build's (the UI says so).
    pub fn rules_differ(&self) -> bool {
        self.rules != self.claimed_rules
    }
}

/// **Replay an ear result against its keys, with no render** (the F1e review's M5): the ear challenge of its spec
/// ([`Challenge::ear`]: `code_too_new` for a game, generator or level this build lacks), judged with the result's ear
/// rules when this build has them, otherwise with this build's ([`EarVerified::rules_differ`]); the questions' keys
/// from the frozen generators ([`question::prepare`]); every answer judged against its key ([`judge_set`]: a missing
/// answer is wrong; taps are judged after the best-fit offset, so their stored offset from the first tap changes
/// nothing). The keys are integers: the same score on every platform and engine version.
pub fn verify_ear(result: &EarResult) -> Result<EarVerified, VerifyError> {
    let claimed_rules = result.rules();
    let rules = if claimed_rules.is_known() {
        claimed_rules
    } else {
        EarRules::current()
    };
    let mut challenge = Challenge::ear(*result.challenge()).map_err(VerifyError::Ear)?;
    if let Some(scoring) = rules.scoring(challenge.answer) {
        challenge.scoring = scoring;
    }
    let set = question::prepare(&challenge).map_err(VerifyError::Ear)?;
    let score =
        judge_set(set.scoring(), &set.keys(), result.answers()).map_err(VerifyError::EarScore)?;
    Ok(EarVerified {
        score,
        rules,
        claimed_rules,
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
    use crate::result::{EarResult, EarRules, Scorecard};
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

    /// The F1e review's M4: a scorecard scored with a set this build lacks (a newer build's `sound/2`, say) is shown
    /// as reported and checked with this build's set, flagged, never refused.
    #[test]
    fn a_newer_scorer_set_is_checked_with_this_builds_and_flagged() {
        let challenge = code(1, 11);
        let round = prepare_round(challenge.spec()).unwrap();
        let answer = round.target().clone();
        let report = score_round(&round, &answer).unwrap();
        let code = AnswerCode::new(challenge, 99, &answer).unwrap();
        let own = Scorecard::new(
            challenge,
            &report,
            &round.challenge().scoring,
            1,
            5_000,
            &AssistsUsed::default(),
            code.commitment(),
        )
        .unwrap();
        let mut fields = own.fields().clone();
        fields.scorers = ScorerSet::from_index(2).unwrap();
        let newer = Scorecard::decode(&Scorecard::from_fields(fields).unwrap().encode()).unwrap();
        assert!(!newer.fields().scorers.is_known());
        assert_eq!(scorer_set(Some(&newer)), ScorerSet::current());
        let v = verify(&code, Some(&newer)).unwrap();
        assert_eq!(v.scorers, ScorerSet::current());
        let claim = v.claim.unwrap();
        assert!(claim.scorers_differ());
        assert_eq!(claim.claimed_scorers.index(), 2);
        assert!(!claim.as_reported(), "two measures are never 'as reported'");
        assert!(claim.commitment_matches);
        // The same numbers under the known set read as reported.
        let mine = verify(&code, Some(&own)).unwrap().claim.unwrap();
        assert!(!mine.scorers_differ() && mine.as_reported(), "{mine:?}");
        assert_eq!(mine.sound_tenths, claim.sound_tenths);
    }

    fn spot(seed: u64, count: u8) -> crate::model::EarSpec {
        crate::model::EarSpec {
            game: crate::ear::params::SPOT_THE_CHANGE,
            generator: 1,
            level: 1,
            seed,
            count,
        }
    }

    fn keys_of(spec: crate::model::EarSpec) -> Vec<crate::ear::scorers::EarKey> {
        question::prepare(&Challenge::ear(spec).unwrap())
            .unwrap()
            .keys()
    }

    /// The F1e review's M5: an ear result replays against the generator's keys with no render, through the code.
    #[test]
    fn an_ear_result_replays_against_its_keys() {
        use crate::ear::scorers::EarAnswer;
        let spec = spot(4242, 10);
        let mut answers: Vec<Option<EarAnswer>> = keys_of(spec)
            .iter()
            .map(|k| Some(k.right_answer()))
            .collect();
        // One wrong (another option) and one unanswered: 8 of 10, which passes at 80.
        let Some(EarAnswer::Choice(right)) = answers[3] else {
            panic!("a choice");
        };
        answers[3] = Some(EarAnswer::Choice(u8::from(right == 0)));
        answers[7] = None;
        let result = EarResult::new(spec, EarRules::current(), answers, 64_000, 0, false).unwrap();
        let result = EarResult::decode(&result.encode()).unwrap();
        let v = verify_ear(&result).unwrap();
        assert_eq!(
            (v.score.right, v.score.percent, v.score.passed),
            (8, 80.0, true)
        );
        assert_eq!(v.rules, EarRules::V1);
        assert!(!v.rules_differ());
        // Rules this build lacks: judged by this build's, flagged.
        let newer = EarResult::new(
            spec,
            EarRules::from_index(7).unwrap(),
            result.answers().to_vec(),
            64_000,
            0,
            false,
        )
        .unwrap();
        let v = verify_ear(&EarResult::decode(&newer.encode()).unwrap()).unwrap();
        assert!(v.rules_differ());
        assert_eq!((v.rules, v.claimed_rules.index()), (EarRules::V1, 7));
        assert_eq!(v.score.right, 8);
        // A game this build cannot replay is a newer build's.
        let other = EarResult::new(
            crate::model::EarSpec { game: 99, ..spec },
            EarRules::V1,
            vec![None; 10],
            0,
            0,
            false,
        )
        .unwrap();
        assert_eq!(verify_ear(&other).unwrap_err().code(), "code_too_new");
        // An answer of another kind than its question's is refused, not judged.
        let mut wrong_kind: Vec<Option<EarAnswer>> = vec![None; 10];
        wrong_kind[0] = Some(EarAnswer::Value(1.0));
        let wrong = EarResult::new(spec, EarRules::V1, wrong_kind, 0, 0, false).unwrap();
        assert_eq!(verify_ear(&wrong).unwrap_err().code(), "bad_answer");
    }

    /// Taps on any clock: the result stores them from the first tap, and the replay (with the best-fit offset) scores
    /// them as tapped (Tap back's level 1, `enth-music`'s keys).
    #[test]
    fn tap_answers_on_any_clock_replay_as_tapped() {
        use crate::ear::scorers::{EarAnswer, EarKey};
        let spec = crate::model::EarSpec::new(enth_music::ear::Game::TapBack, 1, 31, 6);
        let keys = keys_of(spec);
        let answers: Vec<Option<EarAnswer>> = keys
            .iter()
            .map(|k| {
                let EarKey::Taps(t) = k else {
                    panic!("a taps key");
                };
                // The shell's clock: some time since the plugin loaded.
                Some(EarAnswer::Taps(
                    t.onsets_ms.iter().map(|ms| 1_234_567 + ms).collect(),
                ))
            })
            .collect();
        let result = EarResult::new(spec, EarRules::V1, answers, 30_000, 0, false).unwrap();
        for a in result.answers().iter().flatten() {
            let EarAnswer::Taps(t) = a else {
                panic!("taps");
            };
            assert_eq!(t.first(), Some(&0), "stored from the first tap");
        }
        let v = verify_ear(&EarResult::decode(&result.encode()).unwrap()).unwrap();
        assert_eq!((v.score.right, v.score.percent), (6, 100.0));
    }
}
