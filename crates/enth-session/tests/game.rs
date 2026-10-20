//! The Recreate round through the public API (lane SG): the lifecycle, A/B, submit, hints, reveal, give up, the
//! injected clock, undo, host automation and loads, and the saved round (docs/session.md, "The Recreate round").
//! The shell is played by hand: each test drains the game outbox and answers the jobs it asks for. No wall clock and
//! no random seed anywhere: every timestamp is a literal.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use enth_challenge::score::{ModuleTerm, ParamScore};
use enth_challenge::{
    Combined, Direction, EngineVersion, PhraseId, RoundResult, ScoringSpec, SoundScore, SpecPreset,
    TargetFingerprint, TargetPrefix,
};
use enth_engine::{Engine, Event, EventKind, NoteKey};
use enth_patch::canon::exact_hash;
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModuleKind, Op, Patch, Position, apply_all};
use enth_session::game::{
    Agreement, Challenge, ChallengeCode, EditSource, EditorView, EndChoice, EndedBy, GameEdit,
    GameError, GameRequest, Job, JobId, JobKind, JobResult, Listen, LogEvent, Outcome, Phase,
    Prepared, RandomSpec, RoundLogEntry, RoundSource, RoundView, ScoreReport, StructureScore,
    Verdict,
};
use enth_session::state::STATE_VERSION;
use enth_session::{
    Activation, GameState, HostValues, InstanceState, SavedState, Session, ShellRequest,
};

use common::{addr, custom_patch, id, init_patch, macro_index};

const CUTOFF: &str = "filter1.cutoff";
const RESONANCE: &str = "filter1.resonance";

// ---------------------------------------------------------------------------------------------------------------
// The shell, by hand
// ---------------------------------------------------------------------------------------------------------------

fn set(a: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr(a),
        value,
    }
}

/// The start patch a generator would give: the init patch plus a filter, no slots.
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

/// The target: the start with another cutoff and resonance.
fn target_patch() -> Patch {
    let mut p = start_patch();
    apply_all(&mut p, &[set(CUTOFF, 800.0), set(RESONANCE, 40.0)]).unwrap();
    p
}

/// Level 2 (a preset, so a short code) with generator 1 and `seed`.
fn spec_seed(seed: u64) -> RandomSpec {
    SpecPreset::by_id(2).unwrap().spec(1, seed)
}

fn spec() -> RandomSpec {
    spec_seed(42)
}

fn source() -> RoundSource {
    RoundSource::Settings(spec())
}

/// The round's code: level 2's short code for seed 42.
fn code() -> String {
    ChallengeCode::short(spec()).unwrap().to_string()
}

/// Custom settings (not a preset): their code is the long form, made once the target exists.
fn custom_spec() -> RandomSpec {
    RandomSpec {
        musical: 80,
        ..spec()
    }
}

/// The challenge's scoring (this build's default scorers), which every worker report must match.
fn scoring() -> ScoringSpec {
    ScoringSpec::recreate(spec().difficulty)
}

/// Lane ST's structure score (from the challenge's structure scorer) with one ring on the cutoff.
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

/// A report with this sound % (it passes at the default pass mark, 90).
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

/// A fake target's fingerprint (a real worker passes `prepare_round`'s): different targets, different prints.
fn fingerprint_of(target: &Patch) -> TargetFingerprint {
    TargetFingerprint(exact_hash(target).0)
}

fn fingerprint() -> TargetFingerprint {
    fingerprint_of(&target_patch())
}

/// The randomized parameters of the fake target.
fn params() -> Vec<enth_patch::ParamAddr> {
    vec![addr(CUTOFF), addr(RESONANCE)]
}

/// A prepare job's answer: `challenge` over the start and `target`.
fn prepared_with(challenge: Challenge, target: Patch, target_lufs: Option<f32>) -> JobResult {
    let print = fingerprint_of(&target);
    JobResult::Prepared(Box::new(Prepared::new(
        challenge,
        start_patch(),
        target,
        params(),
        print,
        target_lufs,
    )))
}

fn prepared_for(spec: RandomSpec) -> JobResult {
    prepared_with(
        Challenge::recreate(spec, PhraseId(1)),
        target_patch(),
        Some(-14.0),
    )
}

fn prepared() -> JobResult {
    prepared_for(spec())
}

/// The round-log line's round result (lane CT's `enth.round-result/1`), checked to read back as one.
fn result(line: &RoundLogEntry) -> RoundResult {
    let result = line.result.clone().expect("the code decodes");
    let json = serde_json::to_string(&result).unwrap();
    assert_eq!(
        serde_json::from_str::<RoundResult>(&json).unwrap(),
        result,
        "a valid round result"
    );
    result
}

fn drain(s: &mut Session) -> Vec<GameRequest> {
    s.drain_game_requests().collect()
}

fn shell_requests(s: &mut Session) -> Vec<ShellRequest> {
    s.drain_requests().collect()
}

fn runs(requests: &[GameRequest]) -> Vec<Job> {
    requests
        .iter()
        .filter_map(|r| match r {
            GameRequest::Run(job) => Some(job.clone()),
            _ => None,
        })
        .collect()
}

/// The one job of `kind` the session asked for.
fn job(requests: &[GameRequest], kind: &str) -> Job {
    let found: Vec<Job> = runs(requests)
        .into_iter()
        .filter(|j| j.kind.name() == kind)
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

fn view(s: &Session) -> RoundView {
    s.round().expect("a round is live")
}

/// Start a round at shell time `now` (the editor open) and answer its prepare job.
fn play(s: &mut Session, now: u64) {
    s.game_tick(now, EditorView::Opened);
    s.game_start(source()).unwrap();
    let prepare = job(&drain(s), "prepare");
    s.game_job_done(prepare.id, prepared()).unwrap();
}

/// Submit, and answer the score job with `report`. Returns the submit's job.
fn submit(s: &mut Session, report: ScoreReport) -> Job {
    s.game_submit().unwrap();
    let score = job(&drain(s), "score");
    s.game_job_done(score.id, JobResult::Scored(Box::new(report)))
        .unwrap();
    score
}

fn value(s: &Session, a: &str) -> f32 {
    s.patch().value(&addr(a)).unwrap()
}

// ---------------------------------------------------------------------------------------------------------------
// The lifecycle
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_round_runs_from_start_to_pass_and_back_to_the_players_patch() {
    let mut s = Session::new(custom_patch());
    let before = s.save(None);
    shell_requests(&mut s);
    assert_eq!(s.round(), None);

    s.game_tick(1_000, EditorView::Opened);
    s.game_start(source()).unwrap();
    // Preparing: the player's patch still plays, and the target is asked for.
    let v = view(&s);
    assert_eq!(
        (v.phase, v.pending.preparing, v.code.clone(), v.difficulty),
        (Phase::Preparing, true, code(), Some(25))
    );
    assert_eq!(s.patch().to_json(), before.patch);
    let requests = drain(&mut s);
    let prepare = job(&requests, "prepare");
    assert!(matches!(&prepare.kind, JobKind::Prepare { spec: s } if *s == spec()));
    assert_eq!(prepare.round, s.game_record().unwrap().id);
    assert!(requests.contains(&GameRequest::Dirty));
    assert_eq!(s.game_submit(), Err(GameError::NotReady));

    // Prepared: the stash is the player's patch, the start loads like a load, the round has its own history.
    s.game_job_done(prepare.id, prepared()).unwrap();
    assert_eq!(view(&s).phase, Phase::Playing);
    assert_eq!(s.patch().to_json(), start_patch().to_json());
    assert_eq!(
        s.game_record().unwrap().stash.as_deref(),
        Some(before.patch.as_str())
    );
    assert!(!s.history().can_undo());
    let shell = shell_requests(&mut s);
    assert!(shell.contains(&ShellRequest::PatchEdited), "{shell:?}");
    assert!(
        shell.contains(&ShellRequest::RenameParams),
        "the slots changed: {shell:?}"
    );

    // Five seconds of play, an edit, a failing submit.
    s.game_tick(6_000, EditorView::Visible);
    assert_eq!(view(&s).active_ms, 5_000);
    s.edit(vec![set(CUTOFF, 900.0)], None).unwrap();
    assert_eq!(s.game_submit(), Ok(1));
    assert_eq!(
        s.game_submit(),
        Err(GameError::Busy { what: "score" }),
        "one score at a time"
    );
    let score = job(&drain(&mut s), "score");
    match &score.kind {
        JobKind::Score {
            patch,
            submit,
            target,
            ..
        } => {
            assert_eq!(*submit, 1);
            assert_eq!(patch.to_json(), s.patch().to_json());
            assert_eq!(target.to_json(), target_patch().to_json());
        }
        other => panic!("{other:?}"),
    }
    // The player keeps editing while the score is computed: the job scores the patch at Submit.
    s.edit(vec![set(CUTOFF, 850.0)], None).unwrap();
    s.game_job_done(score.id, JobResult::Scored(Box::new(report(41.0))))
        .unwrap();
    let v = view(&s);
    assert_eq!(v.phase, Phase::Submitted);
    assert_eq!(v.first, v.last);
    assert_eq!(v.first.unwrap().at_ms, 5_000);
    assert_eq!(v.first.unwrap().score.sound, 4100);
    assert_eq!(
        v.knobs[0].verdict,
        Verdict::Bad,
        "the rings show after the first submit"
    );
    assert_eq!(v.time_to_pass_ms, None);

    // Two more seconds, a passing submit: the time to pass is its active time.
    s.game_tick(8_000, EditorView::Visible);
    submit(&mut s, report(92.5));
    let v = view(&s);
    assert_eq!(
        (v.phase, v.time_to_pass_ms, v.submits),
        (Phase::Passed, Some(7_000), 2)
    );
    assert_eq!(
        v.first.unwrap().score.sound,
        4100,
        "the first submit stays the recorded score"
    );
    assert_eq!(v.last.unwrap().score.sound, 9250);
    assert!(!v.assisted);

    // A later passing submit does not move the time.
    s.game_tick(9_000, EditorView::Visible);
    submit(&mut s, report(95.0));
    assert_eq!(view(&s).time_to_pass_ms, Some(7_000));

    s.game_vote(Agreement::AboutRight).unwrap();
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.round(), None);
    let requests = drain(&mut s);
    let lines = logs(&requests);
    assert_eq!(lines.len(), 1);
    let line = &lines[0];
    assert_eq!(line.schema, "enth.round-log/1");
    assert_eq!(
        (line.event, line.ended_by, line.broken),
        (LogEvent::End, Some(EndedBy::Restore), false)
    );
    let r = result(line);
    assert_eq!(r.outcome, Outcome::Passed);
    assert_eq!((r.active_ms, r.time_to_pass_ms), (8_000, Some(7_000)));
    assert_eq!((r.submits, line.submits.len()), (3, 3));
    assert_eq!(
        r.first_submit.unwrap().sound.percent,
        41.0,
        "the recorded score is the first"
    );
    assert_eq!(r.agreement, Some(Agreement::AboutRight));
    assert_eq!((r.code.to_string(), line.code.clone()), (code(), code()));
    assert!(!r.assists.any());
    assert!(line.report.is_some() && line.patch.is_some());
    // Back to exactly the player's patch, and both saved fields are byte for byte what they were.
    assert_eq!(s.save(None), before);
}

