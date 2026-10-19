//! The modulation matrix as a target × source grid (rows are targets). Each cell shows its
//! amount in percent with a bar from the centre; drag a cell to change it, double-click to clear.
//! It is the same data the CLI prints.

use egui::{Align2, Id, IdSalt, Rect, Response, Sense, Ui, pos2, vec2};

use super::paint;
use crate::theme::Theme;

pub struct MatrixGrid<'a> {
    theme: &'a Theme,
    salt: IdSalt,
    id: Option<Id>,
    sources: &'a [&'a str],
    targets: &'a [&'a str],
    /// Row-major, `targets.len() × sources.len()`, each −1..1.
    amounts: &'a mut [f32],
}

impl<'a> MatrixGrid<'a> {
    /// # Panics
    /// If `amounts` is not `targets.len() * sources.len()` long.
    pub fn new(
        theme: &'a Theme,
        id_salt: impl egui::AsIdSalt,
        sources: &'a [&'a str],
        targets: &'a [&'a str],
        amounts: &'a mut [f32],
    ) -> Self {
        assert_eq!(amounts.len(), sources.len() * targets.len());
        Self {
            theme,
            salt: IdSalt::new(id_salt),
            id: None,
            sources,
            targets,
            amounts,
        }
    }

    /// Interact under this global id (cell `i` under `id.with(i)`, row-major) instead of one
    /// salted within the parent `Ui`.
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        self.show_cells(ui).0
    }

    /// Like [`show`](Self::show), plus each cell's own response, row-major: a cell's drag start
    /// and stop are the edges of one gesture.
    pub fn show_cells(self, ui: &mut Ui) -> (Response, Vec<Response>) {
        let t = self.theme;
        let c = &t.color;
        let m = &t.size.matrix;
        let id = self.id.unwrap_or_else(|| ui.make_persistent_id(self.salt));
        let mut cells = Vec::with_capacity(self.amounts.len());
        let width = m.target_width + m.cell_width * self.sources.len() as f32;
        let height = m.row_height * (self.targets.len() + 1) as f32;
        let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let painter = ui.painter();
        for (s, source) in self.sources.iter().enumerate() {
            let x = rect.left() + m.target_width + m.cell_width * (s as f32 + 0.5);
            paint::text_in(
                ui,
                painter,
                pos2(x, rect.top() + m.row_height / 2.0),
                Align2::CENTER_CENTER,
                source,
                &t.text.label,
                c.text.label,
                m.cell_width,
            );
        }
        let gap = t.space.hairline;
        let fine = ui.input(|i| i.modifiers.shift);
        for (row, target) in self.targets.iter().enumerate() {
            let top = rect.top() + m.row_height * (row + 1) as f32;
            paint::text_in(
                ui,
                painter,
                pos2(rect.left(), top + m.row_height / 2.0),
                Align2::LEFT_CENTER,
                target,
                &t.text.label,
                c.text.label,
                m.target_width - t.space.gap,
            );
            for s in 0..self.sources.len() {
                let left = rect.left() + m.target_width + m.cell_width * s as f32;
                let cell = Rect::from_min_size(pos2(left, top), vec2(m.cell_width, m.row_height))
                    .shrink(gap / 2.0);
                let index = row * self.sources.len() + s;
                let hit = ui.interact(cell, id.with(index), Sense::click_and_drag());
                let value = f64::from(self.amounts[index]);
                hit.widget_info(|| {
                    egui::WidgetInfo::slider(
                        true,
                        value,
                        format!("{target} from {}", self.sources[s]),
                    )
                });
                let amount = &mut self.amounts[index];
                if hit.dragged() {
                    let scale = if fine { t.size.knob.fine_ratio } else { 1.0 };
                    let delta = hit.drag_delta();
                    *amount = (*amount + (delta.x - delta.y) / t.size.knob.drag_travel * scale)
                        .clamp(-1.0, 1.0);
                    response.mark_changed();
                }
                if hit.double_clicked() {
                    *amount = 0.0;
                    response.mark_changed();
                }
                let fill = if hit.hovered() || hit.dragged() {
                    c.surface.raised
                } else {
                    c.surface.recessed
                };
                painter.rect_filled(cell, t.radius.small, fill);
                if *amount != 0.0 {
                    let bar_y = cell.bottom() - gap / 2.0;
                    let mid = cell.center().x;
                    let reach = cell.width() / 2.0 * amount.abs();
                    let (from, to) = if *amount > 0.0 {
                        (mid, mid + reach)
                    } else {
                        (mid - reach, mid)
                    };
                    painter.hline(
                        cell.x_range(),
                        bar_y,
                        t.stroke.thin(c.accent.modulation_wash),
                    );
                    painter.hline(from..=to, bar_y, t.stroke.thin(c.accent.modulation));
                    painter.text(
                        pos2(mid, cell.center().y - gap / 2.0),
                        Align2::CENTER_CENTER,
                        format!("{:+.0}", *amount * 100.0),
                        t.text.value.clone(),
                        c.text.primary,
                    );
                }
                cells.push(hit);
            }
        }
        (response, cells)
    }
}
