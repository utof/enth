//! Size tokens for module cards (component tier). A lane that needs a dimension for this area adds it here.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardSizes {
    /// A generator strip (header plus one row of knobs beside its display).
    pub generator_height: f32,
    /// A collapsed card: the header row alone.
    pub collapsed_height: f32,
    /// Width of an effect lane and of the cards in it.
    pub lane_width: f32,
    /// Width of the display slot in a generator strip.
    pub display_width: f32,
    /// Height of the display in a stacked (lane) card.
    pub lane_display_height: f32,
    /// Side of the enable toggle square.
    pub toggle: f32,
}

impl CardSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            generator_height: 96.0,
            collapsed_height: 24.0,
            lane_width: 220.0,
            display_width: 128.0,
            lane_display_height: 40.0,
            toggle: 12.0,
        }
    }
}
