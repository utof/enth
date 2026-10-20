//! Repaint gating and telemetry through a real session and engine (lane ED2). Each test runs the editor in a raw
//! egui context, the way the plugin's window does (pump, frame, drain), renders the engine's blocks in between by hand
//! (deterministic: no audio thread, no clock), and reads the repaint delay egui reports for the frame: exact delays,
//! never wall-clock time.
//!
//! - an idle editor (the tap quiet, nothing visible changing) asks for no frame: the delay is `Duration::MAX`;
//! - while visible live data changes it asks for exactly `motion.animated_interval()` (30 Hz), never less;
//! - a stalled engine (the audio thread never runs) still gets every editor frame, and edits still apply.

mod common;

use std::time::Duration;

use egui::{Event, Pos2, RawInput, Rect, ViewportId, vec2};
use enth_patch::Patch;
use enth_session::{Activation, Session};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::cards::ids;
use enth_ui::editor::live::is_visible;

/// The editor over an active session. The engine lives in `render` (its crate is not a dependency of this one).
struct Rig {
    ctx: egui::Context,
    theme: Theme,
    editor: Editor,
    session: Session,
    /// Renders `n` 64-frame blocks (with an optional audition note held).
    render: Box<dyn FnMut(usize)>,
    /// Takes the tap's wake flag, as the shell's `process()` does after each block.
    wake: Box<dyn FnMut() -> bool>,
    time: f64,
}

impl Rig {
    fn new(patch: Patch) -> Rig {
        let theme = Theme::dark();
        let ctx = egui::Context::default();
        theme.apply(&ctx);
        let mut session = Session::new(patch);
        let mut engine = session
            .activate(Activation {
                sample_rate: 48_000.0,
                offline: false,
            })
            .expect("activates");
        let tap = engine.tap().cloned().expect("a tap");
        let render = Box::new(move |n: usize| {
            let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
            for _ in 0..n {
                engine.process(&mut l, &mut r, &[]);
            }
        });
        let wake = Box::new(move || tap.take_wake());
        let mut editor = Editor::new(&theme);
        editor.open(&session, &theme);
        Rig {
            ctx,
            theme,
            editor,
            session,
            render,
            wake,
            time: 0.0,
        }
    }

    /// One editor frame `dt` seconds after the last, with `events`; returns the repaint delay egui reports.
    fn frame_after(&mut self, dt: f64, events: Vec<Event>) -> Duration {
        self.time += dt;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                self.theme.size.window.default,
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let Rig {
            editor,
            session,
            theme,
            ..
        } = self;
        let mut output = self.ctx.run_ui(input, |ui| {
            let down = ui.input(|i| i.pointer.any_down());
            session.pump();
            editor.frame(ui, theme, session, down);
        });
        output.textures_delta.clear();
        self.session.drain_requests().for_each(drop);
        output.viewport_output[&ViewportId::ROOT].repaint_delay
    }

    fn frame(&mut self, events: Vec<Event>) -> Duration {
        self.frame_after(1.0 / 30.0, events)
    }
}

/// A few frames with audio between them, so egui's own start-up transitions (0.1 s) are over.
fn warm_up(rig: &mut Rig) {
    for _ in 0..6 {
        (rig.render)(25);
        rig.frame(Vec::new());
    }
}

/// A few frames with no audio (a stalled engine) and no input.
fn warm_up_without_audio(rig: &mut Rig) {
    for _ in 0..6 {
        rig.frame(Vec::new());
    }
}

/// The init patch: one oscillator and its envelope, nothing global that moves in silence.
fn quiet_patch() -> Patch {
    Patch::init()
}

#[test]
fn an_idle_editor_asks_for_no_frames_and_new_data_wakes_it() {
    let mut rig = Rig::new(quiet_patch());
    // The tap publishes the opening silence until its scope ring is full (4,096 samples), then goes quiet.
    for _ in 0..4 {
        (rig.render)(40);
        rig.frame(Vec::new());
    }
    (rig.render)(200);
    rig.frame(Vec::new());
    for n in 0..20 {
        (rig.render)(25);
        let delay = rig.frame(Vec::new());
        assert_eq!(
            delay,
            Duration::MAX,
            "frame {n} of an idle editor asked for a repaint"
        );
        assert_eq!(rig.editor.report().repaint, None);
    }
    assert!(!(rig.wake)(), "nothing new: no wake-up");

    // An audition note brings new data: the next block's publish sets the wake flag the shell turns into a repaint,
    // and the frame that follows animates at 30 Hz.
    rig.session.audition_on(60, 0.8).unwrap();
    rig.session.pump();
    (rig.render)(4);
    assert!((rig.wake)(), "the sleeping editor is woken by the tap");
    let delay = rig.frame(Vec::new());
    assert_eq!(delay, rig.theme.motion.animated_interval());
}

