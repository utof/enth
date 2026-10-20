//! No audio-thread contact (lane SG): while a test thread stands in for the host's audio thread and renders flat out
//! with every `process()` inside `assert_no_alloc`, the main thread plays whole Recreate rounds (the start patch,
//! edits, submits, A/B, a reveal, the restore). The audio thread never waits on the session and never allocates:
//! a round reaches it only as plans, through the same path as a load. Debug builds only (`assert_no_alloc` drops its
//! checks in release); an allocation inside the guard aborts the test binary.
#![cfg(debug_assertions)]

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use enth_challenge::score::ParamScore;
use enth_challenge::{Combined, PhraseId, ScorerId, ScoringSpec, SoundScore, SpecPreset};
use enth_engine::{Event, EventKind, NoteKey};
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModuleKind, Op, Patch, Position, apply_all};
use enth_session::game::{
    Challenge, EditorView, EndChoice, GameEdit, GameRequest, JobKind, JobResult, Listen,
    RandomSpec, RoundSource, ScoreReport, StructureScore, Verdict,
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

/// A start with a different structure from the player's patch (a second filter), so every round start and restore
/// is a crossfading plan swap: the rarest path a round takes through the engine.
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
        scorer: ScorerId::STRUCTURE_V1,
        percent: 30.0,
        params: vec![ParamScore {
            addr: "filter1.cutoff".to_owned(),
            score: 0.5,
            verdict: Verdict::Near,
        }],
        modules: Vec::new(),
    }
}

/// Answer every job in the outbox the way a shell's worker would, at once.
fn answer_jobs(s: &mut Session) {
    let requests: Vec<GameRequest> = s.drain_game_requests().collect();
    for request in requests {
        let GameRequest::Run(job) = request else {
            continue;
        };
        let result = match job.kind {
            JobKind::Prepare { spec } => JobResult::Prepared {
                challenge: Box::new(Challenge::recreate(spec, PhraseId(1))),
                start: Box::new(start_patch()),
                target: Box::new(target_patch()),
                target_lufs: Some(-16.0),
            },
            JobKind::Score { challenge, .. } => JobResult::Scored(Box::new(ScoreReport::new(
                &challenge.scoring,
                SoundScore {
                    scorer: ScorerId::SOUND_V1,
                    percent: 40.0,
                    distance: 0.5,
                    floor: 0.0,
                    scale: 0.1,
                    jnd: 5.0,
                    components: Vec::new(),
                },
                structure(),
                Combined {
                    scorer: ScorerId::COMBINED_V1,
                    percent: 35.0,
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

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "a test thread stands in for the host's audio thread"
)]
fn rounds_never_stop_the_audio_thread_or_make_it_allocate() {
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
            s.game_tick(now, EditorView::Opened);
            let spec: RandomSpec = SpecPreset::by_id(2).unwrap().spec(1, u64::from(round));
            s.game_start(RoundSource::Settings(spec)).unwrap();
            answer_jobs(&mut s); // the start patch: a structural plan swap
            step(&mut s, round * 10);
            s.game_listen(Listen::Target).unwrap();
            for k in 0..6 {
                s.edit(vec![set("filter2.cutoff", 400.0 + 100.0 * k as f32)], None)
                    .unwrap();
                now += 400;
                s.game_tick(now, EditorView::Visible);
                answer_jobs(&mut s);
                step(&mut s, round * 10 + 1);
            }
            s.game_listen(Listen::Mine).unwrap();
            s.game_submit().unwrap();
            answer_jobs(&mut s);
            step(&mut s, round * 10 + 2);
            if round % 2 == 0 {
                s.game_reveal().unwrap(); // the target: a value plan
                answer_jobs(&mut s);
                step(&mut s, round * 10 + 3);
            }
            s.game_end(EndChoice::Restore).unwrap(); // the player's patch: a structural plan swap
            answer_jobs(&mut s);
            step(&mut s, round * 10 + 4);
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
        "every start and restore swapped structure"
    );
    assert_eq!(engine.stats().dropped_events, 0);
}
