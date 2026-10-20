//! T5 of the F1r review: a property test that drives the round as a state machine (lane F1R). Random sequences of the
//! editor's intents, edits and undo, ticks, saves and host loads, and job results handed back in any order (right,
//! failed, of the wrong kind, stale, or a report the challenge could not have produced). After every step it checks
//! that nothing panicked, that every round-log line's `RoundResult` reads back as a valid `enth.round-result/1`, that
//! `time_to_pass_ms ≤ active_ms`, that no submit is accepted after a reveal, that at most one job of each kind is in
//! flight, and that the outbox drains. Fixed seed and 256 cases in the gate (`PROPTEST_RNG_SEED`), as every property
//! test.

mod common;

use std::collections::BTreeMap;

use enth_challenge::score::ParamScore;
use enth_challenge::{
    Combined, Direction, PhraseId, ScoringSpec, SoundScore, SpecPreset, TargetFingerprint,
};
use enth_patch::canon::exact_hash;
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModuleKind, Op, Patch, Position, apply_all};
use enth_session::game::{
    Agreement, Challenge, ChallengeCode, CoachHint, EditorView, EndChoice, GameEdit, GameError,
    GameRequest, Job, JobId, JobKind, JobResult, Listen, Phase, Prepared, RandomSpec, RoundResult,
    RoundSource, ScoreReport, StructureScore, Verdict,
};
use enth_session::{SavedState, Session};
use proptest::prelude::*;

use common::{addr, custom_patch, id, init_patch};

const CUTOFF: &str = "filter1.cutoff";

fn start_patch() -> Patch {
    let mut p = init_patch();
    apply_all(
        &mut p,
        &[Op::AddModule {
            at: Position {
                section: Section::VoiceFx,
                index: 0,
            },
            module: ModuleInstance::new(id("filter1"), ModuleKind::Svf),
        }],
    )
    .unwrap();
    p
}

fn target_patch() -> Patch {
    let mut p = start_patch();
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

fn spec(level: u8, seed: u64) -> RandomSpec {
    SpecPreset::by_id(level).unwrap().spec(1, seed)
}

fn scoring() -> ScoringSpec {
    ScoringSpec::recreate(0)
}

fn structure(percent: f64) -> StructureScore {
    StructureScore {
        scorer: scoring().structure,
        percent,
        params: vec![ParamScore {
            addr: CUTOFF.to_owned(),
            answer_addr: Some(CUTOFF.to_owned()),
            score: percent / 100.0,
            verdict: if percent >= 90.0 {
                Verdict::Good
            } else {
                Verdict::Bad
            },
            direction: Direction::Higher,
            randomized: true,
        }],
        modules: Vec::new(),
    }
}

fn report(sound: f64) -> ScoreReport {
    let scoring = scoring();
    ScoreReport::new(
        &scoring,
        SoundScore {
            scorer: scoring.sound.clone(),
            percent: sound,
            distance: 1.0,
            floor: 0.0,
            scale: 1.0,
            jnd: 1.0,
            components: Vec::new(),
        },
        structure(sound),
        Combined {
            scorer: scoring.combined.clone(),
            percent: sound,
            sound_weight: scoring.sound_weight,
        },
    )
}

/// The right answer to a job.
fn answer(job: &Job, sound: f64) -> JobResult {
    match &job.kind {
        JobKind::Prepare { spec } => JobResult::Prepared(Box::new(Prepared::new(
            Challenge::recreate(*spec, PhraseId(1)),
            start_patch(),
            target_patch(),
            vec![addr(CUTOFF)],
            TargetFingerprint(exact_hash(&target_patch()).0),
            Some(-14.0),
        ))),
        JobKind::Score { .. } => JobResult::Scored(Box::new(report(sound))),
        JobKind::Rings { .. } => JobResult::Rings(Box::new(structure(sound))),
        JobKind::Loudness { .. } => JobResult::Loudness { lufs: Some(-18.0) },
        JobKind::Hint { level, .. } => JobResult::Hint(CoachHint::new(*level, "a hint")),
        _ => JobResult::Failed {
            message: "unknown job".to_owned(),
        },
    }
}

/// One step of the machine.
#[derive(Clone, Debug)]
enum Step {
    Tick { dt: u16, view: u8 },
    Start { level: u8, seed: u16, code: bool },
    Submit,
    Hint,
    Reveal,
    Rate(u8),
    Vote(u8),
    Dispute,
    Listen(u8),
    End(bool),
    Edit(u16),
    Undo,
    Answer { pick: u8, how: u8, sound: u8 },
    Save,
    HostLoad(u8),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (0u16..2_000, 0u8..3).prop_map(|(dt, view)| Step::Tick { dt, view }),
        2 => (1u8..=5, 0u16..4, any::<bool>()).prop_map(|(level, seed, code)| Step::Start { level, seed, code }),
        2 => Just(Step::Submit),
        1 => Just(Step::Hint),
        1 => Just(Step::Reveal),
        1 => (0u8..7).prop_map(Step::Rate),
        1 => (0u8..3).prop_map(Step::Vote),
        1 => Just(Step::Dispute),
        1 => (0u8..3).prop_map(Step::Listen),
        1 => any::<bool>().prop_map(Step::End),
        2 => (20u16..20_000).prop_map(Step::Edit),
        1 => Just(Step::Undo),
        6 => (any::<u8>(), 0u8..6, 0u8..=100).prop_map(|(pick, how, sound)| Step::Answer { pick, how, sound }),
        1 => Just(Step::Save),
        1 => any::<u8>().prop_map(Step::HostLoad),
    ]
}

