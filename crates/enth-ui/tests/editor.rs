//! Interaction tests for the whole editor through a real session (lane ED1): kittest drives egui's input and finds
//! controls by their accessibility labels, with no renderer; every assertion is on the session (its patch, its
//! history, its host writes), never on pixels.
//!
//! Each test runs `Editor::frame` over a `Session` the way the plugin does, minus the lock: frame (draw, then apply
//! the intents through `SessionEdit`), then drain the session's requests.

mod common;

use egui::{Key, Pos2, UiBuilder, Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use enth_patch::{MacroIndex, ParamAddr, Patch, SlotIndex};
use enth_session::{GestureStep, HostParam, HostWrite, Session, ShellRequest};
use enth_ui::Theme;
use enth_ui::editor::cards::ids;
use enth_ui::editor::layout::LayoutSpec;
use enth_ui::editor::{Editor, Region};

/// The editor, its session, and every request the session made.
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    requests: Vec<ShellRequest>,
}

impl App {
    fn value(&self, addr: &str) -> f32 {
        self.session.patch().value(&addr.parse().unwrap()).unwrap()
    }

    /// The host writes made since the last call, in order.
    fn take_writes(&mut self) -> Vec<HostWrite> {
        self.requests
            .drain(..)
            .filter_map(|r| match r {
                ShellRequest::Host(w) => Some(w),
                _ => None,
            })
            .collect()
    }
}

fn harness(patch: Patch) -> Harness<'static, App> {
    let theme = Theme::dark();
    let app = App {
        editor: Editor::new(&theme),
        theme: theme.clone(),
        session: Session::new(patch),
        requests: Vec::new(),
    };
    Harness::builder()
        .with_size(theme.size.window.default)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .build_ui_state(
            |ui, app: &mut App| {
                let down = ui.input(|i| i.pointer.any_down());
                // The plugin's window is the whole viewport; kittest's root Ui has a margin.
                let rect = ui.ctx().content_rect();
                let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
                app.editor
                    .frame(&mut root, &app.theme, &mut app.session, down);
                app.requests.extend(app.session.drain_requests());
            },
            app,
        )
}

fn p1() -> Harness<'static, App> {
    let mut h = harness(common::patches::p1_default());
    h.run_steps(2);
    h
}

/// Presses at `from`, moves by `delta` in `frames` equal steps (one frame each), releases.
fn drag(h: &mut Harness<'_, App>, from: Pos2, delta: Vec2, frames: usize) {
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for i in 1..=frames {
        h.hover_at(from + delta * i as f32 / frames as f32);
        h.step();
    }
    h.drop_at(from + delta);
    h.step();
    h.step();
}

fn center(h: &Harness<'_, App>, label: &str) -> Pos2 {
    h.get_by_label(label).rect().center()
}

/// The centre of the control labelled `label` inside the card titled `card` (several cards have an "Attack").
fn center_in(h: &Harness<'_, App>, card: &str, label: &str) -> Pos2 {
    let card = h.get_by_label(&format!("Move {card}")).rect();
    h.get_all_by_label(label)
        .map(|n| n.rect())
        .find(|r| (card.left()..=card.right()).contains(&r.center().x) && r.top() >= card.top())
        .map_or_else(|| panic!("no {label:?} in {card:?}"), |r| r.center())
}

/// Right-clicks the control at `at` and clicks the menu item `item`.
fn menu(h: &mut Harness<'_, App>, at: Pos2, item: &str) {
    h.hover_at(at);
    h.step();
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
    h.get_by_label(item).click();
    h.run_steps(3);
}

/// Opens "Type value…" on the control at `at`, types `text` over the selected value and presses Enter.
fn type_value(h: &mut Harness<'_, App>, at: Pos2, field: &str, text: &str) {
    menu(h, at, "Type value…");
    h.get_by_label(field).type_text(text);
    h.step();
    h.key_press(Key::Enter);
    h.run_steps(3);
}

