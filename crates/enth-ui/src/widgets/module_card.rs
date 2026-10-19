//! The module card: category stripe, enable toggle, title (with the module type), a display slot
//! and a parameter grid. Two shapes: a generator *strip* (stripe on the left, display beside one
//! row of knobs, fixed height) and a *stacked* lane card (stripe on top, display above the knobs).
//!
//! The header is a drag handle: the caller reorders cards from [`CardResponse::header`]. Give
//! every card an id from its module id, never its position, so a drag survives a reorder.

use egui::{
    Align, Align2, CornerRadius, IdSalt, Layout, Rect, Response, Role, Sense, Ui, UiBuilder, pos2,
    vec2,
};

use super::paint;
use crate::theme::{Category, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardShape {
    /// Generator strip: `size.card.generator_height` tall, display left of the knobs.
    Strip,
    /// Lane card: `size.card.lane_width` wide, display above the knobs, height from content.
    Stacked,
}

pub struct ModuleCard<'a> {
    theme: &'a Theme,
    salt: IdSalt,
    title: &'a str,
    kind: Option<&'a str>,
    category: Category,
    enabled: &'a mut bool,
    shape: CardShape,
    display_height: f32,
    lifted: bool,
}

pub struct CardResponse {
    /// The header row: drag it to reorder.
    pub header: Response,
    /// The whole card.
    pub rect: Rect,
    /// The enable toggle was clicked this frame.
    pub toggled: bool,
}

impl<'a> ModuleCard<'a> {
    pub fn new(
        theme: &'a Theme,
        id_salt: impl egui::AsIdSalt,
        title: &'a str,
        category: Category,
        enabled: &'a mut bool,
    ) -> Self {
        Self {
            theme,
            salt: IdSalt::new(id_salt),
            title,
            kind: None,
            category,
            enabled,
            shape: CardShape::Strip,
            display_height: 0.0,
            lifted: false,
        }
    }

    /// The module type shown after the title ("Saw", "Low-pass 12").
    #[must_use]
    pub fn kind(mut self, kind: &'a str) -> Self {
        self.kind = Some(kind);
        self
    }

    /// A lane card with a display of this height (0 for none).
    #[must_use]
    pub fn stacked(mut self, display_height: f32) -> Self {
        self.shape = CardShape::Stacked;
        self.display_height = display_height;
        self
    }

    /// Drawn raised while it is being dragged.
    #[must_use]
    pub fn lifted(mut self, lifted: bool) -> Self {
        self.lifted = lifted;
        self
    }

    /// Lays the card out. `display` paints into the display slot; `params` adds the knobs.
    pub fn show(
        self,
        ui: &mut Ui,
        display: impl FnOnce(&mut Ui, Rect),
        params: impl FnOnce(&mut Ui),
    ) -> CardResponse {
        match self.shape {
            CardShape::Strip => self.show_strip(ui, display, params),
            CardShape::Stacked => self.show_stacked(ui, display, params),
        }
    }

    fn fill(&self) -> egui::Color32 {
        if self.lifted {
            self.theme.color.surface.raised
        } else {
            self.theme.color.surface.card
        }
    }