#[test]
fn reveal_loads_the_target_marks_the_round_assisted_and_ends_submits() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.edit(vec![set(CUTOFF, 3000.0)], None).unwrap();
    shell_requests(&mut s);
    s.game_reveal().unwrap();
    assert_eq!(s.patch().to_json(), target_patch().to_json());
    assert!(shell_requests(&mut s).contains(&ShellRequest::PatchEdited));
    let v = view(&s);
    assert_eq!(
        (v.phase, v.revealed, v.assisted),
        (Phase::Revealed, true, true)
    );
    assert!(
        !s.history().can_undo(),
        "a reveal starts a new line (an undoable reveal is PBC's load)"
    );
    assert_eq!(s.game_submit(), Err(GameError::Revealed));
    assert_eq!(s.game_hint(), Err(GameError::Revealed));
    assert_eq!(s.game_vote(Agreement::TooLow), Err(GameError::NotScored));
    let edits = &s.game_record().unwrap().edits;
    assert_eq!(edits.last().unwrap().source, EditSource::Reveal);

    s.game_end(EndChoice::Keep).unwrap();
    let r = result(&logs(&drain(&mut s))[0]);
    assert_eq!(
        (r.outcome, r.assists.revealed, r.assists.any()),
        (Outcome::GaveUp, true, true)
    );
    assert_eq!(
        s.patch().to_json(),
        target_patch().to_json(),
        "Keep keeps the round's patch"
    );
}

#[test]
fn a_submit_in_flight_when_revealing_still_counts() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.game_tick(4_000, EditorView::Visible);
    s.game_submit().unwrap();
    let score = job(&drain(&mut s), "score");
    s.game_reveal().unwrap();
    s.game_job_done(score.id, JobResult::Scored(Box::new(report(90.0))))
        .unwrap();
    let v = view(&s);
    assert_eq!((v.phase, v.time_to_pass_ms), (Phase::Revealed, Some(4_000)));
}

#[test]
fn hints_are_levels_that_mark_the_round_assisted() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    drain(&mut s);
    for level in 1..=4u8 {
        assert_eq!(s.game_hint(), Ok(level));
        assert_eq!(s.game_hint(), Err(GameError::Busy { what: "hint" }));
        let hint = job(&drain(&mut s), "hint");
        assert!(matches!(hint.kind, JobKind::Hint { level: l, .. } if l == level));
        let text = format!("level {level}");
        s.game_job_done(
            hint.id,
            JobResult::Hint(enth_session::game::CoachHint::new(level, text.clone())),
        )
        .unwrap();
        let v = view(&s);
        assert_eq!((v.hints, v.assisted), (level, true));
        assert_eq!(v.hint.unwrap().text, text);
    }
    assert_eq!(s.game_hint(), Err(GameError::NoMoreHints));
}

#[test]
fn a_new_round_keeps_the_original_stash_and_logs_the_old_one() {
    let mut s = Session::new(custom_patch());
    let own = s.patch().to_json();
    play(&mut s, 0);
    s.edit(vec![set(CUTOFF, 2000.0)], None).unwrap();
    s.game_tick(3_000, EditorView::Visible);
    s.game_start(RoundSource::Settings(spec_seed(43))).unwrap();
    let requests = drain(&mut s);
    let line = &logs(&requests)[0];
    assert_eq!(
        (result(line).outcome, line.ended_by, line.code.clone()),
        (Outcome::NotPassed, Some(EndedBy::NewRound), code())
    );
    assert!(
        requests.iter().any(|r| matches!(r, GameRequest::Cancel(_))),
        "the old loudness job is cancelled"
    );
    let prepare = job(&requests, "prepare");
    assert_eq!(
        view(&s).code,
        ChallengeCode::short(spec_seed(43)).unwrap().to_string()
    );
    assert_eq!(view(&s).active_ms, 0);
    assert_eq!(s.game_record().unwrap().id.serial, 2);
    s.game_job_done(prepare.id, prepared_for(spec_seed(43)))
        .unwrap();
    assert_eq!(
        s.game_record().unwrap().stash.as_deref(),
        Some(own.as_str())
    );
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.patch().to_json(), own);
}

#[test]
fn ending_a_round_that_never_started_logs_nothing_and_changes_nothing() {
    let mut s = Session::new(custom_patch());
    let own = s.patch().to_json();
    s.game_start(source()).unwrap();
    let prepare = job(&drain(&mut s), "prepare");
    s.game_end(EndChoice::Restore).unwrap();
    let requests = drain(&mut s);
    assert_eq!(logs(&requests), []);
    assert!(requests.contains(&GameRequest::Cancel(prepare.id)));
    assert_eq!(s.patch().to_json(), own);
    assert_eq!(
        s.game_job_done(prepare.id, prepared()),
        Err(GameError::StaleJob(prepare.id))
    );
    assert_eq!(s.patch().to_json(), own, "a late result changes nothing");
    assert_eq!(s.game_end(EndChoice::Keep), Err(GameError::NoRound));
}

#[test]
fn giving_up_without_a_submit_logs_an_abandoned_round() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.game_tick(2_500, EditorView::Visible);
    assert_eq!(s.game_dispute("too hard"), Err(GameError::NotScored));
    s.game_end(EndChoice::Restore).unwrap();
    let r = result(&logs(&drain(&mut s))[0]);
    assert_eq!(
        (r.outcome, r.active_ms, r.submits, r.first_submit),
        (Outcome::NotPassed, 2_500, 0, None)
    );
}

#[test]
fn a_dispute_is_logged_at_once_and_again_with_the_end() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    submit(&mut s, report(30.0));
    s.game_vote(Agreement::TooLow).unwrap();
    s.game_vote(Agreement::TooHigh).unwrap();
    let note = "x".repeat(5_000);
    s.game_dispute(&note).unwrap();
    let line = logs(&drain(&mut s)).pop().unwrap();
    assert_eq!(
        (line.event, result(&line).outcome, line.ended_by),
        (LogEvent::Dispute, Outcome::NotPassed, None)
    );
    assert_eq!(
        line.dispute.as_ref().unwrap().chars().count(),
        2_000,
        "bounded"
    );
    assert_eq!(
        result(&line).agreement,
        Some(Agreement::TooHigh),
        "a later vote replaces the earlier one"
    );
    s.game_end(EndChoice::Keep).unwrap();
    let end = logs(&drain(&mut s)).pop().unwrap();
    assert_eq!(
        (end.event, end.dispute.map(|d| d.len())),
        (LogEvent::End, Some(2_000))
    );
}

#[test]
fn stale_wrong_and_failed_results_are_handled() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.game_submit().unwrap();
    let score = job(&drain(&mut s), "score");
    // A result of the wrong kind is refused and the job is still awaited.
    assert_eq!(
        s.game_job_done(
            score.id,
            JobResult::Rings(Box::new(structure(0.0, Verdict::Bad)))
        ),
        Err(GameError::WrongResult {
            job: "score",
            result: "rings"
        })
    );
    assert_eq!(
        s.game_job_done(JobId(999), prepared()),
        Err(GameError::StaleJob(JobId(999)))
    );
    // A failed score records no submit and allows another.
    s.game_job_done(
        score.id,
        JobResult::Failed {
            message: "worker died".to_owned(),
        },
    )
    .unwrap();
    let v = view(&s);
    assert_eq!((v.phase, v.submits), (Phase::Playing, 0));
    assert_eq!(v.error.unwrap().code(), "job_failed");
    assert_eq!(s.game_submit(), Ok(1));
}

