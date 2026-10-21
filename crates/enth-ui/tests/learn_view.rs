//! The Learn view (lane LUI) over a real `Session`, the way the plugin's glue drives it each frame
//! (`enth-plugin/src/editor.rs`): a clock tick, the worker's Learn and game results handed back, the round and the
//! lesson views handed over (`Editor::set_round`, `Editor::set_lesson`), the frame, then the round's and the lesson's
//! intents applied (`Editor::apply_game`, `Editor::apply_learn`).
//!
//! The worker is played by the test between frames with the real functions: sound checks are `enth-learn`'s
//! `evaluate_sounds` (the render and the judgement the plugin's worker runs), a lesson's round is prepared with
//! `enth_challenge::prepare_challenge` and scored with `score_round` and `score_structure`; the examples answer with
//! fixed loudness readings and the hints with fixed texts. The lessons are the `basics` pack's (lane C1), built from
//! `content/lessons/basics` as `cargo xtask pack` builds it.
//!
//! Acceptance (docs/phase-3.md, LUI): two lessons completed by label; the overlay's snapshots (lavapipe); no peeking
//! in a lesson's Recreate step; the overflow report at 1200 × 800; no idle frames.
#![cfg(feature = "learn")]
#![allow(
    clippy::disallowed_methods,
    reason = "the test reads the basics pack from content/lessons"
)]

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use egui::{UiBuilder, ViewportId};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, SnapshotResults};
use enth_challenge::{prepare_challenge, score_round, score_structure};
use enth_learn::pack::{MANIFEST_PATH, lesson_path};
use enth_learn::player::{CheckVerdict, MeasuredHint, StepRecord};
use enth_learn::reader::write_archive;
use enth_learn::store::{MemStore, Origin, PackSource, load_library};
use enth_learn::{Lesson, Manifest, Progress, StepKind, StepPhase};
use enth_patch::{Op, ParamAddr, Patch};
use enth_session::Session;
use enth_session::game::{
    CoachHint, EditorView, GameEdit, GameRequest, Job, JobKind, JobResult, Prepared,
};
use enth_session::learn::{LearnEdit, LearnJob, LearnJobKind, LearnRequest, LearnResult};
use enth_ui::Theme;
use enth_ui::editor::cards::format::value_text;
use enth_ui::editor::overlay::{Emphasis, Overlay};
use enth_ui::editor::{Editor, Region};

/// The bundled example pack, as the plugin embeds it.
const EXAMPLE: &[u8] = include_bytes!("../../enth-plugin/packs/example.enthpack");

/// `content/lessons/<pack>/` built as `cargo xtask pack` builds it (lane LCK): TOML through the same serde types,
/// texts, patches and credits copied, the manifest sealed, the canonical archive.
fn build(pack: &str) -> Vec<u8> {
    build_with(pack, &[])
}

/// [`build`], with some files' contents replaced (`(pack path, text)`).
fn build_with(pack: &str, replace: &[(&str, &str)]) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../content/lessons")
        .join(pack);
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
    for (path, text) in replace {
        files.insert((*path).to_owned(), text.as_bytes().to_vec());
    }
    let sealed = manifest.seal(&files).unwrap();
    files.insert(MANIFEST_PATH.to_owned(), sealed.to_json().into_bytes());
    write_archive(&files)
}

/// The basics pack's starter (`patches/saw.patch.json`): what most lessons start from.
fn starter() -> Patch {
    let text = include_str!("../../../content/lessons/basics/patches/saw.patch.json");
    enth_patch::file::load(text).unwrap().patch
}

/// A library with the basics pack (and the example pack when `example`), as the plugin's store reads it.
fn library(example: bool) -> enth_learn::store::Library {
    let mut store = MemStore::new();
    store.insert(
        Origin::Bundled,
        "basics.enthpack",
        PackSource::Archive(build("basics")),
    );
    if example {
        store.insert(
            Origin::Bundled,
            "example.enthpack",
            PackSource::Archive(EXAMPLE.to_vec()),
        );
    }
    let library = load_library(&mut store);
    assert!(library.problems.is_empty(), "{:?}", library.problems);
    library
}

/// The editor, its session, the worker's queues and what the shell did.
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    now: u64,
    learn_results: Vec<(enth_session::learn::LearnJobId, LearnResult)>,
    game_results: Vec<(enth_session::game::JobId, JobResult)>,
    records: Vec<StepRecord>,
    /// Each lesson round's prepared target, newest last (the test turns the knobs to it).
    targets: Vec<enth_challenge::PreparedRound>,
}