#[test]
fn visible_live_data_animates_at_30_fps_and_no_faster() {
    // The P1 default's global LFO 2 runs in silence too: its tile and the reverb's Wet ring move.
    let mut rig = Rig::new(common::patches::p1_default());
    let interval = rig.theme.motion.animated_interval();
    assert_eq!(interval, Duration::from_secs_f32(1.0 / 30.0));
    let wet = "reverb1.wet".parse().unwrap();
    let mut rings = Vec::new();
    warm_up(&mut rig);
    for n in 0..30 {
        // A 30 Hz frame's worth of 48 kHz audio between frames.
        (rig.render)(25);
        let delay = rig.frame(Vec::new());
        assert_eq!(delay, interval, "frame {n}: exactly 1/30 s, never sooner");
        assert_eq!(rig.editor.report().repaint, Some(interval));
        rings.push(
            rig.editor
                .live()
                .value(&wet)
                .expect("the ring has a live value"),
        );
    }
    rings.dedup();
    assert!(
        rings.len() > 20,
        "the ring moved from frame to frame: {rings:?}"
    );
    // A note: the scope and the meters run, the delay stays the same.
    rig.session.audition_on(48, 1.0).unwrap();
    rig.session.pump();
    for _ in 0..10 {
        (rig.render)(25);
        assert_eq!(rig.frame(Vec::new()), interval);
    }
    let (scope, _) = rig.editor.live().scope().expect("scope samples");
    assert!(scope.iter().any(|&s| s != 0.0), "the scope holds the note");
    assert!(rig.editor.live().peaks().unwrap()[0] > 0.0);
    assert!(rig.editor.live().voices().unwrap() >= 1);
}

#[test]
fn a_closed_editor_asks_for_nothing_and_stops_the_tap() {
    let mut rig = Rig::new(common::patches::p1_default());
    warm_up(&mut rig);
    // No block since the last frame: nothing new, so even the moving patch sleeps until the next publish.
    assert_eq!(rig.frame(Vec::new()), Duration::MAX);
    (rig.render)(25);
    assert!((rig.wake)(), "the publish woke the sleeping editor");
    assert_eq!(rig.frame(Vec::new()), rig.theme.motion.animated_interval());
    rig.editor.close();
    (rig.render)(25);
    assert_eq!(rig.frame(Vec::new()), Duration::MAX);
    assert!(
        rig.editor
            .live()
            .value(&"reverb1.wet".parse().unwrap())
            .is_none()
    );
    // Reopened, it picks up where the engine is.
    rig.editor.open(&rig.session, &rig.theme);
    (rig.render)(25);
    assert_eq!(rig.frame(Vec::new()), rig.theme.motion.animated_interval());
    assert!(
        rig.editor
            .live()
            .value(&"reverb1.wet".parse().unwrap())
            .is_some()
    );
}

#[test]
fn a_stalled_engine_still_gets_editor_frames_and_edits() {
    let mut rig = Rig::new(common::patches::p1_default());
    // The audio thread never runs from here on: no block is rendered, nothing is published, no plan comes back.
    rig.frame(Vec::new());
    rig.frame(Vec::new());
    let cutoff = "filter1.cutoff".parse().unwrap();
    let before = rig.session.patch().value(&cutoff).unwrap();
    let knob = rig
        .ctx
        .read_response(ids::param(&cutoff))
        .expect("the cutoff knob")
        .rect
        .center();
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    rig.frame(vec![Event::PointerMoved(knob), button(knob, true)]);
    // 200 frames of dragging, each one an edit through the session into a ring nobody drains.
    for n in 1..=200 {
        let at = knob + vec2(0.0, f32::from((n % 40) as u8));
        rig.frame(vec![Event::PointerMoved(at)]);
    }
    rig.frame(vec![button(knob, false)]);
    // No telemetry arrives: once input stops, the editor asks for nothing (and never waited for the engine).
    warm_up_without_audio(&mut rig);
    assert_eq!(rig.frame(Vec::new()), Duration::MAX);
    assert_eq!(rig.editor.report().repaint, None);
    assert_ne!(rig.session.patch().value(&cutoff).unwrap(), before);
    assert_eq!(rig.session.history().next_undo().unwrap().forward.len(), 1);
    // Structural edits while stalled: plans pile up to the in-flight limit, then a compile waits; nothing blocks.
    for _ in 0..6 {
        let id = enth_patch::ModuleId::new("osc2").unwrap();
        let enabled = rig.session.patch().find(&id).unwrap().1.enabled;
        rig.session
            .edit(
                vec![enth_patch::Op::SetEnabled {
                    id,
                    enabled: !enabled,
                }],
                None,
            )
            .unwrap();
        rig.frame(Vec::new());
    }
    let status = rig.session.status();
    assert_eq!(status.plans_in_flight, 2);
    assert!(
        status.compile_pending,
        "the next plan waits for the audio thread"
    );
    // And the stalled audio thread resumes: everything is delivered.
    (rig.render)(20);
    rig.frame(Vec::new());
    (rig.render)(20);
    rig.frame(Vec::new());
    assert!(!rig.session.status().compile_pending);
}

#[test]
fn a_widget_scrolled_out_of_its_region_is_not_visible() {
    let ctx = egui::Context::default();
    let inside = egui::Id::unique("inside");
    let outside = egui::Id::unique("outside");
    let run = |ctx: &egui::Context| {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 100.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            egui::ScrollArea::vertical()
                .max_height(50.0)
                .show(ui, |ui| {
                    let (a, _) = ui.allocate_exact_size(vec2(100.0, 20.0), egui::Sense::hover());
                    ui.interact(a, inside, egui::Sense::click());
                    ui.allocate_exact_size(vec2(100.0, 200.0), egui::Sense::hover());
                    let (b, _) = ui.allocate_exact_size(vec2(100.0, 20.0), egui::Sense::hover());
                    ui.interact(b, outside, egui::Sense::click());
                    assert!(is_visible(ui.ctx(), inside));
                    assert!(!is_visible(ui.ctx(), outside));
                });
        });
        output.textures_delta.clear();
    };
    run(&ctx);
    run(&ctx);
}