#[test]
fn a_failed_preparation_breaks_the_round_and_ending_it_restores_the_stash() {
    let mut s = Session::new(custom_patch());
    let own = s.patch().to_json();
    play(&mut s, 0);
    // A second round whose target cannot be prepared: the stash carried over must survive.
    s.game_start(RoundSource::Settings(spec_seed(43))).unwrap();
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(
        prepare.id,
        JobResult::Failed {
            message: "no generator".to_owned(),
        },
    )
    .unwrap();
    let v = view(&s);
    assert_eq!(v.phase, Phase::Broken);
    assert_eq!(s.game_submit(), Err(GameError::Broken));
    assert_eq!(s.game_listen(Listen::Mine), Err(GameError::Broken));
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.patch().to_json(), own);
}

// ---------------------------------------------------------------------------------------------------------------
// Codes and settings (lane CT's types)
// ---------------------------------------------------------------------------------------------------------------

fn long_code(spec: RandomSpec, engine: EngineVersion, prefix: TargetPrefix) -> ChallengeCode {
    ChallengeCode::long(spec, engine, prefix).unwrap()
}

fn target_prefix() -> TargetPrefix {
    fingerprint().prefix()
}

#[test]
fn custom_settings_get_their_long_code_once_the_target_exists() {
    assert_eq!(ChallengeCode::short(custom_spec()), None, "not a preset");
    let mut s = Session::new(custom_patch());
    s.game_start(RoundSource::Settings(custom_spec())).unwrap();
    assert_eq!(
        view(&s).code,
        "",
        "the long code carries the target's hash prefix"
    );
    let prepare = job(&drain(&mut s), "prepare");
    assert!(matches!(prepare.kind, JobKind::Prepare { spec } if spec == custom_spec()));
    s.game_job_done(prepare.id, prepared_for(custom_spec()))
        .unwrap();
    let long = long_code(custom_spec(), EngineVersion::CURRENT, target_prefix());
    assert_eq!(view(&s).code, long.to_string());
    assert_eq!(view(&s).phase, Phase::Playing);
    // Saved and reopened, the round regenerates from its long code.
    let saved = s.save(None);
    let mut t = Session::default();
    t.load(&saved);
    let prepare = job(&drain(&mut t), "prepare");
    assert!(matches!(prepare.kind, JobKind::Prepare { spec } if spec == custom_spec()));
    t.game_job_done(prepare.id, prepared_for(custom_spec()))
        .unwrap();
    assert_eq!(
        (view(&t).phase, view(&t).warnings.len()),
        (Phase::Playing, 0)
    );
}

#[test]
fn a_pasted_long_code_whose_target_differs_is_refused() {
    let mut s = Session::new(custom_patch());
    let own = s.patch().to_json();
    let code = long_code(
        custom_spec(),
        EngineVersion::CURRENT,
        TargetPrefix(0xDEAD_BEEF),
    );
    s.game_start(RoundSource::Code(code)).unwrap();
    assert_eq!(
        view(&s).code,
        code.to_string(),
        "the code text the round started from"
    );
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared_for(custom_spec()))
        .unwrap();
    let v = view(&s);
    assert_eq!(v.phase, Phase::Broken);
    assert_eq!(v.error.unwrap().code(), "target_mismatch");
    assert_eq!(
        s.patch().to_json(),
        own,
        "never scored against a different target"
    );
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.patch().to_json(), own);
    assert_eq!(logs(&drain(&mut s)), [], "never played");
}

#[test]
fn a_code_made_with_another_engine_warns_and_plays() {
    let mut s = Session::new(custom_patch());
    let other = EngineVersion {
        major: 0,
        minor: 1,
        patch: 0,
    };
    assert_ne!(other, EngineVersion::CURRENT);
    let code = long_code(custom_spec(), other, target_prefix());
    s.game_start(RoundSource::Code(code)).unwrap();
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared_for(custom_spec()))
        .unwrap();
    let v = view(&s);
    assert_eq!(v.phase, Phase::Playing);
    assert_eq!(v.warnings.len(), 1);
    assert_eq!(v.warnings[0].code(), "engine_differs");
    assert_eq!(
        v.code,
        code.to_string(),
        "the maker's engine version stays in the code"
    );
}

#[test]
fn refused_settings_change_nothing() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    drain(&mut s);
    let bad = RandomSpec {
        difficulty: 101,
        ..spec()
    };
    let err = s.game_start(RoundSource::Settings(bad)).unwrap_err();
    assert_eq!(err.code(), "out_of_range");
    let newer = RandomSpec {
        generator: 999,
        ..spec()
    };
    assert_eq!(
        s.game_start(RoundSource::Settings(newer))
            .unwrap_err()
            .code(),
        "code_too_new"
    );
    assert_eq!(
        (view(&s).phase, view(&s).code.clone()),
        (Phase::Playing, code())
    );
    assert_eq!(drain(&mut s), [], "the live round did not end");
}

#[test]
fn a_challenge_for_other_settings_breaks_the_round() {
    let mut s = Session::new(custom_patch());
    s.game_start(source()).unwrap();
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared_for(spec_seed(99)))
        .unwrap();
    let v = view(&s);
    assert_eq!(
        (v.phase, v.error.unwrap().code()),
        (Phase::Broken, "job_failed")
    );
}

#[test]
fn the_challenges_hint_budget_limits_hints() {
    for (budget, allowed) in [(Some(0u8), 0u8), (Some(2), 2), (Some(9), 4), (None, 4)] {
        let mut s = Session::new(custom_patch());
        s.game_start(source()).unwrap();
        let prepare = job(&drain(&mut s), "prepare");
        let mut challenge = Challenge::recreate(spec(), PhraseId(1));
        challenge.assists.hint_budget = budget;
        s.game_job_done(prepare.id, prepared_with(challenge, target_patch(), None))
            .unwrap();
        for level in 1..=allowed {
            assert_eq!(s.game_hint(), Ok(level), "{budget:?}");
            let hint = job(&drain(&mut s), "hint");
            s.game_job_done(
                hint.id,
                JobResult::Failed {
                    message: "-".into(),
                },
            )
            .unwrap();
        }
        assert_eq!(s.game_hint(), Err(GameError::NoMoreHints), "{budget:?}");
    }
}

#[test]
fn a_custom_round_saved_before_its_code_exists_is_dropped_or_kept_for_its_stash() {
    // A first round: nothing was played and no stash taken, so the reload has no round.
    let mut s = Session::new(custom_patch());
    s.game_start(RoundSource::Settings(custom_spec())).unwrap();
    let saved = s.save(None);
    let mut t = Session::default();
    t.load(&saved);
    assert_eq!(t.round(), None);
    assert_eq!(drain(&mut t), []);
    // A new round after a played one carries the stash: it restores broken, and ending it brings the stash back.
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.game_start(RoundSource::Settings(custom_spec())).unwrap();
    let saved = s.save(None);
    let mut t = Session::default();
    t.load(&saved);
    assert_eq!(view(&t).phase, Phase::Broken);
    t.game_end(EndChoice::Restore).unwrap();
    assert_eq!(t.patch().to_json(), custom_patch().to_json());
}

// ---------------------------------------------------------------------------------------------------------------
// A/B
// ---------------------------------------------------------------------------------------------------------------

fn ab_requests(requests: &[GameRequest]) -> Vec<enth_session::game::AbState> {
    requests
        .iter()
        .filter_map(|r| match r {
            GameRequest::Ab(ab) => Some(*ab),
            _ => None,
        })
        .collect()
}

#[test]
fn ab_switches_sides_and_attenuates_the_louder_one() {
    let mut s = Session::new(custom_patch());
    assert_eq!(s.game_listen(Listen::Target), Err(GameError::NoRound));
    assert_eq!(s.game_listen(Listen::Off), Ok(()));
    s.game_tick(0, EditorView::Opened);
    s.game_start(source()).unwrap();
    assert_eq!(s.game_listen(Listen::Target), Err(GameError::NotReady));
    assert_eq!(
        s.game_listen(Listen::Mine),
        Err(GameError::NotReady),
        "the start is not loaded yet"
    );
    let requests = drain(&mut s);
    assert_eq!(
        ab_requests(&requests),
        [],
        "nothing to tell the shell before the target exists"
    );
    s.game_job_done(job(&requests, "prepare").id, prepared())
        .unwrap();

    s.game_listen(Listen::Target).unwrap();
    let ab = ab_requests(&drain(&mut s));
    assert_eq!(ab.len(), 1);
    assert_eq!((ab[0].listen, ab[0].target_ready), (Listen::Target, true));
    assert_eq!(
        (ab[0].target_gain_db, ab[0].mine_gain_db),
        (0.0, 0.0),
        "Mine is not measured yet"
    );

    // The settled loudness job: Mine at -20 LUFS against the target's -14, so Target drops 6 dB.
    s.game_tick(SETTLE, EditorView::Visible);
    let loudness = job(&drain(&mut s), "loudness");
    s.game_job_done(loudness.id, JobResult::Loudness { lufs: Some(-20.0) })
        .unwrap();
    let ab = ab_requests(&drain(&mut s));
    assert_eq!((ab[0].target_gain_db, ab[0].mine_gain_db), (-6.0, 0.0));
    assert_eq!(view(&s).ab, ab[0]);

    // A flip; no request when nothing changed.
    s.game_listen(view(&s).ab.listen.flipped()).unwrap();
    assert_eq!(ab_requests(&drain(&mut s))[0].listen, Listen::Mine);
    s.game_listen(Listen::Mine).unwrap();
    assert_eq!(ab_requests(&drain(&mut s)), []);

    // An edit that makes Mine louder: measured again after it settles, and Mine is attenuated instead.
    s.edit(vec![set(RESONANCE, 70.0)], None).unwrap();
    s.game_tick(SETTLE + 100, EditorView::Visible);
    assert_eq!(runs(&drain(&mut s)), [], "not settled yet");
    s.game_tick(SETTLE + 400, EditorView::Visible);
    let loudness = job(&drain(&mut s), "loudness");
    s.game_job_done(loudness.id, JobResult::Loudness { lufs: Some(-11.0) })
        .unwrap();
    let ab = ab_requests(&drain(&mut s));
    assert_eq!((ab[0].target_gain_db, ab[0].mine_gain_db), (0.0, -3.0));

    // The end switches A/B off and tells the shell to drop the target's buffer.
    s.game_end(EndChoice::Keep).unwrap();
    let ab = ab_requests(&drain(&mut s));
    assert_eq!((ab[0].listen, ab[0].target_ready), (Listen::Off, false));
}

