//! The Recreate round through the public API (lane SG): the lifecycle, A/B, submit, hints, reveal, give up, the
//! injected clock, undo, host automation and loads, and the saved round (docs/session.md, "The Recreate round").
//! The shell is played by hand: each test drains the game outbox and answers the jobs it asks for. No wall clock and
//! no random seed anywhere: every timestamp is a literal.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use enth_engine::{Engine, Event, EventKind, NoteKey};
use enth_patch::model::{ModuleInstance, Section};
use enth_patch::{ModuleKind, Op, Patch, Position, apply_all};
use enth_session::game::{
    Challenge, EditSource, EditorView, EndChoice, EndedBy, GameEdit, GameError, GameRequest, Job,
    JobId, JobKind, JobResult, KnobScore, Listen, LogEvent, Outcome, Phase, RoundLogEntry,
    RoundView, ScoreReport, Verdict, Vote,
};
use enth_session::state::STATE_VERSION;
use enth_session::{
    Activation, GameState, HostValues, InstanceState, SavedState, Session, ShellRequest,
};

use common::{addr, custom_patch, id, init_patch, macro_index};

const CODE: &str = "R1-TEST-0001";
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

fn challenge() -> Challenge {
    Challenge::new(CODE)
}

fn report(passed: bool, sound: f32) -> ScoreReport {
    ScoreReport {
        passed,
        sound,
        structure: sound - 10.0,
        combined: sound - 5.0,
        knobs: vec![KnobScore {
            addr: addr(CUTOFF),
            verdict: if passed { Verdict::Good } else { Verdict::Bad },
        }],
    }
}

fn prepared() -> JobResult {
    JobResult::Prepared {
        start: Box::new(start_patch()),
        target: Box::new(target_patch()),
        target_lufs: Some(-14.0),
    }
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
            GameRequest::Log(entry) => Some(entry.clone()),
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
    s.game_start(challenge()).unwrap();
    let prepare = job(&drain(s), "prepare");
    s.game_job_done(prepare.id, prepared()).unwrap();
}

