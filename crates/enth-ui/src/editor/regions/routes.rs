//! The compact route list (lane ED2): the modulation overview when the sources × targets grid does not fit its panel.
//! Layout 1 at 1200 × 800 gives the overview 98 px of height, and the grid of the P1 default patch needs 161 (a header
//! row and seven target rows); the list shows the same eight routes in two columns of four rows.
//!
//! One row per route, in the grid's order (targets as the cards list them, then sources): the source, an arrow, the
//! target, and the depth cell, which edits like a grid cell: drag it (right or up) for one gesture of `SetDepth`s,
//! double-click it or use its menu to remove the route. Each cell is named "<target> from <source>", as in the grid.

use egui::{Align2, Rect, Sense, Ui, Vec2, WidgetInfo, pos2, vec2};
use enth_patch::RouteAddr;

use super::{MatrixView, region_ui};
use crate::Theme;
use crate::editor::cards::format::signed_percent;
use crate::editor::cards::{Grip, Hovered, Intent, Intents, ids};
use crate::theme::glyph;

/// The route list.
pub struct RouteList<'a> {
    theme: &'a Theme,
    view: &'a MatrixView,
}

impl<'a> RouteList<'a> {
    pub fn new(theme: &'a Theme, view: &'a MatrixView) -> Self {
        Self { theme, view }
    }

    /// Columns a panel `width` wide holds.
    fn columns(t: &Theme, width: f32) -> usize {
        let gap = t.space.gap;
        let inner = width - 2.0 * gap;
        (((inner + gap) / (t.size.modulation.route_min_width + gap)).floor() as usize).max(1)
    }

    /// The panel size the list needs at `width`: the caption, then the rows of the columns it gets.
    pub fn needed(t: &Theme, view: &MatrixView, width: f32, caption: f32) -> Vec2 {
        let columns = Self::columns(t, width);
        let rows = view.routes.len().div_ceil(columns).max(1);
        vec2(
            t.size.modulation.route_min_width + 2.0 * t.space.gap,
            rows as f32 * t.size.matrix.row_height + caption + 2.0 * t.space.gap,
        )
    }

    /// Draws a captioned panel with the routes into `rect`, column by column, scrolling if they do not fit.
    pub fn show(self, ui: &mut Ui, rect: Rect, out: &mut Intents) {
        let t = self.theme;
        let c = &t.color;
        let v = self.view;
        let m = &t.size.matrix;
        let mut ui = region_ui(ui, rect, "matrix");
        ui.painter()
            .rect_filled(rect, t.radius.medium, c.surface.card);
        let inner = rect.shrink(t.space.gap);
        let caption = ui.painter().text(
            inner.left_top(),
            Align2::LEFT_TOP,
            "Modulation",
            t.text.label.clone(),
            c.text.label,
        );
        let body = Rect::from_min_max(
            pos2(inner.left(), caption.bottom() + t.space.tight),
            inner.max,
        );
        let columns = Self::columns(t, rect.width());
        let per_column = v.routes.len().div_ceil(columns).max(1);
        let width = (body.width() - (columns - 1) as f32 * t.space.gap) / columns as f32;
        let mut list = ui.new_child(egui::UiBuilder::new().max_rect(body));
        egui::ScrollArea::vertical()
            .id_salt("routes")
            .auto_shrink(false)
            .show(&mut list, |ui| {
                let height = per_column as f32 * m.row_height;
                let (area, _) = ui.allocate_exact_size(vec2(body.width(), height), Sense::hover());
                for (n, &cell) in v.routes.iter().enumerate() {
                    let (column, row) = (n / per_column, n % per_column);
                    let left = area.left() + column as f32 * (width + t.space.gap);
                    let top = area.top() + row as f32 * m.row_height;
                    let row_rect = Rect::from_min_size(pos2(left, top), vec2(width, m.row_height));
                    route_row(ui, t, v, cell, row_rect, out);
                }
            });
    }
}