const SETTLE: u64 = enth_session::game::SETTLE_MS;

#[test]
fn a_loudness_result_for_an_older_patch_asks_again() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.game_tick(SETTLE, EditorView::Visible);
    let first = job(&drain(&mut s), "loudness");
    s.edit(vec![set(RESONANCE, 60.0)], None).unwrap();
    s.game_tick(2 * SETTLE + 1, EditorView::Visible);
    assert_eq!(runs(&drain(&mut s)), [], "one loudness job at a time");
    s.game_job_done(first.id, JobResult::Loudness { lufs: Some(-18.0) })
        .unwrap();
    s.game_tick(2 * SETTLE + 2, EditorView::Visible);
    let again = job(&drain(&mut s), "loudness");
    assert!(
        matches!(&again.kind, JobKind::Loudness { patch, .. } if patch.value(&addr(RESONANCE)) == Some(60.0))
    );
}

#[test]
fn the_rings_update_live_after_the_first_submit() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.game_tick(SETTLE, EditorView::Visible);
    assert_eq!(
        runs(&drain(&mut s)).len(),
        1,
        "only loudness before the first submit"
    );
    submit(&mut s, report(50.0));
    s.edit(vec![set(CUTOFF, 800.0)], None).unwrap();
    s.game_tick(3 * SETTLE, EditorView::Visible);
    let requests = drain(&mut s);
    let rings = job(&requests, "rings");
    s.game_job_done(
        rings.id,
        JobResult::Rings(Box::new(structure(100.0, Verdict::Good))),
    )
    .unwrap();
    assert_eq!(view(&s).knobs[0].verdict, Verdict::Good);
    assert_eq!(
        view(&s).first.unwrap().score.sound,
        5000,
        "the rings never change the recorded score"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The injected clock
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn time_counts_only_while_the_editor_is_visible_and_the_target_is_ready() {
    let mut s = Session::new(custom_patch());
    s.game_tick(10_000, EditorView::Opened);
    s.game_start(source()).unwrap();
    // Preparing takes 1.5 s: not counted.
    s.game_tick(11_500, EditorView::Visible);
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared()).unwrap();
    assert_eq!(view(&s).active_ms, 0);
    s.game_tick(12_000, EditorView::Visible);
    assert_eq!(view(&s).active_ms, 500);
    // No frames for 4 s with the editor open (an idle editor draws nothing): counted at the next tick.
    s.game_tick(16_000, EditorView::Visible);
    assert_eq!(view(&s).active_ms, 4_500);
    // Closed for a minute: the time up to the close counts, the minute does not.
    s.game_tick(16_250, EditorView::Hidden);
    s.game_tick(76_250, EditorView::Opened);
    assert_eq!(view(&s).active_ms, 4_750);
    // A close nobody reported: the next build() is Opened, so the gap is not counted either.
    s.game_tick(77_000, EditorView::Visible);
    s.game_tick(200_000, EditorView::Opened);
    s.game_tick(200_100, EditorView::Visible);
    assert_eq!(view(&s).active_ms, 5_600);
    // A timestamp from the past is treated as the last one.
    s.game_tick(150_000, EditorView::Visible);
    assert_eq!(view(&s).active_ms, 5_600);
    s.game_tick(200_200, EditorView::Visible);
    assert_eq!(view(&s).active_ms, 5_700);
    // The time to pass is the active time at the passing submit.
    submit(&mut s, report(99.0));
    assert_eq!(view(&s).time_to_pass_ms, Some(5_700));
}

// ---------------------------------------------------------------------------------------------------------------
// Undo, the edit log, host automation
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn undo_stays_inside_the_round_and_the_players_history_comes_back() {
    let mut s = Session::new(custom_patch());
    s.edit(vec![set(CUTOFF, 4321.0)], None).unwrap();
    let edited = s.patch().to_json();
    play(&mut s, 0);
    assert!(!s.history().can_undo(), "Undo cannot leave the round");
    s.edit(vec![set(CUTOFF, 1500.0)], None).unwrap();
    assert_eq!(s.undo(), Ok(true));
    assert_eq!(s.patch().to_json(), start_patch().to_json());
    assert_eq!(s.undo(), Ok(false));
    assert_eq!(s.redo(), Ok(true));

    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.patch().to_json(), edited);
    assert_eq!(s.undo(), Ok(true), "the player's own step is back");
    assert_eq!(value(&s, CUTOFF), 1234.5);
}

#[test]
fn keeping_the_rounds_patch_keeps_the_rounds_history() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.edit(vec![set(CUTOFF, 1500.0)], None).unwrap();
    s.game_end(EndChoice::Keep).unwrap();
    assert_eq!(value(&s, CUTOFF), 1500.0);
    assert_eq!(s.undo(), Ok(true));
    assert_eq!(s.patch().to_json(), start_patch().to_json());
}

#[test]
fn a_round_closes_an_open_drag_and_a_stale_gesture_id_is_no_gesture() {
    let mut s = Session::new(custom_patch());
    s.game_tick(0, EditorView::Opened);
    s.game_start(source()).unwrap();
    let prepare = job(&drain(&mut s), "prepare");
    let g = s.begin_gesture();
    s.edit(vec![set(CUTOFF, 999.0)], Some(g)).unwrap();
    s.game_job_done(prepare.id, prepared()).unwrap();
    assert!(!s.status().gesture_open);
    s.edit(vec![set(CUTOFF, 1001.0)], Some(g)).unwrap();
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(
        value(&s, CUTOFF),
        999.0,
        "the drag in flight was stashed with the patch"
    );
    assert_eq!(
        s.undo(),
        Ok(true),
        "and is one closed step of the player's history"
    );
    assert_eq!(value(&s, CUTOFF), 1234.5);
}

#[test]
fn edits_are_logged_with_active_time_drags_merge_and_the_log_replays() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.game_tick(1_000, EditorView::Visible);
    // A drag of twenty frames: one entry, with its last value and the time of its last move.
    let g = s.begin_gesture();
    for k in 0..20 {
        if k == 10 {
            s.game_tick(1_400, EditorView::Visible);
        }
        s.edit(vec![set(CUTOFF, 500.0 + 10.0 * k as f32)], Some(g))
            .unwrap();
    }
    s.end_gesture();
    s.game_tick(2_000, EditorView::Visible);
    s.edit(vec![set(RESONANCE, 25.0)], None).unwrap();
    s.undo().unwrap();
    // Host automation on a macro is folded into the round's patch and logged as the host's.
    let mut host = HostValues::default();
    s.fold_host(&host);
    host.macros[0] = 0.75;
    assert_eq!(s.fold_host(&host).written, 1);

    let record = s.game_record().unwrap();
    let sources: Vec<(u64, EditSource)> = record.edits.iter().map(|e| (e.t_ms, e.source)).collect();
    assert_eq!(
        sources,
        [
            (1_400, EditSource::Editor),
            (2_000, EditSource::Editor),
            (2_000, EditSource::Editor),
            (2_000, EditSource::Host),
        ]
    );
    assert_eq!(
        record.edits[0].ops,
        serde_json::json!([{"set": CUTOFF, "to": 690.0}])
    );
    // Replaying the log from the start patch gives the round's patch.
    let mut replay = start_patch();
    for e in &record.edits {
        let ops = enth_patch::wire::resolve(&replay, &e.ops).unwrap();
        apply_all(&mut replay, &ops).unwrap();
    }
    assert_eq!(replay.to_json(), s.patch().to_json());
    assert_eq!(s.patch().macros[macro_index(1).get()].value, 0.75);
}

#[test]
fn edits_while_a_first_round_prepares_are_the_players_own_and_not_logged() {
    let mut s = Session::new(custom_patch());
    s.game_start(source()).unwrap();
    let prepare = job(&drain(&mut s), "prepare");
    s.edit(vec![set(CUTOFF, 2222.0)], None).unwrap();
    s.game_job_done(prepare.id, prepared()).unwrap();
    assert_eq!(s.game_record().unwrap().edits, []);
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(value(&s, CUTOFF), 2222.0, "the edit is in the stash");
}

