//! Lane REV's session side (`enth_session::review`): a Review item plays as a round of origin `review` that stashes
//! like a game round and gives no hint before its first submit; every played round's first-submit rings go to the
//! shell as `LearnRequest::Rings` (a host load's close excepted), and a skill's box follows them once applied.

mod common;

use enth_challenge::score::ParamScore;
use enth_challenge::{
    Combined, Direction, PhraseId, ScoringSpec, SoundScore, SpecPreset, TargetFingerprint,
};
use enth_learn::Progress;
use enth_learn::review::{ReviewItem, RoundRings, Skill};
use enth_patch::canon::exact_hash;
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModuleKind, Op, Patch, Position, apply_all};
use enth_session::game::{
    Challenge, EditorView, EndChoice, GameEdit, GameError, GameRequest, Job, JobResult, Phase,
    Prepared, RandomSpec, RoundLogEntry, RoundOrigin, RoundSource, ScoreReport, StructureScore,
    Verdict,
};
use enth_session::learn::{LearnEdit, LearnError, LearnRequest};
use enth_session::{SavedState, Session};

use common::{addr, custom_patch, id, init_patch};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const CUTOFF: &str = "filter1.cutoff";
const TODAY: u32 = 20_747;

fn round_start() -> Patch {
    let mut p = init_patch();
    let add = Op::AddModule {
        at: Position {
            section: Section::VoiceFx,
            index: 0,
        },
        module: ModuleInstance::new(id("filter1"), ModuleKind::Svf),
    };
    apply_all(&mut p, &[add]).unwrap();
    p
}

fn round_target() -> Patch {
    let mut p = round_start();
    apply_all(
        &mut p,
        &[Op::SetParam {
            addr: addr(CUTOFF),
            value: 800.0,
        }],
    )
    .unwrap();
    p
}

fn spec() -> RandomSpec {
    SpecPreset::by_id(2).unwrap().spec(1, 42)
}

fn challenge() -> Challenge {
    Challenge::recreate(spec(), PhraseId(1))
}

fn prepared() -> JobResult {
    JobResult::Prepared(Box::new(Prepared::new(
        challenge(),
        round_start(),
        round_target(),
        vec![addr(CUTOFF)],
        TargetFingerprint(exact_hash(&round_target()).0),
        Some(-14.0),
    )))
}

fn scoring() -> ScoringSpec {
    ScoringSpec::recreate(spec().difficulty)
}

/// A score with one cutoff row: `verdict` on it, and sound `sound` %.
fn report(sound: f64, verdict: Verdict) -> ScoreReport {
    let scoring = scoring();
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
        StructureScore {
            scorer: scoring.structure.clone(),
            percent: sound - 10.0,
            params: vec![ParamScore {
                addr: CUTOFF.to_owned(),
                answer_addr: Some(CUTOFF.to_owned()),
                score: 0.5,
                verdict,
                direction: Direction::Higher,
                randomized: true,
            }],
            modules: Vec::new(),
        },
        Combined {
            scorer: scoring.combined.clone(),
            percent: sound - 5.0,
            sound_weight: scoring.sound_weight,
        },
    )
}

/// A Review item as the queue makes one (here on the test's seed challenge).
fn item() -> ReviewItem {
    ReviewItem {
        skill: Skill::parse("svf.cutoff").unwrap(),
        level: 1,
        due: true,
        lesson: "basics/cutoff".parse().unwrap(),
        step: enth_learn::pack::StepId::new("round-1").unwrap(),
        seed: 42,
        challenge: challenge(),
    }
}

fn drain_game(s: &mut Session) -> Vec<GameRequest> {
    s.drain_game_requests().collect()
}

