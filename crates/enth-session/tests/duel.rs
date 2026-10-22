//! The duel state (lane SE, freeze point F1e), through the public API: the player's result from the round's first
//! passing submit (or the best one, marked "not passed") with its patch, kept when the round ends; a friend's scorecard
//! shown at once; a friend's answer sealed until the player has played; the verify job, stale results and the receiver's
//! own re-score; loading their patch as one undo step; the first attempt standing; nothing saved. The shell is played by
//! hand and every timestamp is a literal.

mod common;

use enth_challenge::result::{CardAssists, Commitment, Scorecard, ScorecardFields, ScorerSet};
use enth_challenge::score::{ModuleTerm, ParamScore};
use enth_challenge::verify::ClaimCheck;
use enth_challenge::{
    Combined, Direction, EngineVersion, PhraseId, ScoringSpec, SoundScore, SpecPreset,
    TargetFingerprint,
};
use enth_patch::canon::exact_hash;
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModuleKind, Op, Patch, Position, apply_all};
use enth_session::duel::{
    AnswerState, DuelEdit, DuelId, DuelJobId, DuelJobResult, DuelOwn, DuelRequest, DuelResult,
    DuelVerified, FriendAnswer, FriendScorecard, MAX_CODE_TEXT, Rescored, VerifyState,
};
use enth_session::game::{
    Challenge, ChallengeCode, EditorView, EndChoice, GameEdit, GameRequest, Job, JobResult,
    Outcome, Prepared, RandomSpec, RoundSource, ScoreReport, ScoreSummary, StructureScore, Verdict,
};
use enth_session::{Activation, Session, ShellRequest};

use common::{addr, custom_patch, id, init_patch};

const CUTOFF: &str = "filter1.cutoff";
const RESONANCE: &str = "filter1.resonance";
const ACTIVATION: Activation = Activation {
    sample_rate: 48_000.0,
    offline: false,
};

// ---------------------------------------------------------------------------------------------------------------
// The shell, by hand (the Recreate fake of tests/game.rs)
// ---------------------------------------------------------------------------------------------------------------

fn set(a: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr(a),
        value,
    }
}

fn start_patch() -> Patch {
    let mut p = init_patch();
    let ops = [Op::AddModule {
        at: Position {
            section: Section::VoiceFx,
            index: 0,
        },
        module: ModuleInstance::new(id("filter1"), ModuleKind::Svf),
    }];
    apply_all(&mut p, &ops).unwrap();
    p
}

fn target_patch() -> Patch {
    let mut p = start_patch();
    apply_all(&mut p, &[set(CUTOFF, 800.0), set(RESONANCE, 40.0)]).unwrap();
    p
}

fn spec_seed(seed: u64) -> RandomSpec {
    SpecPreset::by_id(2).unwrap().spec(1, seed)
}

fn spec() -> RandomSpec {
    spec_seed(42)
}

fn source() -> RoundSource {
    RoundSource::Settings(spec())
}

fn code() -> String {
    ChallengeCode::short(spec()).unwrap().to_string()
}

/// Another challenge's code (another seed).
fn other_code() -> String {
    ChallengeCode::short(spec_seed(43)).unwrap().to_string()
}

fn scoring() -> ScoringSpec {
    ScoringSpec::recreate(spec().difficulty)
}

fn structure(percent: f64, verdict: Verdict) -> StructureScore {
    StructureScore {
        scorer: scoring().structure,
        percent,
        params: vec![ParamScore {
            addr: CUTOFF.to_owned(),
            answer_addr: Some(CUTOFF.to_owned()),
            score: 0.5,
            verdict,
            direction: Direction::Higher,
            randomized: true,
        }],
        modules: Vec::<ModuleTerm>::new(),
    }
}

/// A report with this sound % (it passes at the default pass mark, 90); combined is 5 below.
fn report(sound: f64) -> ScoreReport {
    let scoring = scoring();
    let verdict = if sound >= 90.0 {
        Verdict::Good
    } else {
        Verdict::Bad
    };
    ScoreReport::new(
        &scoring,
        SoundScore {
            scorer: scoring.sound.clone(),
            percent: sound,
            distance: 0.5,
            floor: 0.0,
            scale: 0.1,
            jnd: 5.0,
            components: Vec::new(),
        },
        structure(sound - 10.0, verdict),
        Combined {
            scorer: scoring.combined.clone(),
            percent: sound - 5.0,
            sound_weight: scoring.sound_weight,
        },
    )
}

fn prepared() -> JobResult {
    let target = target_patch();
    JobResult::Prepared(Box::new(Prepared::new(
        Challenge::recreate(spec(), PhraseId(1)),
        start_patch(),
        target.clone(),
        vec![addr(CUTOFF), addr(RESONANCE)],
        TargetFingerprint(exact_hash(&target).0),
        Some(-14.0),
    )))
}

fn drain(s: &mut Session) -> Vec<GameRequest> {
    s.drain_game_requests().collect()
}

fn duel_requests(s: &mut Session) -> Vec<DuelRequest> {
    s.drain_duel_requests().collect()
}