/// One line of label text at `at` (left, centred vertically), cut with an ellipsis at `width`.
fn label(
    ui: &Ui,
    painter: &egui::Painter,
    at: egui::Pos2,
    text: &str,
    t: &Theme,
    color: egui::Color32,
    width: f32,
) {
    let mut job =
        egui::text::LayoutJob::simple_singleline(text.to_owned(), t.text.label.clone(), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
    let rect = Align2::LEFT_CENTER.anchor_size(at, galley.size());
    painter.galley(rect.min, galley, color);
}

/// One row: source → target, and the depth cell at its right end.
fn route_row(ui: &Ui, t: &Theme, v: &MatrixView, cell: usize, rect: Rect, out: &mut Intents) {
    let c = &t.color;
    let m = &t.size.matrix;
    let columns = v.sources.len();
    let (row, col) = (cell / columns, cell % columns);
    let (src, source) = &v.sources[col];
    let (dst, target) = &v.targets[row];
    let route = RouteAddr {
        src: src.clone(),
        dst: dst.clone(),
    };
    let painter = ui.painter();
    let source_w = t.size.modulation.route_source_width;
    let y = rect.center().y;
    label(
        ui,
        painter,
        pos2(rect.left(), y),
        source,
        t,
        c.text.label,
        source_w,
    );
    let arrow = pos2(rect.left() + source_w, y);
    let arrow = painter.text(
        arrow,
        Align2::LEFT_CENTER,
        glyph::SEND,
        t.text.label.clone(),
        c.text.dimmed,
    );
    let depth_rect = Rect::from_min_size(
        pos2(rect.right() - m.cell_width, rect.top()),
        vec2(m.cell_width, m.row_height),
    )
    .shrink(t.space.hairline / 2.0);
    let target_left = arrow.right() + t.space.tight;
    label(
        ui,
        painter,
        pos2(target_left, y),
        target,
        t,
        c.text.label,
        depth_rect.left() - t.space.tight - target_left,
    );

    let depth = v.depths[cell];
    let hit = ui.interact(
        depth_rect,
        ids::matrix().with(&route),
        Sense::click_and_drag(),
    );
    hit.widget_info(|| {
        WidgetInfo::slider(
            true,
            f64::from(depth / 100.0),
            format!("{target} from {source}"),
        )
    });
    if hit.double_clicked() {
        out.push(Intent::Unroute(route.clone()));
    } else {
        if hit.drag_started() {
            out.push(Intent::Begin(Grip::Route(route.clone())));
        }
        if hit.dragged() {
            let k = &t.size.knob;
            let fine = if ui.input(|i| i.modifiers.shift) {
                k.fine_ratio
            } else {
                1.0
            };
            let delta = hit.drag_delta();
            let travel = delta.x - delta.y;
            if travel != 0.0 {
                let next = enth_params::DEPTH.clamp(depth + travel / k.drag_travel * 100.0 * fine);
                if next.to_bits() != depth.to_bits() {
                    out.push(Intent::SetDepth {
                        route: route.clone(),
                        depth: next,
                    });
                }
            }
        }
        if hit.drag_stopped() {
            out.push(Intent::End(Grip::Route(route.clone())));
        }
    }
    hit.context_menu(|ui| {
        if ui.button(format!("Remove {source} modulation")).clicked() {
            out.push(Intent::Unroute(route.clone()));
            ui.close();
        }
    });
    if hit.hovered() || hit.dragged() {
        out.hovered = Some(Hovered {
            path: format!(
                "Modulation {} {source} {} {target}",
                glyph::PATH,
                glyph::SEND
            ),
            value: signed_percent(depth),
            modulation: "drag to set the depth, double-click to remove".to_owned(),
        });
    }

    // The cell, drawn like a grid cell: a bar from the centre and the depth in percent.
    let painter = ui.painter();
    let fill = if hit.hovered() || hit.dragged() {
        c.surface.raised
    } else {
        c.surface.recessed
    };
    painter.rect_filled(depth_rect, t.radius.small, fill);
    let amount = (depth / 100.0).clamp(-1.0, 1.0);
    let bar_y = depth_rect.bottom() - t.space.hairline / 2.0;
    let mid = depth_rect.center().x;
    let reach = depth_rect.width() / 2.0 * amount.abs();
    let (from, to) = if amount > 0.0 {
        (mid, mid + reach)
    } else {
        (mid - reach, mid)
    };
    painter.hline(
        depth_rect.x_range(),
        bar_y,
        t.stroke.thin(c.accent.modulation_wash),
    );
    painter.hline(from..=to, bar_y, t.stroke.thin(c.accent.modulation));
    painter.text(
        pos2(mid, depth_rect.center().y - t.space.hairline / 2.0),
        Align2::CENTER_CENTER,
        format!("{depth:+.0}"),
        t.text.value.clone(),
        c.text.primary,
    );
}