/// The shell's side: the jobs it was asked for and not yet answered or cancelled, and the saves it took.
struct Shell {
    now: u64,
    in_flight: Vec<Job>,
    saves: Vec<SavedState>,
}

impl Shell {
    /// Drain the outbox (twice: the second must be empty), track the jobs, and check every log line.
    fn drain(&mut self, s: &mut Session) -> Result<(), TestCaseError> {
        let requests: Vec<GameRequest> = s.drain_game_requests().collect();
        let again: Vec<GameRequest> = s.drain_game_requests().collect();
        prop_assert!(again.is_empty(), "the outbox drains: {again:?}");
        s.drain_requests().for_each(drop);
        for r in requests {
            match r {
                GameRequest::Run(job) => self.in_flight.push(job),
                GameRequest::Cancel(id) => self.in_flight.retain(|j| j.id != id),
                GameRequest::Log(line) => {
                    if let Some(result) = &line.result {
                        let json = serde_json::to_string(result).unwrap();
                        let back: RoundResult = serde_json::from_str(&json)
                            .map_err(|e| TestCaseError::fail(format!("{e}: {json}")))?;
                        prop_assert_eq!(&back, result);
                        if let Some(t) = result.time_to_pass_ms {
                            prop_assert!(t <= result.active_ms, "{t} > {}", result.active_ms);
                        }
                    }
                }
                _ => {}
            }
        }
        // At most one job of each kind in flight.
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for j in &self.in_flight {
            *kinds.entry(j.kind.name()).or_default() += 1;
        }
        prop_assert!(
            kinds.values().all(|n| *n <= 1),
            "jobs in flight: {:?}",
            kinds
        );
        Ok(())
    }
}

