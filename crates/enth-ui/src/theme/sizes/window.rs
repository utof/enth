//! Size tokens for the editor window (component tier). A lane that needs a dimension for this area adds it here.

use egui::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSizes {
    /// The window's size, in logical px (the plugin opens at it).
    pub default: Vec2,
    /// The smallest logical size the editor supports. Lane AM (R1/S1) raised it from 1000 × 680 to the default:
    /// measured with the P1 default, layout 1 loses part of a region below 1200 × 800 (under 702 px tall the overview,
    /// and every drag source with it, is gone), so a smaller screen uses the UI scale (75 % draws 1200 × 800 in
    /// 900 × 600 pixels) and a compact layout is a variant's job (docs/editor.md, "The minimum window").
    pub min: Vec2,
}

impl WindowSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            default: egui::vec2(1200.0, 800.0),
            min: egui::vec2(1200.0, 800.0),
        }
    }
}
