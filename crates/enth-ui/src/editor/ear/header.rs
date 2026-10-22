//! **Owner: lane EUI** (the Ear view, feature `ear`). Pre-declared by lane S0e (R4/R5).
//!
//! The header's Ear hook, wired as Learn's, Create's and Recreate's entries are (`learn/header.rs`, `create/header.rs`,
//! `recreate/bar.rs`): `Header::show` ends with one call, behind the feature, so EUI fills the entry from this file
//! without editing `regions/header.rs`. The hook draws one **Ear** button in the header's right-hand row, left of Create,
//! **only when the shell says the Ear games are available** ([`HeaderEar::available`], false by default): a build, a
//! kittest frame or a dev window that never sets it draws exactly as before. Colours, sizes, spacing and fonts come from
//! the theme's tokens.
//!
//! **Inert.** The button does nothing when clicked. Lane SKE turns the click into an `Intent::Ear(EarIntent::Toggle)`
//! that opens its minimal panel (F2e), as lane SK3 did for Create; lane EUI's view opens from here the same way.

use egui::{Button, Rect, RichText, Ui};

use crate::Theme;
use crate::editor::cards::Intents;
use crate::editor::regions::HeaderView;

/// What the header's Ear hook shows (part of [`HeaderView`](crate::editor::regions::HeaderView)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderEar {
    /// The shell can play ear games, so the hook draws its entry. False until a shell (the plugin's `ear` feature, lane
    /// SKE) says so.
    pub available: bool,
}

/// The header's Ear hook point (feature `ear`): called by the header with its right-hand row (laid out right to left,
/// after Create) and its inner rectangle. Draws the **Ear** entry when the games are available; inert.
pub(crate) fn header(
    ui: &mut Ui,
    theme: &Theme,
    _rect: Rect,
    view: &HeaderView,
    _out: &mut Intents,
) {
    if !view.ear.available {
        return;
    }
    ui.add_space(theme.space.section);
    // Inert: the response is dropped, so a click changes nothing. SKE reads it and pushes an intent.
    let _ = ui
        .add(Button::new(
            RichText::new("Ear").font(theme.text.body.clone()),
        ))
        .on_hover_text("Ear training games");
}
