//! The modulation overview (lane ED1 places it; ED2 grows it): the source tiles, whose ⊕ handles drag-to-modulate
//! starts from, beside the matrix of routes. A wide region puts the tiles on the left, in one row if the matrix still
//! gets its natural width beside them (else in half the region), and the matrix in the rest; a tall one stacks them. Each part scrolls on its own
//! when it doesn't fit, and [`OverviewOutput`] says by how much.

use egui::{Rect, Response, Ui, Vec2, pos2, vec2};
use enth_patch::ModSource;

use super::{Matrix, MatrixView, SourceTiles, SourceView};
use crate::Theme;
use crate::editor::cards::Intents;

/// The overview region.
pub struct Overview<'a> {
    theme: &'a Theme,
    sources: &'a [SourceView],
    matrix: &'a MatrixView,
}

/// What the overview drew.
pub struct OverviewOutput {
    /// Each source's ⊕ handle (drag-to-modulate sets its payload from `drag_started`, lane ED2).
    pub handles: Vec<(ModSource, Response)>,
    /// How far the tiles reach past their panel, per axis (zero when they fit).
    pub tiles_overflow: Vec2,
    /// How far the matrix reaches past its panel, per axis (zero when it fits).
    pub matrix_overflow: Vec2,
}

impl<'a> Overview<'a> {
    pub fn new(theme: &'a Theme, sources: &'a [SourceView], matrix: &'a MatrixView) -> Self {
        Self {
            theme,
            sources,
            matrix,
        }
    }

    /// Draws into `rect`.
    pub fn show(self, ui: &mut Ui, rect: Rect, out: &mut Intents) -> OverviewOutput {
        let t = self.theme;
        let caption = ui.ctx().fonts_mut(|f| f.row_height(&t.text.label)) + t.space.tight;
        let pad = 2.0 * t.space.gap;
        let tile = vec2(t.size.tile.width, t.size.tile.height);
        let count = self.sources.len().max(1) as f32;
        let tiles_row = count * tile.x + (count - 1.0) * t.space.tight + pad;
        let m = &t.size.matrix;
        let matrix_size = if self.matrix.targets.is_empty() {
            vec2(m.target_width, m.row_height)
        } else {
            vec2(
                m.target_width + m.cell_width * self.matrix.sources.len() as f32,
                m.row_height * (self.matrix.targets.len() + 1) as f32,
            )
        } + vec2(pad, pad + caption);
        // Rows of tiles a panel this wide holds, and the height they need.
        let tiles_height = |width: f32| {
            let per_row = ((width - pad + t.space.tight) / (tile.x + t.space.tight))
                .floor()
                .max(1.0);
            let rows = (count / per_row).ceil();
            rows * tile.y + (rows - 1.0) * t.space.tight + pad + caption
        };
        let (tiles_rect, matrix_rect) = if rect.width() >= rect.height() {
            // The tiles take one row if the matrix still gets its width beside them, else at most half.
            let matrix_width = matrix_size.x.min(rect.width() * 0.5);
            let width = tiles_row
                .min(rect.width() - matrix_width - t.space.gap)
                .max(tile.x + pad);
            let split = rect.left() + width;
            (
                Rect::from_min_max(rect.min, pos2(split, rect.bottom())),
                Rect::from_min_max(pos2(split + t.space.gap, rect.top()), rect.max),
            )
        } else {
            let split = rect.top() + tiles_height(rect.width()).min(rect.height() * 0.5);
            (
                Rect::from_min_max(rect.min, pos2(rect.right(), split)),
                Rect::from_min_max(pos2(rect.left(), split + t.space.gap), rect.max),
            )
        };
        let handles = SourceTiles::new(t, self.sources).show(ui, tiles_rect, out);
        let tiles_needed = vec2(tile.x + pad, tiles_height(tiles_rect.width()));
        let tiles_overflow = (tiles_needed - tiles_rect.size()).max(Vec2::ZERO);
        let matrix_overflow = if matrix_rect.width() > 0.0 && matrix_rect.height() > 0.0 {
            Matrix::new(t, self.matrix).show(ui, matrix_rect, out);
            (matrix_size - matrix_rect.size()).max(Vec2::ZERO)
        } else {
            matrix_size
        };
        OverviewOutput {
            handles,
            tiles_overflow,
            matrix_overflow,
        }
    }
}
