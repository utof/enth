//! **Owner: lane CT** (F1r review S5). The round's composition as one pure function: from a spec (or a challenge) to
//! a prepared round, and from a prepared round and the player's patch to a [`ScoreReport`]. The reference is
//! docs/challenge.md, "Scoring a round".
//!
//! Shells never compose scorers. The plugin's game worker, the CLI's `enth compare` and the golden challenges all
//! call [`prepare_round`] (or [`prepare_challenge`]) once per round and [`score_round`] per submit, so the product
//! rules (which scorer a challenge names, how the scores combine, what decides pass) live here, once.
//!
//! **The dispatch.** A challenge's [`ScoringSpec`] names its scorers by [`ScorerId`]. [`prepare_challenge`] maps each
//! id to its implementation: the stubs ([`super::stub`]) first, then each lane's own hook, in its own file, so a lane
//! adds its scorer without editing this one:
//!
//! | Kind | Hook | Lane |
//! | --- | --- | --- |
//! | sound | `score::sound::prepare(id, ctx)` | SS |
//! | structure | `score::structure::prepare(id, ctx)` | ST |
//! | combined | `score::combined::combiner(id)` | ST |
//! | time | `time/1` only: the session keeps the clock | — |
//!
//! An id that no hook knows is refused (`unknown_scorer`): a challenge is never scored by a scorer it does not
//! name.

use core::fmt;

use enth_patch::{ParamAddr, Patch};

use super::{
    Combiner, ScoreError, ScoreReport, Scorer, ScorerId, SoundScore, StructureScore, TargetContext,
    combined, sound, structure, stub,
};
use crate::fingerprint::TargetFingerprint;
use crate::model::{Challenge, ChallengeError, Phrase, PhraseId, ScoringSpec};
use crate::random::{self, GenError, Generated};
use crate::spec::RandomSpec;

/// A scorer prepared for one challenge, with its kind's score type and nothing else visible: what the dispatch
/// keeps. [`Ready`] makes one from any [`Scorer`].
pub trait PreparedScorer<S>: Send + Sync {
    /// The scorer's id.
    fn id(&self) -> ScorerId;

    /// Score the player's patch.
    fn score(&self, answer: &Patch) -> Result<S, ScoreError>;
}

/// A scorer and what its `prepare` computed for one challenge.
pub struct Ready<T: Scorer> {
    scorer: T,
    prepared: T::Prepared,
}

impl<T> Ready<T>
where
    T: Scorer + 'static,
    T::Prepared: 'static,
{
    /// Prepare `scorer` on `ctx` and keep both, boxed for the dispatch.
    pub fn prepare(
        scorer: T,
        ctx: &TargetContext<'_>,
    ) -> Result<Box<dyn PreparedScorer<T::Score>>, ScoreError> {
        let prepared = scorer.prepare(ctx)?;
        Ok(Box::new(Ready { scorer, prepared }))
    }
}

impl<T: Scorer> PreparedScorer<T::Score> for Ready<T> {
    fn id(&self) -> ScorerId {
        self.scorer.id()
    }

    fn score(&self, answer: &Patch) -> Result<T::Score, ScoreError> {
        self.scorer.score(&self.prepared, answer)
    }
}

/// A round ready to score: the challenge, the start and the generated target (with its phrase, randomized
/// parameters and fingerprint), and every scorer the challenge names, prepared. Built only by [`prepare_round`] and
/// [`prepare_challenge`], so its parts always belong together. `Send + Sync`: a worker keeps it, or shares it in an
/// `Arc`.
pub struct PreparedRound {
    challenge: Challenge,
    start: Patch,
    generated: Generated,
    sound: Box<dyn PreparedScorer<SoundScore>>,
    structure: Box<dyn PreparedScorer<StructureScore>>,
    combiner: Box<dyn Combiner>,
}

impl PreparedRound {
    /// The challenge (with the generator's phrase and the scorers it names).
    pub fn challenge(&self) -> &Challenge {
        &self.challenge
    }

