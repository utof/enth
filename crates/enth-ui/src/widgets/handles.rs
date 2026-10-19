//! Draggable points on a curve display, as an alternative to the knobs behind them: an envelope's
//! attack peak, its decay/sustain corner, its release.
//!
//! Two steps, so the curve is drawn from the values this frame's drag produced:
//! [`CurveHandles::interact`] moves the values, then [`Display::handles`](super::Display::handles)
//! draws the curve and the handles over it. Handles show while the pointer is over the display;
//! drag one (Shift for fine), or double-click it to reset its values. While one is hovered or
//! dragged, a tooltip reads its values in real units.
//!
//! Values are normalised (0..=1), like the knob's; the caller says how far a drag across the
//! display moves each one, from how its curve places the value, so the point stays under the
//! pointer.

use egui::text::{LayoutJob, TextFormat};
use egui::{IdSalt, Rect, RectAlign, Response, Sense, Ui, Vec2, WidgetInfo, vec2};

use super::display;
use crate::theme::Theme;

/// One parameter a handle drags along one axis.
pub struct Drag<'a> {
    name: &'a str,
    value: &'a mut f32,
    per_span: f32,
    default: Option<f32>,
    format: Option<&'a dyn Fn(f32) -> String>,
}

impl<'a> Drag<'a> {
    /// Drags the normalised `value`, which moves by `per_span` while the pointer crosses the
    /// whole display: its width for a horizontal drag, its height for a vertical one. Negative
    /// moves it the other way; zero leaves it alone. `name` labels it in the readout.
    pub fn new(name: &'a str, value: &'a mut f32, per_span: f32) -> Self {
        Self {
            name,
            value,
            per_span,
            default: None,
            format: None,
        }
    }

    /// Double-clicking the handle resets the value to this (normalised).
    #[must_use]
    pub fn default_value(mut self, value: f32) -> Self {
        self.default = Some(value);
        self
    }

    /// Formats the value in real units for the readout ("7.7 ms"). Default: a percentage.
    #[must_use]
    pub fn format(mut self, format: &'a dyn Fn(f32) -> String) -> Self {
        self.format = Some(format);
        self
    }

    fn text(&self) -> String {
        match self.format {
            Some(format) => format(*self.value),
            None => format!("{:.0} %", *self.value * 100.0),
        }
    }

    /// Moves the value by `travel` (a share of the display) or resets it; returns how far the
    /// point moved, as a share of the display.
    fn apply(&mut self, travel: f32, reset: bool) -> f32 {
        if self.per_span == 0.0 {
            return 0.0;
        }
        let old = *self.value;
        let new = match self.default {
            Some(default) if reset => default,
            _ => (old + travel * self.per_span).clamp(0.0, 1.0),
        };
        *self.value = new;
        (new - old) / self.per_span
    }
}

/// A draggable point on a curve display.
pub struct Handle<'a> {
    label: String,
    at: Vec2,
    x: Option<Drag<'a>>,
    y: Option<Drag<'a>>,
}

impl<'a> Handle<'a> {
    /// A point at `at` in the curve's space (x 0..1 to the right, y 0..1 up), before this frame's
    /// drag. `label` names it for accessibility ("Amp env attack").
    pub fn new(label: impl Into<String>, at: Vec2) -> Self {
        Self {
            label: label.into(),
            at,
            x: None,
            y: None,
        }
    }

    /// Dragging sideways moves this parameter.
    #[must_use]
    pub fn x(mut self, drag: Drag<'a>) -> Self {
        self.x = Some(drag);
        self
    }

    /// Dragging up and down moves this parameter.
    #[must_use]
    pub fn y(mut self, drag: Drag<'a>) -> Self {
        self.y = Some(drag);
        self
    }

    fn drags(&self) -> impl Iterator<Item = &Drag<'a>> {
        self.x.iter().chain(self.y.iter())
    }
}

/// A handle's place and look after this frame's input, for [`super::Display`] to draw.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Point {
    /// In the curve's space.
    pub(crate) at: Vec2,
    /// Hovered or dragged: drawn filled, with a wash behind it.
    pub(crate) lit: bool,
}

/// What [`CurveHandles::interact`] did this frame. Hand it to
/// [`Display::handles`](super::Display::handles) to draw the handles.
pub struct HandleEdit {
    pub(crate) points: Vec<Point>,
    /// The pointer is over the display, or a handle is being dragged.
    pub(crate) shown: bool,
    /// One per handle, in order. `changed()` is set when a drag or a double-click moved any of
    /// the handle's values; a handle that drags two parameters starts and ends one gesture.
    pub responses: Vec<Response>,
}

