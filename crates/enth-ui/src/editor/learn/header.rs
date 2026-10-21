//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2).
//!
//! The header's Learn hook, wired as Recreate's entry is (`recreate/bar.rs`): `Header::show` ends with one call, behind
//! the feature, so LUI fills the entry from this file without editing `regions/header.rs`. The hook draws one
//! **Learn** button in the header's right-hand row, left of Recreate, **only when the shell says lessons are
//! available** ([`HeaderLearn::available`], false by default): a build, a kittest frame or a dev window that never sets
//! it draws exactly as before. Colours, sizes, spacing and fonts come from the theme's tokens.
//!
//! **Inert.** The button does nothing when clicked. Lane LUI turns the click into an `Intent::Learn(LearnIntent::…)`
//! that opens the lesson browser (and, in a lesson, the lesson pane), once the editor has a Learn state to hand it to.

use egui::{Button, Rect, RichText, Ui};

use crate::Theme;
use crate::editor::cards::Intents;
use crate::editor::regions::HeaderView;

/// What the header's Learn hook shows (part of [`HeaderView`](crate::editor::regions::HeaderView)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderLearn {
    /// The shell has lessons to offer, so the hook draws its entry. False until a shell (the plugin's `learn` feature,
    /// lane SKL) says so.
    pub available: bool,
}

/// The header's Learn hook point (feature `learn`): called by the header with its right-hand row (laid out right to
/// left, after Recreate) and its inner rectangle. Draws the **Learn** entry when lessons are available; inert.
pub(crate) fn header(
    ui: &mut Ui,
    theme: &Theme,
    _rect: Rect,
    view: &HeaderView,
    _out: &mut Intents,
) {
    if !view.learn.available {
        return;
    }
    ui.add_space(theme.space.section);
    // Inert: the response is dropped, so a click changes nothing. LUI reads it and pushes an intent.
    let _ = ui
        .add(Button::new(
            RichText::new("Learn").font(theme.text.body.clone()),
        ))
        .on_hover_text("Lessons, one idea each, played on this instrument");
}
