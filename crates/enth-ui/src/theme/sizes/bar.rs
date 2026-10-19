//! Size tokens for the horizontal bands of the editor (component tier). A lane that needs a dimension for this area adds it here.

use serde::{Deserialize, Serialize};

/// Horizontal bands of the editor, top to bottom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarSizes {
    pub header: f32,
    pub macros: f32,
    /// The modulator strip with its cards expanded.
    pub modulator_strip: f32,
    pub status: f32,
}

impl BarSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            header: 40.0,
            macros: 56.0,
            modulator_strip: 140.0,
            status: 22.0,
        }
    }
}