/// Submit, and answer the score job with `report`. Returns the submit's job.
fn submit(s: &mut Session, report: ScoreReport) -> Job {
    s.game_submit().unwrap();
    let score = job(&drain(s), "score");
    s.game_job_done(score.id, JobResult::Scored(report))
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
    s.game_start(challenge()).unwrap();
    // Preparing: the player's patch still plays, and the target is asked for.
    let v = view(&s);
    assert_eq!(
        (v.phase, v.pending.preparing, v.code.as_str()),
        (Phase::Preparing, true, CODE)
    );
    assert_eq!(s.patch().to_json(), before.patch);
    let requests = drain(&mut s);
    let prepare = job(&requests, "prepare");
    assert!(matches!(&prepare.kind, JobKind::Prepare { challenge } if challenge.code() == CODE));
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
    s.game_job_done(score.id, JobResult::Scored(report(false, 41.0)))
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
    submit(&mut s, report(true, 92.5));
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
    submit(&mut s, report(true, 95.0));
    assert_eq!(view(&s).time_to_pass_ms, Some(7_000));

    s.game_vote(Vote::Right).unwrap();
    s.game_end(EndChoice::Restore).unwrap();
    assert_eq!(s.round(), None);
    let requests = drain(&mut s);
    let lines = logs(&requests);
    assert_eq!(lines.len(), 1);
    let line = &lines[0];
    assert_eq!(line.schema, "enth.round-log/1");
    assert_eq!(
        (line.event, line.outcome, line.ended_by),
        (LogEvent::End, Outcome::Passed, Some(EndedBy::Restore))
    );
    assert_eq!((line.active_ms, line.time_to_pass_ms), (8_000, Some(7_000)));
    assert_eq!(line.submits.len(), 3);
    assert_eq!(line.vote, Some(Vote::Right));
    assert_eq!(line.code, CODE);
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
    assert_eq!(s.game_vote(Vote::TooLow), Err(GameError::NotScored));
    let edits = &s.game_record().unwrap().edits;
    assert_eq!(edits.last().unwrap().source, EditSource::Reveal);

    s.game_end(EndChoice::Keep).unwrap();
    let line = &logs(&drain(&mut s))[0];
    assert_eq!(
        (line.outcome, line.revealed, line.assisted),
        (Outcome::Revealed, true, true)
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
    s.game_job_done(score.id, JobResult::Scored(report(true, 90.0)))
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
    s.game_start(Challenge::new("R1-TEST-0002")).unwrap();
    let requests = drain(&mut s);
    let line = &logs(&requests)[0];
    assert_eq!(
        (line.outcome, line.ended_by, line.code.as_str()),
        (Outcome::Abandoned, Some(EndedBy::NewRound), CODE)
    );
    assert!(
        requests.iter().any(|r| matches!(r, GameRequest::Cancel(_))),
        "the old loudness job is cancelled"
    );
    let prepare = job(&requests, "prepare");
    assert_eq!(view(&s).code, "R1-TEST-0002");
    assert_eq!(view(&s).active_ms, 0);
    assert_eq!(s.game_record().unwrap().id.serial, 2);
    s.game_job_done(prepare.id, prepared()).unwrap();
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
    s.game_start(challenge()).unwrap();
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
    let line = &logs(&drain(&mut s))[0];
    assert_eq!(
        (line.outcome, line.active_ms, line.submits.len()),
        (Outcome::Abandoned, 2_500, 0)
    );
}

#[test]
fn a_dispute_is_logged_at_once_and_again_with_the_end() {
    let mut s = Session::new(custom_patch());
    play(&mut s, 0);
    submit(&mut s, report(false, 30.0));
    s.game_vote(Vote::TooLow).unwrap();
    s.game_vote(Vote::TooHigh).unwrap();
    let note = "x".repeat(5_000);
    s.game_dispute(&note).unwrap();
    let line = logs(&drain(&mut s)).pop().unwrap();
    assert_eq!(
        (line.event, line.outcome, line.ended_by),
        (LogEvent::Dispute, Outcome::InProgress, None)
    );
    assert_eq!(
        line.dispute.as_ref().unwrap().chars().count(),
        2_000,
        "bounded"
    );
    assert_eq!(
        line.vote,
        Some(Vote::TooHigh),
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
        s.game_job_done(score.id, JobResult::Rings(vec![])),
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
    s.game_start(Challenge::new("R1-TEST-0002")).unwrap();
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
    s.game_start(challenge()).unwrap();
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
    submit(&mut s, report(false, 50.0));
    s.edit(vec![set(CUTOFF, 800.0)], None).unwrap();
    s.game_tick(3 * SETTLE, EditorView::Visible);
    let requests = drain(&mut s);
    let rings = job(&requests, "rings");
    s.game_job_done(
        rings.id,
        JobResult::Rings(vec![KnobScore {
            addr: addr(CUTOFF),
            verdict: Verdict::Good,
        }]),
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
    s.game_start(challenge()).unwrap();
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
    submit(&mut s, report(true, 99.0));
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
    s.game_start(challenge()).unwrap();
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
    s.game_start(challenge()).unwrap();
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
        submit(&mut s, report(false, 10.0));
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
    submit(&mut s, report(false, 35.0));
    s.game_hint().unwrap();
    let hint = job(&drain(&mut s), "hint");
    s.game_job_done(
        hint.id,
        JobResult::Hint(enth_session::game::CoachHint::new(1, "brighter")),
    )
    .unwrap();
    s.game_tick(7_000, EditorView::Visible);
    s.edit(vec![set(RESONANCE, 33.0)], None).unwrap();
    submit(&mut s, report(false, 61.0));
    s.game_vote(Vote::TooLow).unwrap();
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
    assert_eq!(v.vote, Some(Vote::TooLow));
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
        JobResult::Prepared {
            start: Box::new(start_patch()),
            target: Box::new(other),
            target_lufs: None,
        },
    )
    .unwrap();
    let v = view(&t);
    assert_eq!(v.phase, Phase::Broken);
    assert_eq!(v.error.unwrap().code(), "target_drift");
    assert_eq!(t.game_submit(), Err(GameError::Broken));
    t.game_end(EndChoice::Restore).unwrap();
    assert_eq!(t.patch().to_json(), custom_patch().to_json());
    let line = &logs(&drain(&mut t))[0];
    assert_eq!(line.outcome, Outcome::Broken);
}

#[test]
fn a_saved_round_with_a_bad_code_or_a_newer_phase_restores_broken() {
    let saved = mid_round().save(None);
    for (from, to) in [
        (format!("\"code\":\"{CODE}\""), "\"code\":\"\"".to_owned()),
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
    submit(&mut s, report(true, 93.0));
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
            after.vote
        ),
        (
            live.submits.as_slice(),
            live.active_ms,
            live.time_to_pass_ms,
            live.vote
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
        (line.outcome, line.ended_by),
        (Outcome::Abandoned, Some(EndedBy::Host))
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
    s.game_start(Challenge::new("R1-OTHER")).unwrap();
    let prepare = job(&drain(&mut s), "prepare");
    s.game_job_done(prepare.id, prepared()).unwrap();
    s.load(&saved);
    let requests = drain(&mut s);
    assert_eq!(logs(&requests)[0].code, "R1-OTHER");
    assert_eq!(view(&s).code, CODE);
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
    fn game_start(&mut self, challenge: Challenge) -> Result<(), GameError> {
        let mut v = RoundView::default();
        challenge.code().clone_into(&mut v.code);
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
    fn game_vote(&mut self, _: Vote) -> Result<(), GameError> {
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
        edit.game_start(challenge()).unwrap();
        assert_eq!(edit.round().unwrap().code, CODE);
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
    s.game_start(challenge()).unwrap();
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
    s.game_job_done(score.id, JobResult::Scored(report(false, 20.0)))
        .unwrap();
    s.game_hint().unwrap();
    let hint = job(&drain(&mut s), "hint");
    s.game_job_done(
        hint.id,
        JobResult::Hint(enth_session::game::CoachHint::new(1, "x")),
    )
    .unwrap();
    s.game_vote(Vote::Right).unwrap();
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
