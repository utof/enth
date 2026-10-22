//! **Owner: lane EUI.** Size tokens for the Ear view: the Ear column, its round types, the replay, the thresholds and the
//! practised-days calendar (component tier). Pre-declared empty by lane S0e (R4): the lane adds its tokens here and
//! nowhere else (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so
//! older theme files still load.

use serde::{Deserialize, Serialize};

/// The Ear view: the column, the round types, the replay, thresholds and the calendar (lane EUI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EarSizes {}

impl EarSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {}
    }
}

impl Default for EarSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
