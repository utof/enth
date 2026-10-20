//! **Owner: lane PBU** (the preset browser UI; after lane PBC's preset core). Pre-declared by lane S0r (R1/S1).
//!
//! The preset browser (report 03 §2.3, summary "Presets"): the header's "◀ Name ▶ Browse Save" through the hook
//! below, and a browse panel drawn over the body in any layout (search over name, description, author and tags;
//! click loads, double-click loads and closes, Escape closes; Save never offers read-only folders). A load goes
//! through the session's undoable preset load (lane PBC), never through host parameters. Sizes go in
//! `theme/sizes/browser.rs`.

/// The header's Presets hook point (feature `presets`): called by the header with its own `Ui` and inner rectangle,
/// where the patch name sits, so PBU draws ◀ name ▶, Browse and Save here without editing `regions/header.rs`. A
/// no-op until lane PBU fills it.
#[cfg(feature = "presets")]
pub(crate) fn header(
    _ui: &mut egui::Ui,
    _theme: &crate::Theme,
    _rect: egui::Rect,
    _view: &super::regions::HeaderView,
    _out: &mut super::cards::Intents,
) {
}
