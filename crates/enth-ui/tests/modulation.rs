//! Drag-to-modulate, depth handles, route removal and the route list, through a real session (lane ED2). kittest
//! drives egui's input and finds controls by their accessibility labels, with no renderer; every assertion is on the
//! session (its patch, its history, its epoch), never on pixels.

mod common;

use egui::accesskit::Role;
use egui::{Pos2, UiBuilder, Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use enth_patch::{ModSource, ParamAddr, Patch, RouteAddr};
use enth_session::{Activation, Session, ShellRequest};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::modulate::{DropCheck, NEW_ROUTE_DEPTH, check};

/// The editor, its session (active when `engine` is kept, but never processed here: the audio thread is stalled),
/// and every request the session made.
struct App<E> {
    theme: Theme,
    editor: Editor,
    session: Session,
    /// The engine `activate` returned (its crate is not a dependency of this one, so its type stays inferred).
    engine: Option<E>,
    requests: Vec<ShellRequest>,
}

type Rig = Harness<'static, App<Box<dyn std::any::Any>>>;

impl<E> App<E> {
    fn depth(&self, src: &str, dst: &str) -> Option<f32> {
        let (src, dst): (ModSource, ParamAddr) = (src.parse().unwrap(), dst.parse().unwrap());
        self.session
            .patch()
            .routes
            .iter()
            .find(|r| r.src == src && r.dst == dst)
            .map(|r| r.depth)
    }

    fn routes(&self) -> usize {
        self.session.patch().routes.len()
    }
}

fn harness(patch: Patch, active: bool) -> Rig {
    let theme = Theme::dark();
    let mut session = Session::new(patch);
    let engine = active.then(|| -> Box<dyn std::any::Any> {
        Box::new(
            session
                .activate(Activation {
                    sample_rate: 48_000.0,
                    offline: false,
                })
                .unwrap(),
        )
    });
    let mut editor = Editor::new(&theme);
    editor.open(&session, &theme);
    let app = App {
        editor,
        theme: theme.clone(),
        session,
        engine,
        requests: Vec::new(),
    };
    let mut h = Harness::builder()
        .with_size(theme.size.window.default)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .build_ui_state(
            |ui, app: &mut App<Box<dyn std::any::Any>>| {
                let down = ui.input(|i| i.pointer.any_down());
                let rect = ui.ctx().content_rect();
                let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
                app.session.pump();
                app.editor
                    .frame(&mut root, &app.theme, &mut app.session, down);
                app.requests.extend(app.session.drain_requests());
            },
            app,
        );
    h.run_steps(2);
    h
}

fn p1() -> Rig {
    harness(common::patches::p1_default(), false)
}

/// The centre of the control labelled `label` inside the card titled `card`.
fn center_in(h: &Rig, card: &str, label: &str) -> Pos2 {
    let card = h.get_by_label(&format!("Move {card}")).rect();
    h.get_all_by_label(label)
        .map(|n| n.rect())
        .find(|r| (card.left()..=card.right()).contains(&r.center().x) && r.top() >= card.top())
        .map_or_else(|| panic!("no {label:?} in {card:?}"), |r| r.center())
}

/// A source tile's ⊕ handle.
fn handle(h: &Rig, source: &str) -> Pos2 {
    h.get_by_role_and_label(Role::Button, source)
        .rect()
        .center()
}

fn release(h: &Rig, at: Pos2) {
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
}

/// Presses at `from` and moves to `to` over `frames` frames, the button still held; returns the status line then.
fn drag_over(h: &mut Rig, from: Pos2, to: Pos2, frames: usize) -> String {
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for i in 1..=frames {
        h.hover_at(from + (to - from) * i as f32 / frames as f32);
        h.step();
    }
    h.step();
    status(h)
}

/// Drags from `from` and releases at `to` with the pointer still there (a drop).
fn drag_drop(h: &mut Rig, from: Pos2, to: Pos2) -> String {
    let during = drag_over(h, from, to, 8);
    release(h, to);
    h.step();
    h.step();
    during
}

fn status(h: &Rig) -> String {
    h.query_by_label_contains("Status: ")
        .and_then(|n| n.accesskit_node().label())
        .unwrap_or_default()
}

/// Drags the control at `from` by `delta` over `frames` frames and lets go (the button released where it ends).
fn drag(h: &mut Rig, from: Pos2, delta: Vec2, frames: usize) {
    drag_over(h, from, from + delta, frames);
    release(h, from + delta);
    h.step();
    h.step();
}

#[test]
fn dropping_a_source_on_a_knob_routes_it_as_one_undo_step() {
    let mut h = p1();
    let routes = h.state().routes();
    assert_eq!(h.state().depth("env2", "osc1.level"), None);
    let from = handle(&h, "Env 2");
    let to = center_in(&h, "Osc 1", "Level");
    let during = drag_drop(&mut h, from, to);
    assert!(
        during.contains("Drop to modulate it from Env 2 by +25 %"),
        "the target says what a drop does: {during}"
    );
    let app = h.state();
    assert_eq!(app.depth("env2", "osc1.level"), Some(NEW_ROUTE_DEPTH));
    assert_eq!(app.routes(), routes + 1);
    let step = app.session.history().next_undo().expect("one step");
    assert_eq!(step.forward.len(), 1, "one insert: {step:?}");
    assert_eq!(step.gesture, None, "a drop is one edit, not a gesture");

    // Undo takes the route away; dropping again where it exists changes nothing.
    h.get_by_label("Undo").click();
    h.run_steps(2);
    assert_eq!(h.state().depth("env2", "osc1.level"), None);
    h.get_by_label("Redo").click();
    h.run_steps(2);
    let routed = drag_drop(&mut h, from, to);
    assert!(
        routed.contains("Env 2 already modulates it (+25 %)"),
        "{routed}"
    );
    assert_eq!(h.state().routes(), routes + 1);
}

#[test]
fn a_voice_source_on_a_global_target_is_refused_visibly() {
    let mut h = p1();
    let routes = h.state().routes();
    // LFO 1 runs per voice; the reverb is a global effect.
    let from = handle(&h, "LFO 1");
    let to = center_in(&h, "Reverb 1", "Wet");
    let during = drag_drop(&mut h, from, to);
    assert!(
        during.contains("LFO 1 runs per voice and Reverb 1 is global"),
        "the refusal says why on the status line: {during}"
    );
    assert_eq!(h.state().routes(), routes, "nothing routed");
    assert_eq!(h.state().depth("lfo1", "reverb1.wet"), None);
    assert!(!h.state().session.history().can_undo());

    // A parameter that is not modulatable at all is refused too; the global LFO may drive the reverb.
    let wave = center_in(&h, "Osc 1", "Wave");
    let env2 = handle(&h, "Env 2");
    let refused = drag_drop(&mut h, env2, wave);
    assert!(refused.contains("cannot be modulated"), "{refused}");
    let to = center_in(&h, "Reverb 1", "Dry");
    let lfo2 = handle(&h, "LFO 2");
    drag_drop(&mut h, lfo2, to);
    assert_eq!(
        h.state().depth("lfo2", "reverb1.dry"),
        Some(NEW_ROUTE_DEPTH)
    );
}

#[test]
fn the_drop_check_follows_the_patch_rules() {
    let patch = common::patches::p1_default();
    let spec = |a: &ParamAddr| patch.spec(a).unwrap();
    let at = |s: &str| -> ParamAddr { s.parse().unwrap() };
    let src = |s: &str| -> ModSource { s.parse().unwrap() };
    let cases = [
        ("env2", "osc1.level", "route"),
        ("env2", "filter1.cutoff", "routed"),
        ("lfo1", "reverb1.wet", "refused"),
        ("velocity", "master.level", "refused"),
        ("lfo2", "master.level", "refused"),
        ("lfo2", "voice.pan", "routed"),
        ("velocity", "voice.pan", "route"),
        ("macro3", "comp1.mix", "route"),
        ("env1", "osc1.wave", "refused"),
    ];
    for (s, a, want) in cases {
        let got = match check(&patch, &src(s), &at(a), spec(&at(a))) {
            DropCheck::Route => "route",
            DropCheck::Routed(_) => "routed",
            DropCheck::Refused(_) => "refused",
        };
        assert_eq!(got, want, "{s} -> {a}");
    }
    // Every refused rate case is a route the patch itself rejects.
    let mut bad = patch.clone();
    bad.routes.push(enth_patch::Route {
        src: src("lfo1"),
        dst: at("reverb1.wet"),
        depth: 10.0,
    });
    assert!(enth_patch::validate::validate(&bad).is_err());
}

#[test]
fn the_depth_handle_sets_the_newest_routes_depth_with_depth_events_not_plans() {
    let mut h = harness(common::patches::p1_default(), true);
    let epoch = h.state().session.status().epoch;
    // Cutoff's routes: Env 2 +40, then velocity +10; the handle is the newest route's.
    let at = h.get_by_label("Cutoff depth from Vel").rect().center();
    drag(&mut h, at, vec2(0.0, -40.0), 10);
    let app = h.state();
    let depth = app.depth("velocity", "filter1.cutoff").unwrap();
    assert!(
        (depth - 30.0).abs() < 1e-3,
        "40 px up of a 200 px travel adds 20 %: {depth}"
    );
    assert_eq!(
        app.depth("env2", "filter1.cutoff"),
        Some(40.0),
        "the other route stays"
    );
    let step = app.session.history().next_undo().unwrap();
    assert_eq!(step.forward.len(), 1, "one merged SetDepth: {step:?}");
    assert!(!app.session.status().gesture_open);
    assert_eq!(
        app.session.status().epoch,
        epoch,
        "the drag sent depth events, never a plan"
    );
    assert!(app.engine.is_some());
}

#[test]
fn a_route_is_removed_from_the_knobs_menu() {
    let mut h = p1();
    let at = center_in(&h, "Filter 1", "Cutoff");
    h.hover_at(at);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.run_steps(2);
    h.get_by_label("Remove Vel modulation (+10 %)").click();
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.depth("velocity", "filter1.cutoff"), None);
    assert_eq!(app.depth("env2", "filter1.cutoff"), Some(40.0));
    let step = app.session.history().next_undo().unwrap();
    assert!(matches!(
        step.forward.as_slice(),
        [enth_patch::Op::RemoveRoute { route }] if *route == RouteAddr {
            src: ModSource::Velocity,
            dst: "filter1.cutoff".parse().unwrap(),
        }
    ));
}

