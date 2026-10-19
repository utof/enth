//! Size tokens for the waveform, curve and scope displays (component tier). A lane that needs a dimension for this area adds it here.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplaySizes {
    /// Vertical grid divisions in a display (lines between them).
    pub grid_columns: u8,
    /// Horizontal grid divisions in a display.
    pub grid_rows: u8,
    /// Radius of a position marker on a curve (e.g. an LFO's phase).
    pub marker_radius: f32,
}

impl DisplaySizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            grid_columns: 4,
            grid_rows: 2,
            marker_radius: 3.0,
        }
    }
}
