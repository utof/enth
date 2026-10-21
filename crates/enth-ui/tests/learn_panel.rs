//! A lesson through lane SKL's minimal Learn panel (freeze point F2l; lane LUI's view replaces the panel): kittest
//! drives it by label over a real `Session` and the bundled example pack (`cutoff`: Listen, Do, Choose), the way the
//! plugin's glue does each frame (`enth-plugin/src/editor.rs`): a clock tick, the worker's Learn results handed back
//! (`Session::learn_job_done`), the lesson view handed over (`Editor::set_lesson`), the frame, then the Learn intents
//! applied to the session's `LearnEdit` (`Editor::apply_learn`), and a repaint asked for at the lesson's settle moment.
//!
//! The worker is played by the test between frames: the examples answer with fixed loudness readings, and the Do
//! step's sound check (`brightness<500Hz`) answers by comparing the cutoff with the witness's, standing in for the
//! render-and-judge the plugin's worker runs (`enth-plugin`'s `learn` tests check that on the real renders).
#![cfg(feature = "learn")]

use std::time::Duration;

use egui::{UiBuilder, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_learn::pack::PatchRef;
use enth_learn::player::{CheckVerdict, MeasuredHint, StepRecord};
use enth_learn::{LessonBundle, StepPhase};
use enth_patch::{Op, ParamAddr, Patch};
use enth_session::Session;
use enth_session::game::EditorView;
use enth_session::learn::{LearnEdit, LearnJob, LearnJobKind, LearnRequest, LearnResult};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::overlay::{Emphasis, Overlay, Pointer};

/// The bundled example pack, as the plugin embeds it.
const EXAMPLE: &[u8] = include_bytes!("../../enth-plugin/packs/example.enthpack");

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
}

/// The worker's part, synchronously.
fn run(job: &LearnJob, witness: f32) -> LearnResult {
    match &job.kind {
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
    let theme = Theme::dark();
    let bundle = example();
    let witness = bundle
        .patch(&PatchRef::new("patches/dark.patch.json").unwrap())
        .unwrap()
        .value(&cutoff())
        .unwrap();
    let mut editor = Editor::new(&theme);
    editor.set_lessons(vec![bundle], Vec::new());
    let app = App {
        editor,
        theme: theme.clone(),
        session: Session::new(Patch::init()),
        now: 0,
        jobs: Vec::new(),
        results: Vec::new(),
        records: Vec::new(),
        witness,
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
                    let result = run(&job, app.witness);
                    app.results.push((job.id, result));
                }
            },
            app,
        )
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
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

    // Try: the step's view on the cards: the cutoff outlined, osc 1 dimmed (locked).
    let overlay = h.state().editor.learn().overlay().cloned().unwrap();
    assert_eq!(overlay.param(&cutoff()).hint, Some(Pointer::Here));
    let osc1 = enth_patch::ModuleId::new("osc1").unwrap();
    assert_eq!(overlay.module(&osc1), Emphasis::Dimmed);
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

    // Finish, then end keeping the lesson's patch: the panel goes back to the lesson list.
    click(&mut h, "Finish");
    assert!(LearnEdit::lesson(&h.state().session).unwrap().finished);
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
