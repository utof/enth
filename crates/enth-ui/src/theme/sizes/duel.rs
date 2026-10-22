//! **Owner: lane DUI.** Size tokens for the duel card: the share line, the paste box, verification, the sealed answer and the
//! duel history (component tier). Pre-declared empty by lane S0e (R5): the lane adds its tokens here and
//! nowhere else (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so
//! older theme files still load.

use serde::{Deserialize, Serialize};

/// The duel card: the share line, the paste box, verification, the sealed answer and the history (lane DUI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DuelSizes {}

impl DuelSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {}
    }
}

impl Default for DuelSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
