//! The `wt` card and audio routes in the editor (lane SKS, S2a freeze F2m), through a real `Session`: kittest drives
//! egui's input and finds controls by their accessibility labels; every assertion is on the session (its patch, its
//! history, its epoch) or on where the controls are, never on pixels. The snapshots are the new `wt` and audio-route
//! ones only.
//!
//! `wt` is hidden from the Add menu until lane INT-S2a (`ModuleSpec::hidden`), so these patches load it the way a file
//! or a preset does.
//!
//! Update the snapshots after a deliberate change, then look at them:
//! `UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test wt`.

mod common;

use egui::accesskit::Role;
use egui::{Pos2, Rect, UiBuilder, Vec2, vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, SnapshotResults};
use enth_patch::catalogue::{OscParam, Wave, WtParam, WtTable};
use enth_patch::{ModSource, ModuleId, ModuleInstance, ModuleKind, ParamAddr, Patch, Route};
use enth_session::{Activation, Session};
use enth_ui::Theme;
use enth_ui::editor::cards::present;
use enth_ui::editor::cards::{Card, Intents};
use enth_ui::editor::cards::{CardKind, CardView};
use enth_ui::editor::modulate::{DropCheck, check};
use enth_ui::editor::regions::MatrixView;
use enth_ui::editor::{Editor, Region};

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).expect("valid id")
}

fn route(src: &str, dst: &str, depth: f32) -> Route {
    Route {
        src: src.parse().expect("source"),
        dst: dst.parse().expect("destination"),
        depth,
    }
}

/// The smallest FM patch: a sine `osc1` at +12 st with level 0 (an operator, heard only through its route) into `wt1`'s
/// `pm` at 60 %, `pm_index` 3, under `Patch::init()`'s amplitude envelope.
fn fm_patch() -> Patch {
    let mut p = Patch::init();
    let mut osc = ModuleInstance::new(id("osc1"), ModuleKind::Osc);
    osc.values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    osc.values[OscParam::Semi.index()] = 12.0;
    osc.values[OscParam::Level.index()] = 0.0;
    let mut wt = ModuleInstance::new(id("wt1"), ModuleKind::Wt);
    wt.values[WtParam::PmIndex.index()] = 3.0;
    p.generators = vec![osc, wt];
    p.routes.push(route("osc1", "wt1.pm", 60.0));
    p
}

/// The G1 reference patch with its second oscillator swapped for a `wt` that osc 1 phase-modulates, and an LFO on its
/// position: three generators, a filter, two envelopes, two LFOs, a compressor and a reverb, and eight routes.
fn p1_with_wt() -> Patch {
    let mut p = common::patches::g1();
    p.routes
        .retain(|r| r.dst.to_string() != "osc2.fine" && r.dst.to_string() != "osc2.pw");
    let mut wt = ModuleInstance::new(id("wt1"), ModuleKind::Wt);
    wt.values[WtParam::Table.index()] = WtTable::Formant.index() as f32;
    wt.values[WtParam::PmIndex.index()] = 2.0;
    wt.values[WtParam::Level.index()] = 60.0;
    p.generators[1] = wt;
    p.routes.push(route("osc1", "wt1.pm", 40.0));
    p.routes.push(route("lfo1", "wt1.position", 30.0));
    p
}

/// The editor, its session (active when `active`, though no block is processed: the audio thread is stalled).
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    engine: Option<Box<dyn std::any::Any>>,
}

type Rig = Harness<'static, App>;

impl App {
    fn depth(&self, src: &str, dst: &str) -> Option<f32> {
        let (src, dst): (ModSource, ParamAddr) = (src.parse().unwrap(), dst.parse().unwrap());
        self.session
            .patch()
            .routes
            .iter()
            .find(|r| r.src == src && r.dst == dst)
            .map(|r| r.depth)
    }

    fn value(&self, addr: &str) -> f32 {
        let addr: ParamAddr = addr.parse().unwrap();
        self.session.patch().value(&addr).unwrap()
    }
}

fn harness_sized(patch: Patch, size: Vec2, active: bool) -> Rig {
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
    };
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .build_ui_state(
            |ui, app: &mut App| {
                let down = ui.input(|i| i.pointer.any_down());
                let rect = ui.ctx().content_rect();
                let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
                app.session.pump();
                app.editor
                    .frame(&mut root, &app.theme, &mut app.session, down);
                app.session.drain_requests().for_each(drop);
            },
            app,
        );
    h.run_steps(2);
    h
}

