//! Size tokens for live modulation-source tiles (component tier). A lane that needs a dimension for this area adds it here.

use serde::{Deserialize, Serialize};

/// Live modulation-source tiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileSizes {
    pub width: f32,
    pub height: f32,
    /// Diameter of the route-count badge.
    pub badge: f32,
}

impl TileSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            width: 72.0,
            height: 56.0,
            badge: 14.0,
        }
    }
}