#[test]
fn host_slots_do_nothing_in_a_round_and_the_players_slots_come_back() {
    let mut s = Session::new(custom_patch());
    let mut host = s.host_values();
    s.fold_host(&host);
    play(&mut s, 0);
    // slot_1 drives filter1.cutoff in the player's patch; the start patch has no slots.
    host.slots[0] = 0.9;
    assert_eq!(
        s.fold_host(&host).written,
        0,
        "the first fold after the start only records"
    );
    host.slots[0] = 0.1;
    assert_eq!(
        s.fold_host(&host).written,
        0,
        "an unassigned slot does nothing"
    );
    assert_eq!(s.patch().to_json(), start_patch().to_json());
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.patch().slots[0], Some(addr(CUTOFF)));
    assert!(shell_requests(&mut s).contains(&ShellRequest::RenameParams));
    // The patch wins after the restore: the stale host value is only recorded.
    assert_eq!(s.fold_host(&host).written, 0);
    assert_eq!(value(&s, CUTOFF), 1234.5);
}

// ---------------------------------------------------------------------------------------------------------------
// The saved state
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn without_a_round_every_v1_fixture_keeps_its_bytes() {
    for (name, patch, instance) in common::v1_cases() {
        let expected = SavedState::encode(&patch, &instance);
        let mut s = Session::new(patch.clone());
        s.load_instance(&instance.to_json());
        // Game entries that start nothing.
        s.game_tick(5, EditorView::Opened);
        s.game_tick(50, EditorView::Visible);
        assert_eq!(s.game_listen(Listen::Target), Err(GameError::NoRound));
        assert_eq!(s.game_submit(), Err(GameError::NoRound));
        assert_eq!(drain(&mut s), []);
        assert_eq!(s.save(None), expected, "{name}");
        // A round started and ended with Restore leaves both fields byte for byte as they were.
        play(&mut s, 100);
        s.edit(vec![set(CUTOFF, 640.0)], None).unwrap();
        submit(&mut s, report(10.0));
        s.game_end(EndChoice::Restore).unwrap();
        assert_eq!(s.save(None), expected, "{name}: after a round");
    }
}

/// A round mid-play: two submits (one failing), a hint, a vote and some edits, at fixed times.
fn mid_round() -> Session {
    let mut s = Session::new(custom_patch());
    play(&mut s, 1_000);
    s.game_tick(4_000, EditorView::Visible);
    s.edit(vec![set(CUTOFF, 900.0)], None).unwrap();
    s.game_rate(4).unwrap();
    submit(&mut s, report(35.0));
    s.game_hint().unwrap();
    let hint = job(&drain(&mut s), "hint");
    s.game_job_done(
        hint.id,
        JobResult::Hint(enth_session::game::CoachHint::new(1, "brighter")),
    )
    .unwrap();
    s.game_tick(7_000, EditorView::Visible);
    s.edit(vec![set(RESONANCE, 33.0)], None).unwrap();
    submit(&mut s, report(61.0));
    s.game_vote(Agreement::TooLow).unwrap();
    s.game_tick(8_000, EditorView::Visible);
    drain(&mut s);
    shell_requests(&mut s);
    s
}

#[test]
fn a_project_saved_mid_round_restores_the_round_and_the_stash() {
    let mut s = mid_round();
    let own = custom_patch().to_json();
    let saved = s.save(None);
    let record = s.game_record().unwrap().clone();
    assert_eq!(
        saved.patch,
        s.patch().to_json(),
        "the patch field is the round's patch"
    );
    assert!(
        saved.instance.contains("\"game\":{\"round\":"),
        "{}",
        saved.instance
    );

    // A fresh instance opens the project: nice-plug sets `enth`, then `patch`.
    let mut t = Session::default();
    let report = t.load(&saved);
    assert_eq!(report.errors, []);
    assert_eq!(
        t.game_record(),
        Some(&record),
        "the record survives byte for byte"
    );
    assert_eq!(t.patch().to_json(), saved.patch);
    let v = view(&t);
    assert_eq!(
        (
            v.phase,
            v.pending.preparing,
            v.active_ms,
            v.submits,
            v.hints
        ),
        (Phase::Submitted, true, 7_000, 2, 1)
    );
    assert_eq!(v.agreement, Some(Agreement::TooLow));
    // The target is regenerated before play resumes; the clock waits for it.
    assert_eq!(t.game_submit(), Err(GameError::NotReady));
    assert_eq!(t.game_listen(Listen::Target), Err(GameError::NotReady));
    t.game_tick(50, EditorView::Opened);
    t.game_tick(9_000, EditorView::Visible);
    assert_eq!(view(&t).active_ms, 7_000);
    let prepare = job(&drain(&mut t), "prepare");
    shell_requests(&mut t);
    t.game_job_done(prepare.id, prepared()).unwrap();
    assert_eq!(view(&t).phase, Phase::Submitted);
    assert_eq!(
        t.patch().to_json(),
        saved.patch,
        "a resumed round does not reload the start"
    );
    assert!(!shell_requests(&mut t).contains(&ShellRequest::PatchEdited));
    t.game_tick(10_000, EditorView::Visible);
    assert_eq!(view(&t).active_ms, 8_000);
    // What a reload lost is measured again: Mine's loudness, and the rings of a scored round.
    let requests = drain(&mut t);
    job(&requests, "loudness");
    job(&requests, "rings");
    assert_eq!(t.game_submit(), Ok(3));
    // The stash comes back after the reload (its undo history does not: no history is saved).
    t.game_end(EndChoice::Restore).unwrap();
    assert_eq!(t.patch().to_json(), own);
    assert!(!t.history().can_undo());
}

#[test]
fn a_different_regenerated_target_breaks_the_round_but_keeps_the_stash() {
    let saved = mid_round().save(None);
    let mut t = Session::default();
    t.load(&saved);
    let prepare = job(&drain(&mut t), "prepare");
    let mut other = target_patch();
    apply_all(&mut other, &[set(CUTOFF, 801.0)]).unwrap();
    t.game_job_done(
        prepare.id,
        prepared_with(Challenge::recreate(spec(), PhraseId(1)), other, None),
    )
    .unwrap();
    let v = view(&t);
    assert_eq!(v.phase, Phase::Broken);
    assert_eq!(v.error.unwrap().code(), "target_drift");
    assert_eq!(t.game_submit(), Err(GameError::Broken));
    t.game_end(EndChoice::Restore).unwrap();
    assert_eq!(t.patch().to_json(), custom_patch().to_json());
    let line = &logs(&drain(&mut t))[0];
    assert_eq!(
        (line.broken, result(line).outcome),
        (true, Outcome::NotPassed)
    );
}

#[test]
fn a_saved_round_with_a_bad_code_or_a_newer_phase_restores_broken() {
    let saved = mid_round().save(None);
    for (from, to) in [
        (
            format!("\"code\":\"{}\"", code()),
            format!("\"code\":\"{}X\"", code()),
        ),
        (
            format!("\"code\":\"{}\"", code()),
            "\"code\":\"\"".to_owned(),
        ),
        (
            "\"phase\":\"submitted\"".to_owned(),
            "\"phase\":\"duel\",\"later\":1".to_owned(),
        ),
    ] {
        assert!(saved.instance.contains(&from), "{}", saved.instance);
        let instance = saved.instance.replace(&from, &to);
        let mut t = Session::default();
        let report = t.load(&SavedState {
            patch: saved.patch.clone(),
            instance,
        });
        assert_eq!(report.errors, [], "{to}");
        assert_eq!(view(&t).phase, Phase::Broken, "{to}");
        assert_eq!(runs(&drain(&mut t)), [], "{to}: nothing to prepare");
        t.game_end(EndChoice::Restore).unwrap();
        assert_eq!(t.patch().to_json(), custom_patch().to_json(), "{to}");
    }
}

#[test]
fn a_host_load_of_the_same_round_keeps_the_live_progress() {
    // REAPER's own Undo restores an earlier save of the project: the round's submits and time are not rolled back.
    let mut s = mid_round();
    let earlier = s.save(None);
    s.game_tick(12_000, EditorView::Visible);
    s.edit(vec![set(CUTOFF, 805.0)], None).unwrap();
    submit(&mut s, report(93.0));
    let live = s.game_record().unwrap().clone();
    drain(&mut s);
    s.load(&earlier);
    let after = s.game_record().unwrap();
    let (load, kept) = after.edits.split_last().unwrap();
    assert_eq!(load.source, EditSource::Load, "the host's load is logged");
    assert_eq!(kept, live.edits.as_slice());
    assert_eq!(
        (
            after.submits.as_slice(),
            after.active_ms,
            after.time_to_pass_ms,
            after.agreement
        ),
        (
            live.submits.as_slice(),
            live.active_ms,
            live.time_to_pass_ms,
            live.agreement
        ),
        "the live record wins"
    );
    assert_eq!(view(&s).phase, Phase::Passed);
    assert_eq!(
        s.patch().to_json(),
        earlier.patch,
        "the patch follows the host"
    );
    let requests = drain(&mut s);
    assert_eq!(
        runs(&requests),
        [],
        "the live target is kept: nothing to regenerate"
    );
    assert!(!requests.contains(&GameRequest::Dirty));
    assert!(s.game_submit().is_ok());
}

