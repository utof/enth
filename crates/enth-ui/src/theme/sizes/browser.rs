//! **Owner: lane PBU.** Size tokens for the preset browser: the header's ◀ name ▶, Browse and Save, and the browse
//! panel (component tier). Pre-declared empty by lane S0r (R1/S1): the lane adds its tokens here and nowhere else
//! (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so older theme
//! files still load.
//!
//! The rows, buttons and fields are `size.control_height` high; spacing comes from `space.*`.

use serde::{Deserialize, Serialize};

/// The preset browser: the header's ◀ name ▶, Browse and Save, and the browse panel (lane PBU).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserSizes {
    /// The preset's name between ◀ and ▶ in the header; a longer name ends in "…". At the minimum window (1000 px)
    /// the whole group (◀ name ▶ Browse Save) still clears the header's right-hand buttons.
    pub header_name: f32,
    /// The search field at the top of the panel.
    pub search_width: f32,
    /// The name column of a row ("Sub One", the widest column).
    pub name_column: f32,
    /// The folder column ("bass", "Mine/Leads").
    pub folder_column: f32,
    /// The author column, at the right end; the tags take what is left between the folder and the author.
    pub author_column: f32,
    /// The Save dialog's card, centred over the body.
    pub dialog_width: f32,
    /// The crash notice in the body's lower right corner ("enth stopped unexpectedly last time").
    pub notice_width: f32,
}

impl BrowserSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            header_name: 180.0,
            search_width: 280.0,
            name_column: 240.0,
            folder_column: 140.0,
            author_column: 140.0,
            dialog_width: 480.0,
            notice_width: 380.0,
        }
    }
}

impl Default for BrowserSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