/// The Learn worker's part, synchronously.
fn run_learn(job: &LearnJob) -> LearnResult {
    match &job.kind {
        LearnJobKind::Check(request) => LearnResult::Checked(
            enth_learn::check::evaluate_sounds(&request.sounds, &request.patch)
                .unwrap_or_else(|_| CheckVerdict::default()),
        ),
        LearnJobKind::Examples { examples } => LearnResult::Examples {
            lufs: (0..examples.len())
                .map(|i| Some(-20.0 - 3.0 * i as f32))
                .collect(),
        },
        LearnJobKind::Hint(_) => LearnResult::Hinted(MeasuredHint {
            text: "Turn the highlighted control and listen.".to_owned(),
            highlight: Vec::new(),
        }),
        _ => LearnResult::Failed {
            message: "not in this test".to_owned(),
        },
    }
}

/// The game worker's part for a lesson's round, synchronously: the same functions the plugin's worker calls.
fn run_game(job: &Job, targets: &mut Vec<enth_challenge::PreparedRound>) -> JobResult {
    match &job.kind {
        JobKind::PrepareChallenge { challenge } => {
            let round = prepare_challenge((**challenge).clone()).unwrap();
            let prepared = Prepared::from_round(&round, None);
            targets.push(round);
            JobResult::Prepared(Box::new(prepared))
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
        JobKind::Hint { level, .. } => JobResult::Hint(Box::new(CoachHint::new(
            *level,
            "Check the tone: yours is brighter than the target.",
        ))),
        _ => JobResult::Failed {
            message: "not in this test".to_owned(),
        },
    }
}

/// The harness over `patch` with the given library, rendering on lavapipe when `render` (snapshots). Tooltips show
/// at once, so the no-peek test reads every one.
fn harness_with(patch: Patch, example: bool, render: bool) -> Harness<'static, App> {
    let theme = Theme::dark();
    let mut editor = Editor::new(&theme);
    editor.set_library(&library(example), Progress::new());
    let app = App {
        theme: theme.clone(),
        editor,
        session: Session::new(patch),
        now: 0,
        learn_results: Vec::new(),
        game_results: Vec::new(),
        records: Vec::new(),
        targets: Vec::new(),
    };
    let mut builder = Harness::builder()
        .with_size(theme.size.window.default)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| {
            theme.apply(ctx);
            ctx.global_style_mut(|s| {
                s.interaction.tooltip_delay = 0.0;
                s.interaction.show_tooltips_only_when_still = false;
            });
        });
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
            for (id, result) in std::mem::take(&mut app.learn_results) {
                let _ = app.session.learn_job_done(id, result);
            }
            app.editor.set_round(app.session.round());
            app.editor.set_lesson(LearnEdit::lesson(&app.session));
            app.editor
                .frame(&mut root, &app.theme, &mut app.session, down);
            app.editor.apply_game(&mut app.session, &mut || 7);
            app.editor.apply_learn(&mut app.session);
            let wake = [
                app.session.round().and_then(|r| r.wake_at_ms),
                LearnEdit::lesson(&app.session).and_then(|l| l.wake_at_ms),
            ]
            .into_iter()
            .flatten()
            .min();
            if let Some(at) = wake {
                ui.ctx()
                    .request_repaint_after(Duration::from_millis(at.saturating_sub(app.now)));
            }
            app.session.drain_requests().for_each(drop);
            let game: Vec<GameRequest> = app.session.drain_game_requests().collect();
            for request in game {
                if let GameRequest::Run(job) = request {
                    let result = run_game(&job, &mut app.targets);
                    app.game_results.push((job.id, result));
                }
            }
            let learn: Vec<LearnRequest> = app.session.drain_learn_requests().collect();
            for request in learn {
                match request {
                    LearnRequest::Run(job) => {
                        let result = run_learn(&job);
                        app.learn_results.push((job.id, result));
                    }
                    LearnRequest::Record { record, .. } => app.records.push(record),
                    _ => {}
                }
            }
        },
        app,
    )
}

fn harness() -> Harness<'static, App> {
    harness_with(Patch::init(), false, false)
}

/// Clicks the control named `label`: a button, tab or link when the step's text also says the word (kittest matches
/// a plain label's text too).
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

/// Whether a control (not a plain text run) named `label` is drawn.
fn has(h: &Harness<'_, App>, label: &str) -> bool {
    use egui::accesskit::Role;
    h.query_all_by_label(label).any(|n| {
        matches!(
            n.accesskit_node().role(),
            Role::Button | Role::Tab | Role::Link | Role::Status
        )
    })
}

