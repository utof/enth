//! The create state (lane SK3, freeze point F2s), through the public API: jobs out, posts back, the read model,
//! stale posts refused, cancel and clear, "Use this" as one undo step with one plan and no host writes, and nothing
//! saved.

mod common;

use common::{addr, init_patch};
use enth_patch::{Op, Patch};
use enth_session::create::{
    Candidate, CandidateState, CreateEdit, CreateError, CreateJobId, CreatePhase, CreatePost,
    CreateProgress, CreateRequest, CreateSettings, CreateSource, CreateStop, Provenance, Which,
};
use enth_session::{Activation, Session, ShellRequest};

const ACTIVATION: Activation = Activation {
    sample_rate: 48_000.0,
    offline: false,
};

fn candidate(branch: usize, patch: Patch, proposal: Patch, distance: f64) -> Candidate {
    Candidate {
        branch,
        start: branch,
        provenance: if branch == 0 {
            Provenance::Heuristic
        } else {
            Provenance::Template {
                name: format!("t{branch}"),
            }
        },
        proposal,
        proposal_distance: Some(distance * 2.0),
        patch,
        distance,
        state: CandidateState::Searching,
        rung: 0,
        renders: 10 * branch,
        runs: 1,
        trace: Vec::new(),
    }
}

fn with(patch: &Patch, a: &str, v: f32) -> Patch {
    let mut p = patch.clone();
    enth_patch::apply_all(
        &mut p,
        &[Op::SetParam {
            addr: addr(a),
            value: v,
        }],
    )
    .unwrap();
    p
}

/// Start a job on `kick.wav` and take its `Run`.
fn started(s: &mut Session) -> CreateJobId {
    let id = s
        .create_start(CreateSource::file("/sounds/kick.wav"), CreateSettings::default())
        .unwrap();
    let requests: Vec<CreateRequest> = s.drain_create_requests().collect();
    assert_eq!(requests.len(), 1);
    match &requests[0] {
        CreateRequest::Run(run) => {
            assert_eq!(run.id, id);
            assert_eq!(run.source.name(), "kick.wav");
            assert_eq!(run.settings, CreateSettings::default());
        }
        other => panic!("{other:?}"),
    }
    id
}

fn round(s: &mut Session, id: CreateJobId, candidates: Vec<Candidate>) {
    s.create_post(
        id,
        CreatePost::Round {
            progress: CreateProgress::new(1, 20, 4000, 0.5, 60.0),
            candidates,
        },
    )
    .unwrap();
}

#[test]
fn a_job_goes_out_and_its_posts_build_the_view() {
    let mut s = Session::new(init_patch());
    assert!(s.create().is_none());
    let id = started(&mut s);
    let v = s.create().unwrap();
    assert_eq!((v.id, v.phase, v.file.as_str()), (id, CreatePhase::Starting, "kick.wav"));
    assert!(v.candidates.is_empty());
    // A second start while it runs is refused, and nothing goes out.
    assert_eq!(
        s.create_start(CreateSource::file("b.wav"), CreateSettings::default())
            .map_err(|e| e.code()),
        Err("create_running")
    );
    assert_eq!(s.drain_create_requests().count(), 0);

    let p = init_patch();
    let far = with(&p, "osc1.level", 20.0);
    let near = with(&p, "osc1.level", 60.0);
    round(&mut s, id, vec![candidate(0, far.clone(), p.clone(), 0.5), candidate(1, near.clone(), p.clone(), 0.2)]);
    let v = s.create().unwrap();
    assert_eq!(v.phase, CreatePhase::Searching);
    assert_eq!(v.candidates.len(), 2);
    assert_eq!(v.best(), Some(1), "ranked by distance");
    assert_eq!(v.candidates[0].rank, 1);
    assert_eq!(v.candidates[1].label(), "template t1");
    assert_eq!(v.progress.as_ref().map(|p| p.renders), Some(20));

    s.create_post(
        id,
        CreatePost::Done {
            progress: CreateProgress::new(9, 4000, 4000, 41.0, 60.0),
            stop: CreateStop::Budget,
            candidates: vec![candidate(0, far, p.clone(), 0.4), candidate(1, near, p, 0.1)],
            notes: vec!["no k-NN bank".to_owned()],
        },
    )
    .unwrap();
    let v = s.create().unwrap();
    assert_eq!(v.phase, CreatePhase::Done(CreateStop::Budget));
    assert_eq!(v.notes, vec!["no k-NN bank".to_owned()]);
    // A post after the end, or for another job, is stale and changes nothing.
    assert!(matches!(
        s.create_post(id, CreatePost::Failed { code: "x".into(), message: "x".into(), hint: "x".into() }),
        Err(CreateError::Stale(_))
    ));
    assert!(matches!(
        s.create_post(CreateJobId(99), CreatePost::Failed { code: "x".into(), message: "x".into(), hint: "x".into() }),
        Err(CreateError::Stale(_))
    ));
    assert_eq!(s.create().unwrap().phase, CreatePhase::Done(CreateStop::Budget));
    // A finished job lets the next one start.
    assert!(s.create_start(CreateSource::file("b.wav"), CreateSettings::default()).is_ok());
}

