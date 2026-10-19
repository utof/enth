//! A rotary control: track, value arc, modulation arc, optional score ring with its glyph.
//!
//! Values are normalised (0..=1); the caller maps them to real units and formats the value text.
//! Drag vertically (Shift for fine), scroll, or double-click to reset.

use std::f32::consts::PI;

use egui::{Align2, Pos2, Rect, Response, Sense, Ui, Vec2, Widget, WidgetInfo, pos2, vec2};

use super::paint;
use crate::theme::{Score, Theme};

/// How strongly a control is shown. A lesson step or challenge dims what it doesn't involve; the
/// control stays usable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Emphasis {
    #[default]
    Normal,
    Dimmed,
}

/// Where the label and value sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KnobLayout {
    /// Label above, value below: parameter grids.
    #[default]
    Cell,
    /// Label and value to the right: the macro bar.
    Inline,
}

pub struct Knob<'a> {
    theme: &'a Theme,
    value: &'a mut f32,
    label: &'a str,
    value_text: Option<String>,
    default: Option<f32>,
    modulated: Option<f32>,
    score: Option<Score>,
    bipolar: bool,
    emphasis: Emphasis,
    layout: KnobLayout,
    inline_width: Option<f32>,
}

impl<'a> Knob<'a> {
    pub fn new(theme: &'a Theme, value: &'a mut f32, label: &'a str) -> Self {
        Self {
            theme,
            value,
            label,
            value_text: None,
            default: None,
            modulated: None,
            score: None,
            bipolar: false,
            emphasis: Emphasis::Normal,
            layout: KnobLayout::Cell,
            inline_width: None,
        }
    }

    /// The value in real units, formatted by the caller ("1.20 kHz"). Default: a percentage.
    #[must_use]
    pub fn value_text(mut self, text: impl Into<String>) -> Self {
        self.value_text = Some(text.into());
        self
    }

    /// Double-click resets to this (normalised) value.
    #[must_use]
    pub fn default_value(mut self, value: f32) -> Self {
        self.default = Some(value);
        self
    }

    /// The current modulated value (normalised), drawn as an arc from the set value.
    #[must_use]
    pub fn modulated(mut self, value: f32) -> Self {
        self.modulated = Some(value);
        self
    }

    #[must_use]
    pub fn score(mut self, score: Option<Score>) -> Self {
        self.score = score;
        self
    }

    /// The value arc starts at the top (centre) instead of the minimum.
    #[must_use]
    pub fn bipolar(mut self, bipolar: bool) -> Self {
        self.bipolar = bipolar;
        self
    }

    #[must_use]
    pub fn emphasis(mut self, emphasis: Emphasis) -> Self {
        self.emphasis = emphasis;
        self
    }

    /// Label and value to the right of the knob, in a cell `width` wide.
    #[must_use]
    pub fn inline(mut self, width: f32) -> Self {
        self.layout = KnobLayout::Inline;
        self.inline_width = Some(width);
        self
    }

    /// Size of a [`KnobLayout::Cell`] knob for `theme`.
    pub fn cell_size(theme: &Theme, ui: &Ui) -> Vec2 {
        let label = paint::row_height(ui, &theme.text.label);
        let value = paint::row_height(ui, &theme.text.value);
        vec2(
            theme.size.knob.cell_width,
            label + value + theme.size.knob.diameter + 2.0 * theme.space.hairline,
        )
    }

    /// Angle (radians, clockwise from +x) of a normalised value.
    fn angle(&self, value: f32) -> f32 {
        let sweep = self.theme.size.knob.sweep_degrees.to_radians();
        // The gap is centred at the bottom (90°), so the sweep starts half a gap past it.
        let start = PI / 2.0 + (2.0 * PI - sweep) / 2.0;
        start + sweep * value.clamp(0.0, 1.0)
    }