fn job(requests: &[GameRequest], kind: &str) -> Job {
    let found: Vec<Job> = requests
        .iter()
        .filter_map(|r| match r {
            GameRequest::Run(job) if job.kind.name() == kind => Some(job.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "one {kind} job in {requests:#?}");
    found.into_iter().next().unwrap()
}

/// Start a round at shell time `now` (the editor open) and answer its prepare job.
fn play(s: &mut Session, now: u64) {
    s.game_tick(now, EditorView::Opened);
    s.game_start(source()).unwrap();
    let prepare = job(&drain(s), "prepare");
    s.game_job_done(prepare.id, prepared()).unwrap();
}

/// Submit, and answer the score job with `report`.
fn submit(s: &mut Session, report: ScoreReport) {
    s.game_submit().unwrap();
    let score = job(&drain(s), "score");
    s.game_job_done(score.id, JobResult::Scored(Box::new(report)))
        .unwrap();
}

/// Edit the cutoff to `cutoff`, submit with `sound` %, and return the patch that was submitted.
fn attempt(s: &mut Session, cutoff: f32, sound: f64) -> Patch {
    s.edit(vec![set(CUTOFF, cutoff)], None).unwrap();
    let submitted = s.patch().clone();
    submit(s, report(sound));
    submitted
}

/// A session with a duel open on the round's code and the round started at shell time `1_000`, its target prepared.
fn duel_round() -> (Session, DuelId) {
    let mut s = Session::new(custom_patch());
    let id = s.duel_open(&code()).unwrap();
    play(&mut s, 1_000);
    (s, id)
}

/// Let the engine collect the plans in flight and the session send the pending one.
fn settle(s: &mut Session, engine: &mut enth_engine::Engine) {
    let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
    for _ in 0..6 {
        s.pump();
        engine.process(&mut l, &mut r, &[]);
    }
}

fn friend_answer() -> FriendAnswer {
    FriendAnswer::new(code(), "AN-ANSWER-CODE-TEXT")
}

/// A scorecard as the shell decodes it: `code`'s challenge, scored on `engine`, the claim, 3 submits, 95 s.
fn scorecard(
    code: &str,
    engine: EngineVersion,
    sound: u16,
    combined: u16,
    passed: bool,
) -> Scorecard {
    Scorecard::from_fields(ScorecardFields {
        challenge: ChallengeCode::decode(code).unwrap(),
        engine,
        scorers: ScorerSet::V1,
        sound_tenths: sound,
        combined_tenths: combined,
        passed,
        submits: 3,
        time_ds: 950,
        assists: CardAssists::default(),
        commitment: Commitment::from_bits(7).unwrap(),
    })
    .unwrap()
}

fn friend_card() -> FriendScorecard {
    FriendScorecard::new(
        scorecard(&code(), EngineVersion::CURRENT, 934, 900, true),
        "A-SCORECARD-TEXT",
    )
}

/// `verify`'s claim check of `friend_card` against their re-score: as reported, the committed answer.
fn claim() -> ClaimCheck {
    ClaimCheck {
        claimed_sound_tenths: 934,
        sound_tenths: 934,
        claimed_combined_tenths: 900,
        combined_tenths: 900,
        claimed_passed: true,
        passed: true,
        claimed_engine: EngineVersion::CURRENT,
        engine: EngineVersion::CURRENT,
        commitment_matches: true,
        claimed_scorers: ScorerSet::V1,
        scorers: ScorerSet::V1,
    }
}

fn theirs() -> Rescored {
    Rescored::new(9342, 9000, true)
}

/// The worker's answer to a job that carried no scorecard.
fn verified(own: Option<Rescored>) -> DuelJobResult {
    DuelJobResult::Verified(Box::new(
        DuelVerified::new(own, theirs()).with_patch(target_patch()),
    ))
}

/// The worker's answer to a job that carried their scorecard: with `verify`'s claim check.
fn verified_claimed(own: Option<Rescored>) -> DuelJobResult {
    DuelJobResult::Verified(Box::new(
        DuelVerified::new(own, theirs())
            .with_patch(target_patch())
            .with_claim(Some(claim())),
    ))
}

// ---------------------------------------------------------------------------------------------------------------
// The player's result
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_first_passing_submit_is_the_result_with_its_patch_and_it_outlives_the_round() {
    let mut s = Session::new(custom_patch());
    let before = s.save(None);
    let duel = s.duel_open(&code()).unwrap();
    let view = s.duel().unwrap();
    assert_eq!(
        (view.id, view.code.clone(), view.own.clone()),
        (duel, code(), None)
    );
    assert_eq!(s.duel_own(), None, "nothing played yet");
    play(&mut s, 1_000);
    assert_eq!(
        s.duel().unwrap().own,
        None,
        "a round with no submit has no result"
    );

    // A failing submit is the best so far, marked "not passed"; it is not final while the round can still improve.
    s.game_tick(3_000, EditorView::Visible);
    let first = attempt(&mut s, 900.0, 41.0);
    let own = s.duel().unwrap().own.unwrap();
    assert_eq!(
        (
            own.outcome,
            own.number,
            own.submits,
            own.settled,
            own.time_ms
        ),
        (Outcome::NotPassed, Some(1), 1, false, None)
    );
    assert_eq!(
        own.score,
        Some(ScoreSummary::from_percent(false, 41.0, 31.0, 36.0))
    );
    assert!(own.answer_kept);
    assert!(!own.can_share(), "the round can still improve");

    // The first passing submit is the result, and it is final: a later, better submit does not replace it.
    s.game_tick(6_000, EditorView::Visible);
    let second = attempt(&mut s, 805.0, 92.5);
    let own = s.duel().unwrap().own.unwrap();
    assert_eq!(
        (own.outcome, own.number, own.submits, own.settled),
        (Outcome::Passed, Some(2), 2, true)
    );
    assert_eq!(own.time_ms, Some(5_000), "the time to pass, reported");
    assert_eq!(own.score.unwrap().sound, 9250);
    assert!(own.score.unwrap().passed);
    assert_eq!(own.engine, EngineVersion::CURRENT);
    assert_eq!(
        own.scoring,
        Some(scoring()),
        "the scorers the round was judged with"
    );
    assert!(own.can_share() && own.can_share_answer());
    let result = s.duel_own().unwrap();
    assert_eq!(result.own, own);
    assert_eq!(
        exact_hash(result.answer.as_deref().unwrap()),
        exact_hash(&second),
        "the answer is the patch of the passing submit"
    );
    assert_ne!(exact_hash(&first), exact_hash(&second));

    s.game_tick(7_000, EditorView::Visible);
    attempt(&mut s, 810.0, 95.0);
    let later = s.duel_own().unwrap();
    assert_eq!(later.own.number, Some(2), "the first pass stands");
    assert_eq!(
        exact_hash(later.answer.as_deref().unwrap()),
        exact_hash(&second)
    );
    assert_eq!(later.own.submits, 3);

    // The round ends and the player's patch comes back; the result and its patch are still here.
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.round(), None);
    assert_eq!(s.save(None), before, "the project is exactly as it was");
    let kept = s.duel_own().unwrap();
    assert_eq!(kept.own.number, Some(2));
    assert_eq!(kept.own.submits, 3);
    assert!(kept.own.can_share_answer());
    assert_eq!(
        exact_hash(kept.answer.as_deref().unwrap()),
        exact_hash(&second)
    );
    assert_eq!(s.duel().unwrap().own.as_ref(), Some(&kept.own));
    assert_eq!(s.duel().unwrap().answer, AnswerState::None);
}

#[test]
fn without_a_pass_the_best_submit_is_the_result_marked_not_passed_and_ties_go_to_the_earlier() {
    let (mut s, _) = duel_round();
    s.game_tick(2_000, EditorView::Visible);
    let a = attempt(&mut s, 900.0, 41.0);
    let b = attempt(&mut s, 850.0, 60.0);
    let _c = attempt(&mut s, 870.0, 60.0);
    let _d = attempt(&mut s, 880.0, 55.0);
    let own = s.duel().unwrap().own.unwrap();
    assert_eq!(
        (own.outcome, own.number, own.submits),
        (Outcome::NotPassed, Some(2), 4)
    );
    assert!(!own.score.unwrap().passed, "marked not passed");
    assert!(!own.settled && !own.can_share(), "the round is still going");
    let result = s.duel_own().unwrap();
    assert_eq!(
        exact_hash(result.answer.as_deref().unwrap()),
        exact_hash(&b)
    );
    assert_ne!(exact_hash(&a), exact_hash(&b));

    // Ending the round settles it; the best submit is what can be shared.
    s.game_end(EndChoice::Keep).unwrap();
    let kept = s.duel_own().unwrap();
    assert_eq!(kept.own.number, Some(2));
    assert!(kept.own.settled && kept.own.can_share_answer());
    assert_eq!(exact_hash(kept.answer.as_deref().unwrap()), exact_hash(&b));
}

#[test]
fn a_round_ended_with_no_submit_has_no_result_and_a_reveal_is_a_give_up_without_a_scorecard() {
    // Ending before any submit: nothing was played, nothing is kept.
    let (mut s, _) = duel_round();
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.duel().unwrap().own, None);
    assert_eq!(s.duel_own(), None);

    // A reveal with no submit is a give-up: the round has a result, with no score and no scorecard.
    let (mut s, _) = duel_round();
    s.game_tick(4_000, EditorView::Visible);
    s.game_reveal().unwrap();
    let own = s.duel().unwrap().own.unwrap();
    assert_eq!(
        (own.outcome, own.number, own.score, own.submits),
        (Outcome::GaveUp, None, None, 0)
    );
    assert!(own.assists.revealed && own.settled);
    assert!(!own.can_share(), "no score");
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.duel_own().unwrap().own.outcome, Outcome::GaveUp);

    // A pass, then a reveal: the pass is the result, but a revealed target gives no scorecard.
    let (mut s, _) = duel_round();
    attempt(&mut s, 800.0, 95.0);
    assert!(s.duel().unwrap().own.unwrap().can_share());
    s.game_reveal().unwrap();
    let own = s.duel().unwrap().own.unwrap();
    assert_eq!((own.number, own.score.unwrap().passed), (Some(1), true));
    assert!(own.assists.revealed);
    assert!(
        !own.can_share() && !own.can_share_answer(),
        "a revealed target gives no scorecard"
    );
}

