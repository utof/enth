//! Size tokens for the editor's layout regions and popups (component tier), reserved by lane S0b for lane ED1:
//! region sizes as tokens (`LayoutSpec`), popup row height, drawer size. Empty until ED1 adds fields; every
//! field needs `#[serde(default)]` or a `Default` that reads the dark theme, so older theme files still load.

use serde::{Deserialize, Serialize};

/// Editor regions and popups. Empty until lane ED1 adds fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EditorSizes {}

impl EditorSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {}
    }
}
