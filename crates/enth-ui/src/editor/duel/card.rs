//! **Owner: lane DUI** (the duel card, feature `duel`; lane SKD writes its minimal card first, F2d). Pre-declared by lane
//! S0e (R4/R5).
//!
//! The duel card's entry point and its view, the seam the other files draw into (see [`super`]). Inert: with
//! [`DuelCardView::available`] false, which is the default, [`card`] adds nothing to the `Ui`, so nothing an existing
//! frame draws changes. SKD gives the view its fields (the share line, the pasted tokens, the verification, the sealed
//! answer) from `enth-session`'s duel read model and calls [`card`] from the place the results drawer ends.

use egui::Ui;

use crate::Theme;
use crate::editor::cards::Intents;

/// What the duel card shows (the part of the shell's duel read model the card reads).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DuelCardView {
    /// The shell can play duels, so the card draws. False until a shell (the plugin's `duel` feature, lane SKD) says so.
    pub available: bool,
}

/// The duel card's hook point (feature `duel`). Draws the card when duels are available; inert, and draws nothing
/// otherwise.
pub fn card(ui: &mut Ui, theme: &Theme, view: &DuelCardView, out: &mut Intents) {
    if view.available {
        draw(ui, theme, out);
    }
}

/// The card itself: empty until lane SKD draws the minimal card here and lane DUI the real one.
fn draw(_ui: &mut Ui, _theme: &Theme, _out: &mut Intents) {}
