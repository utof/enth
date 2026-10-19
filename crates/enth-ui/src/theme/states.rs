//! State tokens (semantic tier): how a control or a group of controls reads in a given state.

use serde::{Deserialize, Serialize};

/// How a control or a group of controls reads in a given state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct States {
    /// Opacity of everything inside a disabled module card (title, display, knobs, values),
    /// and of egui's own disabled widgets. The controls stay editable.
    pub disabled_opacity: f32,
}

impl States {
    /// The dark theme's states.
    pub(super) fn dark() -> Self {
        Self {
            disabled_opacity: 0.4,
        }
    }
}

impl Default for States {
    /// The dark theme's states, so theme files written before this token still load.
    fn default() -> Self {
        Self::dark()
    }
}
