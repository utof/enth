//! Size tokens for the output region (component tier, lane ED2): the scope (or spectrum) and the stereo meter fed by
//! the engine's telemetry. Every field has a serde default (the dark theme's value), so older theme files still load.

use serde::{Deserialize, Serialize};

/// The output region: scope, spectrum and meters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputSizes {
    /// Width of the output region in a layout (layout 1: right of the header).
    pub width: f32,
    /// Width of one channel's meter bar.
    pub meter_width: f32,
    /// The quietest level a meter or the spectrum shows, in dB (the bottom).
    pub meter_floor_db: f32,
    /// How fast a meter falls back after a peak, in dB per second.
    pub meter_fall_db_per_s: f32,
    /// How many of the newest samples the scope draws and the spectrum analyses (a power of two; 1,024 is about
    /// 21 ms at 48 kHz).
    pub scope_samples: u16,
}

impl OutputSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            width: 208.0,
            meter_width: 4.0,
            meter_floor_db: -60.0,
            meter_fall_db_per_s: 24.0,
            scope_samples: 1024,
        }
    }
}

impl Default for OutputSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