/// Drags the knob named `label` by `dy` pixels (down lowers it), as a hand would: press, eight moves, release.
fn drag(h: &mut Harness<'_, App>, label: &str, dy: f32) {
    let at = h
        .query_all_by(|n| {
            n.role() == egui::accesskit::Role::Slider && n.label().as_deref() == Some(label)
        })
        .next()
        .unwrap_or_else(|| panic!("no knob {label}"))
        .rect()
        .center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(1);
    for i in 1..=8 {
        h.event(egui::Event::PointerMoved(
            at + egui::vec2(0.0, dy * i as f32 / 8.0),
        ));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton {
        pos: at + egui::vec2(0.0, dy),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
}

/// Walks the live patch to `witness` in eight edits per control, as a knob drag would (a `touched` predicate counts
/// the span the player's own edits covered; one jump covers none).
fn walk_to(h: &mut Harness<'_, App>, witness: &Patch) {
    let current = h.state().session.patch().clone();
    let ops = enth_patch::ops_between(&current, witness).unwrap();
    for op in ops {
        if let Op::SetParam { addr, value } = &op {
            let spec = current.spec(addr).unwrap();
            let (from, to) = (
                spec.to_norm(current.value(addr).unwrap()),
                spec.to_norm(*value),
            );
            for i in 1..=8 {
                let n = from + (to - from) * i as f32 / 8.0;
                let v = if i == 8 {
                    *value
                } else {
                    spec.clamp(spec.from_norm(n))
                };
                h.state_mut()
                    .session
                    .edit(
                        vec![Op::SetParam {
                            addr: addr.clone(),
                            value: v,
                        }],
                        None,
                    )
                    .unwrap();
                h.run_steps(1);
            }
        } else {
            h.state_mut().session.edit(vec![op], None).unwrap();
            h.run_steps(1);
        }
    }
}

fn lesson(h: &Harness<'_, App>) -> Option<enth_session::learn::LearnView> {
    LearnEdit::lesson(&h.state().session)
}

fn step(h: &Harness<'_, App>) -> Option<u16> {
    lesson(h).and_then(|l| l.lesson.map(|r| r.step))
}

/// Runs frames until `done` holds (at most `max`), so jobs, settles and renders have their turns.
fn until(h: &mut Harness<'_, App>, max: usize, done: impl Fn(&Harness<'_, App>) -> bool) -> bool {
    for _ in 0..max {
        if done(h) {
            return true;
        }
        h.run_steps(1);
    }
    done(h)
}

fn passed(h: &Harness<'_, App>) -> bool {
    lesson(h).is_some_and(|l| l.phase == StepPhase::Passed)
}

/// Turns each of the live round's drawn controls to the target's value, as its knob would.
fn match_the_target(h: &mut Harness<'_, App>) {
    let round = h.state().targets.last().unwrap();
    let ops: Vec<Op> = round
        .params()
        .iter()
        .map(|addr| Op::SetParam {
            addr: addr.clone(),
            value: round.target().value(addr).unwrap(),
        })
        .collect();
    h.state_mut().session.edit(ops, None).unwrap();
    h.run_steps(3);
}

/// Plays the live lesson to its end by label, step by step, reading each step's kind from the lesson: Listen plays an
/// example, flips A/B and goes on; Read goes on; Do asks Show me until it passes; Recreate turns the round's controls
/// to the target, submits and skips the rating; Choose picks the right answer. Returns the step ids it passed.
fn play_through(h: &mut Harness<'_, App>) -> Vec<String> {
    let bundle = {
        let r = lesson(h).unwrap().lesson.unwrap();
        h.state()
            .editor
            .learn()
            .lessons()
            .iter()
            .find(|b| b.pack == r.pack && b.lesson.id == r.lesson)
            .unwrap()
            .clone()
    };
    let steps = bundle.lesson.steps.len();
    let mut seen = Vec::new();
    for (i, s) in bundle.lesson.steps.iter().enumerate() {
        assert_eq!(step(h), Some(u16::try_from(i).unwrap()), "{}", s.id);
        let place = format!("Step {} of {steps}", i + 1);
        assert!(
            h.query_all_by(|n| n.label().is_some_and(|l| l.starts_with(&place)))
                .next()
                .is_some(),
            "{place}"
        );
        // Every link in the step's text names something the step has (lane PLY's lint, drawn as links).
        for link in h.query_all_by(|n| n.role() == egui::accesskit::Role::Link) {
            assert!(link.accesskit_node().label().is_some());
        }
        match &s.kind {
            StepKind::Listen { examples } => {
                assert!(until(h, 10, |h| lesson(h).unwrap().listen.ready));
                click(h, &format!("Listen {}", examples[0].label));
                if let Some(second) = examples.get(1) {
                    click(h, &format!("Listen {}", second.label));
                    click(h, "A/B");
                    assert_eq!(
                        lesson(h).unwrap().listen.example,
                        Some(0),
                        "A/B flips back to the first"
                    );
                }
                click(h, "Stop listening");
            }
            StepKind::Read => {}
            StepKind::Do { witness, .. } => {
                let witness = bundle.patch(witness).unwrap().clone();
                walk_to(h, &witness);
                assert!(until(h, 60, passed), "{}: the witness's values pass", s.id);
            }
            StepKind::Recreate { .. } => {
                assert!(
                    until(h, 20, |h| h
                        .state()
                        .session
                        .round()
                        .is_some_and(|r| r.lesson)),
                    "{}: the round starts",
                    s.id
                );
                assert!(until(h, 20, |h| has(h, "Submit")));
                match_the_target(h);
                click(h, "Submit");
                if has(h, "Skip") {
                    click(h, "Skip");
                }
                assert!(until(h, 40, passed), "{}: a matched round passes", s.id);
            }
            StepKind::Choose { options, answer } => {
                let right = &options[usize::from(*answer)].label;
                click(h, &format!("Choose {right}"));
                assert!(passed(h), "{}: the right answer passes", s.id);
            }
        }
        seen.push(s.id.as_str().to_owned());
        click(h, if i + 1 == steps { "Finish" } else { "Next" });
    }
    assert!(lesson(h).unwrap().finished, "the lesson is complete");
    seen
}

/// Learn, then the lesson's Start on the tour.
fn start(h: &mut Harness<'_, App>, title: &str) {
    h.run_steps(2);
    if !h.state().editor.learn().panel_open() {
        click(h, "Learn");
    }
    click(h, &format!("Start {title}"));
    h.run_steps(2);
    assert_eq!(step(h), Some(0), "{title} starts at its first step");
}

#[test]
fn two_basics_lessons_complete_by_label() {
    let mut h = harness();
    for title in ["How a lesson works", "Low-pass cutoff"] {
        start(&mut h, title);
        let seen = play_through(&mut h);
        let records = &h.state().records;
        for id in &seen {
            assert!(
                records
                    .iter()
                    .any(|r| r.step.as_str() == id && r.progress.done),
                "{title}: step {id} is recorded done"
            );
        }
        click(&mut h, "End lesson");
        click(&mut h, "End and keep this patch");
        assert!(lesson(&h).is_none(), "{title} ended");
        // The browser shows it complete.
        let i = h
            .state()
            .editor
            .learn()
            .lessons()
            .iter()
            .position(|b| b.lesson.title == title)
            .unwrap();
        let standing = h.state().editor.learn().standing(i);
        assert!(standing.completed, "{title}: {standing:?}");
        assert!(has(
            &h,
            &format!("{title} progress {}/{}", standing.steps, standing.steps)
        ));
    }
}

#[test]
fn the_browser_lists_the_tour_first_and_every_pack() {
    let mut h = harness_with(Patch::init(), true, false);
    h.run_steps(2);
    click(&mut h, "Learn");
    let learn = h.state().editor.learn();
    assert_eq!(learn.tracks()[0].title, "The tour", "the tour comes first");
    assert_eq!(learn.packs().len(), 2);
    assert!(has(&h, "The tour"));
    assert!(has(&h, "The example tour"));
    // The tour's lessons in order; a lesson of a pack that is not installed is listed as such.
    let tour = &learn.tracks()[0];
    assert_eq!(tour.entries[0].key.to_string(), "basics/how-a-lesson-works");
    assert!(
        tour.entries
            .iter()
            .any(|e| e.key.to_string() == "shaping/vibrato" && e.lesson.is_none()),
        "a missing pack's lesson is listed as missing"
    );
    assert!(has(&h, "Start How a lesson works"));
    assert!(has(&h, "Start Waveforms"));
    assert!(!has(&h, "Start Pulse width"), "not on the tour");
    click(&mut h, "All lessons");
    assert!(has(&h, "Start Pulse width"));
    assert!(has(&h, "Start The low-pass cutoff"));
    // The Review entry is there, behind nothing yet.
    let review = h.get_by_label("Review");
    assert!(review.accesskit_node().is_disabled());
    click(&mut h, "Hide lessons");
    assert!(!h.state().editor.learn().panel_open());
    assert!(!has(&h, "All lessons"));
}

#[test]
fn the_pane_hides_into_the_header_and_comes_back() {
    let mut h = harness();
    start(&mut h, "Low-pass cutoff");
    assert_eq!(
        h.state().editor.layout().name(),
        "lanes",
        "the editor's own layout"
    );
    let rect = |h: &Harness<'_, App>, r: Region| h.state().editor.report().rect(r);
    assert!(
        rect(&h, Region::Lesson).is_some(),
        "the learn layout places the pane"
    );
    click(&mut h, "Hide lesson");
    assert!(rect(&h, Region::Lesson).is_none(), "hidden: layout 1 again");
    assert!(lesson(&h).is_some(), "the lesson goes on");
    let steps = lesson(&h).unwrap().steps;
    let entry = format!("Lesson 1/{steps}");
    assert!(has(&h, &entry), "the header shows where it stands");
    click(&mut h, &entry);
    assert!(rect(&h, Region::Lesson).is_some());
    click(&mut h, "Lessons");
    assert!(
        has(&h, "Back to the lesson"),
        "the browser over a live lesson"
    );
    click(&mut h, "Back to the lesson");
    assert!(has(&h, &format!("Step 1 of {steps}")));
}

/// A step's text: `enth:` links outline their control on hover; the step's view outlines, locks, hides and focuses.
#[test]
fn a_step_outlines_locks_and_links() {
    let mut h = harness();
    start(&mut h, "Low-pass cutoff");
    // To the Try step (index 4): a focus step on the cutoff, osc 1 locked.
    for _ in 0..4 {
        click(&mut h, "Next");
    }
    let r = lesson(&h).unwrap().lesson.unwrap();
    assert_eq!(r.step, 4);
    let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
    let overlay = h.state().editor.learn().overlay().cloned().unwrap();
    assert_eq!(overlay.param(&cutoff).emphasis, Emphasis::Highlighted);
    assert!(overlay.locked(&"osc1.level".parse().unwrap()));
    let osc1 = enth_patch::ModuleId::new("osc1").unwrap();
    assert!(overlay.module_locked(&osc1));
    assert_eq!(
        overlay.module(&osc1),
        Emphasis::Hidden,
        "a focus step shows only its own modules"
    );
    // Nothing overflows on a focus step at 1200 × 800.
    assert!(
        h.state().editor.report().overflow.is_empty(),
        "{:?}",
        h.state().editor.report().overflow
    );
    // The Name step (index 3) links the cutoff: hovering the link outlines the cutoff knob.
    click(&mut h, "Back");
    let link = h.get_by_label("cutoff").rect().center();
    h.hover_at(link);
    h.run_steps(2);
    let overlay = h.state().editor.learn().overlay().cloned().unwrap();
    assert_eq!(
        overlay
            .param(&"filter1.resonance".parse().unwrap())
            .emphasis,
        Emphasis::Normal
    );
    assert_eq!(overlay.param(&cutoff).emphasis, Emphasis::Highlighted);
    h.hover_at(egui::pos2(1.0, 1.0));
    h.run_steps(2);
    // The Name step highlights the cutoff by itself; the link's hover is gone.
    assert_ne!(
        h.state()
            .editor
            .learn()
            .overlay()
            .unwrap()
            .param(&"filter1.drive".parse().unwrap())
            .emphasis,
        Emphasis::Highlighted
    );
}

/// A locked control says why on the status line, as a refused drag-to-modulate target does.
#[test]
fn a_locked_control_says_why() {
    let mut h = harness();
    start(&mut h, "How a lesson works");
    for _ in 0..3 {
        click(&mut h, "Next");
    }
    // The Try step: osc 1, the filter and the envelope are locked (and out of focus); the master level is the goal.
    assert_eq!(step(&h), Some(3));
    click(&mut h, "Hide lesson");
    // Hidden, the full editor shows the locked modules (focus is the pane's layout rule, not a lock).
    let level = h
        .query_all_by(|n| {
            n.role() == egui::accesskit::Role::Slider && n.label().as_deref() == Some("Level")
        })
        .next()
        .map(|n| n.rect().center())
        .unwrap();
    h.hover_at(level);
    h.run_steps(2);
    let status = h
        .query_all_by(|n| n.label().is_some_and(|l| l.starts_with("Status:")))
        .next()
        .and_then(|n| n.accesskit_node().label())
        .unwrap_or_default();
    assert!(status.contains("Locked in this step"), "{status}");
    let refused = h.state_mut().session.edit(
        vec![Op::SetParam {
            addr: "osc1.level".parse().unwrap(),
            value: 10.0,
        }],
        None,
    );
    assert!(refused.is_err(), "the session refuses it");
}

/// The overflow report at 1200 × 800 (docs/editor.md, "The Learn view"): every step of the two lessons with the pane
/// open, and the pane hidden.
#[test]
fn the_overflow_report_at_1200_by_800() {
    let mut h = harness();
    let mut report = Vec::new();
    for title in ["How a lesson works", "Low-pass cutoff"] {
        start(&mut h, title);
        let steps = usize::from(lesson(&h).unwrap().steps);
        for i in 0..steps {
            until(&mut h, 20, |h| {
                lesson(h).is_some_and(|l| !l.round || h.state().session.round().is_some())
            });
            h.run_steps(3);
            let over = h.state().editor.report().overflow.clone();
            let view = lesson(&h).unwrap().view;
            let trimmed = !view.hide.is_empty() || view.layout == enth_learn::lesson::Layout::Focus;
            report.push(format!(
                "{title} step {} ({}): {over:?}",
                i + 1,
                if trimmed {
                    "hides or focuses"
                } else {
                    "the whole patch"
                }
            ));
            if trimmed {
                assert!(
                    over.is_empty(),
                    "{title} step {}: a step that hides what it does not use fits beside the pane: {over:?}",
                    i + 1
                );
            } else {
                // The starter carried over whole: report 03's 276 px band, and the effects lane by 84 px.
                for (name, o) in &over {
                    assert!(
                        matches!(*name, "modulators" | "effects"),
                        "{title} step {}: {name} {o:?}",
                        i + 1
                    );
                }
            }
            // A round's results drawer takes the overview slot after Submit (lane RUI's, unchanged).
            if lesson(&h).unwrap().round {
                match_the_target(&mut h);
                click(&mut h, "Submit");
                if has(&h, "Skip") {
                    click(&mut h, "Skip");
                }
                until(&mut h, 40, passed);
                h.run_steps(3);
                let over = h.state().editor.report().overflow.clone();
                report.push(format!("{title} step {} after Submit: {over:?}", i + 1));
                assert!(
                    over.is_empty(),
                    "{title} step {}: the results drawer fits the learn layout's overview: {over:?}",
                    i + 1
                );
            }
            if i + 1 < steps {
                click(&mut h, "Next");
            }
        }
        click(&mut h, "End lesson");
        click(&mut h, "End and restore my patch");
    }
    println!("{}", report.join("\n"));
}

/// The full P1 default with the pane open overflows as report 03 measured (the band by 276 px; the effects lane by
/// 84 px, since the overview row spans the window); hidden, layout 1 has no overflow.
#[test]
fn a_full_patch_beside_the_pane_and_the_pane_hidden() {
    let mut h = harness_with(common::patches::p1_default(), false, false);
    h.run_steps(2);
    click(&mut h, "Learn");
    let over = h.state().editor.report().overflow.clone();
    println!("P1 default, browser open: {over:?}");
    let band = over.iter().find(|(n, _)| *n == "modulators").unwrap().1;
    assert!((band.x - 276.0).abs() < 2.0, "{over:?}");
    let effects = over.iter().find(|(n, _)| *n == "effects").unwrap().1;
    assert!((effects.y - 84.0).abs() < 2.0, "{over:?}");
    assert_eq!(
        over.iter().filter(|(n, _)| *n != "learn").count(),
        2,
        "the browser's own list scrolls in the pane: {over:?}"
    );
    click(&mut h, "Hide lessons");
    assert_eq!(h.state().editor.report().overflow, Vec::new());
}

#[test]
fn a_settled_lesson_asks_for_no_frames() {
    let mut h = harness();
    start(&mut h, "How a lesson works");
    let idle = |h: &mut Harness<'_, App>, what: &str| {
        h.run_steps(40);
        for n in 0..4 {
            h.step();
            let delay = h.output().viewport_output[&ViewportId::ROOT].repaint_delay;
            assert_eq!(
                delay,
                Duration::MAX,
                "{what}: frame {n} asked for a repaint"
            );
        }
        assert_eq!(h.state().editor.report().repaint, None, "{what}");
    };
    idle(&mut h, "a Listen step");
    click(&mut h, "Listen Saw, A");
    idle(&mut h, "an example playing");
    click(&mut h, "Stop listening");
    for _ in 0..3 {
        click(&mut h, "Next");
    }
    // The Try step's goal is the master level, in the header: dragged down by hand.
    drag(&mut h, "Master", 100.0);
    assert!(until(&mut h, 60, passed), "a drag down passes");
    idle(&mut h, "a passed Do step");
    click(&mut h, "Lessons");
    idle(&mut h, "the browser");
    click(&mut h, "Back to the lesson");
    click(&mut h, "Next");
    // The round's own timer asks for its next second while it runs (lane RUI's Recreate UI, unchanged); once the
    // round has passed, nothing asks.
    assert!(until(&mut h, 20, |h| has(h, "Submit")));
    match_the_target(&mut h);
    click(&mut h, "Submit");
    if has(&h, "Skip") {
        click(&mut h, "Skip");
    }
    assert!(until(&mut h, 40, passed));
    // The round's clock still asks for its next second (the round stays live until the step is left); the Learn view
    // and the editor's live gating ask for nothing.
    h.run_steps(40);
    for n in 0..4 {
        h.step();
        let delay = h.output().viewport_output[&ViewportId::ROOT].repaint_delay;
        let round = h.state().session.round().and_then(|r| r.wake_at_ms);
        let ms = round.map(|at| at.saturating_sub(h.state().now - 16));
        assert!(
            ms.is_some_and(|ms| delay >= Duration::from_millis(ms.saturating_sub(16))),
            "a passed lesson round: frame {n} asked for {delay:?}, the round's clock for {ms:?} ms"
        );
        assert_eq!(lesson(&h).and_then(|l| l.wake_at_ms), None);
        assert_eq!(h.state().editor.report().repaint, None);
    }
}

