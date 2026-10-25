//! **Owner: lane DUI.** Size tokens for the duel card: the share line, the paste box, verification, the sealed answer and the
//! duel history (component tier). Pre-declared empty by lane S0e (R5); lane SKD added the minimal card's three (F2d), lane
//! DUI the Paste button's room and the side-by-side grid's column: the lane adds its tokens here and nowhere else (docs/design-system.md, "Token files"). Every field gets a serde default
//! (the dark theme's value), so older theme files still load.

use serde::{Deserialize, Serialize};

/// The duel card: the share line, the paste box, verification, the sealed answer and the history (lane DUI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DuelSizes {
    /// The width of the duel card's column at the window's right edge, under the layout's top row (lane SKD's minimal
    /// card, F2d), as the Create panel's: the layout's other regions take the rest while it is open.
    pub pane: f32,
    /// The height of the paste box (a whole chat message: a scorecard's two human lines and its code, or an answer).
    pub paste_height: f32,
    /// The width of the button that opens the card from the overview's corner.
    pub opener: f32,
    /// The room kept at the paste box's right for **Paste** (lane DUI): the box takes the rest of the row.
    pub paste_button: f32,
    /// The narrowest column of the side-by-side grid (you, them; lane DUI).
    pub side_column: f32,
}

impl DuelSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            pane: 340.0,
            paste_height: 72.0,
            opener: 64.0,
            paste_button: 52.0,
            side_column: 72.0,
        }
    }
}

impl Default for DuelSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
