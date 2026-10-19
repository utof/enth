//! An on/off parameter (an oscillator's hard sync) in the shape of a knob cell: label above, a
//! switch on the knob's centre line, the state below. The switch reads like the card's enable
//! toggle: an accent thumb on the right when on, a dimmed thumb in an outlined track when off.

use egui::{
    Align2, Id, Rect, Response, Role, Sense, StrokeKind, Ui, Widget, WidgetInfo, pos2, vec2,
};

use super::knob::Knob;
use super::paint;
use crate::theme::Theme;

pub struct Toggle<'a> {
    theme: &'a Theme,
    on: &'a mut bool,
    label: &'a str,
    id: Option<Id>,
    texts: (&'a str, &'a str),
}

impl<'a> Toggle<'a> {
    pub fn new(theme: &'a Theme, on: &'a mut bool, label: &'a str) -> Self {
        Self {
            theme,
            on,
            label,
            id: None,
            texts: ("On", "Off"),
        }
    }

    /// Interact under this global id instead of an automatic one (see [`Knob::id`]).
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// The state text below the switch, on and off. Default: "On" and "Off".
    #[must_use]
    pub fn texts(mut self, on: &'a str, off: &'a str) -> Self {
        self.texts = (on, off);
        self
    }
}

impl Widget for Toggle<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = self.theme;
        let c = &t.color;
        let (auto_id, rect) = ui.allocate_space(Knob::cell_size(t, ui));
        let mut response = ui.interact(rect, self.id.unwrap_or(auto_id), Sense::click());
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let on = *self.on;
        response
            .widget_info(|| WidgetInfo::selected(Role::Switch, ui.is_enabled(), on, self.label));
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let active = response.hovered();
        let painter = ui.painter();
        let label_h = paint::row_height(ui, &t.text.label);
        let center = pos2(
            rect.center().x,
            rect.top() + label_h + t.space.hairline + t.size.knob.diameter / 2.0,
        );
        let track = Rect::from_center_size(center, t.size.cards.switch);
        let side = t.size.card.toggle;
        let inset = (track.height() - side) / 2.0;
        let thumb_x = if on {
            track.right() - inset - side / 2.0
        } else {
            track.left() + inset + side / 2.0
        };
        let thumb = Rect::from_center_size(pos2(thumb_x, center.y), vec2(side, side));
        painter.rect_filled(track, t.radius.medium, c.surface.recessed);
        if on {
            painter.rect_filled(thumb, t.radius.small, c.accent.value);
        } else {
            painter.rect_stroke(
                track,
                t.radius.medium,
                t.stroke.thin(c.line.grid),
                StrokeKind::Inside,
            );
            painter.rect_filled(thumb, t.radius.small, c.text.dimmed);
        }
        let (label_ink, value_ink) = if active {
            (c.text.primary, c.text.primary)
        } else {
            (c.text.label, c.text.primary)
        };
        paint::text_in(
            ui,
            painter,
            rect.center_top(),
            Align2::CENTER_TOP,
            self.label,
            &t.text.label,
            label_ink,
            rect.width(),
        );
        let text = if on { self.texts.0 } else { self.texts.1 };
        paint::text_in(
            ui,
            painter,
            rect.center_bottom(),
            Align2::CENTER_BOTTOM,
            text,
            &t.text.value,
            value_ink,
            rect.width(),
        );
        response
    }
}