fn harness(patch: Patch, active: bool) -> Rig {
    harness_sized(patch, Theme::dark().size.window.default, active)
}

/// The card titled `title`: its header node is named "Move <title>".
fn card_header(h: &Rig, title: &str) -> Rect {
    h.get_by_label(&format!("Move {title}")).rect()
}

/// The nodes labelled `label` that sit in the card titled `title` (below its header, between its edges).
fn in_card(h: &Rig, title: &str, label: &str) -> Vec<Rect> {
    let header = card_header(h, title);
    h.query_all_by_label(label)
        .map(|n| n.rect())
        .filter(|r| {
            (header.left()..=header.right()).contains(&r.center().x) && r.top() >= header.top()
        })
        .collect()
}

fn center_in(h: &Rig, title: &str, label: &str) -> Pos2 {
    in_card(h, title, label)
        .first()
        .map_or_else(|| panic!("no {label:?} in card {title:?}"), Rect::center)
}

/// The "More" button of the card titled `title` (the one on its header row).
fn more_of(h: &Rig, title: &str) -> Pos2 {
    let header = card_header(h, title);
    h.query_all_by_label("More")
        .map(|n| n.rect())
        .find(|r| r.center().y >= header.top() && r.center().y <= header.bottom())
        .map_or_else(|| panic!("no More on {title:?}"), |r| r.center())
}

fn open_more(h: &mut Rig, title: &str) {
    let at = more_of(h, title);
    h.hover_at(at);
    h.step();
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    release(h, at);
    h.run_steps(3);
}

fn release(h: &Rig, at: Pos2) {
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
}

fn right_click(h: &mut Rig, at: Pos2) {
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
}

fn status(h: &Rig) -> String {
    h.query_by_label_contains("Status: ")
        .and_then(|n| n.accesskit_node().label())
        .unwrap_or_default()
}

/// Presses at `from` and moves to `to` over `frames` frames, the button still held.
fn drag_over(h: &mut Rig, from: Pos2, to: Pos2, frames: usize) {
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for i in 1..=frames {
        h.hover_at(from + (to - from) * i as f32 / frames as f32);
        h.step();
    }
    h.step();
}

/// Drags the control at `from` by `delta` and lets go.
fn drag(h: &mut Rig, from: Pos2, delta: Vec2, frames: usize) {
    drag_over(h, from, from + delta, frames);
    release(h, from + delta);
    h.step();
    h.step();
}

/// The labels of `wt`'s parameters as its card shows them.
fn labels() -> Vec<&'static str> {
    ModuleKind::Wt
        .params()
        .iter()
        .map(|spec| present::label(CardKind::Module(ModuleKind::Wt), spec))
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// The card
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_wt_card_shows_its_basic_controls_and_keeps_every_other_behind_more() {
    let mut h = harness(fm_patch(), false);
    let basic: Vec<&str> = ModuleKind::Wt
        .params()
        .iter()
        .filter(|s| s.learn.tier == enth_params::Tier::Basic)
        .map(|s| present::label(CardKind::Module(ModuleKind::Wt), s))
        .collect();
    assert_eq!(basic.len(), 5, "table, position, semitones, fine, level");
    for label in &basic {
        assert!(
            !in_card(&h, "WT 1", label).is_empty(),
            "{label} is on the wt card"
        );
    }
    let hidden: Vec<&str> = labels()
        .into_iter()
        .filter(|l| !basic.contains(l))
        .collect();
    assert_eq!(hidden.len(), 13);
    for label in &hidden {
        assert!(
            in_card(&h, "WT 1", label).is_empty(),
            "{label} waits behind More"
        );
    }
    assert_eq!(
        h.query_all_by_label("More").count(),
        2,
        "osc 1's and wt 1's"
    );

    // More shows all 18 parameters, every one inside the card: nothing is cut off at its right edge.
    open_more(&mut h, "WT 1");
    let card = card_header(&h, "WT 1");
    for label in labels() {
        let found = in_card(&h, "WT 1", label);
        assert!(!found.is_empty(), "{label} shows with More open");
        for r in found {
            assert!(
                r.right() <= card.right() + 0.5,
                "{label} at {r:?} reaches past the card's edge at {}",
                card.right()
            );
        }
    }
    let rows: std::collections::BTreeSet<i32> = labels()
        .into_iter()
        .map(|l| in_card(&h, "WT 1", l)[0].center().y as i32)
        .collect();
    assert_eq!(rows.len(), 3, "the basic row, then the rest in two rows");
    assert!(
        h.query_all_by_label("Less").count() >= 1,
        "More turned into Less"
    );
}

