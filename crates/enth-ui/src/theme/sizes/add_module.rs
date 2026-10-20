//! **Owner: lane AM.** Size tokens for adding and removing modules: the add menus and the dashed "+ Add" slot
//! (component tier). Pre-declared empty by lane S0r (R1/S1): the lane adds its tokens here and nowhere else
//! (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so older theme
//! files still load.

use serde::{Deserialize, Serialize};

/// Adding and removing modules: the add menus and the dashed "+ Add" slot (lane AM).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AddModuleSizes {}

impl AddModuleSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {}
    }
}

impl Default for AddModuleSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
