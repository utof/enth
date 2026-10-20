//! **Owner: lane RUI.** Size tokens for the Recreate UI: the challenge bar, the results drawer and the randomizer
//! settings page (component tier). Pre-declared empty by lane S0r (R1/S1): the lane adds its tokens here and nowhere
//! else (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so older
//! theme files still load.

use serde::{Deserialize, Serialize};

/// The Recreate UI: the challenge bar, the results drawer and the randomizer settings page (lane RUI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecreateSizes {}

impl RecreateSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {}
    }
}

impl Default for RecreateSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