impl HandleEdit {
    /// The handle being dragged, else the one under the pointer.
    pub fn active(&self) -> Option<usize> {
        self.responses
            .iter()
            .position(Response::dragged)
            .or_else(|| self.responses.iter().position(Response::hovered))
    }
}

/// Hit-testing and dragging for the handles of one display.
pub struct CurveHandles<'a> {
    theme: &'a Theme,
    salt: IdSalt,
}

impl<'a> CurveHandles<'a> {
    /// Give it an id from the module, never its position, like the card around it.
    pub fn new(theme: &'a Theme, id_salt: impl egui::AsIdSalt) -> Self {
        Self {
            theme,
            salt: IdSalt::new(id_salt),
        }
    }

    /// Hit-tests and drags `handles` on the display at `rect`, changing their values in place.
    pub fn interact(self, ui: &mut Ui, rect: Rect, handles: &mut [Handle<'_>]) -> HandleEdit {
        let t = self.theme;
        let id = ui.make_persistent_id(self.salt);
        let area = display::trace_area(t, rect);
        let fine = if ui.input(|i| i.modifiers.shift) {
            t.size.knob.fine_ratio
        } else {
            1.0
        };
        let grab = Vec2::splat(2.0 * t.size.handle.reach);
        let mut points = Vec::with_capacity(handles.len());
        let mut responses = Vec::with_capacity(handles.len());
        for (index, handle) in handles.iter_mut().enumerate() {
            let hit = Rect::from_center_size(display::to_screen(area, handle.at), grab);
            let mut response =
                ui.interact(hit.intersect(rect), id.with(index), Sense::click_and_drag());
            // Pointer travel as a share of the display, right and up positive.
            let travel = if response.dragged() {
                // A drag starts once the pointer has moved a little; count that first stretch
                // too, so the point stays under the pointer.
                let delta = if response.drag_started() {
                    response.total_drag_delta().unwrap_or_default()
                } else {
                    response.drag_delta()
                } * fine;
                vec2(delta.x / area.width(), -delta.y / area.height())
            } else {
                Vec2::ZERO
            };
            let reset = response.double_clicked();
            let moved = vec2(
                handle.x.as_mut().map_or(0.0, |d| d.apply(travel.x, reset)),
                handle.y.as_mut().map_or(0.0, |d| d.apply(travel.y, reset)),
            );
            if moved != Vec2::ZERO {
                response.mark_changed();
            }
            let value = handle.drags().next().map_or(0.0, |d| *d.value);
            response.widget_info(|| {
                WidgetInfo::slider(ui.is_enabled(), f64::from(value), &handle.label)
            });
            // While a widget is pressed or dragged it is the only hovered one.
            let lit = response.hovered() || response.dragged();
            let at = handle.at + moved;
            if lit {
                let circle = Rect::from_center_size(
                    display::to_screen(area, at),
                    Vec2::splat(2.0 * t.size.handle.radius),
                );
                readout(t, &response, circle, handle);
            }
            points.push(Point { at, lit });
            responses.push(response);
        }
        let shown = ui.rect_contains_pointer(rect) || responses.iter().any(Response::dragged);
        HandleEdit {
            points,
            shown,
            responses,
        }
    }
}

/// A tooltip above the handle's `circle` with its values in real units, one row per parameter.
/// Above, because that is where an envelope's curve leaves room and the pointer doesn't cover it.
fn readout(t: &Theme, response: &Response, circle: Rect, handle: &Handle<'_>) {
    let mut tooltip = egui::containers::Tooltip::for_widget(response).gap(t.space.tight);
    tooltip.popup = tooltip.popup.anchor(circle).align(RectAlign::TOP);
    tooltip.show(|ui| {
        ui.spacing_mut().item_spacing.y = t.space.hairline;
        for drag in handle.drags() {
            let mut row = LayoutJob::default();
            let name = TextFormat::simple(t.text.label.clone(), t.color.text.label);
            let value = TextFormat::simple(t.text.value.clone(), t.color.text.primary);
            row.append(drag.name, 0.0, name);
            row.append(&drag.text(), t.space.tight, value);
            ui.label(row);
        }
    });
}
