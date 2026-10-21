//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2).
//!
//! The header's Learn hook, wired as Recreate's entry is (`recreate/bar.rs`): `Header::show` ends with one call, behind
//! the feature, so LUI fills the entry from this file without editing `regions/header.rs`. The hook draws one
//! **Learn** button in the header's right-hand row, left of Recreate, **only when the shell says lessons are
//! available** ([`HeaderLearn::available`], false by default): a build, a kittest frame or a dev window that never sets
//! it draws exactly as before. Colours, sizes, spacing and fonts come from the theme's tokens.
//!
//! **The click** (lane SKL, F2l) pushes `Intent::Learn(LearnIntent::Toggle)`, which shows or hides the minimal Learn
//! panel (the lesson list, or the live lesson's step); lane LUI builds the real browser and pane on the same intent.

use egui::{Button, Rect, RichText, Ui};

use super::LearnIntent;
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::editor::regions::HeaderView;

/// What the header's Learn hook shows (part of [`HeaderView`](crate::editor::regions::HeaderView)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderLearn {
    /// The shell has lessons to offer, so the hook draws its entry. False until a shell (the plugin's `learn` feature,
    /// lane SKL) says so.
    pub available: bool,
}

/// The header's Learn hook point (feature `learn`): called by the header with its right-hand row (laid out right to
/// left, after Recreate) and its inner rectangle. Draws the **Learn** entry when lessons are available; a click
/// toggles the Learn panel.
pub(crate) fn header(
    ui: &mut Ui,
    theme: &Theme,
    _rect: Rect,
    view: &HeaderView,
    out: &mut Intents,
) {
    if !view.learn.available {
        return;
    }
    ui.add_space(theme.space.section);
    let response = ui
        .add(Button::new(
            RichText::new("Learn").font(theme.text.body.clone()),
        ))
        .on_hover_text("Lessons, one idea each, played on this instrument");
    if response.clicked() {
        out.push(Intent::Learn(LearnIntent::Toggle));
    }
}
