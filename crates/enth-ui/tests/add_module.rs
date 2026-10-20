//! Adding and removing modules through the editor (lane AM, R1/S1), on a real `Session`: kittest drives egui's input
//! and finds controls and menu items by their accessibility labels, with no renderer; every assertion is on the
//! session (its patch, its history, its plans), never on pixels.
//!
//! Each test runs `Editor::frame` over a `Session` the way the plugin does, minus the lock: frame (draw, then apply
//! the intents through `SessionEdit`), then drain the session's requests.

mod common;

use egui::{Pos2, UiBuilder, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use enth_patch::canon;
use enth_patch::{ModSource, ModuleId, ModuleKind, Op, ParamAddr, Patch, Section, SlotIndex};
use enth_session::{Activation, Session};
use enth_ui::Theme;
use enth_ui::editor::{Editor, Region};

/// The editor and its session, with an optional engine the test renders by hand (no audio thread, no clock).
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    render: Option<Box<dyn FnMut(usize)>>,
}

fn harness_sized(patch: Patch, size: egui::Vec2, active: bool) -> Harness<'static, App> {
    let theme = Theme::dark();
    let mut session = Session::new(patch);
    let render: Option<Box<dyn FnMut(usize)>> = if active {
        let mut engine = session
            .activate(Activation {
                sample_rate: 48_000.0,
                offline: false,
            })
            .expect("activates");
        Some(Box::new(move |blocks: usize| {
            let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
            for _ in 0..blocks {
                engine.process(&mut l, &mut r, &[]);
            }
        }))
    } else {
        None
    };
    let app = App {
        editor: Editor::new(&theme),
        theme: theme.clone(),
        session,
        render,
    };
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .build_ui_state(
            |ui, app: &mut App| {
                let down = ui.input(|i| i.pointer.any_down());
                app.session.pump();
                // The plugin's window is the whole viewport; kittest's root Ui has a margin.
                let rect = ui.ctx().content_rect();
                let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
                app.editor
                    .frame(&mut root, &app.theme, &mut app.session, down);
                app.session.drain_requests().for_each(drop);
            },
            app,
        );
    h.run_steps(2);
    h
}

fn harness(patch: Patch) -> Harness<'static, App> {
    harness_sized(patch, Theme::dark().size.window.default, false)
}

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).unwrap()
}

/// Clicks the control labelled `label` and lets the editor apply it.
fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

/// Opens the header's Add menu and clicks `item` in it.
fn add_from_header(h: &mut Harness<'_, App>, item: &str) {
    click(h, "+ Add");
    click(h, item);
}

/// Opens the card `title`'s "…" menu and clicks `item` in it.
fn card_menu(h: &mut Harness<'_, App>, title: &str, item: &str) {
    click(h, &format!("{title} menu"));
    click(h, item);
}

/// A secondary click at `at`.
fn right_click(h: &mut Harness<'_, App>, at: Pos2) {
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

/// The title a fresh module of `kind` gets as the first of its kind in `Patch::init()` (one osc, one envelope).
fn first_title(kind: ModuleKind) -> &'static str {
    match kind {
        ModuleKind::Osc => "Osc 2",
        ModuleKind::Noise => "Noise 1",
        ModuleKind::Svf => "Filter 1",
        ModuleKind::Adsr => "Env 2",
        ModuleKind::Lfo => "LFO 1",
        ModuleKind::Reverb => "Reverb 1",
        ModuleKind::Comp => "Comp 1",
    }
}

#[test]
fn every_kind_is_added_and_removed_with_one_undo_step_each() {
    for &kind in ModuleKind::ALL {
        let mut h = harness(Patch::init());
        let start = h.state().session.patch().clone();
        let new = start.free_id(kind);
        add_from_header(&mut h, kind.spec().name);
        let after_add = h.state().session.patch().clone();
        let (at, module) = after_add
            .find(&new)
            .unwrap_or_else(|| panic!("{kind:?}: {new} was added"));
        assert_eq!(module.kind, kind);
        assert_eq!(at.section, kind.section());
        assert_eq!(at.index, start.section(kind.section()).len(), "appended");
        assert_eq!(module.values, kind.defaults(), "at its defaults");
        let step = h.state().session.history().next_undo().unwrap();
        assert!(
            matches!(step.forward.as_slice(), [Op::AddModule { .. }]),
            "{kind:?}: one op, one step: {step:?}"
        );
        // Undo and redo through the header's buttons.
        click(&mut h, "Undo");
        assert!(
            canon::identical(h.state().session.patch(), &start),
            "{kind:?}: undone"
        );
        click(&mut h, "Redo");
        assert!(
            canon::identical(h.state().session.patch(), &after_add),
            "{kind:?}: redone"
        );
        // Remove it through its card's menu, then undo the removal.
        card_menu(&mut h, first_title(kind), "Remove");
        assert!(
            h.state().session.patch().find(&new).is_none(),
            "{kind:?}: removed"
        );
        let step = h.state().session.history().next_undo().unwrap();
        assert!(
            matches!(step.forward.as_slice(), [Op::RemoveModule { .. }]),
            "{kind:?}: one op, one step: {step:?}"
        );
        click(&mut h, "Undo");
        assert!(
            canon::identical(h.state().session.patch(), &after_add),
            "{kind:?}: the removal undone"
        );
    }
}