fn run(steps: &[Step]) -> Result<(), TestCaseError> {
    let mut s = Session::new(custom_patch());
    let mut shell = Shell {
        now: 0,
        in_flight: Vec::new(),
        saves: Vec::new(),
    };
    s.game_tick(0, EditorView::Opened);
    for step in steps {
        match step {
            Step::Tick { dt, view } => {
                shell.now += u64::from(*dt);
                let view = [EditorView::Opened, EditorView::Visible, EditorView::Hidden]
                    [usize::from(*view)];
                s.game_tick(shell.now, view);
            }
            Step::Start { level, seed, code } => {
                let spec = spec(*level, u64::from(*seed));
                let source = if *code {
                    RoundSource::Code(ChallengeCode::short(spec).unwrap())
                } else {
                    RoundSource::Settings(spec)
                };
                s.game_start(source).unwrap();
            }
            Step::Submit => {
                let revealed = s.game_record().is_some_and(|r| r.revealed);
                let r = s.game_submit();
                if revealed {
                    prop_assert!(r.is_err(), "a submit after a reveal: {r:?}");
                }
            }
            Step::Hint => {
                let _ = s.game_hint();
            }
            Step::Reveal => {
                let _ = s.game_reveal();
            }
            Step::Rate(r) => {
                let _ = s.game_rate(*r);
            }
            Step::Vote(v) => {
                let _ = s.game_vote(
                    [Agreement::TooLow, Agreement::AboutRight, Agreement::TooHigh][usize::from(*v)],
                );
            }
            Step::Dispute => {
                let _ = s.game_dispute("disagree");
            }
            Step::Listen(l) => {
                let _ = s.game_listen([Listen::Off, Listen::Target, Listen::Mine][usize::from(*l)]);
            }
            Step::End(keep) => {
                let _ = s.game_end(if *keep {
                    EndChoice::Keep
                } else {
                    EndChoice::Restore
                });
            }
            Step::Edit(v) => {
                let _ = s.edit(
                    vec![Op::SetParam {
                        addr: addr(CUTOFF),
                        value: f32::from(*v),
                    }],
                    None,
                );
            }
            Step::Undo => {
                let _ = s.undo();
            }
            Step::Answer { pick, how, sound } => {
                let sound = f64::from(*sound);
                if shell.in_flight.is_empty() {
                    let r = s.game_job_done(JobId(u64::MAX), JobResult::Loudness { lufs: None });
                    prop_assert_eq!(r, Err(GameError::StaleJob(JobId(u64::MAX))));
                } else {
                    let i = usize::from(*pick) % shell.in_flight.len();
                    let job = shell.in_flight[i].clone();
                    let result = match how {
                        2 => JobResult::Failed {
                            message: "worker failed".to_owned(),
                        },
                        3 => JobResult::Hint(CoachHint::new(1, "wrong kind")),
                        4 => {
                            let mut r = report(sound);
                            r.passed = !r.passed;
                            JobResult::Scored(Box::new(r))
                        }
                        _ => answer(&job, sound),
                    };
                    let id = if *how == 5 {
                        JobId(job.id.0 + 1_000_000)
                    } else {
                        job.id
                    };
                    match s.game_job_done(id, result) {
                        // Consumed: answered, failed, or a report refused for good.
                        Ok(()) | Err(GameError::BadReport { .. }) => {
                            shell.in_flight.retain(|j| j.id != id);
                        }
                        // Still owed its real result.
                        Err(GameError::WrongResult { .. }) => {}
                        Err(GameError::StaleJob(stale)) => {
                            prop_assert_eq!(stale, id);
                            // A job the session forgot without a cancel would show here.
                            prop_assert!(
                                *how == 5 || !shell.in_flight.iter().any(|j| j.id == stale),
                                "job {} in flight but stale",
                                stale.0
                            );
                        }
                        Err(e) => prop_assert!(false, "unexpected {e:?}"),
                    }
                }
            }
            Step::Save => {
                shell.saves.push(s.save(None));
                if shell.saves.len() > 4 {
                    shell.saves.remove(0);
                }
            }
            Step::HostLoad(i) => {
                if !shell.saves.is_empty() {
                    let saved = shell.saves[usize::from(*i) % shell.saves.len()].clone();
                    s.load(&saved);
                }
            }
        }
        shell.drain(&mut s)?;
        if let Some(v) = s.round() {
            if let Some(t) = v.time_to_pass_ms {
                prop_assert!(t <= v.active_ms, "view: {t} > {}", v.active_ms);
            }
            if v.phase == Phase::Revealed {
                prop_assert!(s.game_submit().is_err());
            }
        }
    }
    // Ending the round empties the outbox's jobs: everything in flight is cancelled.
    let _ = s.game_end(EndChoice::Restore);
    shell.drain(&mut s)?;
    prop_assert!(s.round().is_none());
    prop_assert!(
        shell.in_flight.is_empty(),
        "jobs left in flight after the end: {:?}",
        shell
            .in_flight
            .iter()
            .map(|j| j.kind.name())
            .collect::<Vec<_>>()
    );
    Ok(())
}

proptest! {
    #[test]
    fn t5_the_round_survives_any_sequence(steps in proptest::collection::vec(step(), 1..60)) {
        run(&steps)?;
    }
}
