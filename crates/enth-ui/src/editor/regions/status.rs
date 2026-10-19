//! The status line: the hovered control's path, value and modulation, and a de-emphasised note on the right.

use egui::{Rect, Ui};

use super::region_ui;
use crate::Theme;
use crate::editor::cards::Hovered;
use crate::widgets::StatusLine;

/// The status line.
pub struct Status<'a> {
    theme: &'a Theme,
    hovered: Option<&'a Hovered>,
    right: &'a str,
}

impl<'a> Status<'a> {
    /// `hovered` is this frame's hovered control (from [`Intents::hovered`](crate::editor::cards::Intents)), drawn
    /// after the regions that report it.
    pub fn new(theme: &'a Theme, hovered: Option<&'a Hovered>) -> Self {
        Self {
            theme,
            hovered,
            right: "",
        }
    }

    /// Right-aligned, dimmed text (the repaint state, a hint).
    #[must_use]
    pub fn right(mut self, right: &'a str) -> Self {
        self.right = right;
        self
    }

    /// Draws into `rect` (one `bar.status` band at its top).
    pub fn show(self, ui: &mut Ui, rect: Rect) {
        let mut ui = region_ui(ui, rect, "status");
        let line = StatusLine::new(self.theme).right(self.right);
        match self.hovered {
            Some(h) => line.path(&h.path).value(&h.value).modulation(&h.modulation),
            None => line.path("Hover a control to see its value and modulation"),
        }
        .show(&mut ui);
    }
}
