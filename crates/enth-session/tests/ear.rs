//! The ear state (lane SE, freeze point F1e), through the public API: a run asked for, its questions posted back, each
//! question asked, answered and judged, the key hidden until then, the replay, stale posts and answers refused, and
//! nothing saved. The shell is played by hand; every timestamp is a literal.

mod common;

use common::init_patch;
use enth_session::ear::{
    EarAnswer, EarEdit, EarError, EarExample, EarKey, EarPhase, EarPlan, EarPost, EarQuestion,
    EarRequest, EarRunId, EarSpec, MAX_SEED,
};
use enth_session::game::EditorView;
use enth_session::{Session, ShellRequest};
use proptest::prelude::*;

fn plan(count: u8) -> EarPlan {
    EarPlan::new(EarSpec {
        game: 1,
        generator: 1,
        level: 3,
        seed: 4242,
        count,
    })
}

/// Three label questions: the right option is 0, 1, 2; the second has a clip that gives the answer away.
fn questions(count: usize) -> Vec<EarQuestion> {
    (0..count)
        .map(|i| {
            let options = ["m3", "M3", "P4", "P5"].map(str::to_owned).to_vec();
            let q = EarQuestion::choice(format!("ear.interval.{i}"), options, (i % 3) as u8);
            q.with_examples(vec![
                EarExample::open("Example"),
                EarExample::replay("The answer"),
            ])
        })
        .collect()
}

fn ready(s: &mut Session, id: EarRunId, count: usize) {
    s.ear_post(
        id,
        EarPost::Ready {
            questions: questions(count),
        },
    )
    .unwrap();
}

fn started(s: &mut Session, count: u8) -> EarRunId {
    let id = s.ear_start(plan(count)).unwrap();
    let requests: Vec<EarRequest> = s.drain_ear_requests().collect();
    assert_eq!(requests.len(), 1, "{requests:?}");
    match &requests[0] {
        EarRequest::Prepare(p) => {
            assert_eq!(p.id, id);
            assert_eq!(p.plan, plan(count));
        }
        other => panic!("{other:?}"),
    }
    id
}

fn drain(s: &mut Session) -> Vec<EarRequest> {
    s.drain_ear_requests().collect()
}