    /// The patch the player starts from (the spec's template).
    pub fn start(&self) -> &Patch {
        &self.start
    }

    /// The target patch (never persisted).
    pub fn target(&self) -> &Patch {
        &self.generated.patch
    }

    /// Everything the generator returned.
    pub fn generated(&self) -> &Generated {
        &self.generated
    }

    /// The randomized parameters, in address order: the rings, `dim_unused`, `TargetContext::randomized`.
    pub fn params(&self) -> &[ParamAddr] {
        &self.generated.params
    }

    /// The target's fingerprint: what a long code's prefix and a saved round check.
    pub fn fingerprint(&self) -> TargetFingerprint {
        self.generated.fingerprint
    }

    /// The phrase the target and the answer are heard and scored through.
    pub fn phrase(&self) -> &'static Phrase {
        self.generated.phrase()
    }
}

impl fmt::Debug for PreparedRound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedRound")
            .field("challenge", &self.challenge)
            .field("fingerprint", &self.generated.fingerprint)
            .field("sound", &self.sound.id())
            .field("structure", &self.structure.id())
            .field("combined", &self.combiner.id())
            .finish_non_exhaustive()
    }
}

/// Why a round could not be prepared.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RoundError {
    /// The generator refused the spec or its template (`out_of_range`, `code_too_new`, …).
    Generate(GenError),
    /// The challenge fails its own checks.
    Challenge(ChallengeError),
    /// The challenge names another phrase than the generator picks for its spec: it is not this target's challenge.
    PhraseMismatch {
        /// The challenge's phrase.
        challenge: PhraseId,
        /// The generator's.
        generated: PhraseId,
    },
    /// A scorer could not prepare, or the challenge names one this build does not have (`unknown_scorer`).
    Score(ScoreError),
}

impl RoundError {
    /// A stable snake-case code: the inner error's, or `phrase_mismatch`.
    pub fn code(&self) -> &'static str {
        match self {
            RoundError::Generate(e) => e.code(),
            RoundError::Challenge(e) => e.code(),
            RoundError::PhraseMismatch { .. } => "phrase_mismatch",
            RoundError::Score(e) => e.code(),
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            RoundError::Generate(e) => e.hint(),
            RoundError::Challenge(e) => e.hint(),
            RoundError::PhraseMismatch { .. } => {
                "a generated challenge plays the phrase its generator picks".to_owned()
            }
            RoundError::Score(e) => e.hint(),
        }
    }
}