#[test]
fn a_patch_with_a_wt_fits_the_minimum_window_and_more_scrolls_only_the_sources() {
    let min = Theme::dark().size.window.min;
    let mut h = harness_sized(p1_with_wt(), min, false);
    let report = h.state().editor.report().clone();
    assert!(
        report.overflow.is_empty(),
        "nothing scrolls with a wt at {min:?}: {:?}",
        report.overflow
    );
    for region in [Region::Sources, Region::VoiceFx, Region::Overview] {
        assert!(report.rect(region).is_some(), "{region:?} is there");
    }
    // The route list holds all eight routes, the two audio-and-wt ones included.
    for label in ["WT 1 PM input from Osc 1", "WT 1 Position from LFO 1"] {
        assert!(h.query_by_label(label).is_some(), "{label} is in the list");
    }

    // With More open the wt card grows by two rows and the generator region scrolls: nothing else moves or breaks.
    open_more(&mut h, "WT 1");
    let report = h.state().editor.report().clone();
    let names: Vec<&str> = report.overflow.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["sources"], "{:?}", report.overflow);
    assert!(report.overflow[0].1.x < 0.5, "it scrolls vertically only");
}

#[test]
fn the_wt_card_is_not_in_the_add_menu() {
    let mut h = harness(fm_patch(), false);
    h.get_by_label("+ Add").click();
    h.run_steps(3);
    assert!(h.query_by_label("Oscillator").is_some(), "the menu is open");
    for hidden in ["Wavetable", "WT", "Distortion"] {
        assert!(
            h.query_by_label(hidden).is_none(),
            "{hidden} is hidden from the Add menu"
        );
    }
}

