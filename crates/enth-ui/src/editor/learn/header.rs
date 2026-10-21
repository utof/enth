//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2).
//!
//! The header's Learn hook, wired as Recreate's entry is (`recreate/bar.rs`): `Header::show` ends with one call, behind
//! the feature, so LUI fills the entry from this file without editing `regions/header.rs`. The hook draws one button
//! in the header's right-hand row, left of Recreate, **only when the shell says lessons are available**
//! ([`HeaderLearn::available`], false by default): a build, a kittest frame or a dev window that never sets it draws
//! exactly as before. Colours, sizes, spacing and fonts come from the theme's tokens.
//!
//! **The click** pushes `Intent::Learn(LearnIntent::Toggle)`: it shows or hides the lesson pane. With no lesson the
//! button reads **Learn** (the pane opens on the lesson browser); while a lesson is live it reads **Lesson n/m**, so
//! a hidden pane (the collapsed state, where the full editor gets the whole window back) still shows where the lesson
//! stands, and a click brings the pane back. In Recreate mode the entry steps aside for the challenge bar.

use egui::{Button, Rect, RichText, Ui};

use super::LearnIntent;
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::editor::regions::HeaderView;

/// What the header's Learn hook shows (part of [`HeaderView`](crate::editor::regions::HeaderView)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderLearn {
    /// The shell has lessons to offer (or a lesson is live), so the hook draws its entry. False until a shell (the
    /// plugin's `learn` feature) says so.
    pub available: bool,
    /// The live lesson's step (1-based) and its step count, if a lesson is live.
    pub lesson: Option<(u16, u16)>,
    /// The lesson pane shows.
    pub open: bool,
}

impl HeaderLearn {
    /// The entry's text, which is also its accessible name: "Learn", or "Lesson n/m" while a lesson is live.
    pub fn text(&self) -> String {
        match self.lesson {
            Some((step, steps)) if steps > 0 => format!("Lesson {step}/{steps}"),
            Some(_) => "Lesson".to_owned(),
            None => "Learn".to_owned(),
        }
    }
}

/// The header's Learn hook point (feature `learn`): called by the header with its right-hand row (laid out right to
/// left, after Recreate) and its inner rectangle. Draws the entry when lessons are available; a click shows or hides
/// the lesson pane.
pub(crate) fn header(
    ui: &mut Ui,
    theme: &Theme,
    _rect: Rect,
    view: &HeaderView,
    out: &mut Intents,
) {
    let learn = &view.learn;
    if !learn.available {
        return;
    }
    // In Recreate mode the challenge bar takes the header's free span (lane RUI), so the entry steps aside, as the
    // Presets group does. A lesson's round is played there with the pane beside it (the pane's Hide waits for the
    // round's end); leaving Recreate mode brings the entry back.
    #[cfg(feature = "recreate")]
    if view.recreate.mode {
        return;
    }
    ui.add_space(theme.space.section);
    let tip = match (learn.lesson.is_some(), learn.open) {
        (false, _) => "Lessons, one idea each, played on this instrument",
        (true, true) => "Hide the lesson: the whole editor, the lesson keeps going",
        (true, false) => "Show the lesson",
    };
    let response = ui
        .add(
            Button::new(RichText::new(learn.text()).font(theme.text.body.clone()))
                .selected(learn.open),
        )
        .on_hover_text(tip);
    if response.clicked() {
        out.push(Intent::Learn(LearnIntent::Toggle));
    }
}
