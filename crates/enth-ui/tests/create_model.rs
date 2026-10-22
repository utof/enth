//! The Create view with a proposal model (lane `PUIb`, R3b; feature `create`), over a real `Session`, the test playing
//! the shell as `create_view.rs` does: the intake post names the model (or says why there is none), the tiles of
//! branches that grew from the model's hypotheses read "model k", the compare area names it, and an idle view asks
//! for no frames. Nothing plays.
#![cfg(feature = "create")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use egui::{UiBuilder, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_patch::Patch;
use enth_session::Session;
use enth_session::create::{
    Candidate, CandidateState, CreateEdit, CreateIntake, CreateJobId, CreatePost, CreateProgress,
    CreateRequest, CreateStop, Intake, Provenance,
};
use enth_ui::Theme;
use enth_ui::editor::Editor;

struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    posts: Vec<(CreateJobId, CreatePost)>,
    asked: Vec<CreateRequest>,
}

fn harness() -> Harness<'static, App> {
    let theme = Theme::dark();
    let app = App {
        editor: Editor::new(&theme),
        theme: theme.clone(),
        session: Session::new(Patch::init()),
        posts: Vec::new(),
        asked: Vec::new(),
    };
    Harness::builder()
        .with_size(theme.size.window.default)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .build_ui_state(
            |ui, app: &mut App| {
                let down = ui.input(|i| i.pointer.any_down());
                let rect = ui.ctx().content_rect();
                let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
                for (id, post) in std::mem::take(&mut app.posts) {
                    let _ = app.session.create_post(id, post);
                }
                app.editor.set_create(CreateEdit::create(&app.session));
                app.editor
                    .frame(&mut root, &app.theme, &mut app.session, down);
                app.editor.apply_create(&mut app.session);
                app.session.drain_requests().for_each(drop);
                app.asked.extend(app.session.drain_create_requests());
            },
            app,
        )
}

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("the view never reads the file".to_owned())
    }
}

/// A decaying A3 tone at 48 kHz.
fn tone() -> Vec<f32> {
    let sr = 48_000.0_f32;
    (0..(0.6 * sr) as usize)
        .map(|i| {
            let t = i as f32 / sr;
            (std::f32::consts::TAU * 220.0 * t).sin() * 0.5 * (-4.0 * t).exp()
        })
        .collect()
}

/// The intake post, saying which model the shell uses (`None`: a shell without a lookup).
fn intake(model: Option<&str>) -> CreateIntake {
    let it = Intake::new(&tone(), 1, 48_000).expect("the intake reads the clip");
    let post = CreateIntake::new(
        "WAV, 16-bit, 48000 Hz, mono",
        it.report,
        vec!["objective: logmel-pool85-robust/1".to_owned()],
    );
    match model {
        Some(line) => post.with_model(line),
        None => post,
    }
}

/// Branch `b` of K = 8: the heuristic, three templates, then four of the model's hypotheses (3, 1, 6, 0 by confidence).
fn candidate(b: usize, distance: f64, state: CandidateState) -> Candidate {
    Candidate {
        branch: b,
        start: b,
        provenance: match b {
            0 => Provenance::Heuristic,
            1..=3 => Provenance::Template {
                name: ["pluck_1osc", "pad_2osc", "bass_sub"][b - 1].to_owned(),
            },
            _ => Provenance::Model {
                model: "propose-v0".to_owned(),
                hypothesis: [3, 1, 6, 0][b - 4],
                confidence: Some(1.5 - b as f64 * 0.1),
            },
        },
        proposal: Patch::init(),
        proposal_distance: Some(distance * 2.0),
        patch: Patch::init(),
        distance,
        state,
        rung: 0,
        renders: 50,
        runs: 1,
        trace: Vec::new(),
    }
}

/// The branches after a round: a model branch (hypothesis 4, branch 4) is the closest.
fn candidates(state: CandidateState) -> Vec<Candidate> {
    let d = [0.61, 0.55, 0.72, 0.41, 0.21, 0.34, 0.58, 0.70];
    (0..8).map(|b| candidate(b, d[b], state)).collect()
}

