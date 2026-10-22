//! No audio-thread contact (lane SE): while a test thread stands in for the host's audio thread and renders flat out
//! with every `process()` inside `assert_no_alloc`, the main thread runs whole ear runs and whole duels (a duel
//! opened, a scorecard and an answer imported, a Recreate round played to a pass, the check posted, their patch
//! auditioned and loaded, the undo). An ear run never reaches the engine at all; a duel reaches it only through the
//! rounds it sits beside and through "use their patch", which is a preset load: one plan, through the same path as a
//! load. The audio thread never waits on the session and never allocates. Debug builds only (`assert_no_alloc` drops
//! its checks in release); an allocation inside the guard aborts the test binary.
#![cfg(debug_assertions)]

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use enth_challenge::score::ParamScore;
use enth_challenge::{
    Combined, Direction, EngineVersion, PhraseId, ScoringSpec, SoundScore, SpecPreset,
    TargetFingerprint,
};
use enth_engine::{Event, EventKind, NoteKey};
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModuleKind, Op, Patch, Position, apply_all};
use enth_session::duel::{
    DuelEdit, DuelJobResult, DuelRequest, DuelVerified, FriendAnswer, FriendScorecard, Rescored,
};
use enth_session::ear::{
    EarAnswer, EarEdit, EarExample, EarPlan, EarPost, EarQuestion, EarRequest, EarSpec,
};
use enth_session::game::{
    Challenge, ChallengeCode, EditorView, EndChoice, GameEdit, GameRequest, JobKind, JobResult,
    Prepared, RandomSpec, RoundSource, ScoreReport, StructureScore, Verdict,
};
use enth_session::{Activation, Session};

use common::{addr, custom_patch, id, init_patch};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn set(a: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr(a),
        value,
    }
}

/// A start with a different structure from the player's patch (a second filter), so their patch loads as a
/// crossfading plan swap: the rarest path through the engine.
fn start_patch() -> Patch {
    let mut p = init_patch();
    let add = |id_: &str, index| Op::AddModule {
        at: Position {
            section: Section::VoiceFx,
            index,
        },
        module: ModuleInstance::new(id(id_), ModuleKind::Svf),
    };
    apply_all(&mut p, &[add("filter1", 0), add("filter2", 1)]).unwrap();
    p
}

fn target_patch() -> Patch {
    let mut p = start_patch();
    apply_all(&mut p, &[set("filter1.cutoff", 700.0)]).unwrap();
    p
}

fn structure() -> StructureScore {
    StructureScore {
        scorer: ScoringSpec::recreate(25).structure,
        percent: 80.0,
        params: vec![ParamScore {
            addr: "filter1.cutoff".to_owned(),
            answer_addr: Some("filter1.cutoff".to_owned()),
            score: 0.9,
            verdict: Verdict::Good,
            direction: Direction::Higher,
            randomized: true,
        }],
        modules: Vec::new(),
    }
}

/// Answer every game job in the outbox the way a shell's worker would, at once; every submit passes.
fn answer_jobs(s: &mut Session) {
    let requests: Vec<GameRequest> = s.drain_game_requests().collect();
    for request in requests {
        let GameRequest::Run(job) = request else {
            continue;
        };
        let result = match job.kind {
            JobKind::Prepare { spec } => JobResult::Prepared(Box::new(Prepared::new(
                Challenge::recreate(spec, PhraseId(1)),
                start_patch(),
                target_patch(),
                vec![addr("filter1.cutoff")],
                TargetFingerprint([0x5A; 32]),
                Some(-16.0),
            ))),
            JobKind::Score { challenge, .. } => JobResult::Scored(Box::new(ScoreReport::new(
                &challenge.scoring,
                SoundScore {
                    scorer: challenge.scoring.sound.clone(),
                    percent: 95.0,
                    distance: 0.5,
                    floor: 0.0,
                    scale: 0.1,
                    jnd: 5.0,
                    components: Vec::new(),
                },
                structure(),
                Combined {
                    scorer: challenge.scoring.combined.clone(),
                    percent: 90.0,
                    sound_weight: ScoringSpec::recreate(25).sound_weight,
                },
            ))),
            JobKind::Rings { .. } => JobResult::Rings(Box::new(structure())),
            JobKind::Loudness { .. } => JobResult::Loudness { lufs: Some(-20.0) },
            _ => JobResult::Failed {
                message: "not in this test".to_owned(),
            },
        };
        s.game_job_done(job.id, result).unwrap();
    }
    s.drain_requests().for_each(drop);
}

