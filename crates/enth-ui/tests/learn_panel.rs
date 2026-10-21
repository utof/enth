//! A lesson through the Learn view (lane SKL's test of its minimal panel, F2l, kept for lane LUI's pane): kittest
//! drives it by label over a real `Session` and the example pack (`cutoff`: Listen, Do, Choose; the REAPER points
//! over the bundled basics pack since lane LFU), the way the plugin's glue does each frame
//! (`enth-plugin/src/editor.rs`): a clock tick, the worker's Learn results handed back (`Session::learn_job_done`), the
//! lesson view handed over (`Editor::set_lesson`), the frame, then the Learn intents applied to the session's
//! `LearnEdit` (`Editor::apply_learn`), and a repaint asked for at the lesson's settle moment.
//!
//! The worker is played by the test between frames: the examples answer with fixed loudness readings, and the Do
//! step's sound check (`brightness<500Hz`) answers by comparing the cutoff with the witness's, standing in for the
//! render-and-judge the plugin's worker runs (`enth-plugin`'s `learn` tests check that on the real renders).
#![cfg(feature = "learn")]

use std::time::Duration;

use egui::{UiBuilder, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use enth_learn::pack::PatchRef;
use enth_learn::player::{CheckVerdict, MeasuredHint, StepRecord};
use enth_learn::{LessonBundle, StepPhase};
use enth_patch::{Op, ParamAddr, Patch};
use enth_session::Session;
use enth_session::game::EditorView;
use enth_session::learn::{LearnEdit, LearnJob, LearnJobKind, LearnRequest, LearnResult};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::overlay::{Emphasis, Overlay};

/// The example pack, as the plugin builds it (its tests' fixture since lane LFU bundles `basics`).
const EXAMPLE: &[u8] = include_bytes!("../../enth-plugin/packs/example.enthpack");

/// The bundled basics pack, as the plugin embeds it (lane LFU): the REAPER `lesson` scenario plays its first lesson.
const BASICS: &[u8] = include_bytes!("../../enth-plugin/packs/basics.enthpack");

fn example() -> LessonBundle {
    let pack = enth_learn::reader::read_archive(EXAMPLE).unwrap();
    pack.bundle(&"cutoff".parse().unwrap()).unwrap()
}

fn cutoff() -> ParamAddr {
    "filter1.cutoff".parse().unwrap()
}

/// The editor, its session, the worker's queue and what the shell did.
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    now: u64,
    jobs: Vec<LearnJob>,
    results: Vec<(enth_session::learn::LearnJobId, LearnResult)>,
    records: Vec<StepRecord>,
    /// The witness's cutoff: the stand-in check passes at or under it.
    witness: f32,
    /// Sound checks run for real (`enth_learn::check::evaluate_sounds`), not by the stand-in (the basics lesson).
    real_checks: bool,
}

/// The worker's part, synchronously.
fn run(job: &LearnJob, witness: f32, real_checks: bool) -> LearnResult {
    match &job.kind {
        LearnJobKind::Check(request) if real_checks => LearnResult::Checked(
            enth_learn::check::evaluate_sounds(&request.sounds, &request.patch).unwrap_or_default(),
        ),
        LearnJobKind::Check(request) => {
            let passed = request.patch.value(&cutoff()).is_some_and(|c| c <= witness);
            LearnResult::Checked(CheckVerdict {
                passed,
                items: Vec::new(),
            })
        }
        LearnJobKind::Examples { examples } => LearnResult::Examples {
            lufs: (0..examples.len())
                .map(|i| Some(-20.0 - 6.0 * i as f32))
                .collect(),
        },
        LearnJobKind::Hint(_) => LearnResult::Hinted(MeasuredHint {
            text: "Try turning Cutoff (filter1.cutoff) down.".to_owned(),
            highlight: Vec::new(),
        }),
        _ => LearnResult::Failed {
            message: "not in this test".to_owned(),
        },
    }
}

fn harness() -> Harness<'static, App> {
    harness_with(Patch::init())
}