#[test]
fn only_a_games_round_on_the_duels_code_counts() {
    // A round on another code is not the duel's.
    let mut s = Session::new(custom_patch());
    s.duel_open(&other_code()).unwrap();
    play(&mut s, 0);
    attempt(&mut s, 800.0, 95.0);
    assert_eq!(
        s.duel().unwrap().own,
        None,
        "the duel is on another challenge"
    );
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.duel().unwrap().own, None);

    // With no duel open, the live round answers `duel_own` whatever its code (the share line after a pass).
    let mut s = Session::new(custom_patch());
    assert_eq!(s.duel(), None);
    play(&mut s, 0);
    let second = attempt(&mut s, 800.0, 95.0);
    let own = s.duel_own().unwrap();
    assert_eq!(own.own.code, code());
    assert_eq!(
        exact_hash(own.answer.as_deref().unwrap()),
        exact_hash(&second)
    );
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(
        s.duel_own(),
        None,
        "no duel keeps it: the shell's history does"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Their scorecard and answer
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_scorecard_is_shown_at_once_as_reported_and_a_newer_one_replaces_it() {
    let mut s = Session::new(init_patch());
    assert_eq!(
        s.duel_import_scorecard(friend_card()).map(|_| ()),
        Ok(()),
        "with no duel open it opens one"
    );
    let v = s.duel().unwrap();
    assert_eq!(v.code, code());
    let card = v.card.clone().unwrap();
    let f = card.card.fields();
    assert_eq!(
        (f.sound_tenths, f.combined_tenths, f.passed),
        (934, 900, true)
    );
    assert_eq!((f.submits, f.time_ds), (3, 950));
    assert_eq!(card.code(), code());
    assert!(!v.engine_differs);
    assert!(!v.scorers_differ);
    assert_eq!(
        (v.answer, v.verify.clone(), v.own.is_some()),
        (AnswerState::None, VerifyState::Idle, false)
    );

    // Another engine: both numbers are shown, and the view says so.
    let other_engine = EngineVersion {
        major: 0,
        minor: 0,
        patch: 1,
    };
    assert_ne!(other_engine, EngineVersion::CURRENT);
    let mut fields = scorecard(&code(), other_engine, 500, 450, false)
        .fields()
        .clone();
    fields.assists.hints = 2;
    let newer = FriendScorecard::new(Scorecard::from_fields(fields).unwrap(), "ANOTHER");
    s.duel_import_scorecard(newer).unwrap();
    let v = s.duel().unwrap();
    assert!(v.engine_differs);
    let card = v.card.unwrap();
    assert_eq!(
        (card.text.as_str(), card.card.fields().assists.hints),
        ("ANOTHER", 2)
    );

    // A newer build's scorer set (the F1e review's M4): shown as reported, and the view says both numbers follow.
    let mut fields = scorecard(&code(), EngineVersion::CURRENT, 900, 880, true)
        .fields()
        .clone();
    fields.scorers = ScorerSet::from_index(2).unwrap();
    let sound2 = FriendScorecard::new(Scorecard::from_fields(fields).unwrap(), "SOUND-2");
    s.duel_import_scorecard(sound2).unwrap();
    let v = s.duel().unwrap();
    assert!(v.scorers_differ && !v.engine_differs);
    assert_eq!(v.card.unwrap().card.fields().sound_tenths, 900);
    let mut fields = scorecard(&code(), other_engine, 500, 450, false)
        .fields()
        .clone();
    fields.assists.hints = 2;
    s.duel_import_scorecard(FriendScorecard::new(
        Scorecard::from_fields(fields).unwrap(),
        "ANOTHER",
    ))
    .unwrap();

    // A card for another challenge does not replace this duel's; a damaged card is refused.
    assert_eq!(
        s.duel_import_scorecard(FriendScorecard::new(
            scorecard(&other_code(), EngineVersion::CURRENT, 1, 1, false),
            "X",
        ))
        .map_err(|e| e.code()),
        Err("other_challenge")
    );
    // A card's numbers are in range by construction (`Scorecard` refuses 100.1 %); a text too long is refused.
    let mut fields = scorecard(&code(), EngineVersion::CURRENT, 0, 0, true)
        .fields()
        .clone();
    fields.sound_tenths = 1001;
    assert!(Scorecard::from_fields(fields).is_err());
    let long = FriendScorecard::new(
        scorecard(&code(), EngineVersion::CURRENT, 0, 0, true),
        "X".repeat(MAX_CODE_TEXT + 1),
    );
    assert!(s.duel_import_scorecard(long).is_err());
    assert_eq!(
        s.duel().unwrap().card.unwrap().text,
        "ANOTHER",
        "refused imports change nothing"
    );
}

#[test]
fn their_answer_stays_sealed_until_the_player_has_played() {
    let (mut s, id) = duel_round();
    assert_eq!(s.duel_import_answer(friend_answer()), Ok(id));
    s.duel_import_scorecard(friend_card()).unwrap();
    let v = s.duel().unwrap();
    assert_eq!((v.answer, v.checked.is_none()), (AnswerState::Sealed, true));
    assert!(
        v.card.is_some(),
        "the scorecard spoils nothing: it shows at once"
    );
    let shown = format!("{v:?}");
    assert!(
        !shown.contains("AN-ANSWER-CODE-TEXT"),
        "nothing of a sealed answer is in the view"
    );

    // Every intent that would read it refuses.
    assert_eq!(s.duel_verify().map_err(|e| e.code()), Err("answer_sealed"));
    assert_eq!(
        s.duel_use_answer("x").map_err(|e| e.code()),
        Err("answer_sealed")
    );
    assert_eq!(
        s.duel_audition().map_err(|e| e.code()),
        Err("answer_sealed")
    );
    let sent = duel_requests(&mut s);
    assert!(sent.is_empty(), "{sent:?}");

    // Playing the round without a submit does not open it; the first submit does.
    s.game_tick(2_000, EditorView::Visible);
    s.edit(vec![set(CUTOFF, 900.0)], None).unwrap();
    assert_eq!(s.duel().unwrap().answer, AnswerState::Sealed);
    let submitted = s.patch().clone();
    submit(&mut s, report(41.0));
    let v = s.duel().unwrap();
    assert_eq!(v.answer, AnswerState::Open);
    assert!(
        !format!("{v:?}").contains("AN-ANSWER-CODE-TEXT"),
        "the view never carries the code text"
    );

    // The job carries the answer text, the card text and the player's own patch (the compared submit).
    let job = s.duel_verify().unwrap();
    let requests = duel_requests(&mut s);
    assert_eq!(requests.len(), 1, "{requests:?}");
    let DuelRequest::Verify(v) = &requests[0] else {
        panic!("{requests:?}")
    };
    assert_eq!((v.job, v.duel, v.code.as_str()), (job, id, code().as_str()));
    assert_eq!(
        (v.answer.as_str(), v.card.as_deref()),
        ("AN-ANSWER-CODE-TEXT", Some("A-SCORECARD-TEXT"))
    );
    assert_eq!(
        exact_hash(v.own.as_deref().unwrap()),
        exact_hash(&submitted)
    );
    assert_eq!(s.duel_verify().map_err(|e| e.code()), Err("busy"));
    assert_eq!(s.duel().unwrap().verify, VerifyState::Running);
}

#[test]
fn an_answer_imported_after_the_player_played_is_open_at_once_and_a_reveal_opens_it_too() {
    let (mut s, _) = duel_round();
    attempt(&mut s, 800.0, 95.0);
    s.duel_import_answer(friend_answer()).unwrap();
    assert_eq!(s.duel().unwrap().answer, AnswerState::Open);

    // No submit, but the player gave up by revealing: their answer opens, there is no score of the player's to show.
    let (mut s, _) = duel_round();
    s.duel_import_answer(friend_answer()).unwrap();
    assert_eq!(s.duel().unwrap().answer, AnswerState::Sealed);
    s.game_reveal().unwrap();
    assert_eq!(s.duel().unwrap().answer, AnswerState::Open);
    s.duel_verify().unwrap();
    let requests = duel_requests(&mut s);
    let DuelRequest::Verify(v) = &requests[0] else {
        panic!("{requests:?}")
    };
    assert!(v.own.is_none(), "nothing of the player's to re-score");

    // An answer with no duel open opens one; one for another challenge is refused.
    let mut s = Session::new(init_patch());
    let id = s.duel_import_answer(friend_answer()).unwrap();
    assert_eq!(s.duel().unwrap().id, id);
    assert_eq!(
        s.duel().unwrap().answer,
        AnswerState::Sealed,
        "nothing was played"
    );
    assert_eq!(
        s.duel_import_answer(FriendAnswer::new(other_code(), "X"))
            .map_err(|e| e.code()),
        Err("other_challenge")
    );
    assert_eq!(
        s.duel_import_answer(FriendAnswer::new(code(), ""))
            .map_err(|e| e.code()),
        Err("bad_import")
    );
    assert!(
        s.duel_import_answer(FriendAnswer::new("nope", "X"))
            .is_err(),
        "a code that does not decode opens nothing"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The verify job
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_verify_job_posts_the_receivers_own_scores_and_stale_results_are_refused() {
    let (mut s, id) = duel_round();
    s.duel_import_answer(friend_answer()).unwrap();
    s.duel_import_scorecard(friend_card()).unwrap();
    attempt(&mut s, 800.0, 92.5);
    let own = Rescored::new(9250, 8750, true);
    let first = s.duel_verify().unwrap();
    let requests = duel_requests(&mut s);
    assert!(
        matches!(requests.as_slice(), [DuelRequest::Verify(v)] if v.job == first && v.duel == id)
    );

    // A result for a job the duel never asked for is stale and changes nothing.
    let stray = DuelJobId(first.0 + 7);
    assert_eq!(
        s.duel_job_done(stray, verified_claimed(Some(own)))
            .map_err(|e| e.code()),
        Err("stale_job")
    );
    assert_eq!(s.duel().unwrap().verify, VerifyState::Running);

    // The result is what the worker measured; the claim is only compared.
    s.duel_job_done(first, verified_claimed(Some(own))).unwrap();
    let v = s.duel().unwrap();
    assert_eq!(v.verify, VerifyState::Done);
    let checked = v.checked.unwrap();
    assert_eq!((checked.own, checked.theirs), (Some(own), theirs()));
    assert_eq!(
        (
            checked.commitment_ok(),
            checked.matches_claim(),
            checked.has_patch,
            checked.own_current
        ),
        (Some(true), Some(true), true, true)
    );
    assert_eq!(
        checked.claim,
        Some(claim()),
        "both numbers, engines and sets, as verify gave them"
    );
    assert_eq!(
        s.duel_job_done(first, verified_claimed(Some(own)))
            .map_err(|e| e.code()),
        Err("stale_job"),
        "a job is answered once"
    );

    // A result that does not fit the job fails the check, which can be asked again: the job carried the player's patch,
    // so the own re-score cannot be left out; and no score is above 100 %.
    let second = s.duel_verify().unwrap();
    assert_eq!(
        s.duel_job_done(second, verified_claimed(None))
            .map_err(|e| e.code()),
        Err("wrong_result")
    );
    let v = s.duel().unwrap();
    assert!(
        matches!(&v.verify, VerifyState::Failed(f) if f.code == "wrong_result"),
        "{:?}",
        v.verify
    );
    assert!(v.checked.is_none(), "nothing of a bad result is kept");
    let third = s.duel_verify().unwrap();
    let high = DuelVerified::new(Some(own), Rescored::new(10_001, 0, true));
    assert_eq!(
        s.duel_job_done(third, DuelJobResult::Verified(Box::new(high)))
            .map_err(|e| e.code()),
        Err("wrong_result")
    );

    // A refusal from the worker (this build cannot read their answer) is shown with its hint, and asked again later.
    let fourth = s.duel_verify().unwrap();
    s.duel_job_done(
        fourth,
        DuelJobResult::Failed {
            code: "answer_too_new".to_owned(),
            message: "their app is newer".to_owned(),
            hint: "update enth".to_owned(),
        },
    )
    .unwrap();
    match s.duel().unwrap().verify {
        VerifyState::Failed(f) => assert_eq!(
            (f.code.as_str(), f.hint.as_str()),
            ("answer_too_new", "update enth")
        ),
        other => panic!("{other:?}"),
    }
    assert!(s.duel_verify().is_ok());
}

#[test]
fn replacing_the_answer_or_the_card_or_closing_the_duel_cancels_a_running_check() {
    let (mut s, _) = duel_round();
    s.duel_import_answer(friend_answer()).unwrap();
    attempt(&mut s, 800.0, 92.5);
    let own = Rescored::new(9250, 8750, true);

    // A newer answer replaces the old one: the job is cancelled and its result is stale.
    let job = s.duel_verify().unwrap();
    let _ = duel_requests(&mut s);
    s.duel_import_answer(FriendAnswer::new(code(), "ANOTHER-ANSWER"))
        .unwrap();
    assert_eq!(duel_requests(&mut s), [DuelRequest::Cancel(job)]);
    assert_eq!(s.duel().unwrap().verify, VerifyState::Idle);
    assert_eq!(
        s.duel_job_done(job, verified(Some(own)))
            .map_err(|e| e.code()),
        Err("stale_job")
    );

    // A finished check is forgotten when the inputs change: a card replaces the card, their patch is gone with it.
    let job = s.duel_verify().unwrap();
    s.duel_job_done(job, verified(Some(own))).unwrap();
    assert!(s.duel().unwrap().checked.is_some());
    s.duel_import_scorecard(friend_card()).unwrap();
    let v = s.duel().unwrap();
    assert_eq!(
        (v.verify, v.checked, v.used),
        (VerifyState::Idle, None, false)
    );
    assert_eq!(
        s.duel_use_answer("x").map_err(|e| e.code()),
        Err("not_checked")
    );

    // Closing the duel cancels the job, and its result is stale.
    let _ = duel_requests(&mut s);
    let job = s.duel_verify().unwrap();
    let _ = duel_requests(&mut s);
    s.duel_close();
    assert_eq!(duel_requests(&mut s), [DuelRequest::Cancel(job)]);
    assert_eq!(s.duel(), None);
    assert_eq!(
        s.duel_job_done(job, verified(Some(own)))
            .map_err(|e| e.code()),
        Err("stale_job")
    );
    assert_eq!(s.duel_verify().map_err(|e| e.code()), Err("no_duel"));
    assert_eq!(s.duel_use_answer("x").map_err(|e| e.code()), Err("no_duel"));
    assert_eq!(s.duel_audition().map_err(|e| e.code()), Err("no_duel"));
    s.duel_close();
    assert!(
        duel_requests(&mut s).is_empty(),
        "closing nothing sends nothing"
    );

    // Opening another duel closes the first the same way.
    let (mut s, _) = duel_round();
    s.duel_import_answer(friend_answer()).unwrap();
    attempt(&mut s, 800.0, 92.5);
    let job = s.duel_verify().unwrap();
    let _ = duel_requests(&mut s);
    s.duel_open(&other_code()).unwrap();
    assert_eq!(duel_requests(&mut s), [DuelRequest::Cancel(job)]);
}

#[test]
fn a_check_made_while_the_round_ran_stands_only_while_the_compared_submit_is_the_same() {
    let own = Rescored::new(9250, 8750, true);
    // A pass is final: ending the round keeps the check.
    let (mut s, _) = duel_round();
    s.duel_import_answer(friend_answer()).unwrap();
    attempt(&mut s, 800.0, 92.5);
    let job = s.duel_verify().unwrap();
    s.duel_job_done(job, verified(Some(own))).unwrap();
    s.game_end(EndChoice::Restore).unwrap();
    let v = s.duel().unwrap();
    assert_eq!(v.verify, VerifyState::Done);
    assert!(v.checked.unwrap().own_current);

    // A round that had not passed moves its compared submit on: the view marks the own score out of date at once, and
    // ending the round forgets the check (a running one is cancelled).
    let (mut s, _) = duel_round();
    s.duel_import_answer(friend_answer()).unwrap();
    attempt(&mut s, 900.0, 41.0);
    let job = s.duel_verify().unwrap();
    s.duel_job_done(job, verified(Some(Rescored::new(4100, 3600, false))))
        .unwrap();
    assert!(s.duel().unwrap().checked.unwrap().own_current);
    attempt(&mut s, 850.0, 60.0);
    assert!(
        !s.duel().unwrap().checked.unwrap().own_current,
        "a better submit since"
    );
    let job = s.duel_verify().unwrap();
    let _ = duel_requests(&mut s);
    attempt(&mut s, 840.0, 70.0);
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(duel_requests(&mut s), [DuelRequest::Cancel(job)]);
    let v = s.duel().unwrap();
    assert_eq!((v.verify, v.checked), (VerifyState::Idle, None));
    assert_eq!(
        s.duel_job_done(job, verified(Some(own)))
            .map_err(|e| e.code()),
        Err("stale_job")
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Their patch
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn their_patch_loads_as_one_undo_step_with_no_host_writes_and_plays_once_checked() {
    let mut s = Session::new(custom_patch());
    let mut engine = s.activate(ACTIVATION).unwrap();
    settle(&mut s, &mut engine);
    let before = s.patch().clone();
    s.duel_open(&code()).unwrap();
    s.duel_import_answer(friend_answer()).unwrap();
    play(&mut s, 1_000);
    attempt(&mut s, 800.0, 92.5);

    // Their patch is read from the check.
    assert_eq!(
        s.duel_use_answer("Their take").map_err(|e| e.code()),
        Err("not_checked")
    );
    assert_eq!(s.duel_audition().map_err(|e| e.code()), Err("not_checked"));
    let job = s.duel_verify().unwrap();
    s.duel_job_done(job, verified(Some(Rescored::new(9250, 8750, true))))
        .unwrap();
    // The round's patch is the player's work in progress: it is not replaced under them.
    assert_eq!(
        s.duel_use_answer("Their take").map_err(|e| e.code()),
        Err("round_live")
    );
    assert!(!s.duel().unwrap().used);

    // Audition is the shell's render: it works during the round, once, and stops once.
    let _ = duel_requests(&mut s);
    s.duel_audition().unwrap();
    assert!(s.duel().unwrap().auditioning);
    s.duel_stop_audition();
    s.duel_stop_audition();
    let requests = duel_requests(&mut s);
    assert_eq!(requests.len(), 2, "{requests:?}");
    match &requests[0] {
        DuelRequest::Audition { code: c, patch, .. } => {
            assert_eq!(c, &code());
            assert_eq!(exact_hash(patch), exact_hash(&target_patch()));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(requests[1], DuelRequest::StopAudition);

    s.game_end(EndChoice::Restore).unwrap();
    settle(&mut s, &mut engine);
    let _ = s.drain_requests().count();
    let epoch = s.status().epoch;
    let own_name = s.patch().meta.name.clone();
    s.duel_use_answer("Their take").unwrap();
    assert_eq!(s.patch().meta.name, "Their take");
    assert_eq!(s.patch().value(&addr(CUTOFF)), Some(800.0));
    assert!(s.duel().unwrap().used);
    settle(&mut s, &mut engine);
    let shell: Vec<ShellRequest> = s.drain_requests().collect();
    assert!(shell.contains(&ShellRequest::PatchEdited), "{shell:?}");
    assert!(
        !shell.iter().any(|r| matches!(r, ShellRequest::Host(_))),
        "no host writes: {shell:?}"
    );
    assert_eq!(s.status().epoch, epoch + 1, "one plan");
    assert!(s.history().can_undo());

    // One undo brings the player's patch back whole.
    assert_eq!(s.undo(), Ok(true));
    assert_eq!(s.patch().to_json(), before.to_json());
    assert_eq!(s.patch().meta.name, own_name);
    assert!(!s.history().can_undo(), "one step");

    // An empty name keeps the patch's own.
    s.duel_use_answer("  ").unwrap();
    assert_eq!(s.patch().meta.name, target_patch().meta.name);
}

#[test]
fn a_sealed_answer_refuses_every_intent_and_a_missing_answer_says_so() {
    let mut s = Session::new(init_patch());
    s.duel_import_answer(friend_answer()).unwrap();
    s.duel_import_scorecard(friend_card()).unwrap();
    assert_eq!(s.duel().unwrap().answer, AnswerState::Sealed);
    for e in [
        s.duel_use_answer("x").map_err(|e| e.code()),
        s.duel_audition().map_err(|e| e.code()),
        s.duel_verify().map(|_| ()).map_err(|e| e.code()),
    ] {
        assert_eq!(e, Err("answer_sealed"));
    }
    // With no answer imported at all, verify says so.
    s.duel_close();
    s.duel_import_scorecard(friend_card()).unwrap();
    assert_eq!(s.duel_verify().map_err(|e| e.code()), Err("no_answer"));
}

// ---------------------------------------------------------------------------------------------------------------
// The first attempt, the shell's history, and nothing saved
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_first_attempt_stands_and_a_result_from_the_shells_history_restores() {
    let (mut s, _) = duel_round();
    let passed = attempt(&mut s, 800.0, 92.5);
    s.game_end(EndChoice::Restore).unwrap();
    let first = s.duel_own().unwrap();

    // A rematch on the same code: the duel keeps the first attempt.
    play(&mut s, 20_000);
    attempt(&mut s, 700.0, 99.0);
    assert_eq!(s.duel_own().unwrap(), first);
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(
        s.duel_own().unwrap(),
        first,
        "the first played attempt at a duel's code stands"
    );
    assert_eq!(
        s.duel_set_own(first.clone()).map_err(|e| e.code()),
        Err("already_played")
    );

    // The project closed and the duel reopened from the shell's history: the result and its patch come back.
    let mut s = Session::new(init_patch());
    assert_eq!(
        s.duel_set_own(first.clone()).map_err(|e| e.code()),
        Err("no_duel")
    );
    s.duel_open(&code()).unwrap();
    s.duel_import_answer(friend_answer()).unwrap();
    assert_eq!(s.duel().unwrap().answer, AnswerState::Sealed);
    s.duel_set_own(first.clone()).unwrap();
    let v = s.duel().unwrap();
    assert_eq!(v.own.as_ref(), Some(&first.own));
    assert_eq!(v.answer, AnswerState::Open, "they have played");
    assert_eq!(s.duel_own().unwrap(), first);
    s.duel_verify().unwrap();
    let requests = duel_requests(&mut s);
    let DuelRequest::Verify(v) = &requests[0] else {
        panic!("{requests:?}")
    };
    assert_eq!(
        exact_hash(v.own.as_deref().unwrap()),
        exact_hash(&passed),
        "the restored patch is re-scored"
    );

    // What the shell restores must fit the duel.
    let mut s = Session::new(init_patch());
    s.duel_open(&code()).unwrap();
    let score = ScoreSummary::from_percent(true, 93.0, 80.0, 90.0);
    let other = DuelOwn::new(other_code(), Outcome::Passed, 2, 9_000).with_score(1, score);
    assert_eq!(
        s.duel_set_own(DuelResult::new(other, None))
            .map_err(|e| e.code()),
        Err("other_challenge")
    );
    let unplayed = DuelOwn::new(code(), Outcome::NotPassed, 0, 9_000);
    assert_eq!(
        s.duel_set_own(DuelResult::new(unplayed, None))
            .map_err(|e| e.code()),
        Err("not_played")
    );
    let high = ScoreSummary {
        sound: 10_001,
        ..score
    };
    let bad = DuelOwn::new(code(), Outcome::Passed, 2, 9_000).with_score(1, high);
    assert_eq!(
        s.duel_set_own(DuelResult::new(bad, None))
            .map_err(|e| e.code()),
        Err("bad_import")
    );
    assert_eq!(
        s.duel().unwrap().own,
        None,
        "refused restores change nothing"
    );

    // A result with no patch is shared as a scorecard only: the answer code needs the patch.
    let card_only = DuelOwn::new(code(), Outcome::Passed, 2, 9_000).with_score(1, score);
    s.duel_set_own(DuelResult::new(card_only, None)).unwrap();
    let own = s.duel().unwrap().own.unwrap();
    assert!(own.can_share() && !own.can_share_answer() && !own.answer_kept);
    // With no patch there is nothing to re-score of the player's.
    s.duel_import_answer(friend_answer()).unwrap();
    s.duel_verify().unwrap();
    let requests = duel_requests(&mut s);
    let DuelRequest::Verify(v) = &requests[0] else {
        panic!("{requests:?}")
    };
    assert!(v.own.is_none());
}

#[test]
fn a_duel_is_not_saved_and_the_round_keeps_the_first_passs_patch_across_a_reload() {
    let mut s = Session::new(custom_patch());
    let before = s.save_instance();
    s.duel_open(&code()).unwrap();
    s.duel_import_answer(friend_answer()).unwrap();
    s.duel_import_scorecard(friend_card()).unwrap();
    assert_eq!(s.save_instance(), before, "the enth field keeps its bytes");

    play(&mut s, 1_000);
    assert!(!s.save_instance().contains("\"compared\""), "no submit yet");
    s.game_tick(3_000, EditorView::Visible);
    let second = attempt(&mut s, 805.0, 92.5);
    let saved = s.save(None);
    assert!(
        saved.instance.contains("\"compared\""),
        "the first pass's patch is in the saved round"
    );
    let compared = s.game_record().unwrap().compared.clone().unwrap();
    assert_eq!(compared.submit, 1);
    assert!(compared.ops.is_some());

    // A host state load of the very state leaves the duel alone.
    let loaded = s.load(&saved);
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    assert_eq!(s.duel().unwrap().code, code());
    assert!(s.duel().unwrap().card.is_some());

    // A fresh instance restores the round and no duel. The result is there at once; its patch is back when the target
    // (and with it the start) is regenerated.
    let mut other = Session::new(init_patch());
    let loaded = other.load(&saved);
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    assert_eq!(other.duel(), None);
    assert_eq!(other.game_record().unwrap().compared, Some(compared));
    let own = other.duel_own().unwrap();
    assert_eq!(
        (own.own.number, own.own.answer_kept, own.answer),
        (Some(1), false, None)
    );
    let prepare = job(&drain(&mut other), "prepare");
    other.game_job_done(prepare.id, prepared()).unwrap();
    let own = other.duel_own().unwrap();
    assert!(own.own.answer_kept);
    assert_eq!(
        exact_hash(own.answer.as_deref().unwrap()),
        exact_hash(&second)
    );
}

#[test]
fn the_best_submit_of_a_round_that_has_not_passed_is_held_in_memory_and_not_saved() {
    let mut s = Session::new(custom_patch());
    s.duel_open(&code()).unwrap();
    play(&mut s, 1_000);
    let first = attempt(&mut s, 900.0, 41.0);
    let saved = s.save(None);
    assert!(
        !saved.instance.contains("\"compared\""),
        "a best that can still change is not saved: the record keeps the bytes it had"
    );
    assert!(s.game_record().unwrap().compared.is_none());
    assert_eq!(
        exact_hash(s.duel_own().unwrap().answer.as_deref().unwrap()),
        exact_hash(&first)
    );

    // A reload forgets the patch, not the scores.
    let mut other = Session::new(init_patch());
    other.duel_open(&code()).unwrap();
    let loaded = other.load(&saved);
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    let prepare = job(&drain(&mut other), "prepare");
    other.game_job_done(prepare.id, prepared()).unwrap();
    let own = other.duel_own().unwrap();
    assert_eq!(
        (own.own.number, own.own.score.unwrap().sound),
        (Some(1), 4100)
    );
    assert!(!own.own.answer_kept && own.answer.is_none());
    assert!(!own.own.can_share(), "the round can still improve");

    // A worse submit changes nothing; a better one brings the patch back.
    let worse = attempt(&mut other, 950.0, 30.0);
    assert!(!other.duel_own().unwrap().own.answer_kept);
    let better = attempt(&mut other, 850.0, 60.0);
    let own = other.duel_own().unwrap();
    assert_eq!((own.own.number, own.own.answer_kept), (Some(3), true));
    assert_eq!(
        exact_hash(own.answer.as_deref().unwrap()),
        exact_hash(&better)
    );
    assert_ne!(exact_hash(&worse), exact_hash(&better));

    // Ended without the patch, the scorecard can still go out; the answer code cannot.
    let mut third = Session::new(custom_patch());
    third.duel_open(&code()).unwrap();
    third.load(&saved);
    let prepare = job(&drain(&mut third), "prepare");
    third.game_job_done(prepare.id, prepared()).unwrap();
    third.game_end(EndChoice::Restore).unwrap();
    let kept = third.duel_own().unwrap();
    assert!(kept.own.can_share() && !kept.own.can_share_answer());
}

#[test]
fn a_duel_edits_never_reach_the_engine_or_the_host() {
    let mut s = Session::new(init_patch());
    let _engine = s.activate(ACTIVATION).unwrap();
    s.pump();
    let _ = s.drain_requests().count();
    let epoch = s.status().epoch;
    let score = ScoreSummary::from_percent(true, 93.0, 80.0, 90.0);
    for _ in 0..3 {
        s.duel_open(&code()).unwrap();
        s.duel_import_scorecard(friend_card()).unwrap();
        s.duel_import_answer(friend_answer()).unwrap();
        let own = DuelOwn::new(code(), Outcome::Passed, 2, 9_000).with_score(1, score);
        s.duel_set_own(DuelResult::new(own, Some(target_patch())))
            .unwrap();
        let job = s.duel_verify().unwrap();
        s.duel_job_done(job, verified_claimed(Some(Rescored::new(9300, 9000, true))))
            .unwrap();
        s.duel_audition().unwrap();
        s.duel_stop_audition();
        s.duel_close();
    }
    let shell: Vec<ShellRequest> = s.drain_requests().collect();
    assert!(
        shell.is_empty(),
        "no host write, no rename, no dirty mark: {shell:?}"
    );
    assert_eq!(s.status().epoch, epoch, "no plan");
    assert_eq!(s.drain_game_requests().count(), 0);
}