/// No peeking in a lesson's Recreate step: while the round is live, no text the editor shows (the pane, the bar,
/// every tooltip and the status line while hovering each control, the menus) contains the target's drawn value as
/// the editor writes it. After Reveal it does, so the check can see it.
#[test]
fn a_lesson_round_shows_no_target_value() {
    let mut h = harness();
    start(&mut h, "Low-pass cutoff");
    // Round 1 (index 5): a cutoff drawn from five values.
    for _ in 0..5 {
        click(&mut h, "Next");
    }
    assert!(until(&mut h, 20, |h| has(h, "Submit")));
    let round = h.state().targets.last().unwrap();
    let values: Vec<String> = round
        .params()
        .iter()
        .map(|a| {
            value_text(
                round.target().spec(a).unwrap(),
                round.target().value(a).unwrap(),
            )
        })
        .collect();
    assert_eq!(values.len(), 1, "round 1 draws the cutoff: {values:?}");
    for a in round.params() {
        let start = value_text(
            round.start().spec(a).unwrap(),
            round.start().value(a).unwrap(),
        );
        assert!(
            !values.contains(&start),
            "the start does not already show {start}"
        );
    }
    let mut seen = texts(&h);
    let rects: Vec<egui::Rect> = h
        .root()
        .query_all_by(|n| {
            matches!(
                n.role(),
                egui::accesskit::Role::Button
                    | egui::accesskit::Role::Slider
                    | egui::accesskit::Role::Switch
                    | egui::accesskit::Role::Link
                    | egui::accesskit::Role::Label
            )
        })
        .map(|n| n.rect())
        .collect();
    assert!(rects.len() > 30, "{} controls", rects.len());
    for r in rects {
        h.hover_at(r.center());
        h.run_steps(2);
        seen.extend(texts(&h));
    }
    click(&mut h, "Hint");
    seen.extend(texts(&h));
    click(&mut h, "End lesson");
    seen.extend(texts(&h));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    let leaks: Vec<&String> = seen
        .iter()
        .filter(|t| values.iter().any(|v| t.contains(v.as_str())))
        .collect();
    assert!(
        leaks.is_empty(),
        "the lesson round shows its target: {leaks:#?}"
    );
    click(&mut h, "Reveal");
    h.run_steps(3);
    let after = texts(&h).join("\n");
    assert!(
        values.iter().any(|v| after.contains(v.as_str())),
        "after Reveal the target's value is on the knob: {values:?}"
    );
}

