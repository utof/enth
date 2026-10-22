//! A create job through lane SK3's minimal Create panel (freeze point F2s), kept through lane PUI's view (the typed
//! path moved into the folder browser, and a choice starts the job by itself; `create_view.rs` is the view's own flow):
//! kittest drives it by label over a real `Session`, the way the plugin's glue does each frame
//! (`enth-plugin/src/editor.rs`): the shell's posts handed back (`Session::create_post`), the create view handed over
//! (`Editor::set_create`), the frame, then the panel's intents applied to the session's `CreateEdit`
//! (`Editor::apply_create`). The shell is played by the test: it takes the `Run`, posts an intake and a round of two
//! candidates, and records the auditions and cancels it is asked for.
#![cfg(feature = "create")]

use std::time::Duration;

use egui::{UiBuilder, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_patch::{Op, ParamAddr, Patch};
use enth_session::Session;
use enth_session::create::{
    Candidate, CandidateState, CreateEdit, CreateJobId, CreatePhase, CreatePost, CreateProgress,
    CreateRequest, Provenance,
};
use enth_ui::Theme;
use enth_ui::editor::Editor;

struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    /// The shell's queue: posts to hand back at the next frame.
    posts: Vec<(CreateJobId, CreatePost)>,
    /// What the shell was asked for.
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

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn level() -> ParamAddr {
    "osc1.level".parse().unwrap()
}

fn candidate(branch: usize, level_value: f32, distance: f64) -> Candidate {
    let mut patch = Patch::init();
    enth_patch::apply_all(
        &mut patch,
        &[Op::SetParam {
            addr: level(),
            value: level_value,
        }],
    )
    .unwrap();
    Candidate {
        branch,
        start: branch,
        provenance: if branch == 0 {
            Provenance::Heuristic
        } else {
            Provenance::Template {
                name: "pluck_1osc".to_owned(),
            }
        },
        proposal: Patch::init(),
        proposal_distance: Some(1.0),
        patch,
        distance,
        state: CandidateState::Searching,
        rung: 0,
        renders: 10,
        runs: 1,
        trace: Vec::new(),
    }
}

/// Open the panel, choose a file by its path, start.
fn start(h: &mut Harness<'_, App>) -> CreateJobId {
    h.run_steps(2);
    click(h, "Create");
    assert!(
        h.query_by_label("Start create").is_some(),
        "the panel opened"
    );
    click(h, "Browse files");
    h.get_by_label("Sound file path").click();
    h.run();
    h.get_by_label("Sound file path")
        .type_text("/sounds/kick.wav");
    h.run_steps(2);
    click(h, "Choose file");
    click(h, "Start create");
    let run = h
        .state()
        .asked
        .iter()
        .find_map(|r| match r {
            CreateRequest::Run(run) => Some(run.clone()),
            _ => None,
        })
        .expect("Start sent a Run");
    assert_eq!(run.source.name(), "kick.wav");
    run.id
}

#[test]
fn a_job_from_the_panel_to_use_this() {
    let mut h = harness();
    let id = start(&mut h);
    assert_eq!(
        h.state().session.create().map(|v| v.phase),
        Some(CreatePhase::Starting)
    );
    // The shell posts a round: two candidates, the template one closer.
    h.state_mut().posts.push((
        id,
        CreatePost::Round {
            progress: CreateProgress::new(2, 40, 4000, 1.5, 60.0),
            candidates: vec![candidate(0, 20.0, 0.5), candidate(1, 40.0, 0.2)],
        },
    ));
    h.run_steps(3);
    assert!(h.query_by_label("Searching…").is_some(), "the phase shows");
    // Candidate 1 is the best (rank 1): play it, then use it.
    click(&mut h, "Play candidate 1");
    assert!(
        h.state()
            .asked
            .iter()
            .any(|r| matches!(r, CreateRequest::Audition { branch: 1, .. })),
        "{:?}",
        h.state().asked
    );
    click(&mut h, "Use candidate 1");
    assert_eq!(h.state().session.patch().value(&level()), Some(40.0));
    assert_eq!(h.state().session.patch().meta.name, "kick");
    assert!(h.state().session.history().can_undo(), "one undo step");
    // Cancel goes out once.
    click(&mut h, "Cancel create");
    let cancels = h
        .state()
        .asked
        .iter()
        .filter(|r| matches!(r, CreateRequest::Cancel(_)))
        .count();
    assert_eq!(cancels, 1);
}

#[test]
fn an_idle_panel_asks_for_no_frames() {
    let mut h = harness();
    let id = start(&mut h);
    h.state_mut().posts.push((
        id,
        CreatePost::Round {
            progress: CreateProgress::new(1, 10, 4000, 0.5, 60.0),
            candidates: vec![candidate(0, 20.0, 0.5)],
        },
    ));
    h.run_steps(5);
    // The path field holds the keyboard focus from typing (its cursor blinks): Escape lets it go first.
    h.key_press(egui::Key::Escape);
    h.run_steps(5);
    for n in 0..5 {
        h.step();
        let delay = h.output().viewport_output[&ViewportId::ROOT].repaint_delay;
        assert_eq!(
            delay,
            Duration::MAX,
            "frame {n} of an idle panel asked for a repaint"
        );
    }
}
