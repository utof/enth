//! Size tokens for the editor window (component tier). A lane that needs a dimension for this area adds it here.

use egui::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSizes {
    pub default: Vec2,
    pub min: Vec2,
}

impl WindowSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            default: egui::vec2(1200.0, 800.0),
            min: egui::vec2(1000.0, 680.0),
        }
    }
}