/// A step's text with every kind of link and widget (lane PLY's AST), drawn by the pane: a play link and a `play`
/// widget play their examples, a `control` widget is the live knob (under its own id: a drag is one undo step on the
/// patch, as on the card), a control link outlines its knob on hover, and the scope and an envelope's shape draw.
#[test]
fn a_steps_text_draws_links_and_widgets() {
    const TEXT: &str = "# Widgets\n\nHear [the open one](enth:play/1), then turn the [cutoff](enth:filter1.cutoff) \
                        *here*:\n\n```enth\nplay 2\ncontrol filter1.cutoff\ncontrol osc1.wave\nscope\nshape env1\n```\n\n\
                        1. `filter1.cutoff` is **the** knob.\n2. [More](https://example.org/filters).\n";
    let theme = Theme::dark();
    let mut editor = Editor::new(&theme);
    let mut store = MemStore::new();
    store.insert(
        Origin::User,
        "example.enthpack",
        PackSource::Archive(build_with("example", &[("lessons/cutoff/step-1.md", TEXT)])),
    );
    editor.set_library(&load_library(&mut store), Progress::new());
    let mut h = harness_with(starter(), false, false);
    h.state_mut().editor = editor;
    start(&mut h, "The low-pass cutoff");
    assert!(until(&mut h, 10, |h| lesson(h).unwrap().listen.ready));
    click(&mut h, "Play Closed");
    assert_eq!(
        lesson(&h).unwrap().listen.example,
        Some(1),
        "the play widget"
    );
    click(&mut h, "the open one");
    assert_eq!(lesson(&h).unwrap().listen.example, Some(0), "the play link");
    // The live knob in the text, beside the card's own.
    let knobs = |h: &Harness<'_, App>| {
        h.query_all_by(|n| {
            n.role() == egui::accesskit::Role::Slider && n.label().as_deref() == Some("Cutoff")
        })
        .count()
    };
    assert_eq!(knobs(&h), 2, "the card's knob and the text's");
    let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
    let before = h.state().session.patch().value(&cutoff).unwrap();
    let pane_knob = h
        .query_all_by(|n| {
            n.role() == egui::accesskit::Role::Slider && n.label().as_deref() == Some("Cutoff")
        })
        .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .unwrap()
        .rect()
        .center();
    assert!(pane_knob.x > 880.0, "the text's knob is in the pane");
    h.hover_at(pane_knob);
    h.run_steps(2);
    assert_eq!(
        h.state()
            .editor
            .learn()
            .overlay()
            .unwrap()
            .param(&cutoff)
            .emphasis,
        Emphasis::Highlighted,
        "hovering the text's knob outlines the card's"
    );
    for (pressed, dy) in [(true, 0.0), (true, 30.0), (true, 60.0), (false, 60.0)] {
        let at = pane_knob + egui::vec2(0.0, dy);
        h.event(egui::Event::PointerMoved(at));
        if dy == 0.0 || !pressed {
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.run_steps(2);
    }
    let after = h.state().session.patch().value(&cutoff).unwrap();
    assert!(
        after < before,
        "the text's knob turned the cutoff down: {before} -> {after}"
    );
    assert!(h.state().session.history().can_undo());
    // A switch or a short enum shows its value as a chip; the web link and the code chip draw.
    assert!(has(&h, "More"));
    assert!(
        texts(&h).iter().any(|t| t.contains("filter1.cutoff")),
        "the code chip"
    );
}