/// Drop a file, then post the intake (with `model`) and a round of branches, some from the model.
fn job(h: &mut Harness<'_, App>, model: Option<&str>) -> CreateJobId {
    h.run_steps(2);
    h.input_mut()
        .dropped_files
        .push(Arc::new(Dropped(PathBuf::from("/sounds/pluck.wav"))));
    h.run_steps(3);
    let id = h
        .state()
        .asked
        .iter()
        .find_map(|r| match r {
            CreateRequest::Run(run) => Some(run.id),
            _ => None,
        })
        .expect("a Run went out");
    h.state_mut()
        .posts
        .push((id, CreatePost::Intake(Box::new(intake(model)))));
    h.state_mut().posts.push((
        id,
        CreatePost::Round {
            progress: CreateProgress::new(1, 400, 4000, 0.9, 60.0),
            candidates: candidates(CandidateState::Searching),
        },
    ));
    h.run_steps(3);
    id
}

fn has(h: &Harness<'_, App>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

/// **Branches from the model, and which model** (`PUIb`'s acceptance): the findings name the model; each tile that grew
/// from a hypothesis reads "model k" (k from 1), and the compare area of the selected one too; the end's notes say
/// what the model gave.
#[test]
fn the_view_names_the_model_and_its_branches() {
    let mut h = harness();
    let id = job(
        &mut h,
        Some("Model: propose-v0 (enth-model-propose-v0.bin)"),
    );
    assert!(
        has(&h, "Model: propose-v0 (enth-model-propose-v0.bin)"),
        "the findings name the model"
    );
    // The closest tile (rank 1) is branch 4, hypothesis 3: "model 4".
    h.get_by_label("Select candidate 1").click();
    h.run_steps(3);
    assert!(
        has(&h, "Candidate 1 • model 4"),
        "the compare area names the hypothesis"
    );
    // Each model tile's label, through its tooltip-free accessible name: select them by rank and read the area.
    for (rank, label) in [(2, "model 2"), (5, "model 7"), (7, "model 1")] {
        h.get_by_label(&format!("Select candidate {rank}")).click();
        h.run_steps(3);
        assert!(
            has(&h, &format!("Candidate {rank} • {label}")),
            "rank {rank} is {label}"
        );
    }
    h.state_mut().posts.push((
        id,
        CreatePost::Done {
            progress: CreateProgress::new(9, 4000, 4000, 31.0, 60.0),
            stop: CreateStop::Budget,
            candidates: candidates(CandidateState::Done),
            notes: vec![
                "model propose-v0: 8 of its 8 hypotheses became starts, 4 of them branches (inference 0.61 s)"
                    .to_owned(),
            ],
        },
    ));
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("4 of them branches").is_some(),
        "the end says what the model gave"
    );
}

/// **No model, and why**: the findings say the job runs search alone; a shell that looks for no model says nothing.
#[test]
fn the_view_says_why_there_is_no_model() {
    let mut h = harness();
    job(
        &mut h,
        Some("No model: enth-model-x.bin was refused (model_mismatch); search alone"),
    );
    assert!(has(
        &h,
        "No model: enth-model-x.bin was refused (model_mismatch); search alone"
    ));
    let mut h = harness();
    job(&mut h, None);
    assert!(h.query_by_label_contains("Model:").is_none());
    assert!(h.query_by_label_contains("No model").is_none());
}

/// **No idle frames** with the model's line and its tiles on screen.
#[test]
fn an_idle_view_with_a_model_asks_for_no_frames() {
    let mut h = harness();
    job(
        &mut h,
        Some("Model: propose-v0 (enth-model-propose-v0.bin)"),
    );
    h.get_by_label("Select candidate 1").click();
    h.run_steps(5);
    for n in 0..5 {
        h.step();
        let delay = h.output().viewport_output[&ViewportId::ROOT].repaint_delay;
        assert_eq!(delay, Duration::MAX, "frame {n} asked for a repaint");
    }
}