#[test]
fn removing_a_module_takes_its_routes_and_slots_and_undo_puts_them_back() {
    let mut h = harness(common::patches::p1_default());
    let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
    h.state_mut()
        .session
        .edit(
            vec![Op::SetSlot {
                slot: SlotIndex::from_one_based(1).unwrap(),
                addr: Some(cutoff),
            }],
            None,
        )
        .unwrap();
    h.run_steps(2);
    let before = h.state().session.patch().clone();
    let filter = id("filter1");
    let touching = |p: &Patch| {
        p.routes
            .iter()
            .filter(|r| {
                r.dst.module_id() == Some(&filter) || r.src == ModSource::Module(filter.clone())
            })
            .count()
    };
    let routes = touching(&before);
    assert!(routes >= 2, "the G1 patch modulates the filter: {routes}");
    card_menu(&mut h, "Filter 1", "Remove");
    let after = h.state().session.patch().clone();
    assert!(after.find(&filter).is_none());
    assert_eq!(touching(&after), 0, "its routes went with it");
    assert_eq!(after.routes.len(), before.routes.len() - routes);
    assert!(after.slots[0].is_none(), "and its automation slot");
    let status = h
        .get_by_label_contains("Status:")
        .accesskit_node()
        .label()
        .unwrap_or_default();
    assert!(
        status.contains(&format!(
            "Removed Filter 1, {routes} routes and automation slot 1"
        )),
        "the status line says what went: {status}"
    );
    click(&mut h, "Undo");
    let undone = h.state().session.patch();
    assert!(canon::identical(undone, &before), "undo restores it all");
    assert_eq!(
        undone.routes, before.routes,
        "the routes at their old places"
    );
    assert_eq!(undone.slots, before.slots);
}

#[test]
fn a_right_click_on_empty_region_space_opens_its_add_menu() {
    let mut h = harness(Patch::init());
    let effects = h.state().editor.report().rect(Region::Effects).unwrap();
    // Empty space: under the voice card and the "+ Add" slot.
    right_click(
        &mut h,
        Pos2::new(effects.center().x, effects.bottom() - 4.0),
    );
    assert!(
        h.query_by_label("Oscillator").is_none(),
        "only the region's sections"
    );
    assert!(h.query_by_label("Reverb").is_some());
    click(&mut h, "Compressor");
    let comp = h
        .state()
        .session
        .patch()
        .find(&id("comp1"))
        .map(|(at, _)| at);
    assert_eq!(comp.map(|at| at.section), Some(Section::Fx));
    // A right-click on a card's body is not empty space: no Add menu.
    let sources = h.state().editor.report().rect(Region::Sources).unwrap();
    right_click(
        &mut h,
        sources.center() + vec2(0.0, -sources.height() / 2.0 + 60.0),
    );
    assert!(h.query_by_label("Noise").is_none(), "no menu over a card");
}

#[test]
fn the_add_slot_shows_only_where_there_is_room_and_adds_there() {
    let mut h = harness(common::patches::p1_default());
    assert!(
        h.state().editor.report().overflow.is_empty(),
        "the slots never make a region scroll: {:?}",
        h.state().editor.report().overflow
    );
    // At 1200 × 800 the filter lane and the modulator band have room; the three strips and the effects lane don't.
    assert!(h.query_by_label("Add voice effect").is_some());
    assert!(h.query_by_label("Add modulator").is_some());
    assert!(h.query_by_label("Add source").is_none());
    assert!(h.query_by_label("Add effect").is_none());
    click(&mut h, "Add voice effect");
    click(&mut h, "Filter");
    let patch = h.state().session.patch();
    assert_eq!(patch.voice_fx.len(), 2);
    assert_eq!(patch.voice_fx[1].id, id("filter2"));
    // The maximum patch has no room anywhere: no slot.
    let h = harness(common::patches::maximum());
    for slot in [
        "Add voice effect",
        "Add modulator",
        "Add source",
        "Add effect",
    ] {
        assert!(
            h.query_by_label(slot).is_none(),
            "{slot} at the maximum patch"
        );
    }
}

