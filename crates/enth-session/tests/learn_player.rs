//! The session's Learn API driving lane LM's real player (`enth_learn::Player`, F1L swap item 17): a pack built in
//! memory and verified by `Pack::from_files`, its lesson played through `Session` from start to end. A read step, a
//! Do step with a lock and a Show, a save and a reload that resumes the player at its step, and a Recreate step on a
//! Challenge v2 draw whose locks and editable list reach the session (F1l review M4), prepared and scored by the
//! real `enth_challenge` and logged with its lesson (should-fix 4). `tests/learn.rs` covers the session's rules with
//! a scripted player; this file checks that the two real halves fit. No wall clock and no random seed: every
//! timestamp is a literal.

mod common;

use std::collections::BTreeMap;

use enth_challenge::model::{Start, Target};
use enth_challenge::v2::{Constraints, Draw, DrawAt, DrawItem, DrawValue, DrawValues};
use enth_challenge::{Challenge, Outcome, PhraseId, ScorerId, ScoringSpec};
use enth_learn::pack::MANIFEST_PATH;
use enth_learn::{LessonBundle, Manifest, Pack, Player, StepPhase};
use enth_patch::model::{ModuleInstance, Route, Section};
use enth_patch::{ModSource, ModuleKind, Op, Patch, Position, apply_all};
use enth_session::game::{
    EditorView, EndChoice, GameEdit, GameRequest, Job, JobResult, Phase, Prepared, RoundLogEntry,
    RoundOrigin,
};
use enth_session::learn::{
    LearnEdit, LearnJob, LearnRequest, LearnResult, LessonPlayer, PlayerEffect, Selector,
};
use enth_session::{Session, SessionError};

use common::{addr, custom_patch, id, macro_index};

const CUTOFF: &str = "filter1.cutoff";

fn set(a: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr(a),
        value,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The pack, in memory
// ---------------------------------------------------------------------------------------------------------------

/// The Do step's start: the fixture patch with a second envelope and macro 1 routed into the resonance, the cutoff
/// at 2 kHz.
fn start_patch() -> Patch {
    let mut p = custom_patch();
    apply_all(
        &mut p,
        &[
            Op::AddModule {
                at: Position {
                    section: Section::Modulators,
                    index: 2,
                },
                module: ModuleInstance::new(id("env2"), ModuleKind::Adsr),
            },
            Op::InsertRoute {
                index: 1,
                route: Route {
                    src: ModSource::Macro(macro_index(1)),
                    dst: addr("filter1.resonance"),
                    depth: 30.0,
                },
            },
            set(CUTOFF, 2000.0),
        ],
    )
    .unwrap();
    p
}

/// The Do step's witness: the start with the cutoff at 400 Hz.
fn witness_patch() -> Patch {
    let mut p = start_patch();
    apply_all(&mut p, &[set(CUTOFF, 400.0)]).unwrap();
    p
}

/// The Recreate step's round: a draw of the starter's cutoff, the cutoff the step's control, the oscillator and the
/// resonance locked (Challenge v2).
fn draw_challenge() -> Challenge {
    let mut c = Challenge::new(
        Target::Draw(Draw {
            seed: 7,
            items: vec![DrawItem {
                at: DrawAt::Param(addr(CUTOFF)),
                values: DrawValues::Set(vec![
                    DrawValue::Number(300.0),
                    DrawValue::Number(1200.0),
                    DrawValue::Number(4000.0),
                ]),
            }],
        }),
        Start::Template { template: 0 },
        PhraseId(1),
        ScoringSpec::with_scorers(
            0,
            ScorerId::SOUND_V1,
            ScorerId::STRUCTURE_V1,
            ScorerId::COMBINED_V1,
            ScorerId::TIME_V1,
        ),
    );
    c.constraints = Constraints {
        editable: vec![CUTOFF.parse().unwrap()],
        locked: vec![
            "osc1.*".parse().unwrap(),
            "filter1.resonance".parse().unwrap(),
        ],
    };
    c
}

fn lesson_json() -> String {
    format!(
        r#"{{"schema":"enth.lesson/1","id":"cutoff","title":"The cutoff","tier":"basic","objective":"Turn the cutoff down by ear.","teaches":["svf.cutoff"],"steps":[
        {{"id":"name","kind":"read","text":"text.md"}},
        {{"id":"try","kind":"do","text":"text.md","view":{{"start":"patches/start.patch.json","editable":["filter1.cutoff"],"locked":["filter1.resonance"]}},
          "check":{{"kind":"patch","predicates":[{{"kind":"value","knob":"filter1.cutoff","cmp":"<","value":"1kHz"}}]}},
          "witness":"patches/witness.patch.json"}},
        {{"id":"round","kind":"recreate","text":"text.md","challenge":{}}}]}}"#,
        draw_challenge().to_json()
    )
}

