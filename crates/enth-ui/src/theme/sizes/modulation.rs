//! Size tokens for drag-to-modulate and the modulation overview (component tier), reserved by lane S0b for
//! lane ED2: the target outline, the wire stroke, depth arcs, the matrix overview. Empty until ED2 adds fields;
//! every field needs `#[serde(default)]` or a `Default` that reads the dark theme, so older theme files still load.

use serde::{Deserialize, Serialize};

/// Drag-to-modulate and the matrix overview. Empty until lane ED2 adds fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModulationSizes {}

impl ModulationSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {}
    }
}