#[test]
fn a_wt_parameter_is_one_undo_step_and_a_value_event_not_a_plan() {
    let mut h = harness(fm_patch(), true);
    let epoch = h.state().session.status().epoch;
    let before = h.state().value("wt1.position");
    let at = center_in(&h, "WT 1", "Position");
    drag(&mut h, at, vec2(0.0, -40.0), 10);
    let app = h.state();
    assert!(app.value("wt1.position") > before + 10.0, "the knob moved");
    let step = app.session.history().next_undo().expect("one step");
    assert_eq!(step.forward.len(), 1, "one merged SetParam: {step:?}");
    assert_eq!(app.session.status().epoch, epoch, "a value event, no plan");
    assert!(app.engine.is_some());

    // The advanced control behind More takes a typed value, with the unit of the catalogue's table.
    open_more(&mut h, "WT 1");
    let at = center_in(&h, "WT 1", "PM index");
    right_click(&mut h, at);
    h.get_by_label("Type value…").click();
    h.run_steps(2);
    h.event(egui::Event::Text("5".to_owned()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!((h.state().value("wt1.pm_index") - 5.0).abs() < 1e-4);
}

// ---------------------------------------------------------------------------------------------------------------
// Audio routes
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn an_audio_route_shows_in_the_overview_with_its_source_and_its_target() {
    let h = harness(fm_patch(), false);
    let cell = h.get_by_label("WT 1 PM input from Osc 1");
    assert_eq!(cell.accesskit_node().role(), Role::Slider);
    let held = cell.accesskit_node().numeric_value().unwrap();
    assert!(
        (held - 0.6).abs() < 1e-6,
        "the cell holds the depth, 60 %: {held}"
    );
    // The overview's matrix panel lists it under the tiles of the modulators; a generator has no tile.
    assert!(h.query_by_role_and_label(Role::Button, "Osc 1").is_none());
    let matrix = MatrixView::of(h.state().session.patch());
    assert_eq!(matrix.sources[0].1, "Osc 1");
    assert_eq!(matrix.targets[0].1, "WT 1 PM input");
    assert_eq!(matrix.audio, [true]);
}

#[test]
fn hovering_the_audio_input_and_its_cell_names_the_route_as_audio() {
    let mut h = harness(fm_patch(), false);
    open_more(&mut h, "WT 1");
    let at = center_in(&h, "WT 1", "PM input");
    h.hover_at(at);
    h.run_steps(2);
    let text = status(&h);
    assert!(
        text.contains("Osc 1 audio +60 %"),
        "the knob's route is audio: {text}"
    );
    assert!(!text.contains("modulation"), "{text}");
}

#[test]
fn the_route_lists_audio_row_says_what_its_depth_is_and_removes_from_its_menu() {
    // Eight routes: the overview shows the compact route list, whose rows hover and have a menu.
    let mut h = harness(p1_with_wt(), false);
    let cell = h.get_by_label("WT 1 PM input from Osc 1").rect().center();
    h.hover_at(cell);
    h.run_steps(3);
    let text = status(&h);
    assert!(text.contains("Audio route"), "{text}");
    assert!(text.contains("+40 %"), "{text}");
    assert!(text.contains("Osc 1's output"), "{text}");
    // Its neighbour, a modulation, still says modulation.
    let lfo = h.get_by_label("WT 1 Position from LFO 1").rect().center();
    h.hover_at(lfo);
    h.run_steps(3);
    let text = status(&h);
    assert!(text.starts_with("Status: Modulation"), "{text}");

    let routes = h.state().session.patch().routes.len();
    let cell = h.get_by_label("WT 1 PM input from Osc 1").rect().center();
    right_click(&mut h, cell);
    h.get_by_label("Remove Osc 1 audio route").click();
    h.run_steps(3);
    assert_eq!(h.state().depth("osc1", "wt1.pm"), None);
    assert_eq!(h.state().session.patch().routes.len(), routes - 1);
    let lfo = h.get_by_label("WT 1 Position from LFO 1").rect().center();
    right_click(&mut h, lfo);
    h.get_by_label("Remove LFO 1 modulation").click();
    h.run_steps(3);
    assert_eq!(h.state().depth("lfo1", "wt1.position"), None);
}

#[test]
fn an_audio_input_has_no_depth_handle_and_refuses_a_modulation_source() {
    let mut h = harness(fm_patch(), false);
    open_more(&mut h, "WT 1");
    assert!(
        h.query_by_label_contains("depth from").is_none(),
        "a depth handle on `pm` would sit at a meaningless place on its ring"
    );
    let patch = h.state().session.patch().clone();
    let at = |s: &str| -> ParamAddr { s.parse().unwrap() };
    let src = |s: &str| -> ModSource { s.parse().unwrap() };
    for dst in ["wt1.pm", "wt1.am"] {
        let spec = patch.spec(&at(dst)).unwrap();
        match check(&patch, &src("env1"), &at(dst), spec) {
            DropCheck::Refused(why) => assert!(
                why.contains("takes the audio of a generator"),
                "{dst}: {why}"
            ),
            other => panic!("{dst}: {other:?}"),
        }
    }
    // The modulatable parameters beside them still take a modulation source.
    let spec = patch.spec(&at("wt1.pm_index")).unwrap();
    assert_eq!(
        check(&patch, &src("env1"), &at("wt1.pm_index"), spec),
        DropCheck::Route
    );
}

#[test]
fn an_audio_routes_depth_is_set_in_the_overview_with_depth_events_not_plans() {
    let mut h = harness(fm_patch(), true);
    let epoch = h.state().session.status().epoch;
    let cell = h.get_by_label("WT 1 PM input from Osc 1").rect().center();
    // 20 px right of a 200 px travel is 10 %.
    drag(&mut h, cell, vec2(20.0, 0.0), 5);
    let app = h.state();
    let depth = app.depth("osc1", "wt1.pm").unwrap();
    assert!(depth > 65.0 && depth <= 70.0, "{depth}");
    let step = app.session.history().next_undo().unwrap();
    assert_eq!(step.forward.len(), 1, "one merged SetDepth: {step:?}");
    assert!(!app.session.status().gesture_open);
    assert_eq!(
        app.session.status().epoch,
        epoch,
        "the drag sent depth events, never a plan"
    );
    assert!(app.engine.is_some());

    // Undo puts the depth back.
    h.get_by_label("Undo").click();
    h.run_steps(2);
    assert_eq!(h.state().depth("osc1", "wt1.pm"), Some(60.0));
}

#[test]
fn an_audio_route_is_removed_by_double_click_and_from_the_inputs_menu_and_comes_back_with_undo() {
    let mut h = harness(fm_patch(), false);
    let cell = h.get_by_label("WT 1 PM input from Osc 1").rect().center();
    h.hover_at(cell);
    h.step();
    h.event(egui::Event::PointerButton {
        pos: cell,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    release(&h, cell);
    h.step();
    h.event(egui::Event::PointerButton {
        pos: cell,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    release(&h, cell);
    h.run_steps(3);
    assert_eq!(
        h.state().depth("osc1", "wt1.pm"),
        None,
        "double-click removes it"
    );
    assert!(
        h.state().session.patch().routes.is_empty(),
        "the only route is gone"
    );
    h.get_by_label("Undo").click();
    h.run_steps(2);
    assert_eq!(h.state().depth("osc1", "wt1.pm"), Some(60.0));

    // The same from the input's own right-click menu, which names it an audio route.
    open_more(&mut h, "WT 1");
    let at = center_in(&h, "WT 1", "PM input");
    right_click(&mut h, at);
    h.get_by_label("Remove Osc 1 audio route (+60 %)").click();
    h.run_steps(3);
    assert_eq!(h.state().depth("osc1", "wt1.pm"), None);
    h.get_by_label("Undo").click();
    h.run_steps(2);
    assert_eq!(h.state().depth("osc1", "wt1.pm"), Some(60.0));
}

#[test]
fn a_route_from_a_generator_never_makes_a_source_tile_or_breaks_the_other_regions() {
    // The generator sources of a patch (a wt feeding a wt, an osc feeding both) take a column each in the overview.
    let mut p = fm_patch();
    let mut wt2 = ModuleInstance::new(id("wt2"), ModuleKind::Wt);
    wt2.values[WtParam::Level.index()] = 0.0;
    p.generators.push(wt2);
    p.routes.push(route("wt2", "wt1.am", 25.0));
    p.routes.push(route("osc1", "wt2.pm", -30.0));
    let h = harness(p, false);
    for label in [
        "WT 1 PM input from Osc 1",
        "WT 1 AM input from WT 2",
        "WT 2 PM input from Osc 1",
    ] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    let matrix = MatrixView::of(h.state().session.patch());
    let names: Vec<&str> = matrix.sources.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["Osc 1", "WT 2"], "generators in patch order");
    let overflow = h.state().editor.report().overflow.clone();
    assert!(overflow.is_empty(), "{overflow:?}");
}

// ---------------------------------------------------------------------------------------------------------------
// Snapshots (the new ones only)
// ---------------------------------------------------------------------------------------------------------------

fn editor_snapshot(results: &mut SnapshotResults, name: &str, patch: Patch) {
    let theme = Theme::dark();
    let t = theme.clone();
    let mut h = common::harness(
        &theme,
        theme.size.window.default,
        move |ui, (editor, session): &mut (Editor, Session)| {
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
            editor.frame(&mut root, &t, session, false);
        },
        (Editor::new(&theme), Session::new(patch)),
    );
    h.run();
    h.snapshot(name);
    results.extend_harness(&mut h);
}

fn card_snapshot(results: &mut SnapshotResults, name: &str, card: CardView) {
    let theme = Theme::dark();
    let t = theme.clone();
    let mut h = common::harness(
        &theme,
        vec2(720.0, 600.0),
        move |ui: &mut egui::Ui, card: &mut CardView| {
            egui::Frame::new()
                .fill(t.color.surface.background)
                .inner_margin(t.space.gap)
                .show(ui, |ui| {
                    let mut out = Intents::default();
                    Card::new(&t, card).strip().show(ui, &mut out);
                });
        },
        card,
    );
    h.run();
    h.fit_contents();
    h.snapshot(name);
    results.extend_harness(&mut h);
}

#[test]
fn wt_snapshots() {
    let mut results = SnapshotResults::new();
    editor_snapshot(&mut results, "editor_wt_fm", fm_patch());
    editor_snapshot(&mut results, "editor_wt_p1", p1_with_wt());
    let patch = fm_patch();
    let (_, module) = patch.find(&id("wt1")).unwrap();
    let card = CardView::module(&patch, module);
    card_snapshot(&mut results, "card_wt", card.clone());
    card_snapshot(&mut results, "card_wt_more", card.with_more(true));
    results.unwrap();
}
