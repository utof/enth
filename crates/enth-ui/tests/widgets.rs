//! Widget behaviour through egui's input and the AccessKit tree (no pixels).

use egui::{Modifiers, Role, Sense, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_ui::Theme;
use enth_ui::theme::Category;
use enth_ui::widgets::{
    CurveHandles, Display, Drag, Handle, Knob, MatrixGrid, ModuleCard, Segmented, Trace,
};

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

/// A point on a 200 × 100 display that drags `x` sideways and `y` up and down, each across its
/// whole range over the display, with 0.5 as the default. Frames are 1/60 s apart, so two clicks
/// in a row make a double-click.
fn handle_harness(theme: &Theme) -> Harness<'_, [f32; 2]> {
    Harness::builder()
        .with_size(vec2(400.0, 300.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(|ctx| Theme::dark().apply(ctx))
        .build_ui_state(
            move |ui, values: &mut [f32; 2]| {
                let (rect, _) = ui.allocate_exact_size(vec2(200.0, 100.0), Sense::hover());
                let [x, y] = values;
                let at = vec2(*x, *y);
                let edit = CurveHandles::new(theme, "curve").interact(
                    ui,
                    rect,
                    &mut [Handle::new("Point", at)
                        .x(Drag::new("X", x, 1.0).default_value(0.5))
                        .y(Drag::new("Y", y, 1.0).default_value(0.5))],
                );
                Display::new(theme, Trace::Curve(&|x| x), rect.size())
                    .handles(&edit)
                    .paint(ui, rect);
            },
            [0.5, 0.5],
        )
}

#[test]
fn curve_handles_follow_the_pointer_and_reset_on_double_click() {
    let theme = Theme::dark();
    let mut h = handle_harness(&theme);
    h.run();
    let before = h.get_by_label("Point").rect().center();
    // The trace area is the display less `space.tight` above and below.
    let height = 100.0 - 2.0 * theme.space.tight;
    drag(&mut h, "Point", vec2(50.0, -20.0));
    let [x, y] = *h.state();
    assert!((x - 0.75).abs() < 1e-3, "x after dragging right: {x}");
    assert!(
        (y - (0.5 + 20.0 / height)).abs() < 1e-3,
        "y after dragging up: {y}"
    );
    let after = h.get_by_label("Point").rect().center();
    assert!(
        ((after - before) - vec2(50.0, -20.0)).length() < 1.0,
        "the point stays under the pointer: {before:?} → {after:?}"
    );

    h.get_by_label("Point").click();
    h.run();
    h.get_by_label("Point").click();
    h.run();
    assert_eq!(*h.state(), [0.5, 0.5], "double-click resets both values");
}

#[test]
fn shift_drags_curve_handles_finely() {
    let theme = Theme::dark();
    let mut h = handle_harness(&theme);
    h.run();
    h.event(egui::Event::ModifiersChanged(Modifiers::SHIFT));
    drag(&mut h, "Point", vec2(50.0, 0.0));
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    let [x, y] = *h.state();
    let expected = 0.5 + 0.25 * theme.size.knob.fine_ratio;
    assert!((x - expected).abs() < 1e-3, "x after a fine drag: {x}");
    assert_eq!(y, 0.5, "a sideways drag leaves y alone");
}
