//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3).
//!
//! The header's Create hook, wired as Learn's and Recreate's entries are (`learn/header.rs`, `recreate/bar.rs`):
//! `Header::show` ends with one call, behind the feature, so PUI fills the entry from this file without editing
//! `regions/header.rs`. The hook draws one **Create** button in the header's right-hand row, left of Learn, **only when
//! the shell says create is available** ([`HeaderCreate::available`], false by default): a build, a kittest frame or a
//! dev window that never sets it draws exactly as before. Colours, sizes, spacing and fonts come from the theme's
//! tokens.
//!
//! **Inert.** The button does nothing when clicked. Lane PUI turns the click into an `Intent::Create(CreateIntent::…)`
//! that opens the Create view, once the editor has a create state to hand it to.

use egui::{Button, Rect, RichText, Ui};

use crate::Theme;
use crate::editor::cards::Intents;
use crate::editor::regions::HeaderView;

/// What the header's Create hook shows (part of [`HeaderView`](crate::editor::regions::HeaderView)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderCreate {
    /// The shell can create a patch from a sound, so the hook draws its entry. False until a shell (the plugin's
    /// `create` feature, lane SK3) says so.
    pub available: bool,
}

/// The header's Create hook point (feature `create`): called by the header with its right-hand row (laid out right to
/// left, after Learn) and its inner rectangle. Draws the **Create** entry when create is available; inert.
pub(crate) fn header(
    ui: &mut Ui,
    theme: &Theme,
    _rect: Rect,
    view: &HeaderView,
    _out: &mut Intents,
) {
    if !view.create.available {
        return;
    }
    ui.add_space(theme.space.section);
    // Inert: the response is dropped, so a click changes nothing. PUI reads it and pushes an intent.
    let _ = ui
        .add(Button::new(
            RichText::new("Create").font(theme.text.body.clone()),
        ))
        .on_hover_text("Create patches from a sound file");
}
