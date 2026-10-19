//! Size tokens for the editor's layout regions and popups (component tier, lane ED1). A layout (`editor/layout.rs`)
//! sizes its rows and columns with tokens like these, by path (`"size.editor.band_height"`), never in pixels, so a
//! layout file holds no visual values of its own. Every field has a serde default, so older theme files still load.

use serde::{Deserialize, Serialize};

/// Editor regions and popups.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSizes {
    /// Height of a band of lane cards side by side (the modulators in layout 1): one envelope card, the tallest P1
    /// modulator with "More" closed (header, display, two rows of knob cells), plus a hairline.
    pub band_height: f32,
    /// Width of an inline knob in the header (the master level): the dial, its label and a value such as "−12.0 dB".
    pub header_knob: f32,
    /// Width of the "Type value…" field, which floats over the control it edits.
    pub entry_width: f32,
    /// Width the field's error hint wraps at ("write a number in Hz, e.g. 800, 800Hz, 1.2kHz, A4, C#3, Db2").
    pub entry_hint_width: f32,
}

impl EditorSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            band_height: 224.0,
            header_knob: 112.0,
            entry_width: 120.0,
            entry_hint_width: 240.0,
        }
    }
}

impl Default for EditorSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
