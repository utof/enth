//! **Owner: lane PUI.** Size tokens for the Create view: the drop zone, the folder browser, the candidate tiles and the
//! progress bar (component tier). Pre-declared empty by lane S0s (R3): the lane adds its tokens here and nowhere else
//! (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so older theme
//! files still load.

use serde::{Deserialize, Serialize};

/// The Create view: drop zone, folder browser, candidate tiles, progress bar (lane PUI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateSizes {}

impl CreateSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {}
    }
}

impl Default for CreateSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
