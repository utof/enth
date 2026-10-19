//! A live modulation-source tile: name, route-count badge, a small graph of the source's shape
//! with its current phase, and the ⊕ handle you drag onto a control.

use egui::{Align2, Id, IdSalt, Rect, Response, Role, Sense, Ui, pos2, vec2};

use super::display::{Display, Ink, Trace};
use crate::theme::{Theme, glyph};

pub struct SourceTile<'a> {
    theme: &'a Theme,
    salt: IdSalt,
    name: &'a str,
    shape: &'a dyn Fn(f32) -> f32,
    phase: Option<f32>,
    level: Option<f32>,
    routes: usize,
    id: Option<Id>,
}

impl<'a> SourceTile<'a> {
    pub fn new(
        theme: &'a Theme,
        id_salt: impl egui::AsIdSalt,
        name: &'a str,
        shape: &'a dyn Fn(f32) -> f32,
    ) -> Self {
        Self {
            theme,
            salt: IdSalt::new(id_salt),
            name,
            shape,
            phase: None,
            level: None,
            routes: 0,
            id: None,
        }
    }

    /// Give the ⊕ handle this global id (from the source, never the tile's place) instead of one
    /// salted within the parent `Ui`.
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Current position in the shape (0..1) while the source runs; `None` when idle.
    #[must_use]
    pub fn phase(mut self, phase: Option<f32>) -> Self {
        self.phase = phase;
        self
    }

    /// The source's output now, in the graph's 0..1 (lane ED2: the engine's telemetry publishes outputs, not phases):
    /// a level line across the graph with a dot at its right end. `None` when there is no live value.
    #[must_use]
    pub fn level(mut self, level: Option<f32>) -> Self {
        self.level = level;
        self
    }

    /// How many controls this source modulates.
    #[must_use]
    pub fn routes(mut self, routes: usize) -> Self {
        self.routes = routes;
        self
    }

    /// Returns the handle's response (drag it to modulate).
    pub fn show(self, ui: &mut Ui) -> Response {
        let t = self.theme;
        let c = &t.color;
        let size = vec2(t.size.tile.width, t.size.tile.height);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter();
        let r = t.radius.medium as u8;
        painter.rect_filled(rect, t.radius.medium, c.surface.card);
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(rect.width(), t.stroke.thick)),
            egui::CornerRadius {
                nw: r,
                ne: r,
                sw: 0,
                se: 0,
            },
            c.category.modulator,
        );
        let inner = rect.shrink(t.space.tight);
        let top = inner.top() + t.stroke.thick;
        painter.text(
            pos2(inner.left(), top),
            Align2::LEFT_TOP,
            self.name,
            t.text.label.clone(),
            if self.phase.is_some() || self.level.is_some() {
                c.text.primary
            } else {
                c.text.label
            },
        );
        if self.routes > 0 {
            let radius = t.size.tile.badge / 2.0;
            let center = pos2(inner.right() - radius, top + radius);
            painter.circle_filled(center, radius, c.surface.raised);
            painter.text(
                center,
                Align2::CENTER_CENTER,
                self.routes,
                t.text.label.clone(),
                c.text.primary,
            );
        }
        let graph_top = top + t.size.tile.badge + t.space.hairline;
        let side = t.size.tile.badge;
        let graph = Rect::from_min_max(
            pos2(inner.left(), graph_top),
            pos2(inner.right() - side - t.space.tight, inner.bottom()),
        );
        let mut display =
            Display::new(t, Trace::Curve(self.shape), graph.size()).ink(Ink::Modulation);
        if let Some(phase) = self.phase {
            display = display.marker(phase);
        }
        display.paint(ui, graph);
        if let Some(level) = self.level {
            let area = super::display::trace_area(t, graph);
            let y = area.bottom() - level.clamp(0.0, 1.0) * area.height();
            let painter = ui.painter_at(graph);
            painter.hline(area.x_range(), y, t.stroke.thin(c.accent.modulation_wash));
            painter.circle_filled(
                egui::pos2(area.right() - t.size.display.marker_radius, y),
                t.size.display.marker_radius,
                c.accent.modulation,
            );
        }
        // The handle has its own column right of the graph.
        let handle_rect = Rect::from_center_size(
            pos2(inner.right() - side / 2.0, graph.center().y),
            vec2(side, side),
        );
        let id = self.id.unwrap_or_else(|| ui.make_persistent_id(self.salt));
        let handle = ui.interact(handle_rect, id, Sense::drag());
        handle.widget_info(|| egui::WidgetInfo::labeled(Role::Button, true, self.name));
        let handle_ink = if handle.hovered() || handle.dragged() {
            c.text.primary
        } else {
            c.accent.modulation
        };
        ui.painter().text(
            handle_rect.center(),
            Align2::CENTER_CENTER,
            glyph::HANDLE,
            t.text.body.clone(),
            handle_ink,
        );
        handle
    }
}