/// Every text the last frame painted and every accessible label, value and description.
fn texts(h: &Harness<'_, App>) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &h.output().shapes {
        walk(&clipped.shape, &mut out);
    }
    for node in h.root().query_all_by(|_| true) {
        let a = node.accesskit_node();
        out.extend(
            [a.label(), a.value(), a.description()]
                .into_iter()
                .flatten(),
        );
    }
    out
}

/// The overlay's snapshots (lavapipe, 1200 × 800): the browser with the tour; a focus Try step (its highlight, the
/// lock glyphs hidden with their modules); the Name step with the cutoff's link hovered; the pane hidden on a step that
/// locks (the lock glyphs on osc 1, the filter and the envelope); a lesson's round in the Recreate UI.
#[test]
fn learn_snapshots() {
    let mut results = SnapshotResults::new();
    let mut h = harness_with(starter(), false, true);
    h.run_steps(2);
    click(&mut h, "Learn");
    h.snapshot("learn_browser");
    results.extend_harness(&mut h);
    click(&mut h, "Start Low-pass cutoff");
    for _ in 0..3 {
        click(&mut h, "Next");
    }
    let link = h.get_by_label("cutoff").rect().center();
    h.hover_at(link);
    h.run_steps(3);
    h.snapshot("learn_name_link");
    results.extend_harness(&mut h);
    h.hover_at(egui::pos2(1.0, 1.0));
    click(&mut h, "Next");
    h.run_steps(3);
    h.snapshot("learn_try_focus");
    results.extend_harness(&mut h);
    click(&mut h, "Next");
    assert!(until(&mut h, 20, |h| has(h, "Submit")));
    h.run_steps(3);
    h.snapshot("learn_round");
    results.extend_harness(&mut h);
    click(&mut h, "End lesson");
    click(&mut h, "End and restore my patch");
    click(&mut h, "Start How a lesson works");
    for _ in 0..3 {
        click(&mut h, "Next");
    }
    click(&mut h, "Hide lesson");
    h.run_steps(3);
    h.snapshot("learn_locked_hidden_pane");
    results.extend_harness(&mut h);
    results.unwrap();
}
