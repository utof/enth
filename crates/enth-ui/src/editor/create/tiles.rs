//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3).
//!
//! The K candidate tiles (docs/r3.md, "The Create view and the session"), `size.create.tile_columns` to a row,
//! `size.create.tile_height` tall, **in branch order**, so a tile stays where it is while the search re-ranks them
//! after every round (`enth_solver::create`'s branch index is "stable tiles in a UI"). Each tile shows its rank from
//! best (1) to worst in `text.title`, where its start came from ("heuristic", "template `pluck_1osc`", "k-NN 3", in R3b
//! "model 2"), its distance (the objective's, lower is better; "–" before its first evaluation) and its state:
//! proposed, searching, done, or pruned (successive halving dropped it; the tile is drawn dimmed, and stays playable
//! and usable). "✓ used" marks the one taken last with "Use this".
//!
//! A click selects the tile for the compare area under the grid ([`super::audition`]), where it is played, A/B'd
//! against the upload and taken; the selected tile is outlined in the value accent on its wash. With nothing selected
//! the best is. Each tile is named "Select candidate n" (n its rank), for kittest and screen readers.

use egui::{Rect, Sense, Ui, vec2};
use enth_session::create::{CandidateState, CandidateView, CreateView};

use super::panel::{elided, named};
use super::state::CreateIntent;
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::theme::glyph;

/// The state as a tile says it.
pub(crate) fn state_text(state: CandidateState) -> &'static str {
    match state {
        CandidateState::Proposed => "proposed",
        CandidateState::Searching => "searching",
        CandidateState::Done => "done",
        CandidateState::Pruned => "pruned",
        _ => "",
    }
}

/// A distance as the view writes it ("0.234"; "–" before the candidate is evaluated).
pub(crate) fn distance_text(d: Option<f64>) -> String {
    d.map_or_else(|| "–".to_owned(), |d| format!("{d:.3}"))
}

/// Draws the tiles of `view` into `ui`; `selected` is the compare area's branch.
pub(crate) fn show(
    view: &CreateView,
    selected: Option<usize>,
    ui: &mut Ui,
    t: &Theme,
    out: &mut Intents,
) {
    let cols = t.size.create.tile_columns.max(1);
    let gap = t.space.tight;
    let width = ui.available_width();
    let tile_w = ((width - gap * (cols - 1) as f32) / cols as f32).max(0.0);
    let mut tiles: Vec<&CandidateView> = view.candidates.iter().collect();
    tiles.sort_by_key(|c| c.branch);
    for row in tiles.chunks(cols) {
        let (row_rect, _) =
            ui.allocate_exact_size(vec2(width, t.size.create.tile_height), Sense::hover());
        for (i, c) in row.iter().enumerate() {
            let left = row_rect.left() + i as f32 * (tile_w + gap);
            let rect = Rect::from_min_size(
                egui::pos2(left, row_rect.top()),
                vec2(tile_w, t.size.create.tile_height),
            );
            tile(view, c, selected == Some(c.branch), ui, t, rect, out);
        }
    }
}

fn tile(
    view: &CreateView,
    c: &CandidateView,
    selected: bool,
    ui: &mut Ui,
    t: &Theme,
    rect: Rect,
    out: &mut Intents,
) {
    let id = egui::Id::unique(("enth.create.tile", c.branch));
    let response = ui.interact(rect, id, Sense::click());
    let painter = ui.painter();
    let fill = if selected {
        t.color.accent.value_wash
    } else if response.hovered() {
        t.color.surface.raised
    } else {
        t.color.surface.recessed
    };
    painter.rect_filled(rect, t.radius.medium, fill);
    if selected {
        painter.rect_stroke(
            rect,
            t.radius.medium,
            t.stroke.thin(t.color.accent.value),
            egui::StrokeKind::Inside,
        );
    }
    let pruned = c.state == CandidateState::Pruned;
    let (ink, soft) = if pruned {
        (t.color.text.dimmed, t.color.text.dimmed)
    } else {
        (t.color.text.primary, t.color.text.label)
    };
    let inner = rect.shrink2(vec2(t.space.gap, t.space.tight));
    let n = c.rank + 1;
    painter.text(
        egui::pos2(inner.left(), inner.center().y),
        egui::Align2::LEFT_CENTER,
        n.to_string(),
        t.text.title.clone(),
        ink,
    );
    let text_left = inner.left() + t.size.create.rank_width;
    let text_w = (inner.right() - text_left).max(0.0);
    let used = view.used.is_some_and(|(b, _)| b == c.branch);
    let label = elided(ui, &c.label(), &t.text.body, text_w);
    let painter = ui.painter();
    painter.text(
        egui::pos2(text_left, inner.top()),
        egui::Align2::LEFT_TOP,
        label,
        t.text.body.clone(),
        ink,
    );
    let distance = distance_text(c.distance);
    let dist = painter.text(
        egui::pos2(text_left, inner.bottom()),
        egui::Align2::LEFT_BOTTOM,
        &distance,
        t.text.value.clone(),
        ink,
    );
    let state = if used {
        format!("{} {} used", state_text(c.state), glyph::GOOD)
    } else {
        state_text(c.state).to_owned()
    };
    let state = elided(
        ui,
        &state,
        &t.text.label,
        (inner.right() - dist.right() - t.space.gap).max(0.0),
    );
    ui.painter().text(
        egui::pos2(inner.right(), inner.bottom()),
        egui::Align2::RIGHT_BOTTOM,
        state,
        t.text.label.clone(),
        soft,
    );
    named(&response, &format!("Select candidate {n}"));
    let response = response.on_hover_text(format!(
        "{} {} distance {distance} (lower is closer) {} {} renders",
        c.label(),
        glyph::BULLET,
        glyph::BULLET,
        c.renders
    ));
    if response.clicked() {
        out.push(Intent::Create(CreateIntent::Select(c.branch)));
    }
}
