//! The modulation overview: one row per routed destination, one column per source that has routes, each cell the
//! route's depth. A cell drag is one gesture of `SetDepth`s (on an empty cell it creates the route); a double-click
//! removes the route. It is the same data the CLI prints.

use egui::{Align2, Rect, Ui, pos2};
use enth_patch::{ModSource, ParamAddr, Patch, RouteAddr, Section};

use super::region_ui;
use crate::Theme;
use crate::editor::cards::present;
use crate::editor::cards::view::{CardView, source_name};
use crate::editor::cards::{Grip, Intent, Intents, ids};
use crate::widgets::MatrixGrid;

/// The routes as a grid.
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixView {
    /// Columns: the sources that have routes, modulators first, then velocity, key, the mod wheel and macros.
    pub sources: Vec<(ModSource, String)>,
    /// Rows: the routed destinations, in the order of the patch's cards and their parameters.
    pub targets: Vec<(ParamAddr, String)>,
    /// Depths in percent, row-major (`targets.len() × sources.len()`); 0 where there is no route.
    pub depths: Vec<f32>,
    /// The cells that hold a route (a route may have depth 0), row-major: the compact route list's rows (lane ED2).
    pub routes: Vec<usize>,
}

impl MatrixView {
    /// The patch's routes.
    pub fn of(patch: &Patch) -> Self {
        let mut sources: Vec<(ModSource, String)> = Vec::new();
        let mut order: Vec<ModSource> = patch.routes.iter().map(|r| r.src.clone()).collect();
        order.sort_by_key(|src| source_rank(patch, src));
        order.dedup();
        for src in order {
            sources.push((src.clone(), source_name(patch, &src)));
        }
        let mut targets: Vec<(ParamAddr, String)> = Vec::new();
        let cards = Section::ALL
            .into_iter()
            .flat_map(|s| patch.section(s).iter().map(|m| CardView::module(patch, m)))
            .chain([CardView::voice(patch), CardView::master(patch)]);
        for card in cards {
            for p in &card.params {
                if !p.routes.is_empty() {
                    let name = format!("{} {}", card.title, present::label(card.kind, p.spec));
                    targets.push((p.addr.clone(), name));
                }
            }
        }
        let mut depths = vec![0.0; targets.len() * sources.len()];
        let mut routes = Vec::with_capacity(patch.routes.len());
        for r in &patch.routes {
            let row = targets.iter().position(|(a, _)| *a == r.dst);
            let col = sources.iter().position(|(s, _)| *s == r.src);
            if let (Some(row), Some(col)) = (row, col) {
                depths[row * sources.len() + col] = r.depth;
                routes.push(row * sources.len() + col);
            }
        }
        routes.sort_unstable();
        MatrixView {
            sources,
            targets,
            depths,
            routes,
        }
    }
}

/// Modulators in patch order, then velocity, key, the mod wheel, macros by index.
fn source_rank(patch: &Patch, src: &ModSource) -> (u8, usize) {
    match src {
        ModSource::Module(id) => (
            0,
            patch
                .modulators
                .iter()
                .position(|m| &m.id == id)
                .unwrap_or(usize::MAX),
        ),
        ModSource::Velocity => (1, 0),
        ModSource::Key => (2, 0),
        ModSource::ModWheel => (3, 0),
        ModSource::Macro(i) => (4, i.get()),
    }
}

/// The matrix region.
pub struct Matrix<'a> {
    theme: &'a Theme,
    view: &'a MatrixView,
}

impl<'a> Matrix<'a> {
    pub fn new(theme: &'a Theme, view: &'a MatrixView) -> Self {
        Self { theme, view }
    }

    /// Draws a captioned panel with the grid into `rect` (scrolling when it doesn't fit).
    pub fn show(self, ui: &mut Ui, rect: Rect, out: &mut Intents) {
        let t = self.theme;
        let v = self.view;
        let mut ui = region_ui(ui, rect, "matrix");
        ui.painter()
            .rect_filled(rect, t.radius.medium, t.color.surface.card);
        let inner = rect.shrink(t.space.gap);
        let caption = ui.painter().text(
            inner.left_top(),
            Align2::LEFT_TOP,
            "Modulation",
            t.text.label.clone(),
            t.color.text.label,
        );
        let body = Rect::from_min_max(
            pos2(inner.left(), caption.bottom() + t.space.tight),
            inner.max,
        );
        if v.targets.is_empty() {
            ui.painter().text(
                body.left_top(),
                Align2::LEFT_TOP,
                format!(
                    "No routes yet: drag a source's {} onto a knob",
                    crate::theme::glyph::HANDLE
                ),
                t.text.label.clone(),
                t.color.text.dimmed,
            );
            return;
        }
        let mut grid_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
        let source_names: Vec<&str> = v.sources.iter().map(|(_, n)| n.as_str()).collect();
        let target_names: Vec<&str> = v.targets.iter().map(|(_, n)| n.as_str()).collect();
        let mut amounts: Vec<f32> = v.depths.iter().map(|d| d / 100.0).collect();
        let cells = egui::ScrollArea::both()
            .id_salt("scroll")
            .auto_shrink(false)
            .show(&mut grid_ui, |ui| {
                MatrixGrid::new(t, "matrix", &source_names, &target_names, &mut amounts)
                    .id(ids::matrix())
                    .show_cells(ui)
                    .1
            })
            .inner;
        let columns = v.sources.len();
        for (index, cell) in cells.iter().enumerate() {
            let (row, col) = (index / columns, index % columns);
            let route = RouteAddr {
                src: v.sources[col].0.clone(),
                dst: v.targets[row].0.clone(),
            };
            let before = v.depths[index];
            if cell.double_clicked() {
                if before != 0.0 {
                    out.push(Intent::Unroute(route));
                }
                continue;
            }
            if cell.drag_started() {
                out.push(Intent::Begin(Grip::Route(route.clone())));
            }
            let depth = enth_params::DEPTH.clamp(amounts[index] * 100.0);
            if cell.dragged() && depth.to_bits() != before.to_bits() {
                out.push(Intent::SetDepth {
                    route: route.clone(),
                    depth,
                });
            }
            if cell.drag_stopped() {
                out.push(Intent::End(Grip::Route(route)));
            }
        }
    }
}