/// The plugin's Init patch (`content/presets/init.patch.json`): what a fresh instance in REAPER plays.
fn plugin_init() -> Patch {
    let text = include_str!("../../../content/presets/init.patch.json");
    enth_patch::file::load(text).unwrap().patch
}

fn harness_with(patch: Patch) -> Harness<'static, App> {
    harness_on(patch, "example.enthpack", EXAMPLE, false)
}

/// The harness over `patch` with one bundled pack (`name`, `bytes`), the sound checks real or stood in.
fn harness_on(
    patch: Patch,
    name: &str,
    bytes: &'static [u8],
    real_checks: bool,
) -> Harness<'static, App> {
    let theme = Theme::dark();
    let bundle = example();
    let witness = bundle
        .patch(&PatchRef::new("patches/dark.patch.json").unwrap())
        .unwrap()
        .value(&cutoff())
        .unwrap();
    let mut editor = Editor::new(&theme);
    // As the plugin's glue does at every open (`Glue::set_lessons`): the library and the progress file.
    let mut store = enth_learn::store::MemStore::new();
    store.insert(
        enth_learn::store::Origin::Bundled,
        name,
        enth_learn::store::PackSource::Archive(bytes.to_vec()),
    );
    editor.set_library(
        &enth_learn::store::load_library(&mut store),
        enth_learn::Progress::new(),
    );
    let app = App {
        editor,
        theme: theme.clone(),
        session: Session::new(patch),
        now: 0,
        jobs: Vec::new(),
        results: Vec::new(),
        records: Vec::new(),
        witness,
        real_checks,
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
                app.now += 16;
                let view = if app.now == 16 {
                    EditorView::Opened
                } else {
                    EditorView::Visible
                };
                app.session.game_tick(app.now, view);
                for (id, result) in std::mem::take(&mut app.results) {
                    let _ = app.session.learn_job_done(id, result);
                }
                // The plugin's glue hands over the round too (its header entry stands right of Learn).
                app.editor
                    .set_round(enth_session::game::GameEdit::round(&app.session));
                app.editor.set_lesson(LearnEdit::lesson(&app.session));
                app.editor
                    .frame(&mut root, &app.theme, &mut app.session, down);
                app.editor.apply_learn(&mut app.session);
                if let Some(at) = LearnEdit::lesson(&app.session).and_then(|l| l.wake_at_ms) {
                    ui.ctx()
                        .request_repaint_after(Duration::from_millis(at.saturating_sub(app.now)));
                }
                app.session.drain_requests().for_each(drop);
                app.session.drain_game_requests().for_each(drop);
                let requests: Vec<LearnRequest> = app.session.drain_learn_requests().collect();
                for request in requests {
                    match request {
                        LearnRequest::Run(job) => app.jobs.push(job),
                        LearnRequest::Record { record, .. } => app.records.push(record),
                        _ => {}
                    }
                }
                for job in std::mem::take(&mut app.jobs) {
                    let result = run(&job, app.witness, app.real_checks);
                    app.results.push((job.id, result));
                }
            },
            app,
        )
}

/// Clicks the control named `label` (a button or tab before a text run that says the same word).
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

fn phase(h: &Harness<'_, App>) -> Option<StepPhase> {
    LearnEdit::lesson(&h.state().session).map(|l| l.phase)
}

fn step(h: &Harness<'_, App>) -> Option<u16> {
    LearnEdit::lesson(&h.state().session).and_then(|l| l.lesson.map(|r| r.step))
}

/// Learn, then Start: the lesson's first step, its examples rendered by the worker.
fn start(h: &mut Harness<'_, App>) {
    h.run_steps(2);
    click(h, "Learn");
    click(h, "Start The low-pass cutoff");
    h.run_steps(2);
    assert_eq!(step(h), Some(0));
    assert!(h.query_by_label("Step 1 of 3").is_some());
}