#[test]
fn a_host_load_without_a_round_ends_the_live_one() {
    let mut s = Session::new(custom_patch());
    let before = s.save(None);
    play(&mut s, 0);
    s.game_tick(1_000, EditorView::Visible);
    drain(&mut s);
    // REAPER's Undo of the round start, or opening another project.
    let report = s.load(&before);
    assert_eq!(report.errors, []);
    assert_eq!(s.round(), None);
    let requests = drain(&mut s);
    let line = &logs(&requests)[0];
    assert_eq!(
        (result(line).outcome, line.ended_by),
        (Outcome::NotPassed, Some(EndedBy::Host))
    );
    assert!(requests.iter().any(|r| matches!(r, GameRequest::Cancel(_))));
    assert!(
        !requests.contains(&GameRequest::Dirty),
        "a host load never marks the project dirty (it would cut REAPER's redo)"
    );
    assert_eq!(s.save(None), before);
}

#[test]
fn a_host_load_of_another_round_replaces_the_live_one() {
    let saved = mid_round().save(None);
    let mut s = Session::new(custom_patch());
    s.game_tick(0, EditorView::Opened);
    s.game_start(RoundSource::Settings(spec_seed(44))).unwrap();
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared_for(spec_seed(44)))
        .unwrap();
    s.load(&saved);
    let requests = drain(&mut s);
    assert_eq!(
        logs(&requests)[0].code,
        ChallengeCode::short(spec_seed(44)).unwrap().to_string()
    );
    assert_eq!(view(&s).code, code());
    assert_eq!(job(&requests, "prepare").kind.name(), "prepare");
}

#[test]
fn the_saved_round_stays_within_state_version_1() {
    let saved = mid_round().save(None);
    assert!(
        saved
            .instance
            .starts_with("{\"schema\":\"enth.plugin-state/1\",")
    );
    assert_eq!(STATE_VERSION, 1);
    let (state, version) = InstanceState::from_json(&saved.instance).unwrap();
    assert_eq!(version, 1);
    assert_eq!(
        state.to_json(),
        saved.instance,
        "decode then encode is byte-identical"
    );
    // Fields from a newer version 1 are ignored; missing ones default.
    let newer = saved
        .instance
        .replacen("\"round\":{", "\"round\":{\"streak\":3,", 1);
    assert_eq!(InstanceState::from_json(&newer).unwrap().0, state);
    let (bare, _) = InstanceState::from_json(
        r#"{"schema":"enth.plugin-state/1","game":{"round":{"code":"X"}}}"#,
    )
    .unwrap();
    let round = bare.game.unwrap().round.unwrap();
    assert_eq!(
        (round.phase, round.active_ms, round.stash),
        (Phase::Preparing, 0, None)
    );
    // No round: the reserved field keeps the bytes it had at F0r.
    assert_eq!(GameState::default(), GameState { round: None });
    assert_eq!(serde_json::to_string(&GameState::default()).unwrap(), "{}");
}

/// Every number in `v` that is not an unsigned integer, by path.
fn walk(v: &serde_json::Value, path: &str, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Number(n) if !n.is_u64() => out.push(path.to_owned()),
        serde_json::Value::Array(a) => a
            .iter()
            .enumerate()
            .for_each(|(i, x)| walk(x, &format!("{path}[{i}]"), out)),
        serde_json::Value::Object(m) => m
            .iter()
            .for_each(|(k, x)| walk(x, &format!("{path}.{k}"), out)),
        _ => {}
    }
}

#[test]
fn the_saved_round_never_holds_a_float() {
    let saved = mid_round().save(None);
    let value: serde_json::Value = serde_json::from_str(&saved.instance).unwrap();
    let round = &value["game"]["round"];
    let mut floats = Vec::new();
    // The edit log's ops are patch values in wire form (shortest f32 text): those are allowed.
    for (k, v) in round.as_object().unwrap() {
        if k != "edits" {
            walk(v, k, &mut floats);
        }
    }
    assert_eq!(floats, Vec::<String>::new());
}

// ---------------------------------------------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------------------------------------------

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state/v1")
}

#[allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture files; the library does no I/O"
)]
fn round_fixture() -> SavedState {
    let read = |field: &str| {
        let path = fixture_dir().join(format!("round.{field}.json"));
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    SavedState {
        patch: read("patch"),
        instance: read("enth"),
    }
}

#[test]
#[ignore = "writes the round fixture: run on purpose and review the diff"]
#[allow(
    clippy::disallowed_methods,
    reason = "the regenerate test writes the fixture files; the library does no I/O"
)]
fn regenerate_round_fixture() {
    let saved = mid_round().save(None);
    fs::write(fixture_dir().join("round.patch.json"), saved.patch).unwrap();
    fs::write(fixture_dir().join("round.enth.json"), saved.instance).unwrap();
}

/// The saved round is a persisted type: the bytes a mid-round save writes are pinned, and they load back into the
/// same round.
#[test]
fn the_round_fixture_is_what_this_build_writes_and_loads() {
    let fixture = round_fixture();
    assert_eq!(mid_round().save(None), fixture);
    let mut t = Session::default();
    assert_eq!(t.load(&fixture).errors, []);
    assert_eq!(t.save(None), fixture);
    assert_eq!(view(&t).phase, Phase::Submitted);
}

// ---------------------------------------------------------------------------------------------------------------
// The editor's trait
// ---------------------------------------------------------------------------------------------------------------

/// A kittest-style fake: the trait is object-safe and implementable outside the crate, and its read model can be
/// built from `Default`.
struct Fake(Option<RoundView>);

impl GameEdit for Fake {
    fn round(&self) -> Option<RoundView> {
        self.0.clone()
    }
    fn game_start(&mut self, source: RoundSource) -> Result<(), GameError> {
        let RoundSource::Settings(spec) = source else {
            return Err(GameError::NotReady);
        };
        let mut v = RoundView::default();
        v.code = ChallengeCode::short(spec).unwrap().to_string();
        self.0 = Some(v);
        Ok(())
    }
    fn game_listen(&mut self, _: Listen) -> Result<(), GameError> {
        Err(GameError::NotReady)
    }
    fn game_submit(&mut self) -> Result<u32, GameError> {
        Err(GameError::NotReady)
    }
    fn game_hint(&mut self) -> Result<u8, GameError> {
        Err(GameError::NotReady)
    }
    fn game_reveal(&mut self) -> Result<(), GameError> {
        Err(GameError::NotReady)
    }
    fn game_rate(&mut self, _: u8) -> Result<(), GameError> {
        Err(GameError::NotReady)
    }
    fn game_vote(&mut self, _: Agreement) -> Result<(), GameError> {
        Err(GameError::NotScored)
    }
    fn game_dispute(&mut self, _: &str) -> Result<(), GameError> {
        Err(GameError::NotScored)
    }
    fn game_end(&mut self, _: EndChoice) -> Result<(), GameError> {
        self.0 = None;
        Ok(())
    }
}

#[test]
fn the_editor_drives_a_round_through_a_trait_object() {
    let mut session = Session::new(custom_patch());
    let mut fake = Fake(None);
    for edit in [&mut session as &mut dyn GameEdit, &mut fake] {
        assert_eq!(edit.round(), None);
        edit.game_start(source()).unwrap();
        assert_eq!(edit.round().unwrap().code, code());
        edit.game_end(EndChoice::Restore).unwrap();
        assert_eq!(edit.round(), None);
    }
    for e in [
        GameError::NoRound,
        GameError::TargetDrift {
            expected: "a".into(),
            found: "b".into(),
        },
    ] {
        assert!(!e.code().is_empty() && !e.hint().is_empty() && !e.to_string().is_empty());
    }
}

// ---------------------------------------------------------------------------------------------------------------
// No audio-thread contact
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

/// The engine's plan swaps and the session's epoch and queue, after the engine caught up.
fn engine_side(s: &mut Session, engine: &mut Engine) -> (u64, u64, usize) {
    s.pump();
    run(engine, 4);
    s.pump();
    let st = s.status();
    (engine.stats().swaps, st.epoch, st.queued_events)
}

/// The round reaches the engine only by replacing the patch (the start, a reveal, the end's restore), each exactly
/// one plan, like a load. Every other game entry (the clock, A/B, submits and their scores, rings, loudness, hints,
/// votes, disputes) sends the engine nothing: A/B is the shell's buffer player and gains, not engine events.
#[test]
fn only_patch_replacements_reach_the_engine_one_plan_each() {
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
    let idle = engine_side(&mut s, &mut engine);

    s.game_tick(0, EditorView::Opened);
    s.game_start(source()).unwrap();
    let prepare = job(&drain(&mut s), "prepare");
    assert_eq!(
        engine_side(&mut s, &mut engine),
        idle,
        "starting asks the worker, not the engine"
    );

    s.game_job_done(prepare.id, prepared()).unwrap();
    let started = engine_side(&mut s, &mut engine);
    assert_eq!(
        (started.0, started.1),
        (idle.0 + 1, idle.1 + 1),
        "the start patch is one plan"
    );

    // Everything a round does that leaves the patch alone.
    s.game_listen(Listen::Target).unwrap();
    s.game_listen(Listen::Mine).unwrap();
    s.game_tick(SETTLE, EditorView::Visible);
    let loudness = job(&drain(&mut s), "loudness");
    s.game_job_done(loudness.id, JobResult::Loudness { lufs: Some(-17.0) })
        .unwrap();
    s.game_submit().unwrap();
    let score = job(&drain(&mut s), "score");
    s.game_job_done(score.id, JobResult::Scored(Box::new(report(20.0))))
        .unwrap();
    s.game_hint().unwrap();
    let hint = job(&drain(&mut s), "hint");
    s.game_job_done(
        hint.id,
        JobResult::Hint(enth_session::game::CoachHint::new(1, "x")),
    )
    .unwrap();
    s.game_vote(Agreement::AboutRight).unwrap();
    s.game_dispute("fine").unwrap();
    s.game_listen(Listen::Off).unwrap();
    s.game_tick(9_999, EditorView::Hidden);
    assert_eq!(
        engine_side(&mut s, &mut engine),
        started,
        "no game entry but a patch change reaches the engine"
    );

    s.game_reveal().unwrap();
    let revealed = engine_side(&mut s, &mut engine);
    assert_eq!(
        (revealed.0, revealed.1),
        (started.0 + 1, started.1 + 1),
        "a reveal is one plan"
    );
    s.game_end(EndChoice::Restore).unwrap();
    let ended = engine_side(&mut s, &mut engine);
    assert_eq!(
        (ended.0, ended.1),
        (revealed.0 + 1, revealed.1 + 1),
        "the restore is one plan"
    );
    assert_eq!(engine.stats().dropped_events, 0);
    assert_eq!(engine.stats().stale_events, 0);
}

