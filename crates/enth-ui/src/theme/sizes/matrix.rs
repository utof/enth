//! Size tokens for the modulation matrix (component tier). A lane that needs a dimension for this area adds it here.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatrixSizes {
    pub row_height: f32,
    /// Width of the target-name column.
    pub target_width: f32,
    /// Width of one source column.
    pub cell_width: f32,
}

impl MatrixSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            row_height: 16.0,
            target_width: 88.0,
            cell_width: 36.0,
        }
    }
}