/// The pack's files, sealed: `pack.json`, credits, the lesson and its text, the two patches.
fn pack_files() -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert("CREDITS.md".to_owned(), b"# Credits\n".to_vec());
    files.insert(
        "lessons/cutoff/lesson.json".to_owned(),
        lesson_json().into_bytes(),
    );
    files.insert("lessons/cutoff/text.md".to_owned(), b"# A step\n".to_vec());
    files.insert(
        "patches/start.patch.json".to_owned(),
        start_patch().to_json().into_bytes(),
    );
    files.insert(
        "patches/witness.patch.json".to_owned(),
        witness_patch().to_json().into_bytes(),
    );
    let manifest = Manifest::from_json(
        r#"{"schema":"enth.pack/1","id":"basics","version":"1.0.0","title":"Basics","authors":["enth contributors"],
        "license":{"text":"CC-BY-4.0","patches":"CC0-1.0"},
        "requires":{"lesson":1,"challenge":2,"patch":1,"scorers":["combined/1","sound/1","structure/1","time/1"]},
        "lessons":["cutoff"]}"#,
    )
    .unwrap();
    let sealed = manifest.seal(&files).unwrap();
    files.insert(MANIFEST_PATH.to_owned(), sealed.to_json().into_bytes());
    files
}

fn bundle() -> LessonBundle {
    Pack::from_files(&pack_files())
        .unwrap()
        .bundle(&"cutoff".parse().unwrap())
        .unwrap()
}

// ---------------------------------------------------------------------------------------------------------------
// The shell, by hand
// ---------------------------------------------------------------------------------------------------------------

fn learn_jobs(s: &mut Session) -> Vec<LearnJob> {
    s.drain_learn_requests()
        .filter_map(|r| match r {
            LearnRequest::Run(job) => Some(job),
            _ => None,
        })
        .collect()
}

fn game_requests(s: &mut Session) -> Vec<GameRequest> {
    s.drain_game_requests().collect()
}

