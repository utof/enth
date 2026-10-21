//! The lesson browser's **Review** (lane REV) over a real `Session`, driven the way the plugin's glue drives the
//! Learn view (`tests/learn_view.rs`, lane LUI): a tick, the worker's results back, the round and the lesson handed
//! over, the frame, the intents applied. The test plays the shell too: it runs the round's jobs with the real
//! challenge functions, and on each `LearnRequest::Rings` it applies the rings to its progress file on its day and
//! hands the file back to the editor (`Editor::set_progress`), as `enth-plugin` does.
//!
//! What it checks: the button shows the due count and is enabled once lessons with Recreate rounds are passed; it plays
//! the queue's head item as a Review round (no hint before the first submit); the round's rings move the skill's box,
//! and the due count drops; it is disabled while a lesson is live. Plus the browser's snapshot with Review enabled.
#![cfg(feature = "learn")]
#![allow(
    clippy::disallowed_methods,
    reason = "the test reads the basics pack from content/lessons"
)]

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use egui::UiBuilder;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, SnapshotResults};
use enth_challenge::{prepare_challenge, score_round, score_structure};
use enth_learn::pack::{MANIFEST_PATH, lesson_path};
use enth_learn::player::StepRecord;
use enth_learn::reader::write_archive;
use enth_learn::review::Day;
use enth_learn::store::{MemStore, Origin, PackSource, load_library};
use enth_learn::{Lesson, Manifest, Progress, StepProgress};
use enth_patch::Patch;
use enth_session::Session;
use enth_session::game::{
    CoachHint, EditorView, EndChoice, GameEdit, GameError, GameRequest, Job, JobKind, JobResult,
    Phase, Prepared, RoundOrigin,
};
use enth_session::learn::{LearnEdit, LearnRequest};
use enth_ui::Theme;
use enth_ui::editor::Editor;

/// The day the test's shell says it is.
const DAY: Day = 20_747;

/// `content/lessons/basics/` built as `cargo xtask pack` builds it.
fn basics() -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/lessons/basics");
    let manifest =
        Manifest::from_toml(&fs::read_to_string(dir.join("pack.toml")).unwrap()).unwrap();
    let mut files = BTreeMap::new();
    files.insert(
        "CREDITS.md".to_owned(),
        fs::read(dir.join("CREDITS.md")).unwrap(),
    );
    for entry in fs::read_dir(dir.join("patches")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        files.insert(format!("patches/{name}"), fs::read(&path).unwrap());
    }
    for id in &manifest.lessons {
        for entry in fs::read_dir(dir.join("lessons").join(id.as_str())).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            if name == "lesson.toml" {
                let lesson = Lesson::from_toml(&fs::read_to_string(&path).unwrap()).unwrap();
                files.insert(lesson_path(id), lesson.to_json().into_bytes());
            } else {
                files.insert(format!("lessons/{id}/{name}"), fs::read(&path).unwrap());
            }
        }
    }
    let sealed = manifest.seal(&files).unwrap();
    files.insert(MANIFEST_PATH.to_owned(), sealed.to_json().into_bytes());
    write_archive(&files)
}

fn library() -> enth_learn::store::Library {
    let mut store = MemStore::new();
    store.insert(
        Origin::Bundled,
        "basics.enthpack",
        PackSource::Archive(basics()),
    );
    load_library(&mut store)
}

/// Progress with every basics lesson done (none of them had rung a box yet).
fn every_lesson_passed(library: &enth_learn::store::Library) -> Progress {
    let mut p = Progress::new();
    for entry in &library.packs {
        for id in &entry.pack.manifest().lessons {
            let bundle = entry.pack.bundle(id).unwrap();
            for step in &bundle.lesson.steps {
                p.record(&StepRecord {
                    lesson: bundle.key(),
                    step: step.id.clone(),
                    progress: StepProgress {
                        done: true,
                        ..StepProgress::default()
                    },
                });
            }
        }
    }
    p
}

/// The editor, its session, and the shell's part: the worker's queue and the progress file.
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    now: u64,
    game_results: Vec<(enth_session::game::JobId, JobResult)>,
    progress: Progress,
}

/// The game worker, synchronously: the functions the plugin's worker calls.
fn run_game(job: &Job) -> JobResult {
    match &job.kind {
        JobKind::PrepareChallenge { challenge } => {
            let round = prepare_challenge((**challenge).clone()).unwrap();
            JobResult::Prepared(Box::new(Prepared::from_round(&round, None)))
        }
        JobKind::Score {
            challenge, patch, ..
        } => {
            let round = prepare_challenge((**challenge).clone()).unwrap();
            JobResult::Scored(Box::new(score_round(&round, patch).unwrap()))
        }
        JobKind::Rings {
            challenge, patch, ..
        } => {
            let round = prepare_challenge((**challenge).clone()).unwrap();
            JobResult::Rings(Box::new(score_structure(&round, patch).unwrap()))
        }
        JobKind::Loudness { .. } => JobResult::Loudness { lufs: None },
        JobKind::Hint { level, .. } => JobResult::Hint(Box::new(CoachHint::new(*level, "Listen."))),
        _ => JobResult::Failed {
            message: "not in this test".to_owned(),
        },
    }
}

