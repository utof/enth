//! **Owner: lane AM** (add and remove modules, the richer Init patch). Pre-declared by lane S0r (R1/S1).
//!
//! Adding and removing modules (report 03, summary "Add/remove modules"): the card "⋯" menu, a right-click on empty
//! region space, a dashed "+ Add" slot, and the header's Add hook below; one op and one undo step each, through
//! additive intents. AM also owns the Init patch's gain staging (at most −1 dBFS on a four-note chord) and the
//! minimum-window measurement. Sizes go in `theme/sizes/add_module.rs`.

/// The header's Add hook point (feature `add-module`): called by the header with its right-to-left button row, just
/// left of Undo, so AM adds its header control here without editing `regions/header.rs`. A no-op until lane AM
/// fills it.
#[cfg(feature = "add-module")]
pub(crate) fn header(
    _row: &mut egui::Ui,
    _theme: &crate::Theme,
    _rect: egui::Rect,
    _view: &super::regions::HeaderView,
    _out: &mut super::cards::Intents,
) {
}
