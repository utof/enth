//! Size tokens for knobs (component tier). A lane that needs a dimension for this area adds it here.

use serde::{Deserialize, Serialize};

use crate::theme::primitives as p;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnobSizes {
    pub diameter: f32,
    /// Width of the track and the value arc.
    pub track: f32,
    /// Width of one knob cell (knob, label above, value below).
    pub cell_width: f32,
    /// Vertical drag distance that sweeps the full range.
    pub drag_travel: f32,
    /// Drag and wheel speed while Shift is held (fine adjustment); curve handles use it too.
    pub fine_ratio: f32,
    /// Angle swept from minimum to maximum, in degrees; the gap is at the bottom.
    pub sweep_degrees: f32,
}

impl KnobSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            diameter: 36.0,
            track: p::STROKE_THICK,
            cell_width: 56.0,
            drag_travel: 200.0,
            fine_ratio: 0.1,
            sweep_degrees: 270.0,
        }
    }
}
