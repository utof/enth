//! Drawing helpers shared by the widgets. Geometry only: every colour, width and font is
//! passed in from the theme.

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align2, Color32, FontId, Galley, Painter, Pos2, Rect, Shape, Stroke, Ui, pos2};

/// Tessellation step along arcs and curves, in points. A quality setting, not a look token.
const CURVE_STEP: f32 = 2.0;

/// Height of one line of text in `font`.
pub(crate) fn row_height(ui: &Ui, font: &FontId) -> f32 {
    ui.ctx().fonts_mut(|f| f.row_height(font))
}

/// Points along a circular arc from angle `from` to `to` (radians, clockwise from +x on screen).
pub(crate) fn arc_points(center: Pos2, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
    let length = (to - from).abs() * radius;
    let steps = ((length / CURVE_STEP).ceil() as usize).max(1);
    (0..=steps)
        .map(|i| {
            let angle = from + (to - from) * i as f32 / steps as f32;
            center + egui::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// Strokes an arc; nothing for an empty sweep.
pub(crate) fn arc(
    painter: &Painter,
    center: Pos2,
    radius: f32,
    from: f32,
    to: f32,
    stroke: Stroke,
) {
    if (to - from).abs() > f32::EPSILON {
        painter.add(Shape::line(arc_points(center, radius, from, to), stroke));
    }
}

/// Number of samples to draw a curve across `width` points.
pub(crate) fn curve_steps(width: f32) -> usize {
    ((width / CURVE_STEP).ceil() as usize).max(2)
}

/// Lays out `text` on one line, cut with an ellipsis at `max_width`.
pub(crate) fn one_line(
    ui: &Ui,
    text: &str,
    font: &FontId,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font.clone(), color);
    job.wrap = TextWrapping::truncate_at_width(max_width);
    ui.ctx().fonts_mut(|f| f.layout_job(job))
}

/// Paints `text` on one line anchored at `pos`, cut with an ellipsis at `max_width`.
#[allow(
    clippy::too_many_arguments,
    reason = "a text call: position, anchor, text, font, colour, width"
)]
pub(crate) fn text_in(
    ui: &Ui,
    painter: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: &FontId,
    color: Color32,
    max_width: f32,
) -> Rect {
    let galley = one_line(ui, text, font, color, max_width);
    let rect = anchor.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

/// A filled area between `baseline` (y) and the polyline `points`, as a mesh of vertical strips.
pub(crate) fn area_under(points: &[Pos2], baseline: f32, color: Color32) -> Shape {
    let mut mesh = egui::Mesh::default();
    for pair in points.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        let base = mesh.vertices.len() as u32;
        for p in [a, b, pos2(b.x, baseline), pos2(a.x, baseline)] {
            mesh.colored_vertex(p, color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    Shape::mesh(mesh)
}