#[test]
fn the_overview_fits_the_p1_default_at_1200_by_800_as_a_route_list() {
    let mut h = p1();
    let report = h.state().editor.report().clone();
    assert!(
        report.overflow.is_empty(),
        "nothing scrolls at the P1 default, the overview included: {:?}",
        report.overflow
    );
    let overview = report.rect(enth_ui::editor::Region::Overview).unwrap();
    println!("overview at 1200 x 800: {overview:?}");
    // All eight routes are cells of the list; a drag on one sets its depth (one gesture).
    for (target, source) in [
        ("Filter 1 Cutoff", "Env 2"),
        ("Filter 1 Cutoff", "Vel"),
        ("Osc 1 Fine", "LFO 1"),
        ("Osc 2 Fine", "LFO 1"),
        ("Voice Pan", "LFO 2"),
        ("Reverb 1 Wet", "LFO 2"),
        ("Osc 2 Width", "Key"),
        ("Filter 1 Reso", "M1"),
    ] {
        let label = format!("{target} from {source}");
        assert!(
            h.query_by_label(&label).is_some(),
            "{label} is in the route list"
        );
    }
    let cell = h.get_by_label("Filter 1 Cutoff from Env 2").rect().center();
    drag(&mut h, cell, vec2(20.0, 0.0), 5);
    let depth = h.state().depth("env2", "filter1.cutoff").unwrap();
    // 20 px right of a 200 px travel is up to 10 % (egui starts a drag after a few pixels of motion).
    assert!(depth > 45.0 && depth <= 50.0, "{depth}");
    assert_eq!(
        h.state()
            .session
            .history()
            .next_undo()
            .unwrap()
            .forward
            .len(),
        1
    );
}

#[test]
fn the_maximum_patch_shows_the_grid_where_it_fits() {
    // A taller overview (a 1200 x 1000 window) holds the P1 default's grid of 161 px: the grid comes back.
    let theme = Theme::dark();
    let mut h = harness(common::patches::p1_default(), false);
    h.set_size(vec2(theme.size.window.default.x, 1000.0));
    h.run_steps(3);
    let overflow = &h.state().editor.report().overflow;
    assert!(overflow.is_empty(), "{overflow:?}");
    assert!(h.query_by_label("Filter 1 Cutoff from Env 2").is_some());
    // The grid also has the empty cells: a source × target pair with no route.
    assert!(h.query_by_label("Osc 1 Fine from Env 2").is_some());
}
