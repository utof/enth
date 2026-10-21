//! **Owner: lane LUI.** Size tokens for the Learn view: the lesson pane, the lesson browser and the overlay's lock
//! glyph (component tier). Pre-declared empty by lane S0l (R2): the lane adds its tokens here and nowhere else
//! (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so older theme
//! files still load.

use serde::{Deserialize, Serialize};

/// The Learn view: the lesson pane, the lesson browser and the lock glyph (lane LUI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnSizes {
    /// The lesson pane's width: the `lesson` region's column in the built-in layout `learn` (`layout/learn.json`,
    /// as `{"token": "size.learn.pane"}`). Report 03's 320 px.
    pub pane: f32,
    /// The overview's height in the layout `learn` (its row spans the window under the cards and the pane, so a
    /// Recreate step's results drawer and the route list keep their width): what layout 1 leaves its overview at
    /// 1200 × 800 (800 − the bars, gaps, signal row and band).
    pub overview_height: f32,
    /// The side of the overlay's lock glyph (a shackle over a filled body), drawn inside a locked control's top right
    /// corner.
    pub lock: f32,
    /// The height of an `enth` text block's `scope` widget (the live scope inside the step's text).
    pub scope_height: f32,
    /// The height of an `enth` text block's `shape` widget (an envelope's stages or an LFO's wave, from the live patch).
    pub shape_height: f32,
    /// The lesson browser's progress column ("3/11 ✓"), right of a lesson's title.
    pub progress_width: f32,
}

impl LearnSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            pane: 320.0,
            overview_height: 98.0,
            lock: 9.0,
            scope_height: 64.0,
            shape_height: 56.0,
            progress_width: 56.0,
        }
    }
}

impl Default for LearnSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
