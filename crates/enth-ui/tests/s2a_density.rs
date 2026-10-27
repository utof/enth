//! S2a patches in the editor (lane EDS, S2a wave 5): the dense layout that makes the growl reference, a reese and a
//! wobble fit 1200 x 800 with nothing scrolling, and the audio-route drag (a generator's ⊕ onto a `wt` PM or AM
//! input). kittest drives egui's input and finds controls by their accessibility labels; every assertion is on the
//! session (its patch, its history), on the overflow report, or on where the controls are, never on pixels. The two
//! snapshots are the growl reference in the dense layout and an audio-route drag in progress.
//!
//! The S2a kinds are hidden from the Add menu until lane INT-S2a (`ModuleSpec::hidden`), so these tests load their
//! patches from files, the way the plugin does: the growl reference from the test host's golden, the reese and the
//! wobble from report 04's sketches.
//!
//! Update the snapshots after a deliberate change, then look at them:
//! `UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test s2a_density`.

mod common;

use egui::accesskit::Role;
use egui::{Pos2, Rect, UiBuilder, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, SnapshotResults};
use enth_patch::catalogue::{OscParam, Wave};
use enth_patch::{ModSource, ModuleId, ModuleInstance, ModuleKind, ParamAddr, Patch, Route, file};
use enth_session::Session;
use enth_ui::Theme;
use enth_ui::editor::layout::{Density, LayoutSpec};
use enth_ui::editor::modulate::{AUDIO_ROUTE_DEPTH, DropCheck, check};
use enth_ui::editor::{Editor, Region};

/// The patch file `rel` (from the repository root), compiled in: the UI crate reads no files.
macro_rules! patch_file {
    ($rel:literal) => {
        file::load(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../",
            $rel
        )))
        .unwrap_or_else(|e| panic!("{}: {e}", $rel))
        .patch
    };
}

fn g2() -> Patch {
    patch_file!("tools/test-host/golden/enth-plugin.g2.patch.json")
}

fn reese() -> Patch {
    patch_file!("content/templates/s2a/sketches/s03-reese.patch.json")
}

fn wobble() -> Patch {
    patch_file!("content/templates/s2a/sketches/s02-wobble-eighth.patch.json")
}

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).expect("valid id")
}

/// An oscillator `osc1` and a wavetable `wt1`, no route between them: the audio drag makes it.
fn osc_and_wt() -> Patch {
    let mut p = Patch::init();
    let mut osc = ModuleInstance::new(id("osc1"), ModuleKind::Osc);
    osc.values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    osc.values[OscParam::Level.index()] = 0.0;
    let wt = ModuleInstance::new(id("wt1"), ModuleKind::Wt);
    p.generators = vec![osc, wt];
    p
}

struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
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
}

/// The editor over `patch` at `size`; `dense`: the shell's policy (a patch of S2a's kinds is drawn densely).
fn harness_sized(patch: Patch, size: Vec2, dense: bool) -> Rig {
    let mut editor = Editor::new(&Theme::dark());
    editor.set_auto_dense(dense);
    harness_with(editor, patch, size)
}