#[test]
fn cancel_and_clear_go_out_once() {
    let mut s = Session::new(init_patch());
    assert_eq!(s.create_cancel().map_err(|e| e.code()), Err("no_create_job"));
    let id = started(&mut s);
    s.create_cancel().unwrap();
    s.create_cancel().unwrap();
    let requests: Vec<CreateRequest> = s.drain_create_requests().collect();
    assert_eq!(requests, vec![CreateRequest::Cancel(id)]);
    assert_eq!(s.create().unwrap().phase, CreatePhase::Cancelling);
    // The shell's last post ends it.
    s.create_post(
        id,
        CreatePost::Done {
            progress: CreateProgress::new(2, 100, 4000, 3.0, 60.0),
            stop: CreateStop::Cancelled,
            candidates: Vec::new(),
            notes: vec!["cancelled".to_owned()],
        },
    )
    .unwrap();
    assert_eq!(s.create().unwrap().phase, CreatePhase::Done(CreateStop::Cancelled));

    // Clear while running cancels it; its later posts are stale.
    let id = started(&mut s);
    s.create_clear();
    assert!(s.create().is_none());
    assert_eq!(s.drain_create_requests().collect::<Vec<_>>(), vec![CreateRequest::Cancel(id)]);
    assert!(matches!(
        s.create_post(id, CreatePost::Round { progress: CreateProgress::new(0, 0, 0, 0.0, 0.0), candidates: Vec::new() }),
        Err(CreateError::Stale(_))
    ));
}

#[test]
fn bad_settings_and_no_file_are_refused() {
    let mut s = Session::new(init_patch());
    let bad = CreateSettings {
        candidates: 0,
        ..CreateSettings::default()
    };
    assert_eq!(
        s.create_start(CreateSource::file("a.wav"), bad).map_err(|e| e.code()),
        Err("bad_create_settings")
    );
    assert_eq!(
        s.create_start(CreateSource::file(""), CreateSettings::default())
            .map_err(|e| e.code()),
        Err("no_file")
    );
    assert!(s.create().is_none());
    assert_eq!(s.drain_create_requests().count(), 0);
}

#[test]
fn use_this_is_one_undo_step_one_plan_and_no_host_writes() {
    let mut s = Session::new(init_patch());
    let _engine = s.activate(ACTIVATION).unwrap();
    s.pump();
    let _ = s.drain_requests().count();
    let before = s.patch().clone();
    let epoch = s.status().epoch;

    let id = started(&mut s);
    let refined = with(&before, "osc1.level", 40.0);
    let proposal = with(&before, "osc1.level", 30.0);
    round(&mut s, id, vec![candidate(0, refined.clone(), proposal.clone(), 0.3)]);
    assert_eq!(
        s.create_use(3, Which::Refined).map_err(|e| e.code()),
        Err("no_candidate")
    );
    s.create_use(0, Which::Refined).unwrap();
    assert_eq!(s.patch().value(&addr("osc1.level")), Some(40.0));
    assert_eq!(s.patch().meta.name, "kick", "named after the file");
    assert_eq!(s.create().unwrap().used, Some((0, Which::Refined)));
    let requests: Vec<ShellRequest> = s.drain_requests().collect();
    assert!(requests.contains(&ShellRequest::PatchEdited), "{requests:?}");
    assert!(
        !requests.iter().any(|r| matches!(r, ShellRequest::Host(_))),
        "no host writes: {requests:?}"
    );
    assert_eq!(s.status().epoch, epoch + 1, "one plan");
    assert!(s.history().can_undo());

    // One undo brings the patch back whole.
    assert_eq!(s.undo(), Ok(true));
    assert_eq!(s.patch().value(&addr("osc1.level")), before.value(&addr("osc1.level")));
    assert_eq!(s.patch().meta.name, before.meta.name);
    assert!(!s.history().can_undo());

    // The proposal loads the same way.
    s.create_use(0, Which::Proposal).unwrap();
    assert_eq!(s.patch().value(&addr("osc1.level")), Some(30.0));
}

#[test]
fn audition_goes_out_with_the_patch() {
    let mut s = Session::new(init_patch());
    let id = started(&mut s);
    assert_eq!(
        s.create_audition(0, Which::Refined).map_err(|e| e.code()),
        Err("no_candidate")
    );
    let p = with(&init_patch(), "osc1.level", 50.0);
    round(&mut s, id, vec![candidate(0, p.clone(), init_patch(), 0.3)]);
    s.create_audition(0, Which::Refined).unwrap();
    s.create_stop_audition();
    let requests: Vec<CreateRequest> = s.drain_create_requests().collect();
    assert_eq!(requests.len(), 2);
    match &requests[0] {
        CreateRequest::Audition { id: j, branch, which, patch } => {
            assert_eq!((*j, *branch, *which), (id, 0, Which::Refined));
            assert_eq!(**patch, p);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(requests[1], CreateRequest::StopAudition);
}

#[test]
fn a_create_job_is_not_saved() {
    let mut s = Session::new(init_patch());
    let saved = s.save_instance();
    let id = started(&mut s);
    round(&mut s, id, vec![candidate(0, init_patch(), init_patch(), 0.3)]);
    assert_eq!(s.save_instance(), saved, "the enth field keeps its bytes");
    // A host load leaves the job alone.
    let text = s.save_patch(None);
    let _ = s.load_patch(&text);
    assert!(s.create().is_some());
}