fn game_job(requests: &[GameRequest], kind: &str) -> Job {
    let found: Vec<&Job> = requests
        .iter()
        .filter_map(|r| match r {
            GameRequest::Run(job) if job.kind.name() == kind => Some(job),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "one {kind} job in {requests:#?}");
    found[0].clone()
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

fn cutoff(s: &Session) -> f32 {
    s.patch().value(&addr(CUTOFF)).unwrap()
}

fn selectors(texts: &[&str]) -> Vec<Selector> {
    texts.iter().map(|t| t.parse().unwrap()).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// The lesson, end to end
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_real_player_plays_a_lesson_through_the_session() {
    let mut s = Session::new(custom_patch());
    let own = custom_patch().to_json();
    s.game_tick(1_000, EditorView::Opened);
    let player = Player::new(bundle());
    assert_eq!(LessonPlayer::step_count(&player), 3);
    s.learn_start(Box::new(player)).unwrap();

    // Step 0, a read step: no start, no lock; Next passes it and loads step 1's start (one plan).
    let v = s.lesson().unwrap();
    assert_eq!((v.steps, v.phase), (3, StepPhase::Working));
    assert_eq!(s.locks(), &[] as &[Selector]);
    s.learn_next().unwrap();
    let records: Vec<_> = s
        .drain_learn_requests()
        .filter_map(|r| match r {
            LearnRequest::Record { record, .. } => Some(record),
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(
        (records[0].step.as_str(), records[0].progress.clean),
        ("name", true)
    );

    // Step 1, a Do step: its start plays, its view's locks are the session's rule (the resonance, and macro 1,
    // which is routed into it: the shared table counts the route's destination).
    assert_eq!(cutoff(&s), 2000.0);
    assert_eq!(s.locks(), selectors(&["filter1.resonance"]).as_slice());
    assert_eq!(
        s.lesson().unwrap().view.editable,
        selectors(&["filter1.cutoff"])
    );
    for refused in [
        set("filter1.resonance", 60.0),
        Op::SetMacro {
            index: macro_index(1),
            value: 0.9,
        },
    ] {
        assert!(
            matches!(
                s.edit(vec![refused.clone()], None),
                Err(SessionError::Locked { .. })
            ),
            "{refused:?}"
        );
    }
    // Show applies the witness's cutoff through the edit path; the patch check passes at once, assisted.
    s.learn_show().unwrap();
    assert_eq!(cutoff(&s), 400.0);
    assert_eq!(s.lesson().unwrap().phase, StepPhase::Passed);
    let records: Vec<_> = s
        .drain_learn_requests()
        .filter_map(|r| match r {
            LearnRequest::Record { record, .. } => Some(record),
            _ => None,
        })
        .collect();
    assert_eq!(
        (records[0].step.as_str(), records[0].progress.clean),
        ("try", false)
    );

    // A save mid-lesson, and a reload: the shell reopens the lesson with `Player::resume` at the saved step, which
    // sends the view and no plan; the saved patch stays, and so do the locks.
    let saved = s.save(None);
    let mut t = Session::default();
    assert_eq!(t.load(&saved).errors, []);
    let open = learn_jobs(&mut t).into_iter().next().unwrap();
    let enth_session::learn::LearnJobKind::Open { lesson } = &open.kind else {
        panic!("{open:?}")
    };
    assert_eq!((lesson.step, lesson.lesson.as_str()), (1, "cutoff"));
    let (player, effects) = Player::resume(bundle(), lesson.step).unwrap();
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, PlayerEffect::LoadPatch(_) | PlayerEffect::StartRound(_)))
    );
    t.learn_job_done(
        open.id,
        LearnResult::Opened {
            player: Box::new(player),
            effects,
        },
    )
    .unwrap();
    assert_eq!(cutoff(&t), 400.0, "the saved patch stays");
    assert_eq!(t.locks(), selectors(&["filter1.resonance"]).as_slice());
    assert!(t.edit(vec![set("filter1.resonance", 60.0)], None).is_err());
    let mut s = t;
    s.game_tick(5_000, EditorView::Opened);

    // Step 2, a Recreate step on a Challenge v2 draw: its round starts from the challenge, and the challenge's
    // constraints are the step's (M4): the session enforces `locked`, the view carries `editable`.
    s.learn_next().unwrap();
    assert!(s.lesson().unwrap().round);
    assert_eq!(
        s.locks(),
        selectors(&["osc1.*", "filter1.resonance"]).as_slice()
    );
    assert_eq!(
        s.lesson().unwrap().view.editable,
        selectors(&["filter1.cutoff"])
    );
    let requests = game_requests(&mut s);
    let prepare = game_job(&requests, "prepare_challenge");
    let enth_session::game::JobKind::PrepareChallenge { challenge } = &prepare.kind else {
        panic!("{prepare:?}")
    };
    assert_eq!(**challenge, draw_challenge());
    let round = enth_challenge::prepare_challenge(Challenge::clone(challenge)).unwrap();
    s.game_job_done(
        prepare.id,
        JobResult::Prepared(Box::new(Prepared::from_round(&round, Some(-14.0)))),
    )
    .unwrap();
    assert_eq!(s.patch().to_json(), round.start().to_json());
    assert!(matches!(
        s.edit(vec![set("osc1.semi", 7.0)], None),
        Err(SessionError::Locked { .. })
    ));
    // A submit at the drawn target passes the round, and the step with it.
    let target = round.target().value(&addr(CUTOFF)).unwrap();
    s.edit(vec![set(CUTOFF, target)], None).unwrap();
    s.game_tick(9_000, EditorView::Visible);
    s.game_submit().unwrap();
    let score = game_job(&game_requests(&mut s), "score");
    let report = enth_challenge::score_round(&round, round.target()).unwrap();
    s.game_job_done(score.id, JobResult::Scored(Box::new(report)))
        .unwrap();
    assert_eq!(s.round().unwrap().phase, Phase::Passed);
    assert_eq!(s.lesson().unwrap().phase, StepPhase::Passed);

    // Next past the last step finishes the lesson; the passed round stays until the lesson ends. Ending it (Restore)
    // closes the round: its line names the lesson's step, and with no code (a draw has none) the line says its
    // outcome itself (should-fix 4). Your own patch comes back.
    s.learn_next().unwrap();
    assert!(s.lesson().unwrap().finished);
    assert_eq!(logs(&game_requests(&mut s)), []);
    s.learn_end(EndChoice::Restore).unwrap();
    assert_eq!(s.patch().to_json(), own);
    assert_eq!(s.lesson(), None);
    let lines = logs(&game_requests(&mut s));
    assert_eq!(lines.len(), 1);
    let line = &lines[0];
    assert_eq!(line.origin, RoundOrigin::Lesson);
    assert_eq!(line.result, None, "a draw round has no code");
    assert_eq!(line.outcome, Some(Outcome::Passed));
    let lesson = line.lesson.as_ref().unwrap();
    assert_eq!((lesson.lesson.as_str(), lesson.step), ("cutoff", 2));
    let json = serde_json::to_string(line).unwrap();
    assert_eq!(serde_json::from_str::<RoundLogEntry>(&json).unwrap(), *line);
}