fn harness_with(mut editor: Editor, patch: Patch, size: Vec2) -> Rig {
    let theme = Theme::dark();
    let session = Session::new(patch);
    editor.open(&session, &theme);
    let app = App {
        editor,
        theme: theme.clone(),
        session,
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
    h.run_steps(3);
    h
}

fn min_window() -> Vec2 {
    Theme::dark().size.window.min
}

fn card_header(h: &Rig, title: &str) -> Rect {
    h.get_by_label(&format!("Move {title}")).rect()
}

fn center_in(h: &Rig, title: &str, label: &str) -> Pos2 {
    let header = card_header(h, title);
    h.query_all_by_label(label)
        .map(|n| n.rect())
        .find(|r| {
            (header.left()..=header.right()).contains(&r.center().x) && r.top() >= header.top()
        })
        .map_or_else(|| panic!("no {label:?} in card {title:?}"), |r| r.center())
}

fn release(h: &Rig, at: Pos2) {
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
}

fn status(h: &Rig) -> String {
    h.query_by_label_contains("Status: ")
        .and_then(|n| n.accesskit_node().label())
        .unwrap_or_default()
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

/// Drags from `from` and releases at `to` (a drop); returns the status line while the pointer was over `to`.
fn drag_drop(h: &mut Rig, from: Pos2, to: Pos2) -> String {
    let during = drag_over(h, from, to, 8);
    release(h, to);
    h.step();
    h.step();
    during
}

/// A generator's audio handle: "<title> audio".
fn audio_handle(h: &Rig, title: &str) -> Pos2 {
    h.get_by_role_and_label(Role::Button, &format!("{title} audio"))
        .rect()
        .center()
}

// ---------------------------------------------------------------------------------------------------------------
// The dense layout
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_growl_reference_a_reese_and_a_wobble_fit_the_minimum_window_with_nothing_scrolling() {
    for (name, patch) in [("g2", g2()), ("reese", reese()), ("wobble", wobble())] {
        let h = harness_sized(patch, min_window(), true);
        let report = h.state().editor.report();
        assert_eq!(report.density, Density::Dense, "{name} is drawn densely");
        assert!(
            report.overflow.is_empty(),
            "{name}: nothing scrolls at {:?}: {:?}",
            min_window(),
            report.overflow
        );
        for region in [
            Region::Sources,
            Region::VoiceFx,
            Region::Effects,
            Region::Modulators,
            Region::Overview,
        ] {
            assert!(report.rect(region).is_some(), "{name}: {region:?} is there");
        }
    }
}

#[test]
fn layout_1_does_not_fit_the_growl_reference_which_is_why_the_dense_layout_exists() {
    // The measure the dense layout answers: with the policy off, layout 1 draws G2 and scrolls three regions.
    let h = harness_sized(g2(), min_window(), false);
    let report = h.state().editor.report();
    assert_eq!(report.density, Density::Normal);
    let names: Vec<&str> = report.overflow.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names,
        ["voice_fx", "modulators", "effects"],
        "{:?}",
        report.overflow
    );
}

#[test]
fn a_p1_patch_is_drawn_in_layout_1_whatever_the_policy() {
    for patch in [common::patches::p1_default(), common::patches::maximum()] {
        let off = harness_sized(patch.clone(), min_window(), false);
        let on = harness_sized(patch, min_window(), true);
        assert_eq!(on.state().editor.report().density, Density::Normal);
        assert_eq!(
            on.state().editor.report().regions,
            off.state().editor.report().regions,
            "the same regions, to the pixel"
        );
        assert_eq!(
            on.state().editor.report().overflow,
            off.state().editor.report().overflow
        );
    }
}

#[test]
fn a_layout_asked_for_by_name_is_never_replaced() {
    let theme = Theme::dark();
    // A layout file of the user's own, even with the policy on: the S2a patch is drawn in it.
    let mut spec = LayoutSpec::lanes();
    "mine".clone_into(&mut spec.name);
    let mut editor = Editor::with_layout(spec, &theme).unwrap();
    editor.set_auto_dense(true);
    let h = harness_with(editor, g2(), min_window());
    assert_eq!(h.state().editor.report().density, Density::Normal);
    assert!(
        !h.state().editor.report().overflow.is_empty(),
        "so it overflows, as its author chose"
    );

    // The built-in dense layout by name, the policy off: drawn densely, for a P1 patch as well.
    let editor = Editor::with_layout(LayoutSpec::builtin("dense").unwrap(), &theme).unwrap();
    let h = harness_with(editor, common::patches::p1_default(), min_window());
    assert_eq!(h.state().editor.report().density, Density::Dense);
}

#[test]
fn the_dense_layout_shows_every_basic_and_intermediate_control_of_the_growl_reference() {
    let h = harness_sized(g2(), min_window(), true);
    let patch = h.state().session.patch();
    let mut missing = Vec::new();
    for (_, m) in patch.modules() {
        let title = enth_ui::editor::cards::CardView::module(patch, m).title;
        for spec in m.kind.params() {
            if spec.learn.tier == enth_params::Tier::Advanced && !spec.flags.audio {
                continue;
            }
            let label = enth_ui::editor::cards::present::label_in(
                enth_ui::editor::cards::CardKind::Module(m.kind),
                spec,
                true,
            );
            let header = card_header(&h, &title);
            let found = h.query_all_by_label(label).any(|n| {
                let r = n.rect();
                (header.left()..=header.right()).contains(&r.center().x) && r.top() >= header.top()
            });
            if !found {
                missing.push(format!("{title}: {label}"));
            }
        }
    }
    assert!(missing.is_empty(), "controls not on screen: {missing:?}");
}

/// The width of `text` on one line in `font`.
fn text_width(h: &Rig, text: &str, font: &egui::FontId) -> f32 {
    let job = egui::text::LayoutJob::simple_singleline(
        text.to_owned(),
        font.clone(),
        egui::Color32::WHITE,
    );
    h.ctx.fonts_mut(|f| f.layout_job(job).size().x)
}

/// Every card kind the dense layout can draw, with its parameters: all the module kinds, the voice and the master.
fn dense_cards() -> Vec<(
    enth_ui::editor::cards::CardKind,
    &'static [enth_params::ParamSpec],
)> {
    use enth_patch::catalogue::Builtin;
    use enth_ui::editor::cards::CardKind;
    ModuleKind::ALL
        .iter()
        .map(|k| (CardKind::Module(*k), k.params()))
        .chain([
            (CardKind::Voice, Builtin::Voice.params()),
            (CardKind::Master, Builtin::Master.params()),
        ])
        .collect()
}

#[test]
fn every_label_and_value_of_a_dense_knob_cell_fits_its_cell() {
    use enth_params::Kind;
    use enth_ui::editor::cards::format::value_text;
    use enth_ui::editor::cards::present::{self, Control};
    let h = harness_sized(Patch::init(), min_window(), false);
    let theme = Theme::dark().dense();
    let cell = theme.size.knob.cell_width;
    let mut cut = Vec::new();
    for (ck, params) in dense_cards() {
        let key = present::kind_key(ck);
        for spec in params {
            if spec.learn.tier == enth_params::Tier::Advanced && !spec.flags.audio {
                continue;
            }
            let label = present::label_in(ck, spec, true);
            let control = present::control(&theme, spec);
            if control == Control::Segmented {
                continue;
            }
            let w = text_width(&h, label, &theme.text.label);
            if w > cell {
                cut.push(format!("{key}.{} label {label:?} {w:.1}", spec.key));
            }
            if control == Control::Toggle {
                continue;
            }
            let mut values: Vec<f32> = match spec.kind {
                // What a user sees on load and at the ends of the knob. A value between, on a log range's low end, has
                // three significant digits ("0.108 Hz", "0.143 ms") and can pass the cell by a few px; the status
                // line reads it in full.
                Kind::Float { .. } => vec![spec.min(), spec.default, spec.max()],
                _ => (spec.min() as i32..=spec.max() as i32)
                    .map(|v| v as f32)
                    .collect(),
            };
            values.dedup();
            for v in values {
                let text = match spec.enum_item(v) {
                    Some(item) if spec.enum_items().is_some() => {
                        present::item_label_in(ck, spec, item, true).to_owned()
                    }
                    _ => value_text(spec, v),
                };
                if text_width(&h, &text, &theme.text.value) > cell {
                    let w = text_width(&h, &text, &theme.text.value);
                    cut.push(format!("{key}.{} value {text:?} {w:.1}", spec.key));
                }
            }
        }
    }
    cut.sort();
    cut.dedup();
    assert!(cut.is_empty(), "cut off at {cell} px:\n{}", cut.join("\n"));
}

#[test]
fn every_segmented_row_of_the_growl_reference_holds_its_label_and_all_its_items() {
    use enth_ui::editor::cards::present::{self, Control};
    use enth_ui::editor::cards::{CardKind, CardView};
    let h = harness_sized(g2(), min_window(), true);
    let theme = Theme::dark().dense();
    let patch = h.state().session.patch();
    let mut cut = Vec::new();
    for (_, m) in patch.modules() {
        let title = CardView::module(patch, m).title;
        let ck = CardKind::Module(m.kind);
        for spec in m.kind.params() {
            if spec.learn.tier == enth_params::Tier::Advanced
                || present::control(&theme, spec) != Control::Segmented
            {
                continue;
            }
            let label = present::label_in(ck, spec, true);
            let header = card_header(&h, &title);
            let node = h
                .query_all_by_role_and_label(Role::RadioGroup, label)
                .map(|n| n.rect())
                .find(|r| {
                    (header.left()..=header.right()).contains(&r.center().x)
                        && r.top() >= header.top()
                })
                .unwrap_or_else(|| panic!("no {label:?} in {title}"));
            let label_w = text_width(&h, label, &theme.text.label);
            let track: f32 = spec
                .enum_items()
                .unwrap()
                .iter()
                .map(|i| {
                    text_width(
                        &h,
                        present::item_label_in(ck, spec, i, true),
                        &theme.text.body,
                    ) + 2.0 * theme.space.gap
                })
                .sum::<f32>()
                + 2.0 * theme.space.hairline;
            if node.width() < label_w + theme.space.gap + track - 0.5
                || node.right() > header.right() + 0.5
            {
                cut.push(format!(
                    "{title} {label}: row {:.1} px, needs {:.1} (label {label_w:.1} + track {track:.1})",
                    node.width(),
                    label_w + theme.space.gap + track
                ));
            }
        }
    }
    assert!(cut.is_empty(), "{}", cut.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------
// The audio drag
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn dropping_a_generators_handle_on_a_wt_input_makes_one_audio_route_at_the_default_depth() {
    let mut h = harness_sized(osc_and_wt(), min_window(), true);
    assert_eq!(h.state().editor.report().density, Density::Dense);
    assert_eq!(h.state().session.patch().routes.len(), 0);
    let from = audio_handle(&h, "Osc 1");
    let to = center_in(&h, "WT 1", "PM input");
    let during = drag_drop(&mut h, from, to);
    assert!(
        during.contains("Drop to feed it the audio of Osc 1 at +100 %"),
        "the target says what a drop does: {during}"
    );
    assert!((AUDIO_ROUTE_DEPTH - 100.0).abs() < 1e-6);
    let app = h.state();
    assert_eq!(app.depth("osc1", "wt1.pm"), Some(AUDIO_ROUTE_DEPTH));
    assert_eq!(app.session.patch().routes.len(), 1);
    let step = app.session.history().next_undo().expect("one step");
    assert_eq!(step.forward.len(), 1, "one insert: {step:?}");
    assert_eq!(step.gesture, None, "a drop is one edit, not a gesture");

    // Undo takes the route away, redo brings it back at the same depth.
    h.get_by_label("Undo").click();
    h.run_steps(2);
    assert_eq!(h.state().depth("osc1", "wt1.pm"), None);
    h.get_by_label("Redo").click();
    h.run_steps(2);
    assert_eq!(h.state().depth("osc1", "wt1.pm"), Some(AUDIO_ROUTE_DEPTH));

    // Dropped again where it exists, nothing changes.
    let again = drag_drop(&mut h, from, to);
    assert!(again.contains("Osc 1 already feeds it (+100 %)"), "{again}");
    assert_eq!(h.state().session.patch().routes.len(), 1);
}

#[test]
fn the_am_input_takes_a_drop_too() {
    let mut h = harness_sized(osc_and_wt(), min_window(), true);
    let from = audio_handle(&h, "Osc 1");
    let to = center_in(&h, "WT 1", "AM input");
    drag_drop(&mut h, from, to);
    assert_eq!(h.state().depth("osc1", "wt1.am"), Some(AUDIO_ROUTE_DEPTH));
}

#[test]
fn a_modulator_dropped_on_an_audio_input_is_refused_with_the_reason() {
    let mut h = harness_sized(osc_and_wt(), min_window(), true);
    let from = h
        .get_by_role_and_label(Role::Button, "Env 1")
        .rect()
        .center();
    let to = center_in(&h, "WT 1", "PM input");
    let during = drag_drop(&mut h, from, to);
    assert!(
        during.contains("takes the audio of a generator, not a modulation source"),
        "{during}"
    );
    assert!(
        h.state().session.patch().routes.is_empty(),
        "nothing routed"
    );
    assert!(!h.state().session.history().can_undo());
}

#[test]
fn a_generators_audio_is_refused_on_a_knob_that_is_not_an_audio_input() {
    let mut h = harness_sized(osc_and_wt(), min_window(), true);
    let from = audio_handle(&h, "Osc 1");
    let to = center_in(&h, "WT 1", "Position");
    let during = drag_drop(&mut h, from, to);
    assert!(
        during.contains("its audio can drive only an audio input"),
        "{during}"
    );
    assert_eq!(h.state().session.patch().routes.len(), 0);
}

#[test]
fn the_drop_check_of_a_generator_follows_the_patch_rules() {
    let mut patch = osc_and_wt();
    let at = |s: &str| -> ParamAddr { s.parse().unwrap() };
    let src = |s: &str| -> ModSource { s.parse().unwrap() };
    let verdict = |patch: &Patch, s: &str, a: &str| {
        let a = at(a);
        match check(patch, &src(s), &a, patch.spec(&a).unwrap()) {
            DropCheck::Route => "route",
            DropCheck::Routed(_) => "routed",
            DropCheck::Refused(_) => "refused",
        }
    };
    assert_eq!(verdict(&patch, "osc1", "wt1.pm"), "route");
    assert_eq!(verdict(&patch, "osc1", "wt1.am"), "route");
    assert_eq!(verdict(&patch, "osc1", "wt1.position"), "refused");
    assert_eq!(verdict(&patch, "osc1", "osc1.level"), "refused");
    assert_eq!(verdict(&patch, "env1", "wt1.pm"), "refused");
    // A wavetable cannot drive itself, and cannot drive its own driver.
    assert_eq!(verdict(&patch, "wt1", "wt1.pm"), "refused");
    patch.routes.push(Route {
        src: src("osc1"),
        dst: at("wt1.pm"),
        depth: 100.0,
    });
    assert_eq!(verdict(&patch, "osc1", "wt1.pm"), "routed");
    // With osc1 → wt1 in place, wt1 may not feed osc1's audio back: osc has no audio input, and a second wt that wt1
    // drives may not drive wt1 in return.
    let mut loopy = patch.clone();
    loopy
        .generators
        .push(ModuleInstance::new(id("wt2"), ModuleKind::Wt));
    loopy.routes.push(Route {
        src: src("wt1"),
        dst: at("wt2.pm"),
        depth: 100.0,
    });
    assert_eq!(verdict(&loopy, "wt2", "wt1.am"), "refused");
}

#[test]
fn a_p1_patch_has_no_audio_handle() {
    let h = harness_sized(common::patches::p1_default(), min_window(), true);
    assert!(h.query_by_label_contains(" audio").is_none());
}

// ---------------------------------------------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------------------------------------------

fn snapshot(results: &mut SnapshotResults, name: &str, patch: Patch, drag: bool) {
    let theme = Theme::dark();
    let t = theme.clone();
    let mut editor = Editor::new(&theme);
    editor.set_auto_dense(true);
    let session = Session::new(patch);
    editor.open(&session, &theme);
    let mut h = common::harness(
        &theme,
        theme.size.window.default,
        move |ui, (editor, session): &mut (Editor, Session)| {
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
            editor.frame(&mut root, &t, session, false);
        },
        (editor, session),
    );
    h.run();
    if drag {
        // The generator's handle pressed and carried over wt's PM input, the button still held.
        let from = h
            .get_by_role_and_label(Role::Button, "Osc 1 audio")
            .rect()
            .center();
        let header = h.get_by_label("Move WT 1").rect();
        let to = h
            .query_all_by_label("PM input")
            .map(|n| n.rect())
            .find(|r| (header.left()..=header.right()).contains(&r.center().x))
            .expect("the PM input")
            .center();
        h.hover_at(from);
        h.step();
        h.drag_at(from);
        h.step();
        for i in 1..=8 {
            h.hover_at(from + (to - from) * i as f32 / 8.0);
            h.step();
        }
        h.run_steps(2);
    }
    h.snapshot(name);
    results.extend_harness(&mut h);
}

#[test]
fn dense_snapshots() {
    let mut results = SnapshotResults::new();
    snapshot(&mut results, "editor_dense_g2", g2(), false);
    snapshot(&mut results, "editor_audio_drag", osc_and_wt(), true);
    results.unwrap();
}