fn cutoff() -> ParamAddr {
    "filter1.cutoff".parse().unwrap()
}

#[test]
fn a_knob_drag_reaches_the_session_as_one_undo_step_and_the_header_undoes_it() {
    let mut h = p1();
    let before = h.state().value("filter1.cutoff");
    let at = center(&h, "Cutoff");
    drag(&mut h, at, vec2(0.0, 40.0), 20);
    let app = h.state();
    let dragged = app.value("filter1.cutoff");
    assert!(
        dragged < before,
        "dragging down lowers the cutoff: {before} -> {dragged}"
    );
    assert!(!app.editor.gesture_open() && !app.session.status().gesture_open);
    let step = app.session.history().next_undo().expect("one step");
    assert_eq!(
        step.forward.len(),
        1,
        "the drag merged into one op: {step:?}"
    );

    h.get_by_label("Undo").click();
    h.run_steps(2);
    assert_eq!(h.state().value("filter1.cutoff"), before);
    assert!(!h.state().session.history().can_undo());
    h.get_by_label("Redo").click();
    h.run_steps(2);
    assert_eq!(h.state().value("filter1.cutoff"), dragged);
}

#[test]
fn automate_assigns_a_slot_and_a_drag_is_one_host_gesture() {
    let mut h = p1();
    let at = center(&h, "Cutoff");
    menu(&mut h, at, "Automate");
    let slot = SlotIndex::new(0).unwrap();
    assert_eq!(h.state().session.patch().slots[0], Some(cutoff()));
    assert!(
        h.state().requests.contains(&ShellRequest::RenameParams),
        "the host is asked to rename slot 1"
    );
    h.state_mut().take_writes();

    drag(&mut h, at, vec2(0.0, 30.0), 10);
    let writes = h.state_mut().take_writes();
    let param = HostParam::Slot(slot);
    assert_eq!(
        writes.first().map(|w| (w.param, w.step)),
        Some((param, GestureStep::Begin))
    );
    assert_eq!(
        writes.last().map(|w| (w.param, w.step)),
        Some((param, GestureStep::End))
    );
    let begins = writes
        .iter()
        .filter(|w| w.step == GestureStep::Begin)
        .count();
    let sets = writes
        .iter()
        .filter(|w| matches!(w.step, GestureStep::Set(_)))
        .count();
    assert_eq!(begins, 1, "one host gesture for one drag: {writes:?}");
    assert!(sets >= 2, "the drag's moves reach the host: {writes:?}");

    // The menu now names the slot and can take it back.
    menu(&mut h, at, "Stop automating");
    assert_eq!(h.state().session.patch().slots[0], None);
}

#[test]
fn type_value_takes_units_and_the_typographic_minus() {
    let mut h = p1();
    let at = center(&h, "Cutoff");
    type_value(&mut h, at, "Cutoff value", "800 Hz");
    assert_eq!(h.state().value("filter1.cutoff"), 800.0);
    assert!(
        h.query_by_label("Cutoff value").is_none(),
        "Enter closes the field"
    );
    let step = h.state().session.history().next_undo().unwrap();
    assert_eq!(
        step.gesture, None,
        "a typed value is one edit, not a gesture"
    );

    type_value(&mut h, at, "Cutoff value", "1.2kHz");
    assert_eq!(h.state().value("filter1.cutoff"), 1200.0);
    type_value(&mut h, at, "Cutoff value", "A4");
    assert_eq!(h.state().value("filter1.cutoff"), 440.0);

    // The design system shows negative values with U+2212; what it shows can be typed back.
    let semi = center_in(&h, "Osc 2", "Semi");
    type_value(&mut h, semi, "Semitones value", "\u{2212}7 st");
    assert_eq!(h.state().value("osc2.semi"), -7.0);
}

