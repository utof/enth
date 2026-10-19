//! Widget behaviour through egui's input and the AccessKit tree (no pixels).

use egui::{Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_ui::Theme;
use enth_ui::theme::Category;
use enth_ui::widgets::{Knob, MatrixGrid, ModuleCard, Segmented};

fn harness<'a, S>(app: impl FnMut(&mut egui::Ui, &mut S) + 'a, state: S) -> Harness<'a, S> {
    Harness::builder()
        .with_size(vec2(400.0, 300.0))
        .with_context_setup(|ctx| Theme::dark().apply(ctx))
        .build_ui_state(app, state)
}

/// Drags from the centre of the node labelled `label` by `delta`.
fn drag<S>(harness: &mut Harness<'_, S>, label: &str, delta: egui::Vec2) {
    let start = harness.get_by_label(label).rect().center();
    harness.hover_at(start);
    harness.drag_at(start);
    harness.run();
    for step in 1..=4 {
        harness.hover_at(start + delta * step as f32 / 4.0);
        harness.run();
    }
    harness.drop_at(start + delta);
    harness.run();
}

#[test]
fn dragging_a_knob_up_raises_its_value() {
    let theme = Theme::dark();
    let mut h = harness(
        |ui, value: &mut f32| {
            ui.add(Knob::new(&theme, value, "Cutoff"));
        },
        0.5,
    );
    let travel = theme.size.knob.drag_travel;
    drag(&mut h, "Cutoff", vec2(0.0, -travel / 4.0));
    let value = *h.state();
    assert!(
        value > 0.6 && value <= 0.75 + 1e-3,
        "value after dragging up: {value}"
    );
    drag(&mut h, "Cutoff", vec2(0.0, travel * 2.0));
    assert_eq!(*h.state(), 0.0, "clamped at the minimum");
}

#[test]
fn segmented_selects_the_clicked_option() {
    let theme = Theme::dark();
    let mut h = harness(
        |ui, selected: &mut usize| {
            ui.add(Segmented::new(
                &theme,
                "mode",
                selected,
                &["Play", "Recreate", "Learn"],
            ));
        },
        0,
    );
    h.get_by_label("Learn").click();
    h.run();
    assert_eq!(*h.state(), 2);
}

#[test]
fn card_toggle_flips_enabled_and_header_is_a_drag_handle() {
    let theme = Theme::dark();
    let mut h = harness(
        |ui, enabled: &mut bool| {
            ModuleCard::new(
                &theme,
                "osc-1",
                "Osc 1",
                enth_ui::theme::Category::Generator,
                enabled,
            )
            .show(ui, |_, _| {}, |_| {});
        },
        true,
    );
    h.get_by_role_and_label(Role::CheckBox, "Osc 1").click();
    h.run();
    assert!(!*h.state());
    let _ = h.get_by_label("Move Osc 1");
}

#[test]
fn matrix_cells_edit_their_amount() {
    let theme = Theme::dark();
    let mut h = harness(
        |ui, amounts: &mut [f32; 4]| {
            MatrixGrid::new(
                &theme,
                "m",
                &["LFO 1", "Env 2"],
                &["Cutoff", "Pitch"],
                amounts,
            )
            .show(ui);
        },
        [0.0; 4],
    );
    drag(
        &mut h,
        "Pitch from Env 2",
        vec2(theme.size.knob.drag_travel / 2.0, 0.0),
    );
    let amounts = *h.state();
    assert_eq!(amounts[..3], [0.0; 3]);
    assert!(amounts[3] > 0.3, "{amounts:?}");
}

#[test]
fn a_disabled_card_keeps_its_controls_editable() {
    let theme = Theme::dark();
    let mut h = harness(
        |ui, value: &mut f32| {
            let mut enabled = false;
            ModuleCard::new(&theme, "osc-2", "Osc 2", Category::Generator, &mut enabled).show(
                ui,
                |_, _| {},
                |ui| {
                    ui.add(Knob::new(&theme, value, "Level"));
                },
            );
        },
        0.5,
    );
    drag(
        &mut h,
        "Level",
        vec2(0.0, -theme.size.knob.drag_travel / 4.0),
    );
    assert!(*h.state() > 0.6, "value after dragging up: {}", h.state());
}
