//! The Recreate round flow through the editor (lane SKR's skeleton bar, F2r; lane RUI extends it with the real UI):
//! kittest drives the header's Recreate bar by label over a real `Session`, the way the plugin's glue does each frame
//! (docs/editor.md, "The Recreate hook"): a clock tick, the worker's results handed back through
//! `Editor::hold_scores`, the round view handed over (`Editor::set_round`), the frame, then the bar's intents applied
//! to the session's `GameEdit` (`Editor::apply_game`). The game worker is played by the test between frames, running
//! the same `enth-challenge` functions (`prepare_round`, `score_round`, `score_structure`) with the stub scorers.
//!
//! Start → Target ▶ → Mine ▶ → Submit → the blind rating (the score held back until it is given) → Reveal → End.

mod common;

use egui::UiBuilder;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_challenge::{prepare_round, score_round, score_structure};
use enth_session::Session;
use enth_session::game::{
    EditorView, GameEdit, GameRequest, Job, JobKind, JobResult, Listen, Phase, Prepared,
};
use enth_ui::Theme;
use enth_ui::editor::Editor;

/// The editor, its session, the jobs the session asked for and the results waiting to be handed back.
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    now: u64,
    jobs: Vec<Job>,
    results: Vec<(enth_session::game::JobId, JobResult)>,
    seed: u64,
}

/// The worker's part, synchronously: the same functions the plugin's worker calls.
fn run(job: &Job) -> JobResult {
    match &job.kind {
        JobKind::Prepare { spec } => {
            let round = prepare_round(spec).unwrap();
            JobResult::Prepared(Box::new(Prepared::from_round(&round, None)))
        }
        JobKind::Score {
            challenge, patch, ..
        } => {
            let round = enth_challenge::prepare_challenge((**challenge).clone()).unwrap();
            JobResult::Scored(Box::new(score_round(&round, patch).unwrap()))
        }
        JobKind::Rings {
            challenge, patch, ..
        } => {
            let round = enth_challenge::prepare_challenge((**challenge).clone()).unwrap();
            JobResult::Rings(Box::new(score_structure(&round, patch).unwrap()))
        }
        JobKind::Loudness { .. } => JobResult::Loudness { lufs: None },
        _ => JobResult::Failed {
            message: "not in this test".to_owned(),
        },
    }
}

fn harness() -> Harness<'static, App> {
    harness_with(false)
}

/// The harness, rendering on lavapipe when `render` (snapshots).
fn harness_with(render: bool) -> Harness<'static, App> {
    let theme = Theme::dark();
    let app = App {
        editor: Editor::new(&theme),
        theme: theme.clone(),
        session: Session::new(common::patches::p1_default()),
        now: 0,
        jobs: Vec::new(),
        results: Vec::new(),
        seed: 42,
    };
    let mut builder = Harness::builder()
        .with_size(theme.size.window.default)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| theme.apply(ctx));
    if render {
        builder = builder.renderer(common::cpu_renderer());
    }
    builder.build_ui_state(
        |ui, app: &mut App| {
            let down = ui.input(|i| i.pointer.any_down());
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
            // The glue's frame: the clock, results back, the round view, draw, the bar's intents.
            app.now += 16;
            let view = if app.now == 16 {
                EditorView::Opened
            } else {
                EditorView::Visible
            };
            app.session.game_tick(app.now, view);
            for (id, result) in app.editor.hold_scores(std::mem::take(&mut app.results)) {
                let _ = app.session.game_job_done(id, result);
            }
            app.editor.set_round(app.session.round());
            app.editor
                .frame(&mut root, &app.theme, &mut app.session, down);
            let seed = app.seed;
            app.editor.apply_game(&mut app.session, &mut || seed);
            app.session.drain_requests().for_each(drop);
            for request in app.session.drain_game_requests() {
                if let GameRequest::Run(job) = request {
                    app.jobs.push(job);
                }
            }
            // The worker, between frames.
            for job in std::mem::take(&mut app.jobs) {
                let result = run(&job);
                app.results.push((job.id, result));
            }
        },
        app,
    )
}

