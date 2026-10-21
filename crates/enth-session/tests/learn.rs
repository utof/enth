//! The session's Learn API through the public API (lane SL, R2; docs/session.md, "Learn"): the lifecycle and the
//! stash, every op kind on a locked control refused, one plan per step load, jobs and stale results, the lesson
//! round (`RoundSource::Challenge`, origin `lesson`), the saved `learn` object and its budget, host automation and
//! loads, and no audio-thread contact. The shell and lane LM's player are played by hand: a scripted player
//! ([`Fake`]) stands in for `enth_learn::player::Player` behind the session's `LessonPlayer` trait. No wall clock and
//! no random seed anywhere: every timestamp is a literal.

mod common;

use std::sync::Arc;

use enth_challenge::score::ParamScore;
use enth_challenge::{
    Combined, Direction, PhraseId, ScoringSpec, SoundScore, SpecPreset, TargetFingerprint,
};
use enth_engine::{Engine, Event, EventKind, NoteKey};
use enth_patch::canon::exact_hash;
use enth_patch::model::{ModuleInstance, Route, Section};
use enth_patch::{ModSource, ModuleKind, Op, Patch, PatchMeta, Position, apply_all};
use enth_session::game::{
    Challenge, EditorView, EndChoice, GameEdit, GameError, GameRequest, Job, JobKind, JobResult,
    Phase, Prepared, RandomSpec, RoundLogEntry, RoundOrigin, RoundSource, ScoreReport,
    StructureScore, Verdict,
};
use enth_session::learn::{
    CheckJob, CheckVerdict, EditSource, Example, HintAction, HintJob, LEARN_BYTES,
    LESSON_REF_BYTES, LESSON_ROUND_BYTES, LearnEdit, LearnError, LearnJob, LearnJobKind,
    LearnRequest, LearnResult, LearnState, LessonPlayer, LessonRef, ListenState, MeasuredHint,
    PlayerEffect, PlayerError, RoundEnd, Selector, StepPhase, StepRecord, StepView,
};
use enth_session::{Activation, InstanceState, SavedState, Session, SessionError, ShellRequest};

use common::{addr, custom_patch, id, init_patch, macro_index, slot};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const CUTOFF: &str = "filter1.cutoff";

fn set(a: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr(a),
        value,
    }
}

fn at(section: Section, index: usize) -> Position {
    Position { section, index }
}

// ---------------------------------------------------------------------------------------------------------------
// Patches and the challenge
// ---------------------------------------------------------------------------------------------------------------

/// A lesson's patch: the fixture patch (filter1, lfo1 → filter1.cutoff, slots on the cutoff, master and glide, a
/// named macro) plus a second envelope and a macro route into the resonance, so every op kind has something to touch.
fn lesson_patch() -> Patch {
    let mut p = custom_patch();
    let ops = [
        Op::AddModule {
            at: at(Section::Modulators, 2),
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
    ];
    apply_all(&mut p, &ops).unwrap();
    p
}

fn lesson_patch_with(cutoff: f32) -> Patch {
    let mut p = lesson_patch();
    apply_all(&mut p, &[set(CUTOFF, cutoff)]).unwrap();
    p
}

/// The round's start, as the generator would give it: the init patch plus a filter, no slots.
fn round_start() -> Patch {
    let mut p = init_patch();
    let add = Op::AddModule {
        at: at(Section::VoiceFx, 0),
        module: ModuleInstance::new(id("filter1"), ModuleKind::Svf),
    };
    apply_all(&mut p, &[add]).unwrap();
    p
}

fn round_target() -> Patch {
    let mut p = round_start();
    apply_all(&mut p, &[set(CUTOFF, 800.0)]).unwrap();
    p
}

fn spec() -> RandomSpec {
    SpecPreset::by_id(2).unwrap().spec(1, 42)
}

fn challenge() -> Challenge {
    Challenge::recreate(spec(), PhraseId(1))
}

fn fingerprint() -> TargetFingerprint {
    TargetFingerprint(exact_hash(&round_target()).0)
}

fn prepared() -> JobResult {
    JobResult::Prepared(Box::new(Prepared::new(
        challenge(),
        round_start(),
        round_target(),
        vec![addr(CUTOFF)],
        fingerprint(),
        Some(-14.0),
    )))
}

fn scoring() -> ScoringSpec {
    ScoringSpec::recreate(spec().difficulty)
}

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

// ---------------------------------------------------------------------------------------------------------------
// A scripted player (lane LM's `Player` stands here)
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Kind {
    /// Text only.
    Read,
    /// A patch check: the cutoff below this, by the player's own edit.
    Do { below: f32 },
    /// A sound check, once the patch settles.
    Sound,
    /// A Recreate round.
    Recreate,
    /// An ear question with this answer and two examples.
    Choose { answer: u8 },
}

#[derive(Clone, Debug)]
struct Step {
    kind: Kind,
    start: Option<Patch>,
    lock: Vec<&'static str>,
}

fn step(kind: Kind, start: Option<Patch>, lock: &[&'static str]) -> Step {
    Step {
        kind,
        start,
        lock: lock.to_vec(),
    }
}

/// What the player saw, for the tests to read.
#[derive(Debug, Default)]
struct Seen {
    /// Each `edited`: revision, source, the cutoff and macro 1 in the checked patch.
    edits: Vec<(u64, EditSource, Option<f32>, f32)>,
    rounds: Vec<RoundEnd>,
    ends: Vec<EndChoice>,
    starts: u32,
    hints_done: u32,
}

#[allow(
    clippy::disallowed_types,
    reason = "the test reads what its fake player saw; the player lives inside the session (Send), and no audio path is involved"
)]
type Log = Arc<std::sync::Mutex<Seen>>;

#[derive(Debug)]
struct Fake {
    lesson: LessonRef,
    steps: Vec<Step>,
    phase: StepPhase,
    log: Log,
    last: Option<(u64, Patch)>,
    settle_at: Option<u64>,
    jobs: u64,
    pending: Option<(u64, u64)>,
    hints: u8,
}

fn lesson_ref() -> LessonRef {
    LessonRef {
        pack: "basics".to_owned(),
        version: "1.0.0".to_owned(),
        lesson: "02-filter".to_owned(),
        step: 0,
        content: "0123456789abcdef".to_owned(),
    }
}

impl Fake {
    fn new(steps: Vec<Step>) -> (Box<Fake>, Log) {
        let log = Log::default();
        let fake = Fake {
            lesson: lesson_ref(),
            steps,
            phase: StepPhase::Intro,
            log: Arc::clone(&log),
            last: None,
            settle_at: None,
            jobs: 0,
            pending: None,
            hints: 0,
        };
        (Box::new(fake), log)
    }

    fn at_step(mut self: Box<Self>, step: u16) -> Box<Self> {
        self.lesson.step = step;
        self
    }

    fn current(&self) -> &Step {
        &self.steps[usize::from(self.lesson.step)]
    }

    fn record(&self, assisted: bool) -> PlayerEffect {
        PlayerEffect::Record(StepRecord {
            step: self.lesson.step,
            passed: true,
            assisted,
        })
    }

    /// The current step's load: its view, then its start (a round's challenge, or a patch), then its examples.
    fn load(&mut self) -> Vec<PlayerEffect> {
        self.phase = StepPhase::Working;
        self.pending = None;
        self.settle_at = None;
        let step = self.current().clone();
        let mut out = vec![PlayerEffect::View(StepView {
            lock: step.lock.iter().map(|t| t.parse().unwrap()).collect(),
            ..StepView::default()
        })];
        match step.kind {
            Kind::Recreate => out.push(PlayerEffect::StartRound(Box::new(challenge()))),
            _ => {
                if let Some(start) = step.start {
                    out.push(PlayerEffect::LoadPatch(Arc::new(start)));
                }
            }
        }
        if let Kind::Choose { .. } = step.kind {
            let example = |label: &str, cutoff: f32| Example {
                label: label.to_owned(),
                patch: Arc::new(lesson_patch_with(cutoff)),
                level_match: true,
            };
            out.push(PlayerEffect::Examples(Arc::from(vec![
                example("A", 500.0),
                example("B", 5000.0),
            ])));
        }
        out
    }
}

impl LessonPlayer for Fake {
    fn lesson_ref(&self) -> LessonRef {
        self.lesson.clone()
    }

