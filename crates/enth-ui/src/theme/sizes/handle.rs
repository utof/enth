//! Size tokens for draggable points on a curve display (component tier). A lane that needs a dimension for this area adds it here.

use serde::{Deserialize, Serialize};

/// Draggable points on a curve display. Drags use the knob's `fine_ratio` with Shift.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandleSizes {
    /// Radius of a handle's circle, shown while the pointer is over the display.
    pub radius: f32,
    /// How close the pointer must be to grab a handle; also the radius of the wash behind a
    /// hovered or dragged one.
    pub reach: f32,
}

impl HandleSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            radius: 4.0,
            reach: 8.0,
        }
    }
}

impl Default for HandleSizes {
    /// The dark theme's sizes, so theme files written before this token still load.
    fn default() -> Self {
        Self::dark()
    }
}