/// One ear run, every call: the questions posted, played, answered, replayed, moved past.
fn ear_run(s: &mut Session, seed: u64, now: u64) {
    let run = s
        .ear_start(EarPlan::new(EarSpec {
            game: 1,
            generator: 1,
            level: 3,
            seed,
            count: 3,
        }))
        .unwrap();
    let questions = (0..3u8)
        .map(|i| {
            EarQuestion::choice(
                format!("ear.interval.{i}"),
                ["m3", "M3", "P4", "P5"].map(str::to_owned).to_vec(),
                i,
            )
            .with_examples(vec![
                EarExample::open("A"),
                EarExample::replay("The answer"),
            ])
        })
        .collect();
    s.ear_post(run, EarPost::Ready { questions }).unwrap();
    for item in 0..3u32 {
        s.ear_play(0).unwrap();
        s.ear_answer(
            item,
            EarAnswer::Choice((item % 4) as u8),
            now + u64::from(item),
        )
        .unwrap();
        s.ear_play(1).unwrap();
        s.ear_stop();
        s.ear_next(now + u64::from(item) + 1).unwrap();
    }
    assert!(s.ear().is_some());
    let judged = s
        .drain_ear_requests()
        .filter(|r| matches!(r, EarRequest::Result(_)))
        .count();
    assert_eq!(judged, 3);
    s.ear_clear();
    s.drain_ear_requests().for_each(drop);
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "a test thread stands in for the host's audio thread"
)]
fn ear_runs_and_duels_never_stop_the_audio_thread_or_make_it_allocate() {
    let mut s = Session::new(custom_patch());
    let own = s.patch().to_json();
    let mut engine = s
        .activate(Activation {
            sample_rate: 48_000.0,
            offline: false,
        })
        .unwrap();
    let stop = AtomicBool::new(false);
    let blocks = AtomicU64::new(0);

    let engine = std::thread::scope(|scope| {
        let audio = scope.spawn(|| {
            let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
            let note = [Event {
                offset: 0,
                kind: EventKind::NoteOn {
                    key: NoteKey::new(60, 1),
                    velocity: 0.8,
                },
            }];
            assert_no_alloc(|| engine.process(&mut l, &mut r, &note));
            while !stop.load(Ordering::Relaxed) {
                assert_no_alloc(|| engine.process(&mut l, &mut r, &[]));
                blocks.fetch_add(1, Ordering::Relaxed);
            }
            engine
        });

        let mut now = 0u64;
        let mut last = 0;
        let mut step = |s: &mut Session, k: u32| {
            // The audio thread made progress since the last look: it never waits on the session.
            let mut spins = 0;
            while blocks.load(Ordering::Relaxed) == last && spins < 200_000 {
                std::thread::yield_now();
                spins += 1;
            }
            let seen = blocks.load(Ordering::Relaxed);
            assert!(seen > last, "the audio thread stopped at step {k}");
            last = seen;
            s.pump();
        };
        for round in 0..12u32 {
            now += 1_000;
            // An ear run: no plan, no event, nothing for the engine at all.
            ear_run(&mut s, u64::from(round), now);
            step(&mut s, round * 10);

            // A duel on the round's code: the card and the answer first (the answer sealed), then the round.
            let spec: RandomSpec = SpecPreset::by_id(2).unwrap().spec(1, u64::from(round));
            let code = ChallengeCode::short(spec).unwrap().to_string();
            s.duel_open(&code).unwrap();
            s.duel_import_scorecard(FriendScorecard::new(
                code.clone(),
                "CARD",
                EngineVersion::CURRENT,
                930,
                900,
                true,
            ))
            .unwrap();
            s.duel_import_answer(FriendAnswer::new(code.clone(), "ANSWER"))
                .unwrap();
            assert!(
                s.duel_verify().is_err(),
                "sealed until the player has played"
            );
            s.game_tick(now, EditorView::Opened);
            s.game_start(RoundSource::Settings(spec)).unwrap();
            answer_jobs(&mut s); // the start patch: a structural plan swap
            step(&mut s, round * 10 + 1);
            s.edit(vec![set("filter2.cutoff", 900.0)], None).unwrap();
            now += 400;
            s.game_tick(now, EditorView::Visible);
            s.game_submit().unwrap();
            answer_jobs(&mut s);
            step(&mut s, round * 10 + 2);
            let job = s.duel_verify().unwrap();
            let verify = s.drain_duel_requests().find_map(|r| match r {
                DuelRequest::Verify(v) => Some(v),
                _ => None,
            });
            assert!(verify.is_some_and(|v| v.job == job && v.own.is_some()));
            let own_score = Rescored::new(9500, 9000, true);
            s.duel_job_done(
                job,
                DuelJobResult::Verified(Box::new(
                    DuelVerified::new(Some(own_score), Rescored::new(9300, 9000, true))
                        .with_patch(target_patch())
                        .with_checks(Some(true), Some(true)),
                )),
            )
            .unwrap();
            s.duel_audition().unwrap();
            s.duel_stop_audition();
            s.drain_duel_requests().for_each(drop);
            s.game_end(EndChoice::Restore).unwrap(); // the player's patch: a structural plan swap
            answer_jobs(&mut s);
            step(&mut s, round * 10 + 3);
            let kept = s.duel_own().unwrap();
            assert!(kept.own.can_share_answer());
            s.duel_use_answer("Their take").unwrap(); // their patch: a preset load, one plan
            step(&mut s, round * 10 + 4);
            assert_eq!(s.undo(), Ok(true));
            step(&mut s, round * 10 + 5);
            s.duel_close();
            s.drain_duel_requests().for_each(drop);
            s.drain_requests().for_each(drop);
        }
        stop.store(true, Ordering::Relaxed);
        audio.join().expect("the audio thread did not panic")
    });

    let mut engine = engine;
    let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
    for _ in 0..32 {
        s.pump();
        engine.process(&mut l, &mut r, &[]);
    }
    assert_eq!(s.patch().to_json(), own);
    assert_eq!(s.status().plans_in_flight, 0);
    assert!(
        engine.stats().crossfades >= 12,
        "every start, restore and load of their patch swapped structure"
    );
    assert_eq!(engine.stats().dropped_events, 0);
}