#[test]
fn a_value_that_does_not_parse_keeps_the_field_with_a_hint_and_escape_cancels() {
    let mut h = p1();
    let at = center(&h, "Cutoff");
    type_value(&mut h, at, "Cutoff value", "loud");
    assert_eq!(h.state().value("filter1.cutoff"), 1500.0, "nothing changed");
    assert!(
        h.query_by_label("Cutoff value").is_some(),
        "the field stays open"
    );
    assert!(
        h.query_by_label_contains("✗ write a number in hz")
            .is_some(),
        "the parser's hint is shown under the field, with the glyph"
    );
    assert!(!h.state().session.history().can_undo());

    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Cutoff value").is_none(), "Escape cancels");
    assert_eq!(h.state().value("filter1.cutoff"), 1500.0);
}

#[test]
fn reset_to_default_from_the_menu_is_one_edit() {
    let mut h = p1();
    let at = center(&h, "Cutoff");
    let spec = &enth_patch::catalogue::SVF_PARAMS[enth_patch::catalogue::SvfParam::Cutoff.index()];
    let default = enth_ui::editor::cards::format::value_text(spec, spec.default);
    menu(&mut h, at, &format!("Reset to default ({default})"));
    assert_eq!(h.state().value("filter1.cutoff"), spec.default);
    assert!(h.state().session.history().can_undo());
}

#[test]
fn a_macro_drag_is_one_gesture_and_one_host_gesture() {
    let mut h = p1();
    h.state_mut().take_writes();
    let at = center(&h, "Bright");
    drag(&mut h, at, vec2(0.0, -30.0), 10);
    let app = h.state_mut();
    let value = app.session.patch().macros[0].value;
    assert!(value > 0.7, "dragging up raises macro 1 from 70 %: {value}");
    let writes = app.take_writes();
    let param = HostParam::Macro(MacroIndex::new(0).unwrap());
    assert_eq!(
        writes.first().map(|w| (w.param, w.step)),
        Some((param, GestureStep::Begin))
    );
    assert_eq!(
        writes.last().map(|w| (w.param, w.step)),
        Some((param, GestureStep::End))
    );
    assert_eq!(app.session.history().next_undo().unwrap().forward.len(), 1);
}

#[test]
fn the_master_level_in_the_header_edits_the_built_in() {
    let mut h = p1();
    let before = h.state().value("master.level");
    let at = center(&h, "Master");
    drag(&mut h, at, vec2(0.0, 30.0), 10);
    assert!(h.state().value("master.level") < before);
    let at = center(&h, "Polyphony");
    drag(&mut h, at, vec2(0.0, 40.0), 20);
    assert!(
        h.state().value("voice.polyphony") < 16.0,
        "the voice card is in the effects lane"
    );
}

#[test]
fn a_disabled_module_stays_editable() {
    let mut h = p1();
    // The card's enable switch is a checkbox named after the card.
    h.get_by_label("Osc 2").click();
    h.run_steps(2);
    let osc2 = enth_patch::ModuleId::new("osc2").unwrap();
    let enabled = |h: &Harness<'_, App>| h.state().session.patch().find(&osc2).unwrap().1.enabled;
    assert!(!enabled(&h), "the switch reaches the session");
    let level = center_in(&h, "Osc 2", "Level");
    drag(&mut h, level, vec2(0.0, 20.0), 10);
    assert!(
        h.state().value("osc2.level") < 60.0,
        "a dimmed card still takes edits"
    );
    h.get_by_label("Osc 2").click();
    h.run_steps(2);
    assert!(enabled(&h));
}