    fn interact(&self, ui: &Ui, response: &Response) -> f32 {
        let k = &self.theme.size.knob;
        let fine = if ui.input(|i| i.modifiers.shift) {
            k.fine_ratio
        } else {
            1.0
        };
        let mut value = *self.value;
        if response.dragged() {
            value -= response.drag_delta().y / k.drag_travel * fine;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                value += scroll / k.drag_travel * fine;
                // Consume it, so a scroll area around the knob doesn't scroll as well.
                ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
            }
        }
        if response.double_clicked()
            && let Some(default) = self.default
        {
            value = default;
        }
        value.clamp(0.0, 1.0)
    }

    fn paint_dial(&self, ui: &Ui, center: Pos2, active: bool) {
        let t = self.theme;
        let c = &t.color;
        let k = &t.size.knob;
        let painter = ui.painter();
        let dimmed = self.emphasis == Emphasis::Dimmed;
        let value_ink = if dimmed {
            c.text.dimmed
        } else {
            c.accent.value
        };
        let r_track = k.diameter / 2.0 - k.track / 2.0;
        let (lo, hi) = (self.angle(0.0), self.angle(1.0));
        let at = self.angle(*self.value);
        paint::arc(
            painter,
            center,
            r_track,
            lo,
            hi,
            t.stroke.thick(c.surface.recessed),
        );
        let origin = if self.bipolar { self.angle(0.5) } else { lo };
        paint::arc(
            painter,
            center,
            r_track,
            origin,
            at,
            t.stroke.thick(value_ink),
        );
        // Modulation sits on its own ring inside the track, so it never hides the value arc.
        let r_mod = r_track - k.track - t.space.hairline;
        if let Some(modulated) = self.modulated
            && !dimmed
        {
            let to = self.angle(modulated);
            paint::arc(
                painter,
                center,
                r_mod,
                at,
                to,
                t.stroke.thick(c.accent.modulation),
            );
        }
        let pointer = if active || !dimmed {
            c.text.primary
        } else {
            c.text.dimmed
        };
        let direction = vec2(at.cos(), at.sin());
        painter.line_segment(
            [
                center + direction * t.space.tight,
                center + direction * (r_track - k.track / 2.0),
            ],
            t.stroke.thin(pointer),
        );
        // Score rings go outside the track (docs/design-system.md), always with the glyph.
        if let Some(score) = self.score {
            let r_score = k.diameter / 2.0 + t.space.hairline + t.stroke.thin / 2.0;
            painter.circle_stroke(center, r_score, t.stroke.thin(c.score.of(score)));
        }
    }

    fn paint_score_glyph(&self, ui: &Ui, anchor: Pos2) {
        if let Some(score) = self.score {
            let t = self.theme;
            ui.painter().text(
                anchor,
                Align2::RIGHT_TOP,
                score.glyph(),
                t.text.label.clone(),
                t.color.score.of(score),
            );
        }
    }

    fn texts(&self, active: bool) -> (String, egui::Color32, egui::Color32) {
        let c = &self.theme.color;
        let value_text = self
            .value_text
            .clone()
            .unwrap_or_else(|| format!("{:.0} %", *self.value * 100.0));
        let (label, value) = match (self.emphasis, active) {
            (Emphasis::Dimmed, false) => (c.text.dimmed, c.text.dimmed),
            (_, true) => (c.text.primary, c.text.primary),
            (Emphasis::Normal, false) => (c.text.label, c.text.primary),
        };
        (value_text, label, value)
    }

    fn paint_cell(&self, ui: &Ui, rect: Rect, active: bool) {
        let t = self.theme;
        let label_h = paint::row_height(ui, &t.text.label);
        let center = pos2(
            rect.center().x,
            rect.top() + label_h + t.space.hairline + t.size.knob.diameter / 2.0,
        );
        let (value_text, label_ink, value_ink) = self.texts(active);
        let painter = ui.painter();
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
        self.paint_dial(ui, center, active);
        paint::text_in(
            ui,
            painter,
            rect.center_bottom(),
            Align2::CENTER_BOTTOM,
            &value_text,
            &t.text.value,
            value_ink,
            rect.width(),
        );
        self.paint_score_glyph(ui, rect.right_top());
    }

    fn paint_inline(&self, ui: &Ui, rect: Rect, active: bool) {
        let t = self.theme;
        let d = t.size.knob.diameter;
        let center = pos2(rect.left() + d / 2.0, rect.center().y);
        let (value_text, label_ink, value_ink) = self.texts(active);
        let text_left = rect.left() + d + t.space.gap;
        let text_width = rect.right() - text_left;
        let painter = ui.painter();
        self.paint_dial(ui, center, active);
        paint::text_in(
            ui,
            painter,
            pos2(text_left, rect.center().y),
            Align2::LEFT_BOTTOM,
            self.label,
            &t.text.label,
            label_ink,
            text_width,
        );
        paint::text_in(
            ui,
            painter,
            pos2(text_left, rect.center().y),
            Align2::LEFT_TOP,
            &value_text,
            &t.text.value,
            value_ink,
            text_width,
        );
        self.paint_score_glyph(ui, pos2(rect.left() + d, rect.top()));
    }
}

impl Widget for Knob<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = self.theme;
        let size = match self.layout {
            KnobLayout::Cell => Self::cell_size(t, ui),
            KnobLayout::Inline => vec2(
                self.inline_width.unwrap_or(t.size.knob.cell_width),
                t.size.knob.diameter,
            ),
        };
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let value = self.interact(ui, &response);
        if value != *self.value {
            *self.value = value;
            response.mark_changed();
        }
        response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), f64::from(value), self.label));
        if ui.is_rect_visible(rect) {
            let active = response.hovered() || response.dragged();
            match self.layout {
                KnobLayout::Cell => self.paint_cell(ui, rect, active),
                KnobLayout::Inline => self.paint_inline(ui, rect, active),
            }
        }
        response
    }
}
