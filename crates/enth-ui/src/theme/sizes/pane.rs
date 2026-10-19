//! Size tokens for side panes (component tier). A lane that needs a dimension for this area adds it here.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneSizes {
    pub lesson_width: f32,
}

impl PaneSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            lesson_width: 320.0,
        }
    }
}