#[test]
fn a_lesson_plays_through_the_panel_by_label() {
    let mut h = harness();
    start(&mut h);

    // Hear: the examples play once the worker rendered them, then Next passes the step.
    click(&mut h, "Listen Open");
    let listen = LearnEdit::lesson(&h.state().session).unwrap().listen;
    assert_eq!(listen.example, Some(0));
    assert!(listen.ready);
    assert_eq!(listen.gains_db.len(), 2, "level-matched against each other");
    click(&mut h, "Stop listening");
    click(&mut h, "Next");
    assert_eq!(step(&h), Some(1));
    assert!(h.query_by_label("Step 2 of 3").is_some());
    assert!(
        h.state()
            .records
            .iter()
            .any(|r| r.step.as_str() == "hear" && r.progress.done)
    );

    // Try: the step's view on the cards: the cutoff outlined, osc 1 locked (dimmed, with the lock glyph).
    let overlay = h.state().editor.learn().overlay().cloned().unwrap();
    assert_eq!(overlay.param(&cutoff()).emphasis, Emphasis::Highlighted);
    let osc1 = enth_patch::ModuleId::new("osc1").unwrap();
    assert!(overlay.module_locked(&osc1));
    assert_eq!(
        overlay.param(&"osc1.level".parse().unwrap()).emphasis,
        Emphasis::Dimmed
    );
    // A locked edit is refused by the session; the cutoff turned down (as its knob would) passes once it settles.
    let level: ParamAddr = "osc1.level".parse().unwrap();
    let refused = h.state_mut().session.edit(
        vec![Op::SetParam {
            addr: level,
            value: 50.0,
        }],
        None,
    );
    assert!(refused.is_err(), "osc1 is locked in this step");
    let witness = h.state().witness;
    h.state_mut()
        .session
        .edit(
            vec![Op::SetParam {
                addr: cutoff(),
                value: witness,
            }],
            None,
        )
        .unwrap();
    h.run_steps(30);
    assert_eq!(phase(&h), Some(StepPhase::Passed));
    assert!(h.query_by_label("Step 2 of 3 ✓ Passed").is_some());
    let try_record = h
        .state()
        .records
        .iter()
        .find(|r| r.step.as_str() == "try")
        .expect("the pass is recorded at once");
    assert!(try_record.progress.clean, "no hint, no Show me");

    // Which: a wrong pick, then the right one, by label.
    click(&mut h, "Next");
    assert_eq!(step(&h), Some(2));
    click(&mut h, "Choose A");
    assert_ne!(phase(&h), Some(StepPhase::Passed));
    click(&mut h, "Choose B");
    assert_eq!(phase(&h), Some(StepPhase::Passed));
    let which = h
        .state()
        .records
        .iter()
        .find(|r| r.step.as_str() == "which")
        .unwrap();
    assert!(!which.progress.clean, "a wrong pick first");

    // Finish, then end keeping the lesson's patch: the pane goes back to the lesson browser.
    click(&mut h, "Finish");
    assert!(LearnEdit::lesson(&h.state().session).unwrap().finished);
    click(&mut h, "End lesson");
    click(&mut h, "End and keep this patch");
    assert!(LearnEdit::lesson(&h.state().session).is_none());
    assert!(h.query_by_label("Start The low-pass cutoff").is_some());
}

#[test]
fn hint_and_show_me_help_on_the_do_step_and_mark_it_assisted() {
    let mut h = harness();
    start(&mut h);
    click(&mut h, "Next");
    click(&mut h, "Hint");
    let hint = LearnEdit::lesson(&h.state().session).unwrap().hint;
    assert_eq!(
        hint.as_deref(),
        Some("The cutoff knob is on the filter. Turn it to the left."),
        "the authored hint first"
    );
    click(&mut h, "Show me");
    h.run_steps(30);
    assert_eq!(
        phase(&h),
        Some(StepPhase::Passed),
        "Show me applied the witness's op"
    );
    let record = h
        .state()
        .records
        .iter()
        .find(|r| r.step.as_str() == "try")
        .unwrap();
    assert!(!record.progress.clean);
    click(&mut h, "End lesson");
    click(&mut h, "End and restore my patch");
    let restored = h.state().session.patch();
    assert_eq!(
        restored.generators,
        Patch::init().generators,
        "Restore brings the patch back"
    );
    assert!(
        restored
            .find(&enth_patch::ModuleId::new("filter1").unwrap())
            .is_none()
    );
}