// ---------------------------------------------------------------------------------------------------------------
// The F1r review's fixes (lane F1R)
// ---------------------------------------------------------------------------------------------------------------

/// Non-default editor preferences, to see that a load keeps them.
fn prefs() -> enth_session::UiState {
    enth_session::UiState {
        layout: "grid".to_owned(),
        scale_percent: 150,
        advanced: true,
    }
}

/// T3 (M3): a saved round holding a newer build's value in any of its enums (the phase, an edit's source, the vote,
/// and the first report's verdict, direction, closeness, kind and status) loads: the editor preferences stay, and
/// the stash comes back on Restore.
#[test]
fn t3_newer_enum_values_in_a_saved_round_keep_the_preferences_and_the_stash() {
    let mut s = mid_round();
    s.set_ui(prefs());
    let saved = s.save(None);
    let own = custom_patch().to_json();
    for (from, to) in [
        ("\"phase\":\"submitted\"", "\"phase\":\"practice\""),
        ("\"source\":\"editor\"", "\"source\":\"midi\""),
        ("\"agreement\":\"too_low\"", "\"agreement\":\"disagree\""),
        (
            "\\\"verdict\\\":\\\"bad\\\"",
            "\\\"verdict\\\":\\\"awful\\\"",
        ),
        (
            "\\\"direction\\\":\\\"higher\\\"",
            "\\\"direction\\\":\\\"wobbly\\\"",
        ),
        ("\"hints\":1", "\"hints\":1,\"streak\":{\"kind\":\"new\"}"),
    ] {
        assert!(saved.instance.contains(from), "{from}");
        let instance = saved.instance.replacen(from, to, 1);
        let mut t = Session::default();
        let report = t.load(&SavedState {
            patch: saved.patch.clone(),
            instance,
        });
        assert_eq!(report.errors, [], "{to}");
        assert_eq!(t.instance().ui, prefs(), "{to}: the preferences stay");
        let record = t.game_record().expect("the round is kept").clone();
        assert_eq!(record.stash.as_deref(), Some(own.as_str()), "{to}");
        if to.contains("practice") {
            assert_eq!(
                record.phase,
                Phase::Broken,
                "an unknown phase restores broken"
            );
        }
        if to.contains("disagree") {
            assert_eq!(record.agreement, Some(Agreement::Unknown));
        }
        t.game_end(EndChoice::Restore).unwrap();
        assert_eq!(t.patch().to_json(), own, "{to}: the stash comes back");
    }
}

/// T3 (M3): a `game` object this build cannot decode at all (a field of another type) fails only its round: it
/// restores broken with its stash, and the preferences stay.
#[test]
fn t3_a_round_this_build_cannot_read_keeps_its_stash() {
    let mut s = mid_round();
    s.set_ui(prefs());
    let saved = s.save(None);
    let own = custom_patch().to_json();
    for (from, to) in [
        ("\"active_ms\":7000", "\"active_ms\":\"seven seconds\""),
        ("\"hints\":1", "\"hints\":\"one\""),
        ("\"id\":{\"serial\":1", "\"id\":{\"serial\":-1"),
    ] {
        assert!(saved.instance.contains(from), "{from}");
        let instance = saved.instance.replacen(from, to, 1);
        let mut t = Session::default();
        let report = t.load(&SavedState {
            patch: saved.patch.clone(),
            instance,
        });
        assert_eq!(report.errors, [], "{to}");
        assert_eq!(t.instance().ui, prefs(), "{to}");
        let v = view(&t);
        assert_eq!(v.phase, Phase::Broken, "{to}");
        assert_eq!(t.game_submit(), Err(GameError::Broken));
        t.game_end(EndChoice::Restore).unwrap();
        assert_eq!(t.patch().to_json(), own, "{to}");
    }
    // Without a readable stash there is nothing to keep: no round, and still the preferences.
    let instance = saved
        .instance
        .replacen("\"active_ms\":7000", "\"active_ms\":[]", 1)
        .replacen("\"stash\":\"", "\"stash_v2\":\"", 1);
    let mut t = Session::default();
    assert_eq!(
        t.load(&SavedState {
            patch: saved.patch.clone(),
            instance,
        })
        .errors,
        []
    );
    assert_eq!(t.round(), None);
    assert_eq!(t.instance().ui, prefs());
}

/// T3 (S8): the largest round the budget allows, every part at its cap and escaped at its worst, still fits the
/// `enth` field, and loads back with its stash.
#[test]
fn t3_the_largest_round_fits_the_enth_field() {
    use enth_session::game::{
        DISPUTE_CHARS, EDIT_LOG_BYTES, EditRecord, FIRST_REPORT_BYTES, MAX_SUBMITS,
        MAX_SUBMITTED_PATCH_BYTES, RoundId, RoundRecord, STASH_BYTES, ScoreSummary, SubmitRecord,
        escaped_len,
    };
    use enth_session::state::MAX_FIELD_BYTES;
    // Text whose escaped form is as long as the cap allows, built from the worst character (a control character
    // takes 6 bytes escaped).
    let worst = |cap: usize| "\u{1}".repeat((cap - 2) / 6);
    let stash = worst(STASH_BYTES);
    assert!(escaped_len(&stash) <= STASH_BYTES && escaped_len(&stash) > STASH_BYTES - 8);
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
            source: EditSource::Reveal,
            ops,
        });
    }
    let record = RoundRecord {
        id: RoundId {
            serial: u32::MAX,
            nonce: u64::MAX,
        },
        code: "H000-G07Z-ZZZZ-ZZZZ-ZA4C-07ZZ-ZZG2-8XBS-N5J0-0A".to_owned(),
        phase: Phase::Submitted,
        stash: Some(stash.clone()),
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
        rating: Some(5),
        agreement: Some(Agreement::AboutRight),
        dispute: Some("\u{1}".repeat(DISPUTE_CHARS)),
        first_report: Some(worst(FIRST_REPORT_BYTES)),
        submitted_patch: Some(worst(MAX_SUBMITTED_PATCH_BYTES)),
        edits,
        edits_dropped: u32::MAX,
    };
    let instance = InstanceState {
        ui: enth_session::UiState {
            layout: "x".repeat(64),
            ..enth_session::UiState::default()
        },
        game: Some(GameState {
            round: Some(record),
        }),
        ..InstanceState::default()
    };
    let text = instance.to_json();
    assert!(
        text.len() <= MAX_FIELD_BYTES,
        "{} bytes over {MAX_FIELD_BYTES}",
        text.len()
    );
    let (back, _) = InstanceState::from_json(&text).unwrap();
    assert_eq!(back, instance);
    assert_eq!(
        back.game.unwrap().round.unwrap().stash.as_deref(),
        Some(stash.as_str())
    );
}

/// S8: a player's patch too large to stash (a newer build's patch, kept as text) refuses the round at its start,
/// with a clear error, and changes nothing; the save still loads.
#[test]
fn s8_a_patch_too_large_to_stash_refuses_the_start() {
    use enth_session::game::STASH_BYTES;
    let mut s = Session::new(custom_patch());
    let huge = format!("{{\"schema\":9999,\"x\":\"{}\"}}", "\\\"".repeat(200_000));
    assert!(huge.len() < enth_session::state::MAX_FIELD_BYTES);
    s.load_patch(&huge);
    assert!(s.status().patch_unloaded, "kept as text");
    let e = s.game_start(source()).unwrap_err();
    assert!(
        matches!(e, GameError::TooLarge { what: "stash", bytes, max } if bytes > max && max == STASH_BYTES),
        "{e:?}"
    );
    assert_eq!(e.code(), "too_large");
    assert_ne!(e.hint(), "");
    assert_eq!(s.round(), None);
    assert_eq!(drain(&mut s), []);
    let saved = s.save(None);
    assert_eq!(saved.patch, huge, "the player's text is untouched");
    assert_eq!(
        Session::default().load(&saved).errors.len(),
        1,
        "only the patch's own error"
    );
}