fn harness(render: bool) -> Harness<'static, App> {
    let theme = Theme::dark();
    let library = library();
    let progress = every_lesson_passed(&library);
    let mut editor = Editor::new(&theme);
    editor.set_library(&library, progress.clone());
    editor.set_review_day(DAY);
    let app = App {
        theme: theme.clone(),
        editor,
        session: Session::new(Patch::init()),
        now: 0,
        game_results: Vec::new(),
        progress,
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
            app.now += 16;
            let view = if app.now == 16 {
                EditorView::Opened
            } else {
                EditorView::Visible
            };
            app.session.game_tick(app.now, view);
            for (id, result) in app
                .editor
                .hold_scores(std::mem::take(&mut app.game_results))
            {
                let _ = app.session.game_job_done(id, result);
            }
            app.editor.set_round(app.session.round());
            app.editor.set_lesson(LearnEdit::lesson(&app.session));
            app.editor
                .frame(&mut root, &app.theme, &mut app.session, down);
            app.editor.apply_game(&mut app.session, &mut || 7);
            app.editor.apply_learn(&mut app.session);
            app.session.drain_requests().for_each(drop);
            let game: Vec<GameRequest> = app.session.drain_game_requests().collect();
            for request in game {
                if let GameRequest::Run(job) = request {
                    app.game_results.push((job.id, run_game(&job)));
                }
            }
            let learn: Vec<LearnRequest> = app.session.drain_learn_requests().collect();
            for request in learn {
                match request {
                    // The shell: re-read, apply the rings on today's day, write, hand the file back.
                    LearnRequest::Rings(rings) => {
                        app.progress.observe(&rings, DAY);
                        app.editor.set_progress(app.progress.clone());
                    }
                    LearnRequest::Record { record, .. } => app.progress.record(&record),
                    LearnRequest::Run(job) => {
                        let _ = job; // a lesson's own jobs are lane LUI's test's
                    }
                    _ => {}
                }
            }
        },
        app,
    )
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    use egui::accesskit::Role;
    let control = h.query_all_by_label(label).find(|n| {
        matches!(
            n.accesskit_node().role(),
            Role::Button | Role::Tab | Role::Link
        )
    });
    match control {
        Some(node) => node.click(),
        None => h.get_by_label(label).click(),
    }
    h.run_steps(3);
}

fn review_enabled(h: &Harness<'_, App>) -> bool {
    !h.get_by_label("Review").accesskit_node().is_disabled()
}

#[test]
fn review_plays_the_weakest_due_skill_and_its_rings_move_the_box() {
    let mut h = harness(false);
    h.run_steps(2);
    click(&mut h, "Learn");
    let queue = h.state().editor.learn().review().clone();
    assert!(
        !queue.is_empty() && queue.due() > 0,
        "every basics skill is due: {}",
        queue.due()
    );
    assert!(review_enabled(&h));
    let item = queue.items[0].clone();
    click(&mut h, "Review");
    h.run_steps(3);
    let record = h.state().session.game_record().cloned().expect("a round");
    assert_eq!(record.origin, RoundOrigin::Review);
    assert_eq!(h.state().session.round().unwrap().phase, Phase::Playing);
    assert_eq!(
        h.state_mut().session.game_hint(),
        Err(GameError::FirstTry),
        "no hint before the first submit"
    );
    // Submit the start as it is (the sibling's target differs from it), then end the round.
    h.state_mut().session.game_submit().unwrap();
    h.run_steps(4);
    assert_eq!(h.state().session.round().unwrap().submits, 1);
    h.state_mut().session.game_end(EndChoice::Restore).unwrap();
    h.run_steps(3);
    let b = h.state().progress.review.get(&item.skill);
    assert_eq!(
        (b.rounds, b.day),
        (1, DAY),
        "{}'s box took the round",
        item.skill
    );
    let after = h.state().editor.learn().review();
    assert!(
        after.due() < queue.due(),
        "rung today, {} is not due: {} then {}",
        item.skill,
        queue.due(),
        after.due()
    );
    assert_ne!(
        after.items[0].skill, item.skill,
        "the next press plays the next item"
    );
}

#[test]
fn review_waits_for_a_live_lesson() {
    let mut h = harness(false);
    h.run_steps(2);
    click(&mut h, "Learn");
    assert!(review_enabled(&h));
    click(&mut h, "Start How a lesson works");
    assert!(LearnEdit::lesson(&h.state().session).is_some());
    click(&mut h, "Lessons");
    assert!(!review_enabled(&h), "a lesson's stash is the only one");
}

/// The browser with Review enabled and its count (lavapipe, 1200 × 800).
#[test]
fn learn_review_snapshot() {
    let mut results = SnapshotResults::new();
    let mut h = harness(true);
    h.run_steps(2);
    click(&mut h, "Learn");
    h.snapshot("learn_review");
    results.extend_harness(&mut h);
    results.unwrap();
}
