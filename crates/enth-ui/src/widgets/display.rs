//! Recessed displays: waveforms, curves (envelopes, LFO shapes, responses), scopes, spectra.
//! Every module shows its sound; lesson figures reuse the same drawing.

use egui::{Align2, Pos2, Rect, Response, Sense, Shape, Ui, Vec2, Widget, pos2};

use super::paint;
use crate::theme::Theme;

/// What a display draws.
#[derive(Clone, Copy)]
pub enum Trace<'a> {
    /// Audio samples in −1..1, decimated to a min/max pair per column (oscilloscope).
    Scope(&'a [f32]),
    /// A waveform in −1..1 (one cycle, a table), drawn with its area to the centre line.
    Wave(&'a [f32]),
    /// `y = f(x)` with `x` and `y` in 0..1 (envelope, LFO shape, filter response).
    Curve(&'a dyn Fn(f32) -> f32),
    /// Bin magnitudes in 0..1, bin 0 = DC, linear in frequency; drawn on a log axis.
    Spectrum(&'a [f32]),
}

/// Which accent draws the trace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ink {
    /// Audio: oscillators, filters, scopes.
    #[default]
    Value,
    /// Modulators: envelopes and LFOs.
    Modulation,
}

pub struct Display<'a> {
    theme: &'a Theme,
    trace: Trace<'a>,
    size: Vec2,
    ink: Ink,
    caption: Option<&'a str>,
    marker: Option<f32>,
}

impl<'a> Display<'a> {
    pub fn new(theme: &'a Theme, trace: Trace<'a>, size: Vec2) -> Self {
        Self {
            theme,
            trace,
            size,
            ink: Ink::Value,
            caption: None,
            marker: None,
        }
    }

    #[must_use]
    pub fn ink(mut self, ink: Ink) -> Self {
        self.ink = ink;
        self
    }

    /// A small label in the top-left corner.
    #[must_use]
    pub fn caption(mut self, caption: &'a str) -> Self {
        self.caption = Some(caption);
        self
    }

    /// A dot on a [`Trace::Curve`] at this `x` (0..1), e.g. an LFO's current phase.
    #[must_use]
    pub fn marker(mut self, x: f32) -> Self {
        self.marker = Some(x);
        self
    }

    /// Paints into `rect` without allocating space (for widgets that place displays themselves).
    pub fn paint(&self, ui: &Ui, rect: Rect) {
        let t = self.theme;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, t.radius.small, t.color.surface.recessed);
        let grid = t.stroke.thin(t.color.line.grid);
        let d = &t.size.display;
        for i in 1..d.grid_columns {
            let x = rect.left() + rect.width() * f32::from(i) / f32::from(d.grid_columns);
            painter.vline(x, rect.y_range(), grid);
        }
        for i in 1..d.grid_rows {
            let y = rect.top() + rect.height() * f32::from(i) / f32::from(d.grid_rows);
            painter.hline(rect.x_range(), y, grid);
        }
        let (ink, wash) = match self.ink {
            Ink::Value => (t.color.accent.value, t.color.accent.value_wash),
            Ink::Modulation => (t.color.accent.modulation, t.color.accent.modulation_wash),
        };
        let area = rect.shrink2(egui::vec2(0.0, t.space.tight));
        let stroke = t.stroke.thin(ink);
        match self.trace {
            Trace::Scope(samples) => {
                painter.add(Shape::line(scope_points(samples, area), stroke));
            }
            Trace::Wave(samples) => {
                let points = scope_points(samples, area);
                painter.add(paint::area_under(&points, area.center().y, wash));
                painter.add(Shape::line(points, stroke));
            }
            Trace::Curve(f) => {
                let points = curve_points(f, area);
                painter.add(paint::area_under(&points, area.bottom(), wash));
                painter.add(Shape::line(points, stroke));
                if let Some(x) = self.marker {
                    let x = x.clamp(0.0, 1.0);
                    let at = pos2(area.lerp_inside(egui::vec2(x, 0.0)).x, y_of(area, f(x)));
                    painter.circle_filled(at, d.marker_radius, ink);
                }
            }
            Trace::Spectrum(bins) => {
                let points = spectrum_points(bins, area);
                painter.add(paint::area_under(&points, area.bottom(), wash));
                painter.add(Shape::line(points, stroke));
            }
        }
        if let Some(caption) = self.caption {
            painter.text(
                rect.left_top() + egui::vec2(t.space.tight, t.space.hairline),
                Align2::LEFT_TOP,
                caption,
                t.text.label.clone(),
                t.color.text.label,
            );
        }
    }
}

impl Widget for Display<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(self.size, Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui, rect);
        }
        response
    }
}

/// `y` for a 0..1 value in `area` (1 at the top).
fn y_of(area: Rect, value: f32) -> f32 {
    area.bottom() - value.clamp(0.0, 1.0) * area.height()
}

/// `y` for a −1..1 sample in `area`.
fn y_of_sample(area: Rect, sample: f32) -> f32 {
    area.center().y - sample.clamp(-1.0, 1.0) * area.height() / 2.0
}

/// One point per sample if they fit, else a min/max pair per column.
fn scope_points(samples: &[f32], area: Rect) -> Vec<Pos2> {
    let columns = area.width().max(1.0) as usize;
    let n = samples.len();
    if n < 2 {
        return Vec::new();
    }
    if n <= columns {
        return samples
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let x = area.left() + area.width() * i as f32 / (n - 1) as f32;
                pos2(x, y_of_sample(area, s))
            })
            .collect();
    }
    let mut points = Vec::with_capacity(columns * 2);
    for c in 0..columns {
        let chunk = &samples[c * n / columns..((c + 1) * n / columns).max(c * n / columns + 1)];
        let (lo, hi) = chunk
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &s| {
                (lo.min(s), hi.max(s))
            });
        let x = area.left() + c as f32;
        points.push(pos2(x, y_of_sample(area, hi)));
        points.push(pos2(x, y_of_sample(area, lo)));
    }
    points
}

fn curve_points(f: &dyn Fn(f32) -> f32, area: Rect) -> Vec<Pos2> {
    let steps = paint::curve_steps(area.width());
    (0..=steps)
        .map(|i| {
            let x = i as f32 / steps as f32;
            pos2(area.left() + x * area.width(), y_of(area, f(x)))
        })
        .collect()
}

/// Bins on a log-frequency axis from bin 1 up. Where a column is narrower than a bin (the low
/// end) the level is interpolated between bins; where it spans several, it is their maximum.
fn spectrum_points(bins: &[f32], area: Rect) -> Vec<Pos2> {
    let n = bins.len();
    if n < 2 {
        return Vec::new();
    }
    let columns = area.width().max(1.0) as usize;
    let top = (n - 1) as f32;
    let bin_at = |c: usize| top.powf(c as f32 / columns as f32);
    (0..=columns)
        .map(|c| {
            let (lo, hi) = (bin_at(c), bin_at(c + 1));
            let level = if hi - lo < 1.0 {
                let i = lo.floor() as usize;
                let next = (i + 1).min(n - 1);
                bins[i] + (bins[next] - bins[i]) * (lo - i as f32)
            } else {
                let last = (hi.floor() as usize).min(n - 1);
                bins[lo.ceil() as usize..=last]
                    .iter()
                    .fold(0.0_f32, |m, &b| m.max(b))
            };
            pos2(area.left() + c as f32, y_of(area, level))
        })
        .collect()
}