    fn steps(&self) -> u16 {
        self.steps.len() as u16
    }

    fn phase(&self) -> StepPhase {
        self.phase
    }

    fn start(&mut self, _now_ms: u64) -> Vec<PlayerEffect> {
        self.log.lock().unwrap().starts += 1;
        self.load()
    }

    fn edited(
        &mut self,
        now_ms: u64,
        revision: u64,
        patch: &Patch,
        _ops: &[Op],
        source: EditSource,
    ) -> Vec<PlayerEffect> {
        let cutoff = patch.value(&addr(CUTOFF));
        self.log
            .lock()
            .unwrap()
            .edits
            .push((revision, source, cutoff, patch.macros[0].value));
        self.last = Some((revision, patch.clone()));
        match self.current().kind {
            Kind::Do { below }
                if source == EditSource::Player
                    && self.phase != StepPhase::Passed
                    && cutoff.is_some_and(|c| c < below) =>
            {
                self.phase = StepPhase::Passed;
                vec![self.record(false)]
            }
            Kind::Sound => {
                self.settle_at = Some(now_ms + 300);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn tick(&mut self, now_ms: u64) -> Option<CheckJob> {
        let at = self.settle_at?;
        if now_ms < at {
            return None;
        }
        self.settle_at = None;
        let (revision, patch) = self.last.clone()?;
        self.jobs += 1;
        self.pending = Some((self.jobs, revision));
        self.phase = StepPhase::Checking;
        Some(CheckJob {
            id: self.jobs,
            revision,
            patch: Box::new(patch),
        })
    }

    fn check_done(
        &mut self,
        job: u64,
        revision: u64,
        verdict: CheckVerdict,
    ) -> Result<Vec<PlayerEffect>, PlayerError> {
        if self.pending != Some((job, revision)) {
            return Err(PlayerError {
                code: "stale_result",
                hint: "ignore it".to_owned(),
            });
        }
        self.pending = None;
        if verdict.passed {
            self.phase = StepPhase::Passed;
            Ok(vec![self.record(false)])
        } else {
            self.phase = StepPhase::Working;
            Ok(Vec::new())
        }
    }

    fn choose(&mut self, option: u8) -> Result<Vec<PlayerEffect>, PlayerError> {
        match self.current().kind {
            Kind::Choose { answer } if option == answer => {
                self.phase = StepPhase::Passed;
                Ok(vec![self.record(false)])
            }
            Kind::Choose { .. } => Ok(Vec::new()),
            _ => Err(PlayerError {
                code: "no_choice",
                hint: "this step asks no question".to_owned(),
            }),
        }
    }

    fn round_ended(&mut self, end: RoundEnd) -> Vec<PlayerEffect> {
        self.log.lock().unwrap().rounds.push(end);
        if end.passed {
            self.phase = StepPhase::Passed;
            vec![self.record(end.assisted)]
        } else {
            self.phase = StepPhase::Shown;
            Vec::new()
        }
    }

    fn hint(&mut self) -> Option<HintAction> {
        self.hints += 1;
        match self.hints {
            1 => Some(HintAction::Text("Turn the cutoff down.".to_owned())),
            2 => {
                let (revision, patch) = self.last.clone()?;
                self.jobs += 1;
                Some(HintAction::Measure(HintJob {
                    id: self.jobs,
                    revision,
                    patch: Box::new(patch),
                }))
            }
            _ => None,
        }
    }

    fn hint_done(
        &mut self,
        _job: u64,
        _revision: u64,
        _hint: MeasuredHint,
    ) -> Result<Vec<PlayerEffect>, PlayerError> {
        self.log.lock().unwrap().hints_done += 1;
        Ok(Vec::new())
    }

    fn show(&mut self, patch: &Patch) -> Option<Op> {
        match self.current().kind {
            Kind::Do { below } if patch.value(&addr(CUTOFF)).is_some_and(|c| c >= below) => {
                self.phase = StepPhase::Shown;
                Some(set(CUTOFF, below - 100.0))
            }
            _ => None,
        }
    }

    fn next(&mut self) -> Vec<PlayerEffect> {
        if usize::from(self.lesson.step) + 1 < self.steps.len() {
            self.lesson.step += 1;
            self.load()
        } else {
            vec![PlayerEffect::Finished]
        }
    }

    fn back(&mut self) -> Vec<PlayerEffect> {
        if self.lesson.step == 0 {
            return Vec::new();
        }
        self.lesson.step -= 1;
        self.load()
    }

    fn end(&mut self, choice: EndChoice) -> Vec<PlayerEffect> {
        self.log.lock().unwrap().ends.push(choice);
        Vec::new()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The shell, by hand
// ---------------------------------------------------------------------------------------------------------------

fn drain_learn(s: &mut Session) -> Vec<LearnRequest> {
    s.drain_learn_requests().collect()
}

fn learn_jobs(requests: &[LearnRequest]) -> Vec<LearnJob> {
    requests
        .iter()
        .filter_map(|r| match r {
            LearnRequest::Run(job) => Some(job.clone()),
            _ => None,
        })
        .collect()
}

/// The one learn job of `kind` the session asked for.
fn learn_job(requests: &[LearnRequest], kind: &str) -> LearnJob {
    let found: Vec<LearnJob> = learn_jobs(requests)
        .into_iter()
        .filter(|j| j.kind.name() == kind)
        .collect();
    assert_eq!(found.len(), 1, "one {kind} job in {requests:#?}");
    found.into_iter().next().unwrap()
}

fn records(requests: &[LearnRequest]) -> Vec<StepRecord> {
    requests
        .iter()
        .filter_map(|r| match r {
            LearnRequest::Record { record, .. } => Some(*record),
            _ => None,
        })
        .collect()
}

fn drain_game(s: &mut Session) -> Vec<GameRequest> {
    s.drain_game_requests().collect()
}

fn game_job(requests: &[GameRequest], kind: &str) -> Job {
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

/// A three-step lesson: a patch check from 2 kHz, a sound check from 3 kHz, then text.
fn three_steps() -> Vec<Step> {
    vec![
        step(
            Kind::Do { below: 1000.0 },
            Some(lesson_patch_with(2000.0)),
            &["filter1.resonance"],
        ),
        step(Kind::Sound, Some(lesson_patch_with(3000.0)), &[]),
        step(Kind::Read, None, &[]),
    ]
}

/// A session on the fixture patch, the editor open at shell time 1 000, with `steps` started.
fn started(steps: Vec<Step>) -> (Session, Log) {
    let mut s = Session::new(custom_patch());
    s.game_tick(1_000, EditorView::Opened);
    let (fake, log) = Fake::new(steps);
    s.learn_start(fake).unwrap();
    (s, log)
}

// ---------------------------------------------------------------------------------------------------------------
// The lifecycle and the stash
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_lesson_stashes_your_patch_loads_its_steps_and_restores_you() {
    let mut s = Session::new(custom_patch());
    s.edit(vec![set(CUTOFF, 1500.0)], None).unwrap(); // your own step, undoable after the lesson
    let own = s.save(None);
    s.drain_requests().for_each(drop);
    assert_eq!(s.lesson(), None);

    s.game_tick(1_000, EditorView::Opened);
    let (fake, log) = Fake::new(three_steps());
    s.learn_start(fake).unwrap();
    // The stash is your patch as the `patch` field held it; step 0's start plays, with a history of its own.
    let record = s.learn_record().unwrap().clone();
    assert_eq!(record.stash.as_deref(), Some(own.patch.as_str()));
    assert_eq!(record.lesson.as_ref().map(|l| l.step), Some(0));
    assert_eq!(cutoff(&s), 2000.0);
    assert!(!s.history().can_undo(), "undo never leaves the step");
    let v = s.lesson().unwrap();
    assert_eq!((v.steps, v.phase, v.round), (3, StepPhase::Working, false));
    assert_eq!(
        v.view.lock,
        vec!["filter1.resonance".parse::<Selector>().unwrap()]
    );
    let shell: Vec<ShellRequest> = s.drain_requests().collect();
    assert!(shell.contains(&ShellRequest::PatchEdited), "{shell:?}");
    assert!(drain_learn(&mut s).contains(&LearnRequest::Dirty));
    assert_eq!(log.lock().unwrap().starts, 1);

    // The patch check passes on your own edit and records it for the progress file.
    s.edit(vec![set(CUTOFF, 900.0)], None).unwrap();
    assert_eq!(s.lesson().unwrap().phase, StepPhase::Passed);
    assert_eq!(
        records(&drain_learn(&mut s)),
        [StepRecord {
            step: 0,
            passed: true,
            assisted: false
        }]
    );

    // Next: step 1 loads its start; the saved reference follows; undo cannot reach step 0's edit.
    s.learn_next().unwrap();
    assert_eq!(cutoff(&s), 3000.0);
    assert_eq!(s.learn_record().unwrap().lesson.as_ref().unwrap().step, 1);
    assert!(!s.history().can_undo());
    s.edit(vec![set(CUTOFF, 2500.0)], None).unwrap();
    assert!(s.history().can_undo(), "undo works inside the step");
    s.undo().unwrap();
    assert_eq!(cutoff(&s), 3000.0);
    assert!(!s.history().can_undo());

    // Back reloads step 0's start; Next twice reaches the text step, which keeps the patch; past the end, Finished.
    s.learn_back().unwrap();
    assert_eq!(cutoff(&s), 2000.0);
    s.learn_next().unwrap();
    s.learn_next().unwrap();
    assert_eq!(cutoff(&s), 3000.0, "a step without a start keeps the patch");
    s.learn_next().unwrap();
    assert!(s.lesson().unwrap().finished);

    // Restore: your patch and your own history come back, and the `learn` object is gone.
    s.learn_end(EndChoice::Restore).unwrap();
    assert_eq!(s.lesson(), None);
    assert_eq!(s.learn_record(), None);
    assert_eq!(s.instance().learn, None);
    assert_eq!(s.save(None), own);
    assert!(s.history().can_undo(), "your own step is undoable again");
    s.undo().unwrap();
    assert_eq!(cutoff(&s), 1234.5);
    assert_eq!(log.lock().unwrap().ends, [EndChoice::Restore]);
    assert_eq!(s.locks(), &[] as &[Selector]);
}

#[test]
fn keep_keeps_the_lessons_patch_and_a_new_lesson_keeps_the_first_stash() {
    let (mut s, _) = started(three_steps());
    let own = custom_patch().to_json();
    s.learn_next().unwrap();
    // A second lesson while the first is live: the stash is still your patch from before the first.
    let (again, _) = Fake::new(three_steps());
    s.learn_start(again).unwrap();
    assert_eq!(
        s.learn_record().unwrap().stash.as_deref(),
        Some(own.as_str())
    );
    assert_eq!(cutoff(&s), 2000.0);
    s.edit(vec![set(CUTOFF, 1700.0)], None).unwrap();
    s.learn_end(EndChoice::Keep).unwrap();
    assert_eq!(cutoff(&s), 1700.0, "Keep keeps the lesson's patch");
    assert_eq!(s.instance().learn, None);
    assert!(s.history().can_undo(), "and the step's history");
    assert_eq!(s.learn_end(EndChoice::Keep), Err(LearnError::NoLesson));
    assert_eq!(s.learn_next(), Err(LearnError::NoLesson));
}

#[test]
fn a_lesson_too_large_to_save_is_refused_and_changes_nothing() {
    // A player's patch kept as text (a newer build's) that would not fit as a stash.
    let mut s = Session::new(custom_patch());
    let huge = format!("{{\"schema\":9999,\"x\":\"{}\"}}", "\\\"".repeat(200_000));
    s.load_patch(&huge);
    let (fake, _) = Fake::new(three_steps());
    let e = s.learn_start(fake).unwrap_err();
    assert!(
        matches!(e, LearnError::TooLarge { what: "stash", .. }),
        "{e:?}"
    );
    assert_eq!((e.code(), e.hint().is_empty()), ("too_large", false));
    assert_eq!(s.lesson(), None);
    assert_eq!(s.save(None).patch, huge);

    // A lesson reference longer than the budget (a pack with absurd ids).
    let mut s = Session::new(custom_patch());
    let (mut fake, _) = Fake::new(three_steps());
    fake.lesson.lesson = "x".repeat(LESSON_REF_BYTES);
    let e = s.learn_start(fake).unwrap_err();
    assert!(
        matches!(e, LearnError::TooLarge { what: "lesson", .. }),
        "{e:?}"
    );
    assert_eq!(s.patch().to_json(), custom_patch().to_json());
    assert_eq!(drain_learn(&mut s), []);
}

// ---------------------------------------------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------------------------------------------

/// One case per op kind: the lock, and an op that touches it. Each op is valid on the lesson patch (checked after
/// the lesson ends), so the refusal is the lock's.
fn lock_cases() -> Vec<(&'static str, Op)> {
    vec![
        ("filter1.cutoff", set(CUTOFF, 500.0)),
        (
            "filter2.*",
            Op::AddModule {
                at: at(Section::VoiceFx, 1),
                module: ModuleInstance::new(id("filter2"), ModuleKind::Svf),
            },
        ),
        ("filter1.cutoff", Op::RemoveModule { id: id("filter1") }),
        (
            "lfo1.*",
            Op::Move {
                id: id("lfo1"),
                to: at(Section::Modulators, 2),
            },
        ),
        (
            "filter1.resonance",
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: ModSource::Module(id("lfo1")),
                    dst: addr("filter1.resonance"),
                    depth: 10.0,
                },
            },
        ),
        (
            "lfo1 -> filter1.cutoff",
            Op::RemoveRoute {
                route: "lfo1 -> filter1.cutoff".parse().unwrap(),
            },
        ),
        (
            "lfo1 -> filter1.cutoff",
            Op::SetDepth {
                route: "lfo1 -> filter1.cutoff".parse().unwrap(),
                depth: 50.0,
            },
        ),
        (
            "filter1.cutoff",
            Op::SetEnabled {
                id: id("filter1"),
                enabled: false,
            },
        ),
        ("env1.*", Op::SetAmpEnv { id: id("env2") }),
        (
            "macro1",
            Op::SetMacro {
                index: macro_index(1),
                value: 0.9,
            },
        ),
        (
            "filter1.resonance",
            Op::SetSlot {
                slot: slot(3),
                addr: Some(addr("filter1.resonance")),
            },
        ),
        (
            "filter1.*",
            Op::SetLabel {
                id: id("filter1"),
                label: Some("Mine".to_owned()),
            },
        ),
        (
            "macro1",
            Op::SetMacroName {
                index: macro_index(1),
                name: Some("Mine".to_owned()),
            },
        ),
        // Two more ways in: a macro whose route moves a locked parameter, and clearing a locked parameter's slot.
        (
            "filter1.resonance",
            Op::SetMacro {
                index: macro_index(1),
                value: 0.9,
            },
        ),
        (
            "filter1.cutoff",
            Op::SetSlot {
                slot: slot(1),
                addr: None,
            },
        ),
    ]
}

/// A one-step lesson on the lesson patch that locks `lock`.
fn locked(lock: &'static str) -> Session {
    let (s, _) = started(vec![step(Kind::Read, Some(lesson_patch()), &[lock])]);
    s
}

#[test]
fn every_op_kind_on_a_locked_control_is_refused_and_changes_nothing() {
    let mut kinds = std::collections::BTreeSet::new();
    for (lock, op) in lock_cases() {
        let mut s = locked(lock);
        s.drain_requests().for_each(drop);
        drain_learn(&mut s);
        let before = s.save(None);
        let e = s.edit(vec![op.clone()], None).unwrap_err();
        match &e {
            SessionError::Locked {
                op: name,
                lock: by,
                touched,
            } => {
                assert_eq!(*name, op.name());
                assert_eq!(by, lock, "{op:?}");
                assert_ne!(touched, "");
            }
            other => panic!("{op:?}: {other:?}"),
        }
        assert_eq!(e.code(), "locked");
        assert!(e.hint().contains("locked"), "{}", e.hint());
        assert_eq!(
            s.status().last_error,
            Some(e.clone()),
            "the status line shows it"
        );
        assert_eq!(s.save(None), before, "{op:?} changed nothing");
        assert!(!s.history().can_undo(), "{op:?} recorded nothing");
        assert_eq!(s.drain_requests().count(), 0, "{op:?} delivered nothing");
        // The op itself is valid: once the lesson ends, it applies.
        s.learn_end(EndChoice::Keep).unwrap();
        s.edit(vec![op.clone()], None)
            .unwrap_or_else(|e| panic!("{op:?} after the lesson: {e:?}"));
        kinds.insert(op.name());
    }
    // Every op kind but `SetMeta`, which touches no control (the next test).
    let all = [
        "SetParam",
        "AddModule",
        "RemoveModule",
        "Move",
        "InsertRoute",
        "RemoveRoute",
        "SetDepth",
        "SetEnabled",
        "SetAmpEnv",
        "SetMacro",
        "SetSlot",
        "SetLabel",
        "SetMacroName",
    ];
    assert_eq!(kinds, all.into_iter().collect());
}

#[test]
fn a_transaction_with_one_locked_op_is_refused_whole_and_free_controls_stay_free() {
    let mut s = locked("filter1.*");
    let before = s.save(None);
    let e = s
        .edit(vec![set("osc1.level", 50.0), set(CUTOFF, 400.0)], None)
        .unwrap_err();
    assert!(matches!(e, SessionError::Locked { .. }));
    assert_eq!(s.save(None), before);
    // A transaction that removes a module and then touches its locked id again is judged op by op.
    let e = s
        .edit(
            vec![
                Op::RemoveModule { id: id("env2") },
                Op::SetLabel {
                    id: id("filter1"),
                    label: None,
                },
            ],
            None,
        )
        .unwrap_err();
    assert!(
        matches!(&e, SessionError::Locked { op: "SetLabel", .. }),
        "{e:?}"
    );
    // Free controls, a free module and the metadata stay editable.
    s.edit(vec![set("osc1.level", 50.0)], None).unwrap();
    s.edit(
        vec![Op::SetMeta {
            meta: PatchMeta {
                name: "mine".to_owned(),
                ..PatchMeta::default()
            },
        }],
        None,
    )
    .unwrap();
    s.edit(vec![set("env2.attack", 40.0)], None).unwrap();
    assert!(s.history().can_undo());
    // A drag on a locked control is refused at its first move, and the gesture stays harmless.
    let g = s.begin_gesture();
    assert!(s.edit(vec![set(CUTOFF, 300.0)], Some(g)).is_err());
    s.end_gesture();
    assert_eq!(cutoff(&s), lesson_patch().value(&addr(CUTOFF)).unwrap());
}

#[test]
fn show_me_goes_through_the_edit_path_and_its_locks() {
    let (mut s, log) = started(vec![step(
        Kind::Do { below: 1000.0 },
        Some(lesson_patch_with(2000.0)),
        &[],
    )]);
    s.learn_show().unwrap();
    assert_eq!(cutoff(&s), 900.0);
    assert!(s.history().can_undo(), "one undo step");
    assert_eq!(
        log.lock().unwrap().edits.last().map(|e| e.1),
        Some(EditSource::Player)
    );
    assert_eq!(s.learn_show(), Err(LearnError::NothingToShow));
    // A witness that touches a lock is refused like any edit (a content bug that `lesson-check` catches).
    let (mut s, _) = started(vec![step(
        Kind::Do { below: 1000.0 },
        Some(lesson_patch_with(2000.0)),
        &["filter1.cutoff"],
    )]);
    let e = s.learn_show().unwrap_err();
    assert!(
        matches!(&e, LearnError::Edit(SessionError::Locked { .. })),
        "{e:?}"
    );
    assert_eq!(e.code(), "locked");
    assert_eq!(cutoff(&s), 2000.0);
}

#[test]
fn a_preset_load_and_a_game_round_wait_for_the_lesson_to_end() {
    let (mut s, _) = started(three_steps());
    let e = s.load_preset(&init_patch(), None).unwrap_err();
    assert_eq!(
        e,
        SessionError::LessonLive {
            what: "load_preset"
        }
    );
    assert_eq!(
        (e.code(), e.hint().contains("Restore")),
        ("lesson_live", true)
    );
    assert_eq!(cutoff(&s), 2000.0);
    assert_eq!(
        s.game_start(RoundSource::Settings(spec())),
        Err(GameError::LessonLive)
    );
    assert_eq!(GameError::LessonLive.code(), "lesson_live");
    s.learn_end(EndChoice::Keep).unwrap();
    s.load_preset(&init_patch(), None).unwrap();

    // And a lesson waits for a game round of its own to end: two stashes would not fit the field.
    let mut s = Session::new(custom_patch());
    s.game_start(RoundSource::Settings(spec())).unwrap();
    let (fake, _) = Fake::new(three_steps());
    let e = s.learn_start(fake).unwrap_err();
    assert_eq!((e.clone(), e.code()), (LearnError::RoundLive, "round_live"));
    assert_eq!(s.lesson(), None);
}

// ---------------------------------------------------------------------------------------------------------------
// Host automation and host loads
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn host_automation_of_a_locked_parameter_folds_but_never_counts() {
    // Step 0 locks the cutoff (slot 1 drives it) and macro 1 and has a sound check, so its job shows the checked
    // patch; step 1 is a lesson round that locks macro 1.
    let steps = vec![
        step(
            Kind::Sound,
            Some(lesson_patch_with(2000.0)),
            &["filter1.cutoff", "macro1"],
        ),
        step(Kind::Recreate, None, &["macro1"]),
    ];
    let (mut s, log) = started(steps);
    let mut host = s.host_values();
    s.fold_host(&host); // the first fold after a load only records
    host.slots[0] = 0.1;
    host.macros[0] = 0.9;
    assert_eq!(s.fold_host(&host).written, 2);
    // The patch follows the host (the patch-wins rules hold) ...
    assert_ne!(cutoff(&s), 2000.0);
    assert_eq!(s.patch().macros[0].value, 0.9);
    // ... but every check sees the step's values.
    let checked = s.checked_patch();
    assert_eq!(checked.value(&addr(CUTOFF)), Some(2000.0));
    assert_eq!(checked.macros[0].value, 0.25);
    let last = *log.lock().unwrap().edits.last().unwrap();
    assert_eq!(
        (last.1, last.2, last.3),
        (EditSource::Host, Some(2000.0), 0.25)
    );
    s.game_tick(2_000, EditorView::Visible);
    let job = learn_job(&drain_learn(&mut s), "check");
    match &job.kind {
        LearnJobKind::Check(check) => {
            assert_eq!(check.patch.value(&addr(CUTOFF)), Some(2000.0));
        }
        other => panic!("{other:?}"),
    }
    // In a lesson round, the score job sees the round's value of a locked parameter, whatever the host did.
    s.learn_next().unwrap();
    let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
    s.game_job_done(prepare.id, prepared()).unwrap();
    let mut host = s.host_values();
    s.fold_host(&host); // the round's start is a load: the first fold only records
    host.macros[0] = 0.6;
    assert_eq!(s.fold_host(&host).written, 1);
    assert_eq!(s.patch().macros[0].value, 0.6);
    s.edit(vec![set("filter1.resonance", 20.0)], None).unwrap();
    s.game_submit().unwrap();
    let score = game_job(&drain_game(&mut s), "score");
    match &score.kind {
        JobKind::Score { patch, .. } => {
            assert_eq!(patch.macros[0].value, round_start().macros[0].value);
            assert_eq!(patch.value(&addr("filter1.resonance")), Some(20.0));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_host_load_of_the_same_lesson_goes_on_and_any_other_ends_it() {
    let mut s = Session::new(custom_patch());
    let before = s.save(None);
    s.game_tick(1_000, EditorView::Opened);
    let (fake, log) = Fake::new(three_steps());
    s.learn_start(fake).unwrap();
    s.edit(vec![set(CUTOFF, 1800.0)], None).unwrap();
    let mid = s.save(None);
    // REAPER's Undo within the step: the same lesson, step and stash. The lesson goes on with its player.
    s.edit(vec![set(CUTOFF, 1600.0)], None).unwrap();
    s.load(&mid);
    assert_eq!(cutoff(&s), 1800.0, "the patch follows the host");
    let v = s.lesson().unwrap();
    assert!(!v.pending.opening && v.steps == 3);
    assert_eq!(
        log.lock().unwrap().edits.last().unwrap().1,
        EditSource::Load
    );
    assert!(
        learn_jobs(&drain_learn(&mut s)).is_empty(),
        "nothing to reopen"
    );
    // A state from before the lesson: the host wins, the lesson ends without a restore, and nothing is dirty.
    s.load(&before);
    assert_eq!(s.lesson(), None);
    assert_eq!(s.save(None), before);
    assert!(!drain_learn(&mut s).contains(&LearnRequest::Dirty));
}

// ---------------------------------------------------------------------------------------------------------------
// The saved state
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_stash_survives_a_mid_lesson_save_and_the_lesson_resumes_at_its_step() {
    let (mut s, _) = started(three_steps());
    s.learn_next().unwrap();
    s.edit(vec![set(CUTOFF, 2800.0)], None).unwrap();
    let saved = s.save(None);
    let own = custom_patch().to_json();
    assert!(saved.instance.contains(r#""learn":{"lesson":{"pack":"basics","version":"1.0.0","lesson":"02-filter","step":1,"content":"0123456789abcdef"},"stash":""#), "{}", saved.instance);

    let mut t = Session::default();
    assert_eq!(t.load(&saved).errors, []);
    assert_eq!(
        t.learn_record().unwrap().stash.as_deref(),
        Some(own.as_str())
    );
    assert_eq!(cutoff(&t), 2800.0, "the patch field holds the step's patch");
    let v = t.lesson().unwrap();
    assert!(v.pending.opening);
    assert_eq!((v.lesson.step, v.steps), (1, 0));
    assert_eq!(t.learn_next(), Err(LearnError::NotReady));
    let open = learn_job(&drain_learn(&mut t), "open");
    match &open.kind {
        LearnJobKind::Open { lesson } => assert_eq!(lesson.step, 1),
        other => panic!("{other:?}"),
    }
    // The shell found the lesson: it resumes at its step without reloading the start (your work stays).
    t.game_tick(5_000, EditorView::Opened);
    let (fake, log) = Fake::new(three_steps());
    t.learn_job_done(open.id, LearnResult::Opened(fake.at_step(1)))
        .unwrap();
    assert_eq!(cutoff(&t), 2800.0);
    let v = t.lesson().unwrap();
    assert_eq!((v.lesson.step, v.steps, v.pending.opening), (1, 3, false));
    assert_eq!(
        log.lock().unwrap().edits.last().unwrap().1,
        EditSource::Load
    );
    assert_eq!(
        t.learn_job_done(
            open.id,
            LearnResult::Failed {
                message: String::new()
            }
        ),
        Err(LearnError::StaleJob(open.id))
    );
    t.learn_end(EndChoice::Restore).unwrap();
    assert_eq!(t.patch().to_json(), own);
    assert_eq!(t.instance().learn, None);
}

#[test]
fn a_lesson_that_cannot_be_found_again_keeps_its_stash() {
    let (mut s, _) = started(three_steps());
    s.learn_next().unwrap();
    let saved = s.save(None);
    let own = custom_patch().to_json();
    // The pack is gone: the lesson breaks, and Restore still brings your patch back.
    let mut t = Session::default();
    t.load(&saved);
    let open = learn_job(&drain_learn(&mut t), "open");
    t.learn_job_done(
        open.id,
        LearnResult::Failed {
            message: "no pack basics".to_owned(),
        },
    )
    .unwrap();
    let v = t.lesson().unwrap();
    assert!(v.broken);
    assert_eq!(t.learn_next(), Err(LearnError::Broken));
    t.learn_end(EndChoice::Restore).unwrap();
    assert_eq!(t.patch().to_json(), own);
    // The shell reopened another lesson: refused, broken, stash kept.
    let mut t = Session::default();
    t.load(&saved);
    let open = learn_job(&drain_learn(&mut t), "open");
    let (mut other, _) = Fake::new(three_steps());
    other.lesson.lesson = "03-env".to_owned();
    let e = t
        .learn_job_done(open.id, LearnResult::Opened(other))
        .unwrap_err();
    assert_eq!(e.code(), "other_lesson");
    assert!(t.lesson().unwrap().broken);
    t.learn_end(EndChoice::Restore).unwrap();
    assert_eq!(t.patch().to_json(), own);
    // Before the shell answers at all, Restore works too.
    let mut t = Session::default();
    t.load(&saved);
    t.learn_end(EndChoice::Restore).unwrap();
    assert_eq!(t.patch().to_json(), own);
    // A `learn` object whose lesson this build cannot read keeps its stash: broken, and Restore works.
    let instance = saved.instance.replacen(r#""step":1"#, r#""step":"one""#, 1);
    let mut t = Session::default();
    assert_eq!(
        t.load(&SavedState {
            patch: saved.patch.clone(),
            instance,
        })
        .errors,
        []
    );
    assert!(t.lesson().unwrap().broken);
    t.learn_end(EndChoice::Restore).unwrap();
    assert_eq!(t.patch().to_json(), own);
}

#[test]
fn an_updated_lesson_restarts_at_its_first_step() {
    let (mut s, _) = started(three_steps());
    s.learn_next().unwrap();
    s.edit(vec![set(CUTOFF, 2800.0)], None).unwrap();
    let saved = s.save(None);
    let mut t = Session::default();
    t.load(&saved);
    let open = learn_job(&drain_learn(&mut t), "open");
    // LM's resume restarts a changed lesson at step 0: the session loads that step's start.
    let (mut fake, _) = Fake::new(three_steps());
    fake.lesson.content = "fedcba9876543210".to_owned();
    t.learn_job_done(open.id, LearnResult::Opened(fake))
        .unwrap();
    assert_eq!(cutoff(&t), 2000.0);
    let lesson = t.learn_record().unwrap().lesson.clone().unwrap();
    assert_eq!(
        (lesson.step, lesson.content.as_str()),
        (0, "fedcba9876543210")
    );
}

#[test]
fn the_v1_fixtures_keep_their_bytes_without_a_lesson_and_after_one() {
    for (name, patch, instance) in common::v1_cases() {
        let mut s = Session::new(patch.clone());
        s.load_instance(&instance.to_json());
        let before = s.save(None);
        assert!(!before.instance.contains("learn"), "{name}");
        // Learn calls that start nothing change nothing.
        assert_eq!(s.learn_end(EndChoice::Restore), Err(LearnError::NoLesson));
        assert_eq!(s.learn_listen(Some(0)), Err(LearnError::NoLesson));
        assert_eq!(s.save(None), before, "{name}");
        // A whole lesson, ended with Restore, leaves the field as it was.
        s.game_tick(1_000, EditorView::Opened);
        let (fake, _) = Fake::new(three_steps());
        s.learn_start(fake).unwrap();
        s.learn_next().unwrap();
        s.learn_end(EndChoice::Restore).unwrap();
        assert_eq!(s.save(None), before, "{name}");
    }
}

/// The F1l budget: the largest lesson the budget allows (a reference at its cap, a stash at its cap, escaped at its
/// worst) together with the largest lesson round (every part at its cap, no stash) fits the 1 MiB `enth` field with
/// room for the rest of it, and loads back.
#[test]
fn the_largest_lesson_and_its_round_fit_the_enth_field() {
    use enth_session::game::{
        DISPUTE_CHARS, EDIT_LOG_BYTES, EditRecord, EditSource as RoundEdit, FIRST_REPORT_BYTES,
        GameState, MAX_SUBMITS, MAX_SUBMITTED_PATCH_BYTES, RoundId, RoundRecord, STASH_BYTES,
        ScoreSummary, SubmitRecord, escaped_len,
    };
    use enth_session::state::MAX_FIELD_BYTES;
    // Text whose escaped form is as long as the cap allows, from the worst character (6 bytes escaped).
    let worst = |cap: usize| "\u{1}".repeat((cap - 2) / 6);
    let stash = worst(STASH_BYTES);
    assert!(escaped_len(&stash) <= STASH_BYTES && escaped_len(&stash) > STASH_BYTES - 8);
    let mut lesson = LessonRef {
        step: u16::MAX,
        content: "f".repeat(16),
        ..LessonRef::default()
    };
    let base = serde_json::to_string(&lesson).unwrap().len();
    lesson.pack = "\u{1}".repeat((LESSON_REF_BYTES - base) / 6);
    let lesson_bytes = serde_json::to_string(&lesson).unwrap().len();
    assert!(lesson_bytes <= LESSON_REF_BYTES && lesson_bytes > LESSON_REF_BYTES - 8);

    let mut edits = Vec::new();
    let mut bytes = 0;
    loop {
        let ops = serde_json::json!([{"set": "filter1.cutoff", "to": 1234.5678}]);
        let size = ops.to_string().len() + 64;
        if bytes + size > EDIT_LOG_BYTES {
            break;
        }
        bytes += size;
        edits.push(EditRecord {
            t_ms: u64::MAX,
            source: RoundEdit::Reveal,
            ops,
        });
    }
    let round = RoundRecord {
        id: RoundId {
            serial: u32::MAX,
            nonce: u64::MAX,
        },
        code: "H000-G07Z-ZZZZ-ZZZZ-ZA4C-07ZZ-ZZG2-8XBS-N5J0-0A".to_owned(),
        phase: Phase::Submitted,
        stash: None,
        target_hash: Some("f".repeat(64)),
        active_ms: u64::MAX,
        submits: vec![
            SubmitRecord {
                at_ms: u64::MAX,
                score: ScoreSummary {
                    passed: true,
                    sound: 10_000,
                    structure: 10_000,
                    combined: 10_000,
                },
            };
            MAX_SUBMITS
        ],
        time_to_pass_ms: Some(u64::MAX),
        hints: u8::MAX,
        revealed: true,
        dim_unused: true,
        rating: Some(5),
        agreement: Some(enth_session::game::Agreement::AboutRight),
        dispute: Some("\u{1}".repeat(DISPUTE_CHARS)),
        first_report: Some(worst(FIRST_REPORT_BYTES)),
        submitted_patch: Some(worst(MAX_SUBMITTED_PATCH_BYTES)),
        edits,
        edits_dropped: u32::MAX,
        origin: RoundOrigin::Lesson,
    };
    let instance = InstanceState {
        ui: enth_session::UiState {
            layout: "x".repeat(64),
            ..enth_session::UiState::default()
        },
        game: Some(GameState {
            round: Some(round.clone()),
        }),
        preset: Some(enth_session::PresetState {
            last: Some("p".repeat(4096)),
        }),
        learn: Some(LearnState {
            lesson: Some(lesson),
            stash: Some(stash.clone()),
        }),
        ..InstanceState::default()
    };
    let text = instance.to_json();
    let learn_text = serde_json::to_string(instance.learn.as_ref().unwrap()).unwrap();
    let round_text = serde_json::to_string(&round).unwrap();
    eprintln!(
        "learn object {} B (budget {LEARN_BYTES}), lesson round {} B (budget {LESSON_ROUND_BYTES}), whole field {} B of {MAX_FIELD_BYTES}",
        learn_text.len(),
        round_text.len(),
        text.len()
    );
    assert!(learn_text.len() <= LEARN_BYTES);
    assert!(round_text.len() <= LESSON_ROUND_BYTES);
    assert!(
        text.len() + 64 * 1024 <= MAX_FIELD_BYTES,
        "{} bytes leave less than 64 KiB of the {MAX_FIELD_BYTES}",
        text.len()
    );
    let (back, _) = InstanceState::from_json(&text).unwrap();
    assert_eq!(back, instance);
    // And through a session: the field loads, the stash is there, and Restore brings it back as the patch text.
    let mut s = Session::default();
    let report = s.load_instance(&text);
    assert_eq!(report.errors, []);
    assert_eq!(
        s.learn_record().unwrap().stash.as_deref(),
        Some(stash.as_str())
    );
    assert_eq!(s.game_record().unwrap().origin, RoundOrigin::Lesson);
}

// ---------------------------------------------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_sound_check_runs_once_settled_and_a_stale_result_is_refused() {
    let steps = vec![step(Kind::Sound, Some(lesson_patch_with(3000.0)), &[])];
    let (mut s, _) = started(steps);
    drain_learn(&mut s);
    // Settled 300 ms after the step's load: the player asks, the session sends the job out.
    s.game_tick(1_100, EditorView::Visible);
    assert!(
        learn_jobs(&drain_learn(&mut s)).is_empty(),
        "not settled yet"
    );
    s.game_tick(1_300, EditorView::Visible);
    let job = learn_job(&drain_learn(&mut s), "check");
    assert!(s.lesson().unwrap().pending.checking);
    // An edit before the result: the result measured an older patch, so it is refused and changes nothing.
    s.edit(vec![set(CUTOFF, 2500.0)], None).unwrap();
    let verdict = LearnResult::Checked(CheckVerdict { passed: true });
    assert_eq!(
        s.learn_job_done(job.id, verdict),
        Err(LearnError::StaleJob(job.id))
    );
    assert_ne!(s.lesson().unwrap().phase, StepPhase::Passed);
    // The player asks again once the new patch settles; that result counts.
    s.game_tick(1_700, EditorView::Visible);
    let again = learn_job(&drain_learn(&mut s), "check");
    assert_ne!(again.id, job.id);
    s.learn_job_done(
        again.id,
        LearnResult::Checked(CheckVerdict { passed: true }),
    )
    .unwrap();
    assert_eq!(s.lesson().unwrap().phase, StepPhase::Passed);
    assert_eq!(records(&drain_learn(&mut s)).len(), 1);
    // A late, unknown or repeated result is stale.
    assert_eq!(
        s.learn_job_done(
            again.id,
            LearnResult::Checked(CheckVerdict { passed: false })
        ),
        Err(LearnError::StaleJob(again.id))
    );
}

#[test]
fn a_step_change_cancels_its_jobs_and_wrong_or_failed_results_are_handled() {
    let steps = vec![
        step(Kind::Sound, Some(lesson_patch_with(3000.0)), &[]),
        step(Kind::Read, None, &[]),
    ];
    let (mut s, _) = started(steps);
    s.game_tick(1_300, EditorView::Visible);
    let job = learn_job(&drain_learn(&mut s), "check");
    // The wrong kind of result: refused, the job still awaited.
    let e = s
        .learn_job_done(job.id, LearnResult::Examples { lufs: Vec::new() })
        .unwrap_err();
    assert_eq!(e.code(), "wrong_result");
    assert!(s.lesson().unwrap().pending.checking);
    // A failed check frees the slot and shows the error.
    s.learn_job_done(
        job.id,
        LearnResult::Failed {
            message: "render".to_owned(),
        },
    )
    .unwrap();
    let v = s.lesson().unwrap();
    assert!(!v.pending.checking);
    assert_eq!(v.error.as_ref().map(LearnError::code), Some("job_failed"));
    // A check out when the step moves is cancelled, and its result is stale.
    s.edit(vec![set(CUTOFF, 2900.0)], None).unwrap();
    s.game_tick(2_000, EditorView::Visible);
    let out = learn_job(&drain_learn(&mut s), "check");
    s.learn_next().unwrap();
    assert!(drain_learn(&mut s).contains(&LearnRequest::Cancel(out.id)));
    assert_eq!(
        s.learn_job_done(out.id, LearnResult::Checked(CheckVerdict { passed: true })),
        Err(LearnError::StaleJob(out.id))
    );
}

#[test]
fn hints_give_text_first_then_a_measured_job() {
    let (mut s, log) = started(vec![step(
        Kind::Do { below: 1000.0 },
        Some(lesson_patch_with(2000.0)),
        &[],
    )]);
    s.learn_hint().unwrap();
    assert_eq!(
        s.lesson().unwrap().hint.as_deref(),
        Some("Turn the cutoff down.")
    );
    s.learn_hint().unwrap();
    let job = learn_job(&drain_learn(&mut s), "hint");
    assert!(s.lesson().unwrap().pending.hinting);
    s.learn_job_done(
        job.id,
        LearnResult::Hinted(MeasuredHint {
            text: "lower the cutoff by an octave".to_owned(),
        }),
    )
    .unwrap();
    assert_eq!(log.lock().unwrap().hints_done, 1);
    assert_eq!(s.learn_hint(), Err(LearnError::NoHint));
}

#[test]
fn listen_examples_share_the_target_slot_and_are_level_matched() {
    let steps = vec![
        step(Kind::Choose { answer: 1 }, Some(lesson_patch()), &[]),
        step(Kind::Read, None, &[]),
    ];
    let (mut s, _) = started(steps);
    let requests = drain_learn(&mut s);
    let job = learn_job(&requests, "examples");
    match &job.kind {
        LearnJobKind::Examples { examples } => assert_eq!(examples.len(), 2),
        other => panic!("{other:?}"),
    }
    assert!(s.lesson().unwrap().pending.examples);
    assert_eq!(s.learn_listen(Some(0)), Err(LearnError::NoExample(0)));
    s.learn_job_done(
        job.id,
        LearnResult::Examples {
            lufs: vec![Some(-14.0), Some(-20.0)],
        },
    )
    .unwrap();
    s.learn_listen(Some(0)).unwrap();
    assert_eq!(s.learn_listen(Some(2)), Err(LearnError::NoExample(2)));
    let listen = s.lesson().unwrap().listen;
    assert_eq!(
        (listen.example, listen.ready, listen.gains_db.clone()),
        (Some(0), true, vec![-6.0, 0.0]),
        "the louder example is attenuated, the quieter never boosted"
    );
    let requests = drain_learn(&mut s);
    assert!(requests.contains(&LearnRequest::Listen(listen)));
    // The ear question: a wrong answer changes nothing, the right one passes.
    s.learn_choose(0).unwrap();
    assert_ne!(s.lesson().unwrap().phase, StepPhase::Passed);
    s.learn_choose(1).unwrap();
    assert_eq!(s.lesson().unwrap().phase, StepPhase::Passed);
    // Leaving the step drops the loops: the shell hears `ready: false`.
    s.learn_next().unwrap();
    let requests = drain_learn(&mut s);
    assert!(
        requests.contains(&LearnRequest::Listen(ListenState::default())),
        "{requests:?}"
    );
    let e = s.learn_choose(0).unwrap_err();
    assert_eq!(e.code(), "no_choice");
}

// ---------------------------------------------------------------------------------------------------------------
// The lesson round
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_recreate_step_is_a_real_round_that_never_stashes_and_logs_origin_lesson() {
    let steps = vec![step(Kind::Recreate, None, &[]), step(Kind::Read, None, &[])];
    let (mut s, log) = started(steps);
    let own = custom_patch().to_json();
    // The step starts a round from its challenge: the worker prepares that challenge.
    let v = s.round().unwrap();
    assert_eq!((v.phase, v.lesson), (Phase::Preparing, true));
    assert!(s.lesson().unwrap().round);
    let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
    match &prepare.kind {
        JobKind::PrepareChallenge { challenge: c } => assert_eq!(**c, challenge()),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        cutoff(&s),
        1234.5,
        "nothing loads yet: the fixture patch still plays"
    );
    s.game_job_done(prepare.id, prepared()).unwrap();
    // The round's start is the step's patch; the round holds no stash (the `learn` object holds the only one).
    assert_eq!(s.patch().to_json(), round_start().to_json());
    let record = s.game_record().unwrap();
    assert_eq!(
        (record.stash.clone(), record.origin),
        (None, RoundOrigin::Lesson)
    );
    assert_eq!(
        record.code,
        enth_session::game::ChallengeCode::short(spec())
            .unwrap()
            .to_string()
    );
    assert_eq!(
        s.learn_record().unwrap().stash.as_deref(),
        Some(own.as_str())
    );
    // The lesson's round moves with the lesson.
    assert_eq!(s.game_end(EndChoice::Restore), Err(GameError::LessonRound));
    assert_eq!(GameError::LessonRound.code(), "lesson_round");
    // A passing submit passes the step (the player's round_ended), and the round stays live.
    s.game_tick(2_000, EditorView::Visible);
    s.game_submit().unwrap();
    let score = game_job(&drain_game(&mut s), "score");
    s.game_job_done(score.id, JobResult::Scored(Box::new(report(95.0))))
        .unwrap();
    assert_eq!(s.round().unwrap().phase, Phase::Passed);
    assert_eq!(s.lesson().unwrap().phase, StepPhase::Passed);
    let rounds = log.lock().unwrap().rounds.clone();
    assert_eq!(
        rounds,
        [RoundEnd {
            passed: true,
            assisted: false,
            best_sound_pct: Some(95.0)
        }]
    );
    assert_eq!(records(&drain_learn(&mut s)).len(), 1);
    // Next ends the round: one `end` line with origin `lesson`, which gr1-report leaves out.
    s.learn_next().unwrap();
    assert_eq!(s.round(), None);
    let lines = logs(&drain_game(&mut s));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].origin, RoundOrigin::Lesson);
    let json = serde_json::to_string(&lines[0]).unwrap();
    assert!(json.ends_with(r#","origin":"lesson"}"#), "{json}");
    let back: RoundLogEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back, lines[0]);
    // Restore at the end brings your patch back (the round never stashed).
    s.learn_end(EndChoice::Restore).unwrap();
    assert_eq!(s.patch().to_json(), own);
}

#[test]
fn a_game_rounds_record_and_log_line_keep_their_bytes() {
    let mut s = Session::new(custom_patch());
    s.game_tick(0, EditorView::Opened);
    s.game_start(RoundSource::Settings(spec())).unwrap();
    let prepare = game_job(&drain_game(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared()).unwrap();
    assert_eq!(s.game_record().unwrap().origin, RoundOrigin::Game);
    assert!(!s.save(None).instance.contains("origin"));
    s.game_end(EndChoice::Restore).unwrap();
    let lines = logs(&drain_game(&mut s));
    let json = serde_json::to_string(&lines[0]).unwrap();
    assert!(!json.contains("origin"), "{json}");
    // A line without the field (every line written before it) reads as a game round ...
    let back: RoundLogEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.origin, RoundOrigin::Game);
    // ... and a newer build's origin reads as `other`: not a game round either.
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["origin"] = serde_json::Value::from("duel");
    let back: RoundLogEntry = serde_json::from_value(value).unwrap();
    assert_eq!(back.origin, RoundOrigin::Other);
}

#[test]
fn a_challenge_round_outside_a_lesson_stashes_like_any_round() {
    let mut s = Session::new(custom_patch());
    s.game_tick(0, EditorView::Opened);
    s.game_start(RoundSource::Challenge(Box::new(challenge())))
        .unwrap();
    let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
    // The worker must answer with the challenge it was asked for.
    let mut other = challenge();
    other.phrase = PhraseId(2);
    let wrong = JobResult::Prepared(Box::new(Prepared::new(
        other,
        round_start(),
        round_target(),
        vec![addr(CUTOFF)],
        fingerprint(),
        None,
    )));
    s.game_job_done(prepare.id, wrong).unwrap();
    assert_eq!(s.round().unwrap().phase, Phase::Broken);
    s.game_end(EndChoice::Restore).unwrap();
    s.game_start(RoundSource::Challenge(Box::new(challenge())))
        .unwrap();
    let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
    s.game_job_done(prepare.id, prepared()).unwrap();
    let record = s.game_record().unwrap();
    assert_eq!(record.origin, RoundOrigin::Game);
    assert_eq!(
        record.stash.as_deref(),
        Some(custom_patch().to_json().as_str())
    );
}

#[test]
fn a_lesson_round_resumes_after_a_reload_from_its_lesson() {
    let steps = vec![
        step(Kind::Recreate, None, &["filter1.resonance"]),
        step(Kind::Read, None, &[]),
    ];
    let (mut s, _) = started(steps.clone());
    let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
    s.game_job_done(prepare.id, prepared()).unwrap();
    s.edit(vec![set(CUTOFF, 900.0)], None).unwrap();
    s.game_tick(3_000, EditorView::Visible);
    s.game_submit().unwrap();
    let score = game_job(&drain_game(&mut s), "score");
    s.game_job_done(score.id, JobResult::Scored(Box::new(report(40.0))))
        .unwrap();
    let saved = s.save(None);
    assert!(
        saved.instance.contains(r#""origin":"lesson""#),
        "{}",
        saved.instance
    );

    let mut t = Session::default();
    assert_eq!(t.load(&saved).errors, []);
    // The round waits for its lesson: no prepare job of its own, no code to decode.
    let v = t.round().unwrap();
    assert_eq!((v.phase, v.lesson, v.submits), (Phase::Submitted, true, 1));
    assert_eq!(game_job_kinds(&drain_game(&mut t)), Vec::<&str>::new());
    let open = learn_job(&drain_learn(&mut t), "open");
    let (fake, _) = Fake::new(steps);
    t.learn_job_done(open.id, LearnResult::Opened(fake))
        .unwrap();
    // The lesson hands the challenge back; the round is prepared again, under its own id, and resumes.
    let prepare = game_job(&drain_game(&mut t), "prepare_challenge");
    assert_eq!(prepare.round, t.game_record().unwrap().id);
    t.game_job_done(prepare.id, prepared()).unwrap();
    assert_eq!(cutoff(&t), 900.0, "the round's patch stays");
    assert_eq!(t.round().unwrap().submits, 1, "and its submits");
    assert_eq!(
        t.locks(),
        &["filter1.resonance".parse::<Selector>().unwrap()]
    );
    assert!(t.edit(vec![set("filter1.resonance", 60.0)], None).is_err());

    // Without its lesson (the `learn` object gone), the round breaks; ending it changes nothing.
    let instance = saved
        .instance
        .split(r#","learn":"#)
        .next()
        .unwrap()
        .to_owned()
        + "}";
    let mut t = Session::default();
    assert_eq!(
        t.load(&SavedState {
            patch: saved.patch.clone(),
            instance,
        })
        .errors,
        []
    );
    assert_eq!(t.round().unwrap().phase, Phase::Broken);
    t.game_end(EndChoice::Restore).unwrap();
    assert_eq!(cutoff(&t), 900.0);
}

fn game_job_kinds(requests: &[GameRequest]) -> Vec<&'static str> {
    requests
        .iter()
        .filter_map(|r| match r {
            GameRequest::Run(job) => Some(job.kind.name()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// The engine: one plan per step load, and no audio-thread contact
// ---------------------------------------------------------------------------------------------------------------

const ACTIVATION: Activation = Activation {
    sample_rate: 48_000.0,
    offline: false,
};

fn run(engine: &mut Engine, blocks: usize) {
    let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
    for _ in 0..blocks {
        engine.process(&mut l, &mut r, &[]);
    }
}

/// The engine's plan swaps and the session's epoch, after the engine caught up.
fn plans(s: &mut Session, engine: &mut Engine) -> (u64, u64) {
    s.pump();
    run(engine, 4);
    s.pump();
    (engine.stats().swaps, s.status().epoch)
}

#[test]
fn each_step_load_is_one_plan_and_nothing_else_reaches_the_engine() {
    let steps = vec![
        step(
            Kind::Do { below: 1000.0 },
            Some(lesson_patch_with(2000.0)),
            &[],
        ),
        step(Kind::Recreate, None, &[]),
        step(Kind::Sound, Some(lesson_patch_with(3000.0)), &[]),
        step(Kind::Read, None, &[]),
    ];
    let mut s = Session::new(custom_patch());
    let mut engine = s.activate(ACTIVATION).unwrap();
    let note = Event {
        offset: 0,
        kind: EventKind::NoteOn {
            key: NoteKey::new(60, 1),
            velocity: 0.8,
        },
    };
    engine.process(&mut [0.0; 64], &mut [0.0; 64], &[note]);
    let idle = plans(&mut s, &mut engine);
    s.game_tick(0, EditorView::Opened);
    let (fake, _) = Fake::new(steps);
    s.learn_start(fake).unwrap();
    let one = plans(&mut s, &mut engine);
    assert_eq!(
        one,
        (idle.0 + 1, idle.1 + 1),
        "the first step's start is one plan"
    );
    // Hints, ticks and job traffic leave the engine alone.
    s.learn_hint().unwrap();
    s.game_tick(500, EditorView::Visible);
    drain_learn(&mut s);
    assert_eq!(plans(&mut s, &mut engine), one);

    // A Recreate step: nothing until the round is prepared, then its start: one plan.
    s.learn_next().unwrap();
    assert_eq!(
        plans(&mut s, &mut engine),
        one,
        "the step asks the worker, not the engine"
    );
    let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
    s.game_job_done(prepare.id, prepared()).unwrap();
    let two = plans(&mut s, &mut engine);
    assert_eq!(
        two,
        (one.0 + 1, one.1 + 1),
        "the round's start is the step's one plan"
    );

    // The next step ends the round (no plan) and seats its start: one plan.
    s.learn_next().unwrap();
    let three = plans(&mut s, &mut engine);
    assert_eq!(three, (two.0 + 1, two.1 + 1));
    s.game_tick(900, EditorView::Visible);
    let check = learn_job(&drain_learn(&mut s), "check");
    s.learn_job_done(
        check.id,
        LearnResult::Checked(CheckVerdict { passed: true }),
    )
    .unwrap();
    assert_eq!(plans(&mut s, &mut engine), three);

    // A step without a start: no plan. Back to a step with one: one plan. Restore: one plan.
    s.learn_next().unwrap();
    assert_eq!(plans(&mut s, &mut engine), three);
    s.learn_back().unwrap();
    let back = plans(&mut s, &mut engine);
    assert_eq!(back, (three.0 + 1, three.1 + 1));
    s.learn_end(EndChoice::Restore).unwrap();
    assert_eq!(plans(&mut s, &mut engine), (back.0 + 1, back.1 + 1));
    assert_eq!(engine.stats().dropped_events, 0);
    assert_eq!(engine.stats().stale_events, 0);
}

/// No audio-thread contact: while a test thread stands in for the host's audio thread, rendering flat out with
/// every `process()` inside `assert_no_alloc`, the main thread plays whole lessons (step loads, edits, a lesson
/// round, locked edits refused, Restore). The audio thread never waits on the session and never allocates: a lesson
/// reaches it only as plans. Debug builds only (`assert_no_alloc` drops its checks in release).
#[cfg(debug_assertions)]
#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "a test thread stands in for the host's audio thread"
)]
fn lessons_never_stop_the_audio_thread_or_make_it_allocate() {
    use assert_no_alloc::assert_no_alloc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    let mut s = Session::new(custom_patch());
    let own = s.patch().to_json();
    let mut engine = s.activate(ACTIVATION).unwrap();
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

        // A failed assertion below must stop the audio thread too, or the scope would wait for it forever.
        let stop_guard = StopOnDrop(&stop);
        let mut last = 0;
        let mut step_on = |s: &mut Session, k: u32| {
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
        let mut now = 0u64;
        for lesson in 0..6u32 {
            now += 1_000;
            s.game_tick(now, EditorView::Opened);
            let steps = vec![
                step(
                    Kind::Do { below: 1000.0 },
                    Some(lesson_patch_with(2000.0)),
                    &["filter1.resonance"],
                ),
                step(Kind::Recreate, None, &[]),
                step(Kind::Sound, Some(lesson_patch_with(3000.0)), &[]),
            ];
            let (fake, _) = Fake::new(steps);
            s.learn_start(fake).unwrap(); // a structural plan swap
            step_on(&mut s, lesson * 10);
            for k in 0..4 {
                s.edit(vec![set(CUTOFF, 1900.0 - 300.0 * k as f32)], None)
                    .unwrap();
                assert!(s.edit(vec![set("filter1.resonance", 70.0)], None).is_err());
                step_on(&mut s, lesson * 10 + 1);
            }
            s.learn_next().unwrap();
            let prepare = game_job(&drain_game(&mut s), "prepare_challenge");
            s.game_job_done(prepare.id, prepared()).unwrap(); // the round's start
            step_on(&mut s, lesson * 10 + 2);
            s.learn_next().unwrap(); // the round ends; the sound step's start
            step_on(&mut s, lesson * 10 + 3);
            now += 400;
            s.game_tick(now, EditorView::Visible);
            for request in drain_learn(&mut s) {
                if let LearnRequest::Run(job) = request {
                    let _ = s.learn_job_done(
                        job.id,
                        LearnResult::Checked(CheckVerdict { passed: true }),
                    );
                }
            }
            s.learn_end(EndChoice::Restore).unwrap(); // your patch: a structural plan swap
            drain_game(&mut s);
            s.drain_requests().for_each(drop);
            step_on(&mut s, lesson * 10 + 4);
        }
        drop(stop_guard);
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
    assert_eq!(engine.stats().dropped_events, 0);
}

/// Sets the flag when dropped, also while a panic unwinds.
#[cfg(debug_assertions)]
struct StopOnDrop<'a>(&'a std::sync::atomic::AtomicBool);

#[cfg(debug_assertions)]
impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[test]
fn the_learn_api_errors_have_codes_and_hints() {
    let cases = [
        LearnError::NoLesson,
        LearnError::RoundLive,
        LearnError::NotReady,
        LearnError::Broken,
        LearnError::TooLarge {
            what: "stash",
            bytes: 1,
            max: 0,
        },
        LearnError::NoHint,
        LearnError::NothingToShow,
        LearnError::NoExample(3),
    ];
    for e in cases {
        assert!(!e.code().is_empty() && !e.hint().is_empty() && !e.to_string().is_empty());
    }
}
