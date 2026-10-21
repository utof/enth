//! The Learn header hook (lane S0l's entry point, R2; lane LUI fills the view): with the `learn` feature on, the
//! header draws one **Learn** button when the shell says lessons are available, and draws exactly as before when it
//! does not. Since lane SKL's minimal panel (F2l) the click toggles the Learn panel (`tests/learn_panel.rs` plays a
//! lesson through it). kittest finds the button by its accessibility label, with no renderer.
#![cfg(feature = "learn")]

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_ui::Theme;
use enth_ui::editor::cards::{Intent, Intents};
use enth_ui::editor::learn::{HeaderLearn, LearnIntent};
use enth_ui::editor::regions::{Header, HeaderView};

/// The header band alone, over a view, and the intents its frames pushed.
struct App {
    theme: Theme,
    view: HeaderView,
    out: Intents,
}

fn harness(view: HeaderView) -> Harness<'static, App> {
    let theme = Theme::dark();
    let app = App {
        theme: theme.clone(),
        view,
        out: Intents::default(),
    };
    Harness::builder()
        .with_size(vec2(theme.size.window.default.x, theme.size.bar.header))
        .with_pixels_per_point(1.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .build_ui_state(
            |ui, app: &mut App| {
                let rect = ui.max_rect();
                Header::new(&app.theme, &app.view).show(ui, rect, &mut app.out);
            },
            app,
        )
}

#[test]
fn the_header_has_no_learn_entry_until_the_shell_says_lessons_are_available() {
    let mut h = harness(HeaderView::default());
    h.run();
    assert!(h.query_by_label("Learn").is_none());
    assert!(h.state().out.is_empty());
}

#[test]
fn an_available_learn_entry_is_drawn_and_toggles_the_panel() {
    let view = HeaderView {
        learn: HeaderLearn { available: true },
        ..HeaderView::default()
    };
    let mut h = harness(view);
    h.run();
    h.get_by_label("Learn").click();
    h.run();
    assert_eq!(
        h.state().out.as_slice(),
        &[Intent::Learn(LearnIntent::Toggle)],
        "the entry toggles the Learn panel (lane SKL)"
    );
}
