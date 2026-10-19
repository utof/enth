//! A rotary control: track, value arc, modulation arc, optional score ring with its glyph.
//!
//! Values are normalised (0..=1); the caller maps them to real units and formats the value text.
//! Drag vertically (Shift for fine), scroll, or double-click to reset.
//!
//! Give a knob an [`id`](Knob::id) from what it controls (the editor uses the module id and the parameter
//! address), never its position: a drag then survives a re-layout. A discrete knob ([`steps`](Knob::steps))
//! snaps to its steps but keeps the unrounded position of a running drag or wheel burst in egui's temporary
//! memory under its id, so a slow drag still moves it.

use std::f32::consts::PI;

use egui::{Align2, Id, Pos2, Rect, Response, Sense, Ui, Vec2, Widget, WidgetInfo, pos2, vec2};

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
    range: Option<(f32, f32)>,
    score: Option<Score>,
    bipolar: bool,
    emphasis: Emphasis,
    layout: KnobLayout,
    inline_width: Option<f32>,
    id: Option<Id>,
    steps: Option<u32>,
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
            range: None,
            score: None,
            bipolar: false,
            emphasis: Emphasis::Normal,
            layout: KnobLayout::Cell,
            inline_width: None,
            id: None,
            steps: None,
        }
    }

    /// Interact under this id instead of an automatic one. Use a global id derived from what the knob controls
    /// (`Id::unique(("enth.param", addr))`), so its drag, hover and wheel state survive the knob moving to
    /// another place in the layout.
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// A discrete knob with `steps + 1` values (an integer parameter, a long enum): the value snaps to
    /// `k / steps`. Sub-step motion of a drag or a wheel burst accumulates until it reaches the next step, so
    /// a slow drag moves it. Zero steps leaves the knob continuous.
    #[must_use]
    pub fn steps(mut self, steps: u32) -> Self {
        self.steps = (steps > 0).then_some(steps);
        self
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

    /// The span its modulation routes can reach (normalised `lo..=hi`, lane ED2): a thin orange arc on the modulation
    /// ring, under the thick live arc of [`modulated`](Self::modulated).
    #[must_use]
    pub fn range(mut self, lo: f32, hi: f32) -> Self {
        self.range = Some((lo, hi));
        self
    }

    /// Where the point at normalised `value` sits on the modulation ring of a knob drawn in `rect` (`inline`: the
    /// [`inline`](Self::inline) form), so a caller can put something on the ring: the editor's depth handle (lane ED2).
    pub fn ring_point(theme: &Theme, ui: &Ui, rect: Rect, inline: bool, value: f32) -> Pos2 {
        let k = &theme.size.knob;
        let center = if inline {
            pos2(rect.left() + k.diameter / 2.0, rect.center().y)
        } else {
            let label_h = paint::row_height(ui, &theme.text.label);
            pos2(
                rect.center().x,
                rect.top() + label_h + theme.space.hairline + k.diameter / 2.0,
            )
        };
        let angle = angle_of(theme, value);
        center + vec2(angle.cos(), angle.sin()) * mod_radius(theme)
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
        angle_of(self.theme, value)
    }

    fn interact(&self, ui: &Ui, response: &Response) -> f32 {
        let k = &self.theme.size.knob;
        let fine = if ui.input(|i| i.modifiers.shift) {
            k.fine_ratio
        } else {
            1.0
        };
        let mut travel = 0.0;
        if response.dragged() {
            travel -= response.drag_delta().y;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                travel += scroll;
                // Consume it, so a scroll area around the knob doesn't scroll as well.
                ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
            }
        }
        let id = response.id;
        let mut value = *self.value;
        if travel != 0.0 {
            // A discrete knob moves from where its drag or wheel burst has got to, not from the rounded value.
            let from = match self.steps {
                Some(_) => ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(value),
                None => value,
            };
            value = (from + travel / k.drag_travel * fine).clamp(0.0, 1.0);
            if self.steps.is_some() {
                ui.data_mut(|d| d.insert_temp(id, value));
            }
        }
        let reset = response.double_clicked() && self.default.is_some();
        if let Some(default) = self.default.filter(|_| reset) {
            value = default;
        }
        let ended = response.drag_stopped() || !(response.dragged() || response.hovered());
        if self.steps.is_some() && (ended || reset) {
            ui.data_mut(|d| d.remove::<f32>(id));
        }
        match self.steps {
            // Snap only what moved: the caller's value is already on a step (up to its own rounding).
            Some(steps) if travel != 0.0 => {
                let steps = steps as f32;
                (value * steps).round() / steps
            }
            _ => value,
        }
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
        let r_mod = mod_radius(t);
        if let Some((lo, hi)) = self.range
            && !dimmed
        {
            paint::arc(
                painter,
                center,
                r_mod,
                self.angle(lo),
                self.angle(hi),
                t.stroke.thin(c.accent.modulation),
            );
        }
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

/// Angle (radians, clockwise from +x) of a normalised value on a knob's sweep.
fn angle_of(theme: &Theme, value: f32) -> f32 {
    let sweep = theme.size.knob.sweep_degrees.to_radians();
    // The gap is centred at the bottom (90°), so the sweep starts half a gap past it.
    let start = PI / 2.0 + (2.0 * PI - sweep) / 2.0;
    start + sweep * value.clamp(0.0, 1.0)
}

/// Radius of the modulation ring, inside the track.
fn mod_radius(theme: &Theme) -> f32 {
    let k = &theme.size.knob;
    let r_track = k.diameter / 2.0 - k.track / 2.0;
    r_track - k.track - theme.space.hairline
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
        let (auto_id, rect) = ui.allocate_space(size);
        let mut response = ui.interact(rect, self.id.unwrap_or(auto_id), Sense::click_and_drag());
        let value = self.interact(ui, &response);
        if value != *self.value {
            *self.value = value;
            response.mark_changed();
        }
        let value_text = self.value_text.clone();
        response.widget_info(|| {
            let mut info = WidgetInfo::slider(ui.is_enabled(), f64::from(value), self.label);
            info.current_text_value.clone_from(&value_text);
            info
        });
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
