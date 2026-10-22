//! The duel card's seam (lane S0e's inert entry point, R5; lanes SKD and DUI fill the card): with the `duel` feature on,
//! `card` draws nothing and pushes no intent, available or not, so no existing frame changes. kittest renders no
//! pixels here; it only checks what the card adds to the `Ui`.
#![cfg(feature = "duel")]

use egui::{Vec2, vec2};
use egui_kittest::Harness;
use enth_ui::Theme;
use enth_ui::editor::cards::Intents;
use enth_ui::editor::duel::DuelCardView;
use enth_ui::editor::duel::card::card;

/// The card alone, over a view, with the room it took in its `Ui` and the intents it pushed.
struct App {
    theme: Theme,
    view: DuelCardView,
    out: Intents,
    took: Vec2,
}

/// Runs the card for one frame: whether it pushed no intent, and the room it took.
fn run(view: DuelCardView) -> (bool, Vec2) {
    let theme = Theme::dark();
    let app = App {
        theme: theme.clone(),
        view,
        out: Intents::default(),
        took: vec2(-1.0, -1.0),
    };
    let mut h = Harness::builder()
        .with_size(vec2(320.0, 200.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .build_ui_state(
            |ui, app: &mut App| {
                let before = ui.min_rect().size();
                card(ui, &app.theme, &app.view, &mut app.out);
                app.took = ui.min_rect().size() - before;
            },
            app,
        );
    h.run();
    (h.state().out.is_empty(), h.state().took)
}

#[test]
fn the_card_is_not_available_by_default_and_takes_no_room() {
    assert!(!DuelCardView::default().available);
    assert_eq!(run(DuelCardView::default()), (true, vec2(0.0, 0.0)));
}

#[test]
fn an_available_card_is_still_empty_until_its_lane_draws_it() {
    assert_eq!(
        run(DuelCardView { available: true }),
        (true, vec2(0.0, 0.0))
    );
}
