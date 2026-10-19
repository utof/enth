//! A segmented control for a small enum (filter mode, editor mode): one recessed track, the
//! selected segment raised.
//!
//! With a [`label`](Segmented::label) it takes the shape of a knob cell (label above, the track on
//! the knob's centre line), so it lines up with knobs and toggles in a row of cells; with
//! [`inline`](Segmented::inline) too, it is one control-height row: the label on the left, the
//! track at the right end (a lane card's enum rows).

use egui::{Align2, Id, IdSalt, Rect, Response, Role, Sense, Ui, Widget, WidgetInfo, pos2, vec2};

use super::knob::Knob;
use super::paint;
use crate::theme::Theme;

pub struct Segmented<'a> {
    theme: &'a Theme,
    salt: IdSalt,
    id: Option<Id>,
    selected: &'a mut usize,
    options: &'a [&'a str],
    label: Option<&'a str>,
    inline_width: Option<f32>,
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
            id: None,
            selected,
            options,
            label: None,
            inline_width: None,
        }
    }

    /// Interact under this global id (segments under `id.with(index)`) instead of one salted
    /// within the parent `Ui`, so the control keeps its state wherever the layout puts it.
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// A knob-cell shape with this label above the track; also the control's accessible name.
    #[must_use]
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// One row `width` wide and a control high: the label (if any) on the left, the track at the
    /// right end.
    #[must_use]
    pub fn inline(mut self, width: f32) -> Self {
        self.inline_width = Some(width);
        self
    }

    fn segment_widths(&self, ui: &Ui) -> Vec<f32> {
        let t = self.theme;
        self.options
            .iter()
            .map(|o| {
                paint::one_line(ui, o, &t.text.body, t.color.text.primary, f32::INFINITY)
                    .size()
                    .x
                    + 2.0 * t.space.gap
            })
            .collect()
    }
}

impl Widget for Segmented<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = self.theme;
        let id = self.id.unwrap_or_else(|| ui.make_persistent_id(self.salt));
        let font = &t.text.body;
        let widths = self.segment_widths(ui);
        let inset = t.space.hairline;
        let track_width = widths.iter().sum::<f32>() + 2.0 * inset;
        let (rect, track) = match (self.label, self.inline_width) {
            (_, Some(width)) => {
                let size = vec2(width.max(track_width), t.size.control_height);
                let rect = ui.allocate_space(size).1;
                let track =
                    Rect::from_min_max(pos2(rect.right() - track_width, rect.top()), rect.max);
                (rect, track)
            }
            (None, None) => {
                let size = vec2(track_width, t.size.control_height);
                let rect = ui.allocate_space(size).1;
                (rect, rect)
            }
            (Some(label), None) => {
                // A knob cell's height, and wide enough for the label too.
                let cell = Knob::cell_size(t, ui);
                let label_width =
                    paint::one_line(ui, label, &t.text.label, t.color.text.label, f32::INFINITY)
                        .size()
                        .x;
                let width = track_width.max(label_width).max(cell.x);
                let rect = ui.allocate_space(vec2(width, cell.y)).1;
                let label_h = paint::row_height(ui, &t.text.label);
                let center_y = rect.top() + label_h + t.space.hairline + t.size.knob.diameter / 2.0;
                let track = Rect::from_center_size(
                    pos2(rect.center().x, center_y),
                    vec2(track_width, t.size.control_height),
                );
                (rect, track)
            }
        };
        let mut response = ui.interact(rect, id, Sense::hover());
        let name = self.label.unwrap_or_default();
        let selected_name = self
            .options
            .get(*self.selected)
            .copied()
            .unwrap_or_default();
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(Role::RadioGroup, ui.is_enabled(), name);
            info.current_text_value = Some(selected_name.to_owned());
            info
        });
        let painter = ui.painter();
        let label_ink = if ui.rect_contains_pointer(rect) {
            t.color.text.primary
        } else {
            t.color.text.label
        };
        match (self.label, self.inline_width) {
            (Some(label), Some(_)) => {
                paint::text_in(
                    ui,
                    painter,
                    rect.left_center(),
                    Align2::LEFT_CENTER,
                    label,
                    &t.text.label,
                    label_ink,
                    (track.left() - rect.left() - t.space.gap).max(0.0),
                );
            }
            (Some(label), None) => {
                paint::text_in(
                    ui,
                    painter,
                    rect.center_top(),
                    Align2::CENTER_TOP,
                    label,
                    &t.text.label,
                    label_ink,
                    rect.width(),
                );
            }
            (None, _) => {}
        }
        painter.rect_filled(track, t.radius.medium, t.color.surface.recessed);
        let mut left = track.left() + inset;
        for (index, (option, width)) in self.options.iter().zip(&widths).enumerate() {
            let segment = Rect::from_min_max(
                pos2(left, track.top() + inset),
                pos2(left + width, track.bottom() - inset),
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
            if hit.hovered() {
                // The group reads as hovered while a segment is (the status line names the control).
                response = response.union(hit);
            }
        }
        response
    }
}