impl fmt::Display for RoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RoundError::Generate(e) => write!(f, "{e}"),
            RoundError::Challenge(e) => write!(f, "{e}"),
            RoundError::PhraseMismatch {
                challenge,
                generated,
            } => write!(
                f,
                "the challenge plays phrase {}, the generator picks {}",
                challenge.0, generated.0
            ),
            RoundError::Score(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RoundError {}

impl From<GenError> for RoundError {
    fn from(e: GenError) -> Self {
        RoundError::Generate(e)
    }
}

impl From<ScoreError> for RoundError {
    fn from(e: ScoreError) -> Self {
        RoundError::Score(e)
    }
}

/// Prepare the Recreate round of `spec`: regenerate the target (lane RG), build
/// [`Challenge::recreate`]`(spec, phrase)` with this build's default scorers, and prepare each. What the game worker
/// runs for a `Prepare` job.
pub fn prepare_round(spec: &RandomSpec) -> Result<PreparedRound, RoundError> {
    // The spec's own check first, so a newer template is `code_too_new` rather than an unknown start.
    spec.validate().map_err(GenError::Spec)?;
    let start = random::start(spec.template)?;
    let generated = random::generate(spec, &start)?;
    let challenge = Challenge::recreate(*spec, generated.phrase);
    prepare_parts(challenge, start, generated)
}

/// Prepare a given challenge (a golden challenge, `enth compare`, a challenge file): validate it, regenerate its
/// target, check that its phrase is the generator's pick, and prepare the scorers it names.
pub fn prepare_challenge(challenge: Challenge) -> Result<PreparedRound, RoundError> {
    challenge.validate().map_err(RoundError::Challenge)?;
    let spec = *challenge.spec().ok_or_else(|| {
        RoundError::Score(ScoreError::Unscorable {
            reason: "only generated challenges are prepared in R1".to_owned(),
        })
    })?;
    let start = random::start(spec.template)?;
    let generated = random::generate(&spec, &start)?;
    if generated.phrase != challenge.phrase {
        return Err(RoundError::PhraseMismatch {
            challenge: challenge.phrase,
            generated: generated.phrase,
        });
    }
    prepare_parts(challenge, start, generated)
}

fn prepare_parts(
    challenge: Challenge,
    start: Patch,
    generated: Generated,
) -> Result<PreparedRound, RoundError> {
    let scoring = &challenge.scoring;
    if scoring.time != ScorerId::TIME_V1 {
        return Err(unknown(&scoring.time).into());
    }
    let combiner = combiner_for(&scoring.combined)?;
    let ctx = TargetContext {
        challenge: &challenge,
        target: &generated.patch,
        start: &start,
        phrase: generated.phrase(),
        randomized: &generated.params,
    };
    let sound = prepare_sound(&scoring.sound, &ctx)?;
    let structure = prepare_structure(&scoring.structure, &ctx)?;
    Ok(PreparedRound {
        challenge,
        start,
        generated,
        sound,
        structure,
        combiner,
    })
}

fn unknown(id: &ScorerId) -> ScoreError {
    ScoreError::UnknownScorer { id: id.to_string() }
}

/// The sound scorer a challenge names, prepared: the stub, then lane SS's hook.
fn prepare_sound(
    id: &ScorerId,
    ctx: &TargetContext<'_>,
) -> Result<Box<dyn PreparedScorer<SoundScore>>, ScoreError> {
    if *id == ScorerId::SOUND_STUB {
        return Ready::prepare(stub::SoundStub, ctx);
    }
    sound::prepare(id, ctx).unwrap_or_else(|| Err(unknown(id)))
}

/// The structure scorer a challenge names, prepared: the stub, then lane ST's hook.
fn prepare_structure(
    id: &ScorerId,
    ctx: &TargetContext<'_>,
) -> Result<Box<dyn PreparedScorer<StructureScore>>, ScoreError> {
    if *id == ScorerId::STRUCTURE_STUB {
        return Ready::prepare(stub::StructureStub, ctx);
    }
    structure::prepare(id, ctx).unwrap_or_else(|| Err(unknown(id)))
}

/// The combiner a challenge names: the stub, then lane ST's hook.
fn combiner_for(id: &ScorerId) -> Result<Box<dyn Combiner>, ScoreError> {
    if *id == ScorerId::COMBINED_STUB {
        return Ok(Box::new(stub::CombinedStub));
    }
    combined::combiner(id).ok_or_else(|| unknown(id))
}

/// Score the player's patch against a prepared round: sound, structure, combined, and the pass from sound alone
/// ([`ScoreReport::new`]). Pure and deterministic. A scorer that answers under another id than the challenge names
/// is a bug, refused as `unscorable`.
pub fn score_round(round: &PreparedRound, answer: &Patch) -> Result<ScoreReport, ScoreError> {
    let scoring: &ScoringSpec = &round.challenge.scoring;
    let sound = round.sound.score(answer)?;
    let structure = score_structure(round, answer)?;
    let combined = round.combiner.combine(scoring, &sound, &structure);
    let report = ScoreReport::new(scoring, sound, structure, combined);
    report
        .check_against(scoring)
        .map_err(|reason| ScoreError::Unscorable { reason })?;
    Ok(report)
}

/// The structure score alone (the live rings after the first submit: no render).
pub fn score_structure(
    round: &PreparedRound,
    answer: &Patch,
) -> Result<StructureScore, ScoreError> {
    round.structure.score(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RECREATE_SCORERS;
    use crate::spec::SpecPreset;

    fn spec(level: u8, seed: u64) -> RandomSpec {
        SpecPreset::by_id(level).unwrap().spec(1, seed)
    }

    #[test]
    fn a_round_prepares_and_the_target_scores_100() {
        for level in 1..=5 {
            let round = prepare_round(&spec(level, 42)).unwrap();
            let scoring = &round.challenge().scoring;
            assert_eq!(
                [
                    &scoring.sound,
                    &scoring.structure,
                    &scoring.combined,
                    &scoring.time
                ],
                RECREATE_SCORERS.each_ref()
            );
            assert_eq!(round.fingerprint(), round.generated().fingerprint);
            assert_eq!(round.challenge().phrase, round.generated().phrase);
            let report = score_round(&round, round.target()).unwrap();
            assert_eq!(report.sound.percent, 100.0, "level {level}");
            assert_eq!(report.structure.percent, 100.0);
            assert!(report.passed);
            assert_eq!(report.engine, Some(crate::code::EngineVersion::CURRENT));
            assert!(
                report.structure.params.iter().all(|p| p.randomized),
                "nothing moved that was not randomized"
            );
        }
    }

    #[test]
    fn the_start_scores_below_the_target_and_gets_rings() {
        let round = prepare_round(&spec(3, 7)).unwrap();
        let report = score_round(&round, round.start()).unwrap();
        assert!(report.sound.percent < 100.0, "{}", report.sound.percent);
        assert_ne!(report.structure.params.len(), 0);
        let rings = score_structure(&round, round.start()).unwrap();
        assert_eq!(rings, report.structure);
        assert!(
            rings
                .params
                .iter()
                .any(|p| p.verdict != super::super::Verdict::Good)
        );
    }

    #[test]
    fn unknown_scorers_are_refused_and_named_ones_are_used() {
        let round = prepare_round(&spec(2, 1)).unwrap();
        let mut c = round.challenge().clone();
        c.scoring.sound = ScorerId::new("sound", 9);
        let e = prepare_challenge(c.clone()).unwrap_err();
        assert_eq!(e.code(), "unknown_scorer");
        assert!(e.hint().contains("sound/9"), "{}", e.hint());
        // `sound/2` (GR1's refit) is reserved, not landed: unknown here too, never silently scored by `sound/1`.
        c.scoring.sound = ScorerId::SOUND_V2;
        assert_eq!(
            prepare_challenge(c.clone()).unwrap_err().code(),
            "unknown_scorer"
        );
        let mut c = round.challenge().clone();
        c.scoring.time = ScorerId::new("time", 2);
        assert_eq!(prepare_challenge(c).unwrap_err().code(), "unknown_scorer");
        let mut c = round.challenge().clone();
        c.scoring.combined = ScorerId::new("combined", 7);
        assert_eq!(prepare_challenge(c).unwrap_err().code(), "unknown_scorer");
        // The challenge's own scorers prepare.
        let again = prepare_challenge(round.challenge().clone()).unwrap();
        assert_eq!(again.fingerprint(), round.fingerprint());
    }

    #[test]
    fn a_challenge_for_another_phrase_or_a_bad_spec_is_refused() {
        let round = prepare_round(&spec(1, 5)).unwrap();
        let mut c = round.challenge().clone();
        c.phrase = PhraseId(3);
        assert_eq!(prepare_challenge(c).unwrap_err().code(), "phrase_mismatch");
        let mut s = spec(1, 5);
        s.template = 4;
        assert_eq!(prepare_round(&s).unwrap_err().code(), "code_too_new");
        s.template = 0;
        s.seed = 1 << 60;
        assert_eq!(prepare_round(&s).unwrap_err().code(), "out_of_range");
    }

    #[test]
    fn a_prepared_round_crosses_threads() {
        fn both<T: Send + Sync>() {}
        both::<PreparedRound>();
    }
}
