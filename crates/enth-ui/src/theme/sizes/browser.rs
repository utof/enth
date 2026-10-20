//! **Owner: lane PBU.** Size tokens for the preset browser: the header's ◀ name ▶, Browse and Save, and the browse
//! panel (component tier). Pre-declared empty by lane S0r (R1/S1): the lane adds its tokens here and nowhere else
//! (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so older theme
//! files still load.

use serde::{Deserialize, Serialize};

/// The preset browser: the header's ◀ name ▶, Browse and Save, and the browse panel (lane PBU).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserSizes {}

impl BrowserSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {}
    }
}

impl Default for BrowserSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
