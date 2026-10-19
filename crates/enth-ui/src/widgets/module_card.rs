//! The module card: category stripe, enable toggle, title (with the module type), a display slot
//! and a parameter grid. Two shapes: a generator *strip* (stripe on the left, display beside one
//! row of knobs, fixed height) and a *stacked* lane card (stripe on top, display above the knobs).
//!
//! The header is a drag handle: the caller reorders cards from [`CardResponse::header`]. Give
//! every card an id from its module id, never its position, so a drag survives a reorder.
//!
//! A disabled card fades as a whole: stripe, title, display and every control inside it are
//! drawn at `state.disabled_opacity` through one child `Ui`, so widgets need no disabled look of
//! their own. The fill and the enable toggle stay at full strength, and the controls stay
//! editable (a module can be set up before it is switched on).
//!
//! Built-in sections (voice, master) can't be switched off: [`ModuleCard::switchable`] hides the
//! toggle. A header [`action`](ModuleCard::action) is a small text button at the right of the
//! title row (the catalogue cards' "More" disclosure). A strip can hold more than one row of
//! knobs ([`ModuleCard::rows`]); the parameter closure starts each new row with `ui.end_row()`.

use egui::{
    Align, Align2, CornerRadius, Id, IdSalt, Layout, Rect, Response, Role, Sense, Ui, UiBuilder,
    pos2, vec2,
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
    id: Option<Id>,
    title: &'a str,
    kind: Option<&'a str>,
    category: Category,
    enabled: &'a mut bool,
    shape: CardShape,
    display_height: f32,
    lifted: bool,
    switchable: bool,
    action: Option<&'a str>,
    rows: usize,
}