#[test]
fn a_settled_lesson_asks_for_no_frames() {
    let mut h = harness();
    start(&mut h);
    click(&mut h, "Next");
    let witness = h.state().witness;
    h.state_mut()
        .session
        .edit(
            vec![Op::SetParam {
                addr: cutoff(),
                value: witness,
            }],
            None,
        )
        .unwrap();
    // The settle moment wakes the editor once (the check), then nothing more.
    h.run_steps(30);
    assert_eq!(phase(&h), Some(StepPhase::Passed));
    for n in 0..5 {
        h.step();
        let delay = h.output().viewport_output[&ViewportId::ROOT].repaint_delay;
        assert_eq!(
            delay,
            Duration::MAX,
            "frame {n} of a settled lesson asked for a repaint"
        );
    }
    assert_eq!(h.state().editor.report().repaint, None);
}

/// The REAPER `lesson` scenario (`xtask/reaper/lesson.lua`) clicks the Learn view by position in the plugin's window
/// (the default 1200 x 800, a fresh instance on the plugin's Init patch, the library the plugin hands over: the bundled
/// basics pack since lane LFU): these are its points, so a layout change fails here first. Its path: Learn, Start
/// the tour's first lesson (How a lesson works), ▶ Saw, A on the first Listen step, ■ Stop, Next three times to the
/// Try step, Show me (one jump of the master level passes its `touched` and peak checks), Next to the round.
#[test]
fn reaper_points_hit_their_controls() {
    const LEARN: (f32, f32) = (551.0, 28.0);
    const START: (f32, f32) = (901.0, 160.0);
    const LISTEN_FIRST: (f32, f32) = (910.0, 580.0);
    const STOP: (f32, f32) = (908.0, 608.0);
    const NEXT: (f32, f32) = (1155.0, 636.0);
    const SHOW: (f32, f32) = (957.0, 608.0);
    let print = std::env::var_os("ENTH_PRINT_POINTS").is_some();
    let hits = |h: &Harness<'_, App>, label: &str, (x, y): (f32, f32)| {
        let rect = h
            .query_all_by_label(label)
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
            .unwrap_or_else(|| panic!("no button {label}"))
            .rect();
        if print {
            println!("{label}: {rect:?} (centre {:?})", rect.center());
            return;
        }
        assert!(
            rect.contains(egui::pos2(x, y)),
            "{label} is at {rect:?}, not under ({x}, {y}): update lesson.lua's POINTS and these"
        );
    };
    let mut h = harness_on(plugin_init(), "basics.enthpack", BASICS, true);
    h.run_steps(2);
    hits(&h, "Learn", LEARN);
    click(&mut h, "Learn");
    hits(&h, "Start How a lesson works", START);
    click(&mut h, "Start How a lesson works");
    h.run_steps(3);
    assert_eq!(step(&h), Some(0));
    hits(&h, "Listen Saw, A", LISTEN_FIRST);
    hits(&h, "Stop listening", STOP);
    hits(&h, "Next", NEXT);
    for at in 1..=3 {
        click(&mut h, "Next");
        assert_eq!(step(&h), Some(at));
        hits(&h, "Next", NEXT);
    }
    hits(&h, "Show me", SHOW);
    click(&mut h, "Show me");
    for _ in 0..60 {
        if phase(&h) == Some(StepPhase::Passed) {
            break;
        }
        h.run_steps(1);
    }
    assert_eq!(
        phase(&h),
        Some(StepPhase::Passed),
        "one Show me passes the Try step"
    );
    hits(&h, "Next", NEXT);
}