/// T6 (S10): a worker report the challenge could not have produced is refused, records no submit, and frees the
/// score slot; so are rings from another scorer.
#[test]
fn t6_an_inconsistent_worker_report_is_refused() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    let mut wrong_id = report(95.0);
    wrong_id.sound.scorer = enth_challenge::ScorerId::SOUND_V2;
    let mut wrong_mark = report(95.0);
    wrong_mark.pass_mark = 50;
    let mut flipped = report(95.0);
    flipped.passed = false;
    let mut lying = report(50.0);
    lying.passed = true;
    let mut nan = report(95.0);
    nan.sound.percent = f64::NAN;
    let mut over = report(95.0);
    over.structure.percent = 150.0;
    for bad in [wrong_id, wrong_mark, flipped, lying, nan, over] {
        s.game_submit().unwrap();
        let score = job(&drain(&mut s), "score");
        let e = s
            .game_job_done(score.id, JobResult::Scored(Box::new(bad.clone())))
            .unwrap_err();
        assert_eq!(e.code(), "wrong_result", "{bad:?}");
        assert!(matches!(e, GameError::BadReport { .. }));
        let v = view(&s);
        assert_eq!((v.submits, v.phase), (0, Phase::Playing));
        assert_eq!(v.error.unwrap().code(), "wrong_result");
        assert!(!v.pending.scoring, "the slot is free again");
    }
    // A consistent report still counts.
    submit(&mut s, report(92.0));
    assert_eq!(view(&s).phase, Phase::Passed);
    // Rings from another structure scorer are refused too.
    s.edit(vec![set(CUTOFF, 1000.0)], None).unwrap();
    s.game_tick(1_000, EditorView::Visible);
    let rings = job(&drain(&mut s), "rings");
    let mut other = structure(10.0, Verdict::Bad);
    other.scorer = enth_challenge::ScorerId::STRUCTURE_V1;
    assert_eq!(
        s.game_job_done(rings.id, JobResult::Rings(Box::new(other)))
            .unwrap_err()
            .code(),
        "wrong_result"
    );
}

/// T6 (S4): settings with a seed a JSON reader could change are refused at the start, and change nothing.
#[test]
fn t6_seeds_from_2_pow_53_are_refused_at_the_start() {
    let mut s = Session::new(custom_patch());
    for seed in [1u64 << 53, u64::MAX] {
        let e = s
            .game_start(RoundSource::Settings(RandomSpec { seed, ..spec() }))
            .unwrap_err();
        assert_eq!(e.code(), "out_of_range");
        assert_eq!(s.round(), None);
    }
    let ok = RandomSpec {
        seed: enth_challenge::spec::MAX_SEED,
        ..custom_spec()
    };
    s.game_start(RoundSource::Settings(ok)).unwrap();
}

/// S7: the randomized parameters reach the round's view once the target is ready (for `dim_unused`).
#[test]
fn s7_the_randomized_parameters_reach_the_view() {
    let mut s = Session::new(custom_patch());
    s.game_start(source()).unwrap();
    assert_eq!(view(&s).params, []);
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared()).unwrap();
    assert_eq!(view(&s).params, params());
}

/// S9: the blind rating is taken before the first score shows, one per round (a later one replaces it), 1 to 5, and
/// goes to the round's result.
#[test]
fn s9_the_blind_rating_is_asked_before_the_score_shows() {
    let mut s = Session::new(custom_patch());
    assert_eq!(s.game_rate(3), Err(GameError::NoRound));
    s.game_start(source()).unwrap();
    assert_eq!(s.game_rate(3), Err(GameError::NotReady));
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared()).unwrap();
    assert_eq!(s.game_rate(0).unwrap_err().code(), "bad_rating");
    assert_eq!(s.game_rate(6), Err(GameError::BadRating(6)));
    s.game_submit().unwrap();
    let score = job(&drain(&mut s), "score");
    // Submitted, its score not shown yet: the moment the editor asks.
    s.game_rate(2).unwrap();
    s.game_rate(4).unwrap();
    assert_eq!(view(&s).rating, Some(4));
    s.game_job_done(score.id, JobResult::Scored(Box::new(report(40.0))))
        .unwrap();
    assert_eq!(s.game_rate(5), Err(GameError::AlreadyScored));
    assert_eq!(s.game_record().unwrap().rating, Some(4));
    s.game_end(EndChoice::Restore).unwrap();
    let r = result(&logs(&drain(&mut s))[0]);
    assert_eq!(r.rating, Some(4));
    // After a reveal the player has heard the target as their own: no rating.
    let mut t = Session::new(custom_patch());
    play(&mut t, 0);
    t.game_reveal().unwrap();
    assert_eq!(t.game_rate(3), Err(GameError::Revealed));
}

/// S11: the view says when the round next needs a frame: the settled jobs' time, the running timer's next second,
/// and nothing while the editor is closed.
#[test]
fn s11_the_view_says_when_the_round_next_needs_a_frame() {
    let mut s = Session::new(custom_patch());
    s.game_tick(0, EditorView::Opened);
    s.game_start(source()).unwrap();
    assert_eq!(
        view(&s).wake_at_ms,
        None,
        "preparing: the worker's post wakes the editor"
    );
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared()).unwrap();
    assert_eq!(view(&s).wake_at_ms, Some(0), "the settled jobs are due now");
    s.game_tick(250, EditorView::Visible);
    assert_eq!(view(&s).wake_at_ms, Some(1_000), "the timer's next second");
    s.edit(vec![set(CUTOFF, 1500.0)], None).unwrap();
    assert_eq!(
        view(&s).wake_at_ms,
        Some(550),
        "settled 300 ms after the edit"
    );
    s.game_tick(1_300, EditorView::Visible);
    assert_eq!(view(&s).wake_at_ms, Some(2_000));
    s.game_tick(1_400, EditorView::Hidden);
    assert_eq!(view(&s).wake_at_ms, None, "closed: no timer, no frames");
}

/// S3 and S8: each log line carries the target's fingerprint and the engine, and the `end` line the edit log.
#[test]
fn s3_log_lines_carry_the_fingerprint_the_engine_and_the_edit_log() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    s.edit(vec![set(CUTOFF, 700.0)], None).unwrap();
    submit(&mut s, report(30.0));
    s.game_dispute("too harsh").unwrap();
    let dispute = logs(&drain(&mut s)).pop().unwrap();
    assert_eq!(dispute.edit_log, [], "the dispute line is small");
    let edits = s.game_record().unwrap().edits.clone();
    s.game_end(EndChoice::Keep).unwrap();
    let end = logs(&drain(&mut s)).pop().unwrap();
    for line in [&dispute, &end] {
        assert_eq!(line.fingerprint, Some(fingerprint().to_hex()));
        assert_eq!(line.engine, Some(EngineVersion::CURRENT));
    }
    assert_eq!(end.edit_log, edits);
    assert_eq!(end.edits as usize, edits.len());
    // A line written before these fields existed still reads.
    let mut old = serde_json::to_value(&end).unwrap();
    let fields = old.as_object_mut().unwrap();
    for key in ["fingerprint", "engine", "edit_log"] {
        assert!(fields.remove(key).is_some(), "{key}");
    }
    let back: RoundLogEntry = serde_json::from_value(old).unwrap();
    assert_eq!((back.fingerprint, back.engine), (None, None));
}

/// S6: a ring goes on the player's control (the row's answer address), a row with no match rings nothing, and a
/// row for a parameter the target did not randomize says so.
#[test]
fn s6_rings_go_on_the_players_control() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    let mut r = report(40.0);
    r.structure.params = vec![
        ParamScore {
            addr: CUTOFF.to_owned(),
            answer_addr: Some(RESONANCE.to_owned()),
            score: 0.5,
            verdict: Verdict::Near,
            direction: Direction::Lower,
            randomized: true,
        },
        ParamScore {
            addr: "osc2.level".to_owned(),
            answer_addr: None,
            score: 0.0,
            verdict: Verdict::Bad,
            direction: Direction::Missing,
            randomized: true,
        },
        ParamScore {
            addr: "env1.release".to_owned(),
            answer_addr: Some("env1.release".to_owned()),
            score: 0.0,
            verdict: Verdict::Bad,
            direction: Direction::Higher,
            randomized: false,
        },
    ];
    submit(&mut s, r);
    let knobs = view(&s).knobs;
    assert_eq!(
        knobs
            .iter()
            .map(|k| (k.addr.to_string(), k.verdict, k.randomized))
            .collect::<Vec<_>>(),
        [
            (RESONANCE.to_owned(), Verdict::Near, true),
            ("env1.release".to_owned(), Verdict::Bad, false)
        ]
    );
}

/// M1: a resumed round checks the regenerated target by its fingerprint, which a patch re-encoding never moves:
/// only another target breaks it.
#[test]
fn m1_a_resumed_round_is_checked_by_fingerprint() {
    let saved = mid_round().save(None);
    assert!(saved.instance.contains(&fingerprint().to_hex()));
    let mut t = Session::default();
    t.load(&saved);
    let prepare = job(&drain(&mut t), "prepare");
    // The same target with another fingerprint is another target.
    t.game_job_done(
        prepare.id,
        JobResult::Prepared(Box::new(Prepared::new(
            Challenge::recreate(spec(), PhraseId(1)),
            start_patch(),
            target_patch(),
            params(),
            TargetFingerprint([7; 32]),
            None,
        ))),
    )
    .unwrap();
    assert_eq!(view(&t).error.unwrap().code(), "target_drift");
}
