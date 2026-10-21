//! The Create header hook (lane S0s's entry point, R3; lane SK3's minimal panel, F2s; lane PUI fills the view): with the
//! `create` feature on, the header draws one **Create** button when the shell says create from sound is available,
//! and draws exactly as before when it does not. The click toggles the Create panel (lane SK3). kittest finds the
//! button by its accessibility label, with no renderer.
#![cfg(feature = "create")]

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_ui::Theme;
use enth_ui::editor::cards::{Intent, Intents};
use enth_ui::editor::create::{CreateIntent, HeaderCreate};
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
fn the_header_has_no_create_entry_until_the_shell_says_create_is_available() {
    let mut h = harness(HeaderView::default());
    h.run();
    assert!(h.query_by_label("Create").is_none());
    assert!(h.state().out.is_empty());
}

#[test]
fn an_available_create_entry_is_drawn_and_its_click_toggles_the_panel() {
    let view = HeaderView {
        create: HeaderCreate { available: true },
        ..HeaderView::default()
    };
    let mut h = harness(view);
    h.run();
    h.get_by_label("Create").click();
    h.run();
    assert_eq!(
        h.state().out.as_slice(),
        [Intent::Create(CreateIntent::Toggle)],
        "the entry toggles the panel (lane SK3)"
    );
}
