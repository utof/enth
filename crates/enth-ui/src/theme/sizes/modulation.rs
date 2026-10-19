//! Size tokens for drag-to-modulate and the modulation overview (component tier, lane ED2): the depth handle on a
//! knob and the compact route list the overview shows when the sources × targets grid does not fit. Every field has a
//! serde default (the dark theme's value), so older theme files still load.

use serde::{Deserialize, Serialize};

/// Drag-to-modulate and the modulation overview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModulationSizes {
    /// Radius of a knob's depth handle: the dot on its modulation ring at the end of its newest route's depth.
    pub depth_handle: f32,
    /// How close the pointer must be to grab the depth handle (also the radius of its wash while hovered).
    pub depth_reach: f32,
    /// The narrowest column of the compact route list ("Env 2 → Filter 1 Cutoff +40"); the list takes as many
    /// columns of at least this width as its panel holds.
    pub route_min_width: f32,
    /// Width of the source name at the start of a route-list row.
    pub route_source_width: f32,
}

impl ModulationSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            depth_handle: 3.0,
            depth_reach: 6.0,
            route_min_width: 176.0,
            route_source_width: 40.0,
        }
    }
}

impl Default for ModulationSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
