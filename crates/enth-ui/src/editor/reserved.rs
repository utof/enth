//! The reserved regions (lane ED1): the lesson pane (feature `learn`), the challenge bar and the results drawer
//! (feature `recreate`). A Learn or Recreate layout places them; P1b builds no presenters, so each draws an empty panel
//! with its name in the dimmed ink, which shows where a variant put it.

use egui::{Align2, Rect, Ui};

use super::layout::Region;
use crate::Theme;

/// Draws a reserved region's empty panel into `rect`.
pub fn show(ui: &mut Ui, t: &Theme, rect: Rect, region: Region) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.radius.medium, t.color.surface.card);
    let name = match region {
        #[cfg(feature = "learn")]
        Region::Lesson => "Lesson",
        #[cfg(feature = "recreate")]
        Region::ChallengeBar => "Challenge",
        #[cfg(feature = "recreate")]
        Region::Results => "Results",
        _ => region.name(),
    };
    painter.text(
        rect.left_top() + egui::vec2(t.space.gap, t.space.gap),
        Align2::LEFT_TOP,
        name,
        t.text.label.clone(),
        t.color.text.dimmed,
    );
}