#[test]
fn envelope_handles_and_knobs_stay_in_sync() {
    let mut h = p1();
    let theme = Theme::dark();
    let reach = theme.size.handle.reach;
    let area = h.get_by_label("Env 2 attack").rect();
    let peak = egui::pos2(area.left() + reach, area.bottom() - reach);
    let before = h.state().value("env2.attack");
    drag(&mut h, peak, vec2(12.0, 0.0), 6);
    let attack = h.state().value("env2.attack");
    assert!(
        attack > before,
        "the handle moved the attack: {before} -> {attack}"
    );
    // The knob reads the same value.
    let knob = center_in(&h, "Env 2", "Attack");
    let text = h
        .get_all_by_label("Attack")
        .find(|n| n.rect().contains(knob))
        .and_then(|n| n.value())
        .unwrap();
    let spec = &enth_patch::catalogue::ADSR_PARAMS[0];
    assert_eq!(
        text,
        enth_ui::editor::cards::format::value_text(spec, attack)
    );

    // And a knob drag moves the handle.
    let handle_before = h.get_by_label("Env 2 decay and sustain").rect();
    let decay = center_in(&h, "Env 2", "Decay");
    drag(&mut h, decay, vec2(0.0, -30.0), 10);
    let handle_after = h.get_by_label("Env 2 decay and sustain").rect();
    assert!(
        handle_after.center().x > handle_before.center().x,
        "a longer decay moves its corner right"
    );
}

#[test]
fn the_status_line_reads_the_hovered_control_and_its_modulation() {
    let mut h = p1();
    h.hover_at(center(&h, "Cutoff"));
    h.run_steps(2);
    let status = h
        .query_by_label_contains("Status: ")
        .and_then(|n| n.accesskit_node().label())
        .unwrap();
    assert_eq!(
        status, "Status: Filter 1 › Cutoff 1.50 kHz Env 2 +40 %, Vel +10 %",
        "path, value and the routes into it"
    );
}

/// A second arrangement of the same regions: effects on the left, modulators along the top, the sources and the
/// filters below them, generators as stacked cards in a band.
const MOVED: &str = r#"{
  "schema": "enth.layout/1",
  "name": "moved",
  "root": {"rows": [
    {"size": {"token": "size.bar.header"}, "region": "header"},
    {"size": {"share": 1}, "cols": [
      {"size": {"token": "size.card.lane_width"}, "region": "effects"},
      {"size": {"share": 1}, "rows": [
        {"size": {"token": "size.editor.band_height"}, "region": "modulators"},
        {"size": {"share": 1}, "region": "sources", "arrange": "band"},
        {"size": {"share": 1}, "region": "voice_fx", "arrange": "band"}
      ]}
    ]},
    {"size": {"token": "size.bar.macros"}, "region": "macros"},
    {"size": {"token": "size.bar.status"}, "region": "status"}
  ]},
  "place": [
    {"part": "generators", "region": "sources"},
    {"part": "voice_fx", "region": "voice_fx"},
    {"part": "voice", "region": "voice_fx"},
    {"part": "modulators", "region": "modulators"},
    {"part": "fx", "region": "effects"},
    {"part": "master", "region": "effects"}
  ]
}"#;

/// Every parameter address of the patch's cards.
fn addresses(patch: &Patch) -> Vec<ParamAddr> {
    enth_ui::editor::cards::view::all(patch)
        .into_iter()
        .flat_map(|c| c.shown().map(|p| p.addr.clone()).collect::<Vec<_>>())
        .collect()
}