    fn show_strip(
        mut self,
        ui: &mut Ui,
        display: impl FnOnce(&mut Ui, Rect),
        params: impl FnOnce(&mut Ui),
    ) -> CardResponse {
        let t = self.theme;
        let height = t.size.card.generator_height;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let stripe = t.stroke.thick;
        let r = t.radius.medium as u8;
        ui.painter().rect_filled(rect, t.radius.medium, self.fill());
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(stripe, height)),
            CornerRadius {
                nw: r,
                sw: r,
                ne: 0,
                se: 0,
            },
            t.color.category.of(self.category),
        );
        let inner = Rect::from_min_max(
            pos2(rect.left() + stripe + t.space.gap, rect.top()),
            pos2(rect.right() - t.space.gap, rect.bottom() - t.space.tight),
        );
        let header_rect =
            Rect::from_min_size(inner.min, vec2(inner.width(), t.size.card.collapsed_height));
        let (header, toggled) = self.header(ui, header_rect);
        let body = Rect::from_min_max(pos2(inner.left(), header_rect.bottom()), inner.max);
        let display_rect =
            Rect::from_min_size(body.min, vec2(t.size.card.display_width, body.height()));
        display(ui, display_rect);
        let knobs = Rect::from_min_max(
            pos2(display_rect.right() + t.space.pad, body.top()),
            body.max,
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(knobs)
                .layout(Layout::left_to_right(Align::Min)),
        );
        child.spacing_mut().item_spacing = vec2(t.space.tight, t.space.tight);
        params(&mut child);
        CardResponse {
            header,
            rect,
            toggled,
        }
    }

    fn show_stacked(
        mut self,
        ui: &mut Ui,
        display: impl FnOnce(&mut Ui, Rect),
        params: impl FnOnce(&mut Ui),
    ) -> CardResponse {
        let t = self.theme;
        let width = t.size.card.lane_width;
        let stripe = t.stroke.thick;
        let fill = self.fill();
        let category = t.color.category.of(self.category);
        let top = ui.cursor().min;
        // Background goes in first so it sits under the content painted below.
        let background = ui.painter().add(egui::Shape::Noop);
        let inner_left = top.x + t.space.gap;
        let inner_width = width - 2.0 * t.space.gap;
        let header_rect = Rect::from_min_size(
            pos2(inner_left, top.y + stripe),
            vec2(inner_width, t.size.card.collapsed_height),
        );
        let (header, toggled) = self.header(ui, header_rect);
        let mut y = header_rect.bottom();
        if self.display_height > 0.0 {
            let display_rect =
                Rect::from_min_size(pos2(inner_left, y), vec2(inner_width, self.display_height));
            display(ui, display_rect);
            y = display_rect.bottom() + t.space.gap;
        }
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(inner_left, y),
                    vec2(inner_width, f32::INFINITY),
                ))
                .layout(Layout::left_to_right(Align::Min).with_main_wrap(true)),
        );
        child.spacing_mut().item_spacing = vec2(t.space.tight, t.space.tight);
        params(&mut child);
        let bottom = child.min_rect().bottom().max(y) + t.space.gap;
        let rect = Rect::from_min_max(top, pos2(top.x + width, bottom));
        ui.allocate_rect(rect, Sense::hover());
        let r = t.radius.medium as u8;
        ui.painter().set(
            background,
            egui::Shape::Vec(vec![
                egui::epaint::RectShape::filled(rect, t.radius.medium, fill).into(),
                egui::epaint::RectShape::filled(
                    Rect::from_min_size(rect.min, vec2(width, stripe)),
                    CornerRadius {
                        nw: r,
                        ne: r,
                        sw: 0,
                        se: 0,
                    },
                    category,
                )
                .into(),
            ]),
        );
        CardResponse {
            header,
            rect,
            toggled,
        }
    }

    /// Toggle square, title, module type; returns the header (drag) response.
    fn header(&mut self, ui: &mut Ui, rect: Rect) -> (Response, bool) {
        let t = self.theme;
        let c = &t.color;
        let side = t.size.card.toggle;
        let toggle_rect = Rect::from_center_size(
            pos2(rect.left() + side / 2.0, rect.center().y),
            vec2(side, side),
        );
        let id = ui.make_persistent_id(self.salt);
        let header = ui.interact(rect, id.with("header"), Sense::click_and_drag());
        header.widget_info(|| {
            egui::WidgetInfo::labeled(Role::Button, true, format!("Move {}", self.title))
        });
        let toggle = ui.interact(toggle_rect, id.with("enable"), Sense::click());
        let toggled = toggle.clicked();
        if toggled {
            *self.enabled = !*self.enabled;
        }
        let enabled = *self.enabled;
        toggle
            .widget_info(|| egui::WidgetInfo::selected(Role::CheckBox, true, enabled, self.title));
        let painter = ui.painter();
        if enabled {
            painter.rect_filled(toggle_rect, t.radius.small, c.accent.value);
        } else {
            painter.rect_filled(toggle_rect, t.radius.small, c.surface.recessed);
            painter.rect_stroke(
                toggle_rect,
                t.radius.small,
                t.stroke.thin(c.line.grid),
                egui::StrokeKind::Inside,
            );
        }
        let title_ink = if enabled {
            c.text.primary
        } else {
            c.text.dimmed
        };
        let title_left = toggle_rect.right() + t.space.gap;
        let title = paint::text_in(
            ui,
            painter,
            pos2(title_left, rect.center().y),
            Align2::LEFT_CENTER,
            self.title,
            &t.text.title,
            title_ink,
            rect.right() - title_left,
        );
        if let Some(kind) = self.kind {
            let left = title.right() + t.space.gap;
            paint::text_in(
                ui,
                painter,
                pos2(left, rect.center().y),
                Align2::LEFT_CENTER,
                kind,
                &t.text.label,
                c.text.label,
                (rect.right() - left).max(0.0),
            );
        }
        (header, toggled)
    }
}