/// Open the bar's `menu` and click its `item`.
fn menu(h: &mut Harness<'_, App>, menu: &str, item: &str) {
    h.get_by_label(menu).click();
    h.run_steps(2);
    h.get_by_label(item).click();
}

fn phase(h: &Harness<'_, App>) -> Option<Phase> {
    h.state().session.round().map(|r| r.phase)
}

#[test]
fn a_round_from_start_to_end_through_the_bar() {
    let mut h = harness();
    h.run_steps(2);
    assert_eq!(phase(&h), None);
    let own = h.state().session.patch().clone();

    h.get_by_label("Start").click();
    h.run_steps(3);
    assert_eq!(phase(&h), Some(Phase::Playing), "prepared on the worker");
    let target = enth_challenge::prepare_round(
        &enth_challenge::spec::SpecPreset::by_id(1)
            .unwrap()
            .spec(enth_challenge::spec::LATEST_GENERATOR, 42),
    )
    .unwrap();
    assert_eq!(h.state().session.patch(), target.start(), "the start plays");

    h.get_by_label("Target ▶").click();
    h.run_steps(2);
    assert_eq!(h.state().session.game_ab().listen, Listen::Target);
    h.get_by_label("Mine ▶").click();
    h.run_steps(2);
    assert_eq!(h.state().session.game_ab().listen, Listen::Mine);

    // The first Submit asks the blind rating; the score is computed but held back until it is given.
    menu(&mut h, "Round", "Submit");
    h.run_steps(3);
    assert!(h.state().editor.recreate().asking());
    let round = h.state().session.round().unwrap();
    assert_eq!(round.first, None, "the score waits for the rating");
    assert!(round.pending.scoring);
    menu(&mut h, "Rate yours", "Rate 3");
    h.run_steps(3);
    let round = h.state().session.round().unwrap();
    assert_eq!(round.rating, Some(3), "the rating came before the score");
    assert!(round.first.is_some(), "then the score");
    assert!(
        h.state()
            .editor
            .recreate()
            .readout()
            .is_some_and(|r| r.contains("Sound")),
        "the status line's readout shows the score"
    );

    menu(&mut h, "Round", "Reveal");
    h.run_steps(2);
    assert_eq!(phase(&h), Some(Phase::Revealed));
    assert_eq!(
        h.state().session.patch(),
        target.target(),
        "the target is loaded"
    );

    menu(&mut h, "Round", "End");
    h.run_steps(2);
    assert_eq!(phase(&h), None);
    let mut back = h.state().session.patch().clone();
    back.engine.clone_from(&own.engine);
    assert_eq!(back, own, "the player's own patch is back");
    let _ = h.get_by_label("Start");
}

#[test]
fn skipping_the_rating_shows_the_score_without_one() {
    let mut h = harness();
    h.run_steps(2);
    h.get_by_label("Start").click();
    h.run_steps(3);
    menu(&mut h, "Round", "Submit");
    h.run_steps(3);
    menu(&mut h, "Rate yours", "Skip");
    h.run_steps(3);
    let round = h.state().session.round().unwrap();
    assert_eq!(round.rating, None);
    assert_eq!(round.submits, 1);
    assert!(!h.state().editor.recreate().asking());
}

/// The editor with a live round (Target ▶ on), at the default window: where the skeleton bar sits (between the
/// Presets group and Add), and its readout on the status line. Update on purpose and look at it:
/// `UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test recreate`.
#[test]
fn a_live_round_snapshot() {
    let mut h = harness_with(true);
    h.run_steps(2);
    h.get_by_label("Start").click();
    h.run_steps(3);
    h.get_by_label("Target ▶").click();
    h.run_steps(3);
    h.snapshot("recreate_round_live");
}