#[test]
fn control_ids_stay_the_same_across_two_layouts() {
    let mut h = p1();
    let patch = h.state().session.patch().clone();
    let controls = addresses(&patch);
    assert!(
        controls.len() >= 59,
        "the G1 patch has 59 basic and intermediate controls"
    );
    let ctx = h.ctx.clone();
    let rects = |ctx: &egui::Context| -> Vec<Option<egui::Rect>> {
        controls
            .iter()
            .map(|a| ctx.read_response(ids::param(a)).map(|r| r.rect))
            .collect()
    };
    let lanes = rects(&ctx);
    assert!(
        lanes.iter().all(Option::is_some),
        "every control is drawn in lanes"
    );

    let theme = Theme::dark();
    let moved = LayoutSpec::from_json(MOVED).unwrap();
    h.state_mut().editor.set_layout(moved, &theme).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().editor.layout().name(), "moved");
    let moved = rects(&ctx);
    let kept = moved.iter().filter(|r| r.is_some()).count();
    assert_eq!(
        kept,
        controls.len(),
        "every control keeps its id in the other layout"
    );
    let relocated = lanes
        .iter()
        .zip(&moved)
        .filter(|(a, b)| a.map(|r| r.min) != b.map(|r| r.min))
        .count();
    assert!(
        relocated * 2 > controls.len(),
        "most controls moved ({relocated} of {}), under the same ids",
        controls.len()
    );
    // The accessibility tree uses the same node ids too.
    let node = h.get_by_label("Cutoff");
    assert_eq!(
        node.accesskit_node().locate().0,
        ids::param(&cutoff()).accesskit_id()
    );
}

#[test]
fn a_drag_survives_a_layout_switch_in_the_middle() {
    let mut h = p1();
    let theme = Theme::dark();
    let before = h.state().value("filter1.cutoff");
    let from = center(&h, "Cutoff");
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for i in 1..=5 {
        h.hover_at(from + vec2(0.0, 4.0 * i as f32));
        h.step();
    }
    let half = h.state().value("filter1.cutoff");
    assert!(half < before);
    // Switch layouts mid-drag: the knob is somewhere else now, under the same id, and the pointer is still down.
    let moved = LayoutSpec::from_json(MOVED).unwrap();
    h.state_mut().editor.set_layout(moved, &theme).unwrap();
    for i in 6..=10 {
        h.hover_at(from + vec2(0.0, 4.0 * i as f32));
        h.step();
    }
    h.drop_at(from + vec2(0.0, 40.0));
    h.run_steps(2);
    let app = h.state();
    let after = app.value("filter1.cutoff");
    assert!(
        after < half,
        "the drag went on after the switch: {before} -> {half} -> {after}"
    );
    assert!(!app.session.status().gesture_open, "and its gesture closed");
    assert_eq!(app.session.history().next_undo().unwrap().forward.len(), 1);
}

#[test]
fn nothing_scrolls_at_1200_by_800_for_the_p1_default() {
    let h = p1();
    let report = h.state().editor.report().clone();
    let modules: Vec<_> = report
        .overflow
        .iter()
        .filter(|(part, _)| !part.starts_with("overview"))
        .collect();
    assert!(modules.is_empty(), "a module region scrolls: {modules:?}");
    let overview: Vec<_> = report
        .overflow
        .iter()
        .filter(|(part, _)| part.starts_with("overview"))
        .collect();
    println!("overview overflow at the P1 default: {overview:?}");
    for region in [
        Region::Header,
        Region::Macros,
        Region::Status,
        Region::Overview,
    ] {
        assert!(report.rect(region).is_some(), "{region:?} has room");
    }
}

#[test]
fn the_maximum_patch_scrolls_inside_its_regions() {
    let mut h = harness(common::patches::maximum());
    h.run_steps(2);
    let report = h.state().editor.report().clone();
    let scrolled: Vec<&str> = report.overflow.iter().map(|(p, _)| *p).collect();
    println!("overflow at the maximum patch: {:?}", report.overflow);
    for part in ["sources", "voice_fx", "modulators", "effects"] {
        assert!(
            scrolled.contains(&part),
            "{part} holds more than fits: {scrolled:?}"
        );
    }
}

#[test]
fn a_refused_layout_leaves_the_current_one() {
    let theme = Theme::dark();
    let mut editor = Editor::new(&theme);
    let mut spec = LayoutSpec::lanes();
    spec.place.pop();
    let err = editor.set_layout(spec, &theme).unwrap_err();
    assert!(err.0.contains("not placed"), "{err}");
    assert_eq!(editor.layout().name(), "lanes");
}