#[test]
fn the_card_menu_duplicates_moves_renames_and_names_the_amp_envelope() {
    let mut h = harness(common::patches::p1_default());
    // Duplicate: a copy right after the original, with a new id and no label; its routes stay with the original.
    let osc2 = h
        .state()
        .session
        .patch()
        .find(&id("osc2"))
        .unwrap()
        .1
        .clone();
    card_menu(&mut h, "Osc 2", "Duplicate");
    let patch = h.state().session.patch().clone();
    let copy = &patch.generators[2];
    assert_eq!(copy.id, id("osc3"));
    assert_eq!(
        (copy.kind, &copy.values, copy.enabled),
        (osc2.kind, &osc2.values, osc2.enabled)
    );
    assert_eq!(copy.label, None);
    assert!(
        patch
            .routes
            .iter()
            .all(|r| r.dst.module_id() != Some(&copy.id))
    );
    // Move it up, past osc2: one Move op.
    card_menu(&mut h, "Osc 3", "Move up");
    let order: Vec<&str> = h
        .state()
        .session
        .patch()
        .generators
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(order, ["osc1", "osc3", "osc2", "noise1"]);
    let step = h.state().session.history().next_undo().unwrap();
    assert!(
        matches!(step.forward.as_slice(), [Op::Move { .. }]),
        "{step:?}"
    );
    // Rename through the field over the title row; an empty name gives the kind and number back.
    card_menu(&mut h, "Osc 1", "Rename…");
    h.get_by_label("Osc 1 name").type_text("Body");
    h.step();
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let label = |h: &Harness<'_, App>| {
        h.state()
            .session
            .patch()
            .find(&id("osc1"))
            .unwrap()
            .1
            .label
            .clone()
    };
    assert_eq!(label(&h).as_deref(), Some("Body"));
    assert!(
        h.query_by_label("Body menu").is_some(),
        "the card shows its label"
    );
    card_menu(&mut h, "Body", "Rename…");
    h.get_by_label("Body name").type_text("");
    h.step();
    h.key_press(egui::Key::Backspace);
    h.step();
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(label(&h), None);
    // Env 1 is the amp envelope: it cannot be removed until another envelope takes its place.
    card_menu(&mut h, "Env 1", "Remove");
    assert!(
        h.state().session.patch().find(&id("env1")).is_some(),
        "refused while it is the amp env"
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    card_menu(&mut h, "Env 2", "Make amp envelope");
    assert_eq!(h.state().session.patch().voice.amp_env, id("env2"));
    card_menu(&mut h, "Env 1", "Remove");
    assert!(h.state().session.patch().find(&id("env1")).is_none());
}

#[test]
fn a_card_menu_adds_after_its_card() {
    let mut h = harness(common::patches::p1_default());
    click(&mut h, "LFO 1 menu");
    click(&mut h, "Add after");
    click(&mut h, "Envelope");
    let order: Vec<&str> = h
        .state()
        .session
        .patch()
        .modulators
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(order, ["env1", "env2", "lfo1", "env3", "lfo2"]);
}

#[test]
fn a_full_section_offers_nothing_to_add() {
    let mut h = harness(common::patches::maximum());
    let before = h.state().session.patch().clone();
    click(&mut h, "+ Add");
    for kind in [
        "Oscillator",
        "Noise",
        "Filter",
        "Reverb",
        "Compressor",
        "Envelope",
        "LFO",
    ] {
        assert!(
            h.get_by_label(kind).accesskit_node().is_disabled(),
            "{kind} offered in a full section"
        );
    }
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    click(&mut h, "Osc 1 menu");
    assert!(h.get_by_label("Duplicate").accesskit_node().is_disabled());
    assert!(h.get_by_label("Add after").accesskit_node().is_disabled());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(canon::identical(h.state().session.patch(), &before));
}

#[test]
fn an_add_on_an_active_session_is_one_plan_through_the_structural_path() {
    let mut h = harness_sized(Patch::init(), Theme::dark().size.window.default, true);
    let epoch = h.state().session.status().epoch;
    add_from_header(&mut h, "Filter");
    let status = h.state().session.status();
    assert_eq!(status.epoch, epoch + 1, "one recompiled plan, sent");
    assert!(!status.compile_pending);
    // The engine takes it and renders.
    if let Some(render) = h.state_mut().render.as_mut() {
        render(8);
    }
    h.run_steps(2);
    click(&mut h, "Undo");
    assert_eq!(
        h.state().session.status().epoch,
        epoch + 2,
        "undo is one plan too"
    );
    assert!(h.state().session.patch().voice_fx.is_empty());
}

#[test]
fn the_minimum_window_keeps_every_region_of_layout_1() {
    // The minimum is the size layout 1 is measured at: below 1200 × 800 the P1 default loses part of a region (the
    // band scrolls sideways under 1148 px, the route list under 1200 px, the tiles under about 792 px; the overview,
    // and with it every drag source, is gone under about 702 px).
    let theme = Theme::dark();
    assert_eq!(theme.size.window.min, theme.size.window.default);
    let h = harness_sized(common::patches::p1_default(), theme.size.window.min, false);
    let report = h.state().editor.report();
    assert!(report.overflow.is_empty(), "{:?}", report.overflow);
    let overview = report
        .rect(Region::Overview)
        .expect("the overview has room");
    for source in ["Env 1", "Env 2", "LFO 1", "LFO 2", "Vel", "Key", "Wheel"] {
        let handle = h.get_by_role_and_label(egui::accesskit::Role::Button, source);
        assert!(
            overview.contains_rect(handle.rect()),
            "the {source} tile's handle is there to drag from: {:?} in {overview:?}",
            handle.rect()
        );
    }
}
