//! **Owner: lane RUI** (the Recreate UI). Pre-declared by lane S0r (R1/S1).
//!
//! The challenge bar: difficulty dots, Target ▶ / Mine ▶ / A/B, the timer, Hint and Submit (report 01 §2.6). No
//! live score by default; Submit shows the Good/Near/Bad rings, which then update live.

/// The header's Recreate hook point (feature `recreate`): called by the header with its own `Ui` and inner
/// rectangle, so RUI puts the mode tab and the challenge bar in the header row without editing
/// `regions/header.rs`. A no-op until lane RUI fills it.
pub(crate) fn header(
    _ui: &mut egui::Ui,
    _theme: &crate::Theme,
    _rect: egui::Rect,
    _view: &crate::editor::regions::HeaderView,
    _out: &mut crate::editor::cards::Intents,
) {
}