fn game_job(requests: &[GameRequest], kind: &str) -> Job {
    requests
        .iter()
        .find_map(|r| match r {
            GameRequest::Run(job) if job.kind.name() == kind => Some(job.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {kind} job in {requests:?}"))
}

fn logs(requests: &[GameRequest]) -> Vec<RoundLogEntry> {
    requests
        .iter()
        .filter_map(|r| match r {
            GameRequest::Log(entry) => Some((**entry).clone()),
            _ => None,
        })
        .collect()
}

fn rings(s: &mut Session) -> Vec<RoundRings> {
    s.drain_learn_requests()
        .filter_map(|r| match r {
            LearnRequest::Rings(rings) => Some(rings),
            _ => None,
        })
        .collect()
}

/// A prepared round from `source`, ready to play.
fn ready(source: RoundSource) -> Session {
    let mut s = Session::new(custom_patch());
    s.game_tick(0, EditorView::Opened);
    s.game_start(source).unwrap();
    let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
    s.game_job_done(prepare.id, prepared()).unwrap();
    s
}

/// Ask for the next hint and let its job fail (the level counts either way; a hint in flight would block the next).
fn hint(s: &mut Session, level: u8) {
    assert_eq!(s.game_hint(), Ok(level));
    let job = game_job(&drain_game(s), "hint");
    s.game_job_done(
        job.id,
        JobResult::Failed {
            message: "test".to_owned(),
        },
    )
    .unwrap();
}

/// Submit the current patch and answer the score with `verdict` on the cutoff.
fn submit(s: &mut Session, sound: f64, verdict: Verdict) {
    s.game_submit().unwrap();
    let score = game_job(&drain_game(s), "score");
    s.game_job_done(
        score.id,
        JobResult::Scored(Box::new(report(sound, verdict))),
    )
    .unwrap();
}

#[test]
fn a_review_item_plays_as_a_review_round_that_stashes_and_restores() {
    let mut s = Session::new(custom_patch());
    s.game_tick(0, EditorView::Opened);
    s.review_start(item()).unwrap();
    let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
    s.game_job_done(prepare.id, prepared()).unwrap();
    let record = s.game_record().unwrap();
    assert_eq!(record.origin, RoundOrigin::Review);
    assert_eq!(
        record.stash.as_deref(),
        Some(custom_patch().to_json().as_str())
    );
    assert_eq!(s.round().unwrap().phase, Phase::Playing);
    assert!(s.save(None).instance.contains(r#""origin":"review""#));
    // No hint before the first submit; after it the coach opens.
    assert_eq!(s.game_hint(), Err(GameError::FirstTry));
    assert_eq!(GameError::FirstTry.code(), "first_try");
    submit(&mut s, 40.0, Verdict::Bad);
    hint(&mut s, 1);
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.patch().to_json(), custom_patch().to_json());
    let lines = logs(&drain_game(&mut s));
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].origin,
        RoundOrigin::Review,
        "gr1-report leaves it out"
    );
    let json = serde_json::to_string(&lines[0]).unwrap();
    assert!(json.contains(r#""origin":"review""#), "{json}");
}

#[test]
fn a_game_round_gives_hints_before_its_first_submit_as_before() {
    let mut s = ready(RoundSource::Challenge(Box::new(challenge())));
    hint(&mut s, 1);
}

#[test]
fn every_played_round_sends_its_first_submits_rings() {
    // A game round: first submit Bad on the cutoff, then a passing one. The first one is what counts.
    let mut s = ready(RoundSource::Challenge(Box::new(challenge())));
    submit(&mut s, 40.0, Verdict::Bad);
    submit(&mut s, 95.0, Verdict::Good);
    assert!(rings(&mut s).is_empty(), "nothing until the round ends");
    s.game_end(EndChoice::Restore).unwrap();
    let sent = rings(&mut s);
    assert_eq!(sent.len(), 1);
    assert!(!sent[0].assisted);
    let moves = sent[0].moves();
    assert_eq!(
        moves.get(&Skill::parse("svf.cutoff").unwrap()),
        Some(&(Verdict::Bad, true)),
        "the first submit's ring, on the target's module kind"
    );
    // A round with no submit sends nothing.
    let mut s = ready(RoundSource::Challenge(Box::new(challenge())));
    s.game_end(EndChoice::Restore).unwrap();
    assert!(rings(&mut s).is_empty(), "no submit, no ring");
}

#[test]
fn a_revealed_round_or_the_last_hint_is_assisted() {
    let mut s = ready(RoundSource::Challenge(Box::new(challenge())));
    submit(&mut s, 95.0, Verdict::Good);
    s.game_reveal().unwrap();
    s.game_end(EndChoice::Restore).unwrap();
    let sent = rings(&mut s);
    assert!(sent[0].assisted, "a reveal");
    let mut s = ready(RoundSource::Challenge(Box::new(challenge())));
    for level in 1..=3 {
        hint(&mut s, level);
    }
    submit(&mut s, 95.0, Verdict::Good);
    s.game_end(EndChoice::Keep).unwrap();
    assert!(
        !rings(&mut s)[0].assisted,
        "the first three levels name no value"
    );
    let mut s = ready(RoundSource::Challenge(Box::new(challenge())));
    for level in 1..=4 {
        hint(&mut s, level);
    }
    submit(&mut s, 95.0, Verdict::Good);
    s.game_end(EndChoice::Keep).unwrap();
    assert!(rings(&mut s)[0].assisted, "level 4 gives the move");
}

#[test]
fn a_host_load_that_ends_a_round_sends_no_rings() {
    let mut s = ready(RoundSource::Challenge(Box::new(challenge())));
    submit(&mut s, 40.0, Verdict::Bad);
    // The host loads a state without a round (another project, or REAPER's undo to before it).
    let empty = Session::new(custom_patch()).save(None);
    let _ = s.load(&empty);
    assert!(s.game_record().is_none());
    assert!(
        rings(&mut s).is_empty(),
        "the host may reopen the same round"
    );
}

#[test]
fn a_skills_box_follows_the_rings_a_round_sends() {
    let mut progress = Progress::new();
    let skill = Skill::parse("svf.cutoff").unwrap();
    for (verdict, day, level) in [
        (Verdict::Good, TODAY, 2),
        (Verdict::Good, TODAY + 2, 3),
        (Verdict::Near, TODAY + 6, 3),
        (Verdict::Bad, TODAY + 7, 2),
    ] {
        let mut s = ready(RoundSource::Challenge(Box::new(challenge())));
        submit(&mut s, 50.0, verdict);
        s.game_end(EndChoice::Restore).unwrap();
        for r in rings(&mut s) {
            progress.observe(&r, day); // what the shell does with each `Rings` request
        }
        let b = progress.review.get(&skill);
        assert_eq!((b.level, b.day), (level, day), "{verdict:?}");
    }
    assert_eq!(progress.review.get(&skill).rounds, 4);
}

#[test]
fn the_next_item_takes_over_a_review_round_but_waits_for_a_game_round() {
    let mut s = Session::new(custom_patch());
    s.game_tick(0, EditorView::Opened);
    s.review_start(item()).unwrap();
    let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
    s.game_job_done(prepare.id, prepared()).unwrap();
    s.review_start(item()).unwrap();
    let record = s.game_record().unwrap();
    assert_eq!(record.origin, RoundOrigin::Review);
    assert_eq!(
        record.stash.as_deref(),
        Some(custom_patch().to_json().as_str()),
        "the first item's stash carries over"
    );
    let mut s = ready(RoundSource::Challenge(Box::new(challenge())));
    assert_eq!(s.review_start(item()), Err(LearnError::RoundLive));
    assert_eq!(s.game_record().unwrap().origin, RoundOrigin::Game);
}

#[test]
fn a_review_item_waits_for_a_live_lesson_to_end() {
    let saved = Session::new(custom_patch()).save(None);
    let mut instance: serde_json::Value = serde_json::from_str(&saved.instance).unwrap();
    instance["learn"] = serde_json::json!({ "stash": custom_patch().to_json() });
    let mut s = Session::default();
    let _ = s.load(&SavedState {
        patch: saved.patch,
        instance: instance.to_string(),
    });
    assert_eq!(
        s.review_start(item()),
        Err(LearnError::Round(GameError::LessonLive))
    );
    assert!(s.game_record().is_none());
}
