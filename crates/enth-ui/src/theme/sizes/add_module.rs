//! **Owner: lane AM.** Size tokens for adding and removing modules: the card's "…" menu button and the dashed "+ Add"
//! slot (component tier). Pre-declared empty by lane S0r (R1/S1): the lane adds its tokens here and nowhere else
//! (docs/design-system.md, "Token files"). Every field gets a serde default (the dark theme's value), so older theme
//! files still load.

use serde::{Deserialize, Serialize};

/// Adding and removing modules: the card's "…" menu button and the dashed "+ Add" slot (lane AM).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AddModuleSizes {
    /// Width of a card's "…" menu button at the right end of its title row (the row's height is
    /// `card.collapsed_height`).
    pub menu_width: f32,
    /// Height of the dashed "+ Add" slot under the cards of a strip or lane region (it is the region's width).
    pub slot_height: f32,
    /// Width of the dashed "+ Add" slot after the cards of a band (it is the band's height).
    pub slot_width: f32,
    /// Length of one dash of the slot's outline.
    pub dash: f32,
    /// The space between two dashes of the slot's outline.
    pub dash_gap: f32,
}

impl AddModuleSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            menu_width: 20.0,
            slot_height: 32.0,
            slot_width: 40.0,
            dash: 4.0,
            dash_gap: 4.0,
        }
    }
}

impl Default for AddModuleSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
