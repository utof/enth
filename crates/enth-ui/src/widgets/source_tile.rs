//! A live modulation-source tile: name, route-count badge, a small graph of the source's shape
//! with its current phase, and the ⊕ handle you drag onto a control.

use egui::{Align2, IdSalt, Rect, Response, Role, Sense, Ui, pos2, vec2};

use super::display::{Display, Ink, Trace};
use crate::theme::{Theme, glyph};

pub struct SourceTile<'a> {
    theme: &'a Theme,
    salt: IdSalt,
    name: &'a str,
    shape: &'a dyn Fn(f32) -> f32,
    phase: Option<f32>,
    routes: usize,
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
            routes: 0,
        }
    }

    /// Current position in the shape (0..1) while the source runs; `None` when idle.
    #[must_use]
    pub fn phase(mut self, phase: Option<f32>) -> Self {
        self.phase = phase;
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
            if self.phase.is_some() {
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
        // The handle has its own column right of the graph.
        let handle_rect = Rect::from_center_size(
            pos2(inner.right() - side / 2.0, graph.center().y),
            vec2(side, side),
        );
        let handle = ui.interact(handle_rect, ui.make_persistent_id(self.salt), Sense::drag());
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
