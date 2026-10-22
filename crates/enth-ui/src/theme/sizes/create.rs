//! **Owner: lane PUI.** Size tokens for the Create view: the drop zone, the folder browser, the candidate tiles and the
//! progress bar (component tier). Pre-declared empty by lane S0s (R3): the lane adds its tokens here and nowhere else
//! (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so older theme
//! files still load.

use serde::{Deserialize, Serialize};

/// The Create view: drop zone, folder browser, candidate tiles, progress bar (lane PUI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateSizes {
    /// The width of the Create panel at the window's right edge, under the layout's top row (lane SK3's minimal panel,
    /// F2s): the layout's other regions take the rest while it is open, the lesson pane included. The lesson pane's
    /// width.
    pub pane: f32,
    /// The height of the drop zone.
    pub drop_zone: f32,
    /// The room the folder browser's path field leaves for its **Choose** button.
    pub choose: f32,
    /// The height of one candidate tile (two lines: rank and provenance, then distance and state).
    pub tile_height: f32,
    /// The candidate tiles per row (K = 8 makes four rows of two in the 320 px panel).
    pub tile_columns: usize,
    /// The rank column at a tile's left ("1" … "8", in `text.title`).
    pub rank_width: f32,
    /// The height of one row of the folder browser's list.
    pub browser_row: f32,
    /// The most rows the folder browser's list shows before it scrolls (it scrolls inside the panel, never the panel).
    pub browser_rows: usize,
    /// The height of the progress bar.
    pub progress_height: f32,
}

impl CreateSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            pane: 320.0,
            drop_zone: 56.0,
            choose: 88.0,
            tile_height: 40.0,
            tile_columns: 2,
            rank_width: 18.0,
            browser_row: 20.0,
            browser_rows: 22,
            progress_height: 16.0,
        }
    }
}

impl Default for CreateSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
