//! **Owner: lane LUI.** Size tokens for the Learn view: the lesson pane, the lesson browser and the overlay's lock
//! glyph (component tier). Pre-declared empty by lane S0l (R2): the lane adds its tokens here and nowhere else
//! (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so older theme
//! files still load.

use serde::{Deserialize, Serialize};

/// The Learn view: the lesson pane, the lesson browser and the lock glyph (lane LUI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnSizes {
    /// The width of the Learn panel at the window's right edge (lane SKL's minimal panel, F2l): the layout's regions
    /// take the rest while it is open. Report 03's 320 px.
    pub pane: f32,
}

impl LearnSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self { pane: 320.0 }
    }
}

impl Default for LearnSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