#[test]
fn a_run_goes_from_question_to_answer_to_result() {
    let mut s = Session::new(init_patch());
    s.game_tick(1_000, EditorView::Opened);
    assert!(s.ear().is_none());
    let id = started(&mut s, 3);
    let view = s.ear().unwrap();
    assert_eq!(
        (view.id, view.phase, view.count, view.item),
        (id, EarPhase::Preparing, 3, None)
    );
    assert!(view.question.is_none());
    assert_eq!(
        s.ear_answer(0, EarAnswer::Choice(0), 1_500)
            .map_err(|e| e.code()),
        Err("ear_not_ready")
    );

    s.game_tick(1_200, EditorView::Visible);
    ready(&mut s, id, 3);
    let view = s.ear().unwrap();
    assert_eq!(
        (view.phase, view.item, view.answered),
        (EarPhase::Asking, Some(0), 0)
    );
    let q = view.question.unwrap();
    assert_eq!(q.options, ["m3", "M3", "P4", "P5"]);
    assert_eq!(q.skill, "ear.interval.0");
    assert_eq!(
        (q.chosen, q.key, q.correct),
        (None, None, None),
        "the key is hidden"
    );

    // A right answer, 800 ms after the question opened at the tick of the post.
    let r = s.ear_answer(0, EarAnswer::Choice(0), 2_000).unwrap();
    assert!(r.correct);
    assert_eq!(r.response_ms, 800);
    assert_eq!(r.key, EarKey::Choice(0));
    let view = s.ear().unwrap();
    assert_eq!(
        (view.phase, view.answered, view.correct),
        (EarPhase::Answered, 1, 1)
    );
    let q = view.question.unwrap();
    assert_eq!(q.chosen, Some(EarAnswer::Choice(0)));
    assert_eq!((q.key, q.correct), (Some(EarKey::Choice(0)), Some(true)));
    let out = drain(&mut s);
    assert_eq!(out.len(), 1);
    match &out[0] {
        EarRequest::Result(res) => {
            assert_eq!(
                (res.run, res.item, res.plan, res.correct),
                (id, 0, plan(3), true)
            );
            assert_eq!(
                (res.skill.as_str(), res.response_ms),
                ("ear.interval.0", 800)
            );
        }
        other => panic!("{other:?}"),
    }

    // The next question opens at the time given; a wrong answer.
    s.ear_next(2_500).unwrap();
    let view = s.ear().unwrap();
    assert_eq!((view.phase, view.item), (EarPhase::Asking, Some(1)));
    assert_eq!(view.question.unwrap().key, None);
    let r = s.ear_answer(1, EarAnswer::Choice(3), 2_900).unwrap();
    assert!(!r.correct);
    assert_eq!(r.response_ms, 400);
    s.ear_next(3_000).unwrap();
    s.ear_answer(2, EarAnswer::Choice(2), 3_100).unwrap();

    // The last question: next finishes the run.
    let view = s.ear().unwrap();
    assert_eq!(
        (view.phase, view.answered, view.correct),
        (EarPhase::Answered, 3, 2)
    );
    s.ear_next(3_200).unwrap();
    let view = s.ear().unwrap();
    assert_eq!(
        (view.phase, view.item, view.answered, view.correct),
        (EarPhase::Done, None, 3, 2)
    );
    assert!(view.question.is_none());
    assert_eq!(view.results.len(), 3);
    assert_eq!(
        view.results.iter().map(|r| r.correct).collect::<Vec<_>>(),
        [true, false, true]
    );
    assert_eq!(s.ear_next(3_300).map_err(|e| e.code()), Err("ear_finished"));
    assert_eq!(
        s.ear_answer(2, EarAnswer::Choice(2), 3_300)
            .map_err(|e| e.code()),
        Err("ear_finished")
    );
    // One result per judged answer, in order, and nothing else went out.
    let results: Vec<_> = drain(&mut s)
        .into_iter()
        .map(|r| match r {
            EarRequest::Result(res) => (res.item, res.correct),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(results, [(1, false), (2, true)]);
}

#[test]
fn the_first_answer_stands_and_only_the_open_question_can_be_answered() {
    let mut s = Session::new(init_patch());
    let id = started(&mut s, 3);
    ready(&mut s, id, 3);
    // A question the run has not reached, and one before it.
    assert_eq!(
        s.ear_answer(1, EarAnswer::Choice(1), 100)
            .map_err(|e| e.code()),
        Err("not_current")
    );
    assert_eq!(s.ear_next(100).map_err(|e| e.code()), Err("not_answered"));
    // An option that does not exist is no answer, and the question stays open.
    assert_eq!(
        s.ear_answer(0, EarAnswer::Choice(4), 100)
            .map_err(|e| e.code()),
        Err("bad_answer")
    );
    assert_eq!(s.ear().unwrap().phase, EarPhase::Asking);
    s.ear_answer(0, EarAnswer::Choice(3), 200).unwrap();
    assert_eq!(
        s.ear_answer(0, EarAnswer::Choice(0), 300)
            .map_err(|e| e.code()),
        Err("already_answered"),
        "a second answer cannot turn a miss into a hit"
    );
    let view = s.ear().unwrap();
    assert_eq!((view.answered, view.correct), (1, 0));
    s.ear_next(400).unwrap();
    assert_eq!(
        s.ear_answer(0, EarAnswer::Choice(0), 500)
            .map_err(|e| e.code()),
        Err("not_current"),
        "the run moved on: a late tap for the old question is refused"
    );
    let results = drain(&mut s)
        .into_iter()
        .filter(|r| matches!(r, EarRequest::Result(_)))
        .count();
    assert_eq!(results, 1, "refused answers hand the shell nothing");
}

#[test]
fn stale_posts_are_refused_and_change_nothing() {
    let mut s = Session::new(init_patch());
    // A post with no run.
    assert_eq!(
        s.ear_post(
            EarRunId(1),
            EarPost::Ready {
                questions: questions(2)
            }
        )
        .map_err(|e| e.code()),
        Err("stale_post")
    );
    let old = started(&mut s, 2);
    // A new run replaces the old one: its pending job is cancelled, and its post is stale.
    let new = s.ear_start(plan(2)).unwrap();
    assert_ne!(old, new);
    let out = drain(&mut s);
    assert!(out.contains(&EarRequest::Cancel(old)), "{out:?}");
    assert!(
        matches!(out.last(), Some(EarRequest::Prepare(p)) if p.id == new),
        "{out:?}"
    );
    assert_eq!(
        s.ear_post(
            old,
            EarPost::Ready {
                questions: questions(2)
            }
        )
        .map_err(|e| e.code()),
        Err("stale_post")
    );
    assert_eq!(s.ear().unwrap().id, new);
    assert_eq!(s.ear().unwrap().phase, EarPhase::Preparing);

    // The right run: the first Ready counts, a second is stale (and changes no question).
    ready(&mut s, new, 2);
    s.ear_answer(0, EarAnswer::Choice(0), 10).unwrap();
    assert_eq!(
        s.ear_post(
            new,
            EarPost::Ready {
                questions: questions(2)
            }
        )
        .map_err(|e| e.code()),
        Err("stale_post")
    );
    assert_eq!(s.ear().unwrap().answered, 1);
    // A failure after the questions arrived is stale too.
    assert_eq!(
        s.ear_post(
            new,
            EarPost::Failed {
                code: "x".to_owned(),
                message: String::new(),
                hint: String::new()
            }
        )
        .map_err(|e| e.code()),
        Err("stale_post")
    );
    assert_eq!(s.ear().unwrap().phase, EarPhase::Answered);

    // A cleared run: its job is cancelled when it was still preparing, and its posts are stale.
    let _ = drain(&mut s);
    let third = started(&mut s, 1);
    s.ear_clear();
    assert_eq!(drain(&mut s), [EarRequest::Cancel(third)]);
    assert!(s.ear().is_none());
    assert_eq!(
        s.ear_post(
            third,
            EarPost::Ready {
                questions: questions(1)
            }
        )
        .map_err(|e| e.code()),
        Err("stale_post")
    );
    assert_eq!(s.ear_next(1).map_err(|e| e.code()), Err("no_ear_run"));
    assert_eq!(
        s.ear_answer(0, EarAnswer::Choice(0), 1)
            .map_err(|e| e.code()),
        Err("no_ear_run")
    );
}

#[test]
fn a_failed_job_and_questions_that_do_not_fit_fail_the_run() {
    let mut s = Session::new(init_patch());
    let id = started(&mut s, 2);
    s.ear_post(
        id,
        EarPost::Failed {
            code: "render_failed".to_owned(),
            message: "no stimulus".to_owned(),
            hint: "try again".to_owned(),
        },
    )
    .unwrap();
    let view = s.ear().unwrap();
    assert_eq!(view.phase, EarPhase::Failed);
    assert_eq!(view.error.unwrap().code, "render_failed");
    assert_eq!(
        s.ear_answer(0, EarAnswer::Choice(0), 1)
            .map_err(|e| e.code()),
        Err("ear_not_ready")
    );

    // The wrong count, a key that is no option, one option: each fails the run and the post is an error.
    for bad in [
        questions(1),
        vec![EarQuestion::choice("s", vec!["a".to_owned(), "b".to_owned()], 2); 2],
        vec![EarQuestion::choice("s", vec!["a".to_owned()], 0); 2],
    ] {
        let id = started(&mut s, 2);
        assert_eq!(
            s.ear_post(id, EarPost::Ready { questions: bad })
                .map_err(|e| e.code()),
            Err("wrong_result")
        );
        let view = s.ear().unwrap();
        assert_eq!(view.phase, EarPhase::Failed);
        assert_eq!(view.error.unwrap().code, "wrong_result");
        // A post after the failure is stale.
        assert_eq!(
            s.ear_post(
                id,
                EarPost::Ready {
                    questions: questions(2)
                }
            )
            .map_err(|e| e.code()),
            Err("stale_post")
        );
    }
}

#[test]
fn plans_out_of_range_start_nothing() {
    let mut s = Session::new(init_patch());
    for bad in [
        EarPlan::new(EarSpec {
            game: 1,
            generator: 1,
            level: 1,
            seed: MAX_SEED + 1,
            count: 5,
        }),
        EarPlan::new(EarSpec {
            game: 1,
            generator: 1,
            level: 1,
            seed: 7,
            count: 0,
        }),
        EarPlan::new(EarSpec {
            game: 1,
            generator: 1,
            level: 1,
            seed: 7,
            count: 65,
        }),
    ] {
        assert_eq!(s.ear_start(bad).map_err(|e| e.code()), Err("bad_ear_plan"));
        assert_ne!(EarError::BadPlan("x").hint(), "");
    }
    assert!(s.ear().is_none());
    let sent = drain(&mut s);
    assert!(sent.is_empty(), "{sent:?}");
}

#[test]
fn the_replay_opens_with_the_answer() {
    let mut s = Session::new(init_patch());
    let id = started(&mut s, 2);
    assert_eq!(s.ear_play(0).map_err(|e| e.code()), Err("ear_not_ready"));
    ready(&mut s, id, 2);
    // The open clip plays now; the clip that gives the answer away waits.
    s.ear_play(0).unwrap();
    assert_eq!(s.ear_play(1).map_err(|e| e.code()), Err("example_hidden"));
    assert_eq!(s.ear_play(2).map_err(|e| e.code()), Err("no_example"));
    assert_eq!(
        drain(&mut s),
        [EarRequest::Play {
            id,
            item: 0,
            example: 0
        }]
    );
    s.ear_answer(0, EarAnswer::Choice(0), 50).unwrap();
    let _ = drain(&mut s);
    s.ear_play(1).unwrap();
    s.ear_stop();
    s.ear_stop();
    assert_eq!(
        drain(&mut s),
        [
            EarRequest::Play {
                id,
                item: 0,
                example: 1
            },
            EarRequest::Stop
        ],
        "stop goes out once"
    );
    // Moving on stops what plays.
    s.ear_play(0).unwrap();
    s.ear_next(60).unwrap();
    let out = drain(&mut s);
    assert_eq!(out.last(), Some(&EarRequest::Stop), "{out:?}");
    // The new question hides its answer clip again.
    assert_eq!(s.ear_play(1).map_err(|e| e.code()), Err("example_hidden"));
}

#[test]
fn time_never_runs_backwards() {
    let mut s = Session::new(init_patch());
    s.game_tick(5_000, EditorView::Opened);
    let id = started(&mut s, 2);
    ready(&mut s, id, 2);
    // An answer stamped before the question opened counts as instant, not as a huge number.
    let r = s.ear_answer(0, EarAnswer::Choice(0), 100).unwrap();
    assert_eq!(r.response_ms, 0);
    s.ear_next(4_000).unwrap();
    let r = s.ear_answer(1, EarAnswer::Choice(0), 5_250).unwrap();
    assert_eq!(
        r.response_ms, 250,
        "the question opened at the latest time seen"
    );
}

#[test]
fn an_ear_run_is_not_saved_and_a_host_load_leaves_it_alone() {
    let mut s = Session::new(init_patch());
    let before = s.save_instance();
    let id = started(&mut s, 2);
    ready(&mut s, id, 2);
    s.ear_answer(0, EarAnswer::Choice(0), 10).unwrap();
    assert_eq!(s.save_instance(), before, "the enth field keeps its bytes");

    let saved = s.save(None);
    let loaded = s.load(&saved);
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    assert_eq!(
        s.ear().unwrap().id,
        id,
        "a host state load leaves the run alone"
    );
    assert_eq!(s.ear().unwrap().answered, 1);

    // A fresh instance restores no run.
    let mut other = Session::new(init_patch());
    let loaded = other.load(&saved);
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    assert!(other.ear().is_none());
}

#[test]
fn an_ear_run_never_touches_the_engine_or_the_host() {
    let mut s = Session::new(init_patch());
    let _ = s.drain_requests().count();
    for round in 0..4u8 {
        let id = s.ear_start(plan(3)).unwrap();
        ready(&mut s, id, 3);
        for item in 0..3u32 {
            s.ear_play(0).unwrap();
            s.ear_answer(item, EarAnswer::Choice(round), u64::from(item))
                .unwrap();
            s.ear_next(u64::from(item) + 1).unwrap();
        }
    }
    let shell: Vec<ShellRequest> = s.drain_requests().collect();
    assert!(
        shell.is_empty(),
        "no host write, no rename, no dirty mark: {shell:?}"
    );
    assert_eq!(s.status().epoch, 0, "no plan");
}

/// A state machine of random intents and posts in any order: no panic, one result per judged question, the counts
/// agree with the results, and nothing is judged twice.
#[derive(Clone, Debug)]
enum Step {
    Start(u8),
    Ready(u8, bool),
    Failed,
    Answer(u32, u8),
    Next,
    Play(usize),
    Stop,
    Clear,
    StalePost,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        2 => (0u8..70).prop_map(Step::Start),
        2 => (0u8..5, any::<bool>()).prop_map(|(n, ok)| Step::Ready(n, ok)),
        1 => Just(Step::Failed),
        6 => (0u32..5, 0u8..6).prop_map(|(i, a)| Step::Answer(i, a)),
        4 => Just(Step::Next),
        2 => (0usize..4).prop_map(Step::Play),
        1 => Just(Step::Stop),
        1 => Just(Step::Clear),
        1 => Just(Step::StalePost),
    ]
}

proptest! {
    #[test]
    fn random_intents_never_break_the_run(steps in proptest::collection::vec(step(), 1..80)) {
        let mut s = Session::new(init_patch());
        let mut now = 0u64;
        let mut judged = 0usize;
        let mut results = 0usize;
        let mut run: Option<(EarRunId, usize)> = None;
        for step in steps {
            now += 7;
            match step {
                Step::Start(n) => {
                    if let Ok(id) = s.ear_start(plan(n)) {
                        run = Some((id, usize::from(n)));
                    }
                }
                Step::Ready(n, ok) => {
                    if let Some((id, count)) = run {
                        let n = if ok { count } else { usize::from(n) };
                        let _ = s.ear_post(id, EarPost::Ready { questions: questions(n) });
                    }
                }
                Step::Failed => {
                    if let Some((id, _)) = run {
                        let _ = s.ear_post(id, EarPost::Failed {
                            code: "f".to_owned(), message: String::new(), hint: String::new(),
                        });
                    }
                }
                Step::Answer(i, a) => {
                    if s.ear_answer(i, EarAnswer::Choice(a), now).is_ok() {
                        judged += 1;
                    }
                }
                Step::Next => { let _ = s.ear_next(now); }
                Step::Play(e) => { let _ = s.ear_play(e); }
                Step::Stop => s.ear_stop(),
                Step::Clear => { s.ear_clear(); run = None; }
                Step::StalePost => {
                    let _ = s.ear_post(EarRunId(u64::MAX), EarPost::Ready { questions: questions(1) });
                }
            }
            for request in s.drain_ear_requests() {
                if let EarRequest::Result(_) = request {
                    results += 1;
                }
            }
            if let Some(view) = s.ear() {
                prop_assert_eq!(view.results.len() as u32, view.answered);
                prop_assert!(view.correct <= view.answered);
                prop_assert!(view.answered <= view.count);
                match view.phase {
                    EarPhase::Asking | EarPhase::Answered => {
                        prop_assert!(view.question.is_some() && view.item.is_some());
                        // The key is shown exactly when the question is answered.
                        let q = view.question.unwrap();
                        prop_assert_eq!(q.key.is_some(), view.phase == EarPhase::Answered);
                    }
                    _ => prop_assert!(view.question.is_none()),
                }
            }
        }
        // Every answer the session judged handed the shell exactly one result, or its run was replaced first and the
        // result was drained before (results are drained after every step, so none is lost).
        prop_assert_eq!(judged, results);
    }
}
