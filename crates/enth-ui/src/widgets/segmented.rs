//! A segmented control for a small enum (filter mode, editor mode): one recessed track, the
//! selected segment raised.

use egui::{Align2, IdSalt, Rect, Response, Role, Sense, Ui, Widget, WidgetInfo, pos2, vec2};

use super::paint;
use crate::theme::Theme;

pub struct Segmented<'a> {
    theme: &'a Theme,
    salt: IdSalt,
    selected: &'a mut usize,
    options: &'a [&'a str],
}

impl<'a> Segmented<'a> {
    pub fn new(
        theme: &'a Theme,
        id_salt: impl egui::AsIdSalt,
        selected: &'a mut usize,
        options: &'a [&'a str],
    ) -> Self {
        Self {
            theme,
            salt: IdSalt::new(id_salt),
            selected,
            options,
        }
    }
}

impl Widget for Segmented<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = self.theme;
        let id = ui.make_persistent_id(self.salt);
        let font = &t.text.body;
        let widths: Vec<f32> = self
            .options
            .iter()
            .map(|o| {
                paint::one_line(ui, o, font, t.color.text.primary, f32::INFINITY)
                    .size()
                    .x
                    + 2.0 * t.space.gap
            })
            .collect();
        let inset = t.space.hairline;
        let total = widths.iter().sum::<f32>() + 2.0 * inset;
        let (rect, mut response) =
            ui.allocate_exact_size(vec2(total, t.size.control_height), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, t.radius.medium, t.color.surface.recessed);
        let mut left = rect.left() + inset;
        for (index, (option, width)) in self.options.iter().zip(&widths).enumerate() {
            let segment = Rect::from_min_max(
                pos2(left, rect.top() + inset),
                pos2(left + width, rect.bottom() - inset),
            );
            left += width;
            let hit = ui.interact(segment, id.with(index), Sense::click());
            let selected = *self.selected == index;
            hit.widget_info(|| {
                WidgetInfo::selected(Role::RadioButton, ui.is_enabled(), selected, *option)
            });
            if hit.clicked() && !selected {
                *self.selected = index;
                response.mark_changed();
            }
            if selected {
                painter.rect_filled(segment, t.radius.small, t.color.surface.raised);
            }
            let ink = if selected || hit.hovered() {
                t.color.text.primary
            } else {
                t.color.text.label
            };
            painter.text(
                segment.center(),
                Align2::CENTER_CENTER,
                *option,
                font.clone(),
                ink,
            );
        }
        response
    }
}
