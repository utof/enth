//! The Ear header hook (lane S0e's inert entry point, R4; lane EUI fills the view): with the `ear` feature on, the
//! header draws one **Ear** button when the shell says the ear games are available, and draws exactly as before when it
//! does not. The click is inert for now. kittest finds the button by its accessibility label, with no renderer.
#![cfg(feature = "ear")]

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_ui::Theme;
use enth_ui::editor::cards::Intents;
use enth_ui::editor::ear::HeaderEar;
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
fn the_header_has_no_ear_entry_until_the_shell_says_the_games_are_available() {
    let mut h = harness(HeaderView::default());
    h.run();
    assert!(h.query_by_label("Ear").is_none());
    assert!(h.state().out.is_empty());
}

#[test]
fn an_available_ear_entry_is_drawn_and_inert() {
    let view = HeaderView {
        ear: HeaderEar { available: true },
        ..HeaderView::default()
    };
    let mut h = harness(view);
    h.run();
    h.get_by_label("Ear").click();
    h.run();
    assert!(
        h.state().out.is_empty(),
        "the entry pushes no intent yet: {:?}",
        h.state().out.as_slice()
    );
}