pub struct CardResponse {
    /// The header row: drag it to reorder.
    pub header: Response,
    /// The whole card.
    pub rect: Rect,
    /// The enable toggle was clicked this frame.
    pub toggled: bool,
    /// The header action, if the card has one ([`ModuleCard::action`]).
    pub action: Option<Response>,
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
            id: None,
            title,
            kind: None,
            category,
            enabled,
            shape: CardShape::Strip,
            display_height: 0.0,
            lifted: false,
            switchable: true,
            action: None,
            rows: 1,
        }
    }

    /// Interact under this global id (header under `id.with("header")`, toggle under
    /// `id.with("enable")`) instead of one salted within the parent `Ui`. Use the module id.
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Whether the card has an enable toggle (default). Built-ins that can't be bypassed don't;
    /// their title starts where the toggle would be.
    #[must_use]
    pub fn switchable(mut self, switchable: bool) -> Self {
        self.switchable = switchable;
        self
    }

    /// A small text button at the right end of the header row ("More", "Less"); its response is
    /// in [`CardResponse::action`]. Drawn at full strength, like the toggle.
    #[must_use]
    pub fn action(mut self, text: &'a str) -> Self {
        self.action = Some(text);
        self
    }

    /// A strip with this many rows of knob cells (default 1); each extra row adds a cell's height.
    #[must_use]
    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows.max(1);
        self
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
        let extra_rows = (self.rows - 1) as f32;
        let height = t.size.card.generator_height
            + extra_rows * (super::Knob::cell_size(t, ui).y + t.space.tight);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let stripe = t.stroke.thick;
        let r = t.radius.medium as u8;
        ui.painter().rect_filled(rect, t.radius.medium, self.fill());
        let inner = Rect::from_min_max(
            pos2(rect.left() + stripe + t.space.gap, rect.top()),
            pos2(rect.right() - t.space.gap, rect.bottom() - t.space.tight),
        );
        let header_rect =
            Rect::from_min_size(inner.min, vec2(inner.width(), t.size.card.collapsed_height));
        let (header, toggled) = self.controls(ui, header_rect);
        let mut content = self.content(ui, rect);
        content.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(stripe, height)),
            CornerRadius {
                nw: r,
                sw: r,
                ne: 0,
                se: 0,
            },
            t.color.category.of(self.category),
        );
        self.title(&content, header_rect);
        let body = Rect::from_min_max(pos2(inner.left(), header_rect.bottom()), inner.max);
        let display_rect =
            Rect::from_min_size(body.min, vec2(t.size.card.display_width, body.height()));
        display(&mut content, display_rect);
        let knobs = Rect::from_min_max(
            pos2(display_rect.right() + t.space.pad, body.top()),
            body.max,
        );
        let mut child = content.new_child(
            UiBuilder::new()
                .max_rect(knobs)
                .layout(Layout::left_to_right(Align::Min).with_main_wrap(self.rows > 1)),
        );
        child.spacing_mut().item_spacing = vec2(t.space.tight, t.space.tight);
        params(&mut child);
        let action = self.action_button(ui, header_rect);
        CardResponse {
            header,
            rect,
            toggled,
            action,
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
        // Background and stripe go in first so they sit under the content painted below; their
        // height is known only once the knobs are laid out.
        let background = ui.painter().add(egui::Shape::Noop);
        let inner_left = top.x + t.space.gap;
        let inner_width = width - 2.0 * t.space.gap;
        let header_rect = Rect::from_min_size(
            pos2(inner_left, top.y + stripe),
            vec2(inner_width, t.size.card.collapsed_height),
        );
        let (header, toggled) = self.controls(ui, header_rect);
        let mut content = self.content(ui, Rect::from_min_size(top, vec2(width, f32::INFINITY)));
        let stripe_shape = content.painter().add(egui::Shape::Noop);
        self.title(&content, header_rect);
        let mut y = header_rect.bottom();
        if self.display_height > 0.0 {
            let display_rect =
                Rect::from_min_size(pos2(inner_left, y), vec2(inner_width, self.display_height));
            display(&mut content, display_rect);
            y = display_rect.bottom() + t.space.gap;
        }
        let mut child = content.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(inner_left, y),
                    vec2(inner_width, f32::INFINITY),
                ))
                .layout(Layout::left_to_right(Align::Min).with_main_wrap(true)),
        );
        // Spread whole knob cells across the card's width.
        let cell = t.size.knob.cell_width;
        let per_row = ((inner_width + t.space.tight) / (cell + t.space.tight))
            .floor()
            .max(1.0);
        let spread = if per_row > 1.0 {
            (inner_width - per_row * cell) / (per_row - 1.0)
        } else {
            0.0
        };
        child.spacing_mut().item_spacing = vec2(spread, t.space.tight);
        params(&mut child);
        let bottom = child.min_rect().bottom().max(y) + t.space.gap;
        let rect = Rect::from_min_max(top, pos2(top.x + width, bottom));
        ui.allocate_rect(rect, Sense::hover());
        let r = t.radius.medium as u8;
        ui.painter().set(
            background,
            egui::epaint::RectShape::filled(rect, t.radius.medium, fill),
        );
        content.painter().set(
            stripe_shape,
            egui::epaint::RectShape::filled(
                Rect::from_min_size(rect.min, vec2(width, stripe)),
                CornerRadius {
                    nw: r,
                    ne: r,
                    sw: 0,
                    se: 0,
                },
                category,
            ),
        );
        let action = self.action_button(ui, header_rect);
        CardResponse {
            header,
            rect,
            toggled,
            action,
        }
    }

    /// The header action: right-aligned label text, primary while hovered.
    fn action_button(&self, ui: &mut Ui, header: Rect) -> Option<Response> {
        let text = self.action?;
        let t = self.theme;
        let galley = paint::one_line(ui, text, &t.text.label, t.color.text.label, header.width());
        let size = vec2(galley.size().x + 2.0 * t.space.tight, header.height());
        let rect = Rect::from_min_size(pos2(header.right() - size.x, header.top()), size);
        let response = ui.interact(rect, self.base_id(ui).with("action"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(Role::Button, true, text));
        let ink = if response.hovered() {
            t.color.text.primary
        } else {
            t.color.text.label
        };
        ui.painter().text(
            pos2(rect.right() - t.space.tight, rect.center().y),
            Align2::RIGHT_CENTER,
            text,
            t.text.label.clone(),
            ink,
        );
        Some(response)
    }

    fn base_id(&self, ui: &Ui) -> Id {
        self.id.unwrap_or_else(|| ui.make_persistent_id(self.salt))
    }

    /// A child `Ui` over `rect` for everything but the fill and the toggle. A disabled card fades
    /// it as a whole; nothing inside needs a disabled look of its own.
    fn content(&self, ui: &mut Ui, rect: Rect) -> Ui {
        let mut content = ui.new_child(UiBuilder::new().max_rect(rect));
        if !*self.enabled {
            content.multiply_opacity(self.theme.state.disabled_opacity);
        }
        content
    }

    /// The header's drag handle and the enable toggle (painted at full strength, since it is how
    /// the card comes back). Returns the header response and whether the toggle was clicked.
    fn controls(&mut self, ui: &mut Ui, rect: Rect) -> (Response, bool) {
        let t = self.theme;
        let c = &t.color;
        let id = self.base_id(ui);
        let header = ui.interact(rect, id.with("header"), Sense::click_and_drag());
        header.widget_info(|| {
            egui::WidgetInfo::labeled(Role::Button, true, format!("Move {}", self.title))
        });
        if !self.switchable {
            return (header, false);
        }
        let toggle_rect = self.toggle_rect(rect);
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
        (header, toggled)
    }

    fn toggle_rect(&self, header: Rect) -> Rect {
        let side = self.theme.size.card.toggle;
        Rect::from_center_size(
            pos2(header.left() + side / 2.0, header.center().y),
            vec2(side, side),
        )
    }

    /// Title and module type, right of the toggle, in the card's content `Ui`.
    fn title(&self, content: &Ui, header: Rect) {
        let t = self.theme;
        let c = &t.color;
        let painter = content.painter();
        // Leave the action's place free.
        let right = match self.action {
            Some(text) => {
                let w = paint::one_line(content, text, &t.text.label, c.text.label, header.width())
                    .size()
                    .x;
                header.right() - w - 2.0 * t.space.tight - t.space.gap
            }
            None => header.right(),
        };
        let header = Rect::from_min_max(header.min, pos2(right.max(header.left()), header.max.y));
        let title_left = if self.switchable {
            self.toggle_rect(header).right() + t.space.gap
        } else {
            header.left()
        };
        let title = paint::text_in(
            content,
            painter,
            pos2(title_left, header.center().y),
            Align2::LEFT_CENTER,
            self.title,
            &t.text.title,
            c.text.primary,
            header.right() - title_left,
        );
        if let Some(kind) = self.kind {
            let left = title.right() + t.space.gap;
            paint::text_in(
                content,
                painter,
                pos2(left, header.center().y),
                Align2::LEFT_CENTER,
                kind,
                &t.text.label,
                c.text.label,
                (header.right() - left).max(0.0),
            );
        }
    }
}
