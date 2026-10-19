//! The header band: the wordmark, the patch name, and the Undo and Redo buttons (keyboard undo waits for the
//! fork's captured-shortcut hook, report 02 D6, so the buttons are the way to undo in every host).

use egui::{Align, Align2, Button, Layout, Rect, RichText, Ui, UiBuilder, pos2};

use super::region_ui;
use crate::Theme;
use crate::editor::cards::{Intent, Intents};

/// What the header shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderView {
    /// The patch's name; empty for an unnamed patch.
    pub patch_name: String,
    pub can_undo: bool,
    pub can_redo: bool,
    /// What Undo would undo ("Filter 1 › Cutoff"), for its tooltip.
    pub undo: Option<String>,
    /// What Redo would redo.
    pub redo: Option<String>,
}

/// The header band.
pub struct Header<'a> {
    theme: &'a Theme,
    view: &'a HeaderView,
}

impl<'a> Header<'a> {
    pub fn new(theme: &'a Theme, view: &'a HeaderView) -> Self {
        Self { theme, view }
    }

    /// Draws into `rect`; the buttons push [`Intent::Undo`] and [`Intent::Redo`].
    pub fn show(self, ui: &mut Ui, rect: Rect, out: &mut Intents) {
        let t = self.theme;
        let c = &t.color;
        let mut ui = region_ui(ui, rect, "header");
        ui.painter()
            .rect_filled(rect, t.radius.medium, c.surface.card);
        let inner = rect.shrink2(egui::vec2(t.space.pad, 0.0));
        ui.painter().text(
            pos2(inner.left(), inner.center().y),
            Align2::LEFT_CENTER,
            "enth",
            t.text.title.clone(),
            c.text.primary,
        );
        let (name, ink) = if self.view.patch_name.is_empty() {
            ("Untitled", c.text.label)
        } else {
            (self.view.patch_name.as_str(), c.text.primary)
        };
        ui.painter().text(
            inner.center(),
            Align2::CENTER_CENTER,
            name,
            t.text.body.clone(),
            ink,
        );
        let mut buttons = ui.new_child(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::right_to_left(Align::Center)),
        );
        buttons.spacing_mut().item_spacing.x = t.space.tight;
        let button = |text: &str| Button::new(RichText::new(text).font(t.text.body.clone()));
        // Right to left: Redo is drawn first.
        let redo = buttons.add_enabled(self.view.can_redo, button("Redo"));
        let redo = match &self.view.redo {
            Some(what) => redo.on_hover_text(format!("Redo {what}")),
            None => redo,
        };
        let undo = buttons.add_enabled(self.view.can_undo, button("Undo"));
        let undo = match &self.view.undo {
            Some(what) => undo.on_hover_text(format!("Undo {what}")),
            None => undo,
        };
        if undo.clicked() {
            out.push(Intent::Undo);
        }
        if redo.clicked() {
            out.push(Intent::Redo);
        }
    }
}
