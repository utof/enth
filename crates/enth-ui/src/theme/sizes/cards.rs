//! Size tokens for the catalogue cards and their controls (component tier, lane UI-C): the on/off switch of a
//! toggle parameter, how many items an enum shows as a segmented control, and how an envelope display places its
//! stages. Every field has a default from the dark theme, so theme files written before this area still load.

use egui::Vec2;
use serde::{Deserialize, Serialize};

/// Catalogue cards and the controls generated from `ParamSpec`s.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CardsSizes {
    /// The on/off switch in a toggle cell (a toggle parameter such as an oscillator's hard sync): a track two
    /// thumbs wide; the thumb is `card.toggle` square, like the enable toggle.
    pub switch: Vec2,
    /// The most items an enum parameter shows as a segmented control. A longer enum (a wave, an LFO shape) gets a
    /// stepped knob that reads the item's name, so it fits a card the width of a lane.
    pub max_segments: u8,
    /// Share of an envelope display's width that a stage (attack, decay, release) takes at its shortest, so a
    /// handle never sits on top of its neighbour's.
    pub envelope_min_stage: f32,
    /// Share of the width a stage gains over its whole normalised range: the stage grows with the knob's own
    /// (power-scale) position, so a handle dragged across this share of the width sweeps its knob once.
    pub envelope_stage_range: f32,
}

impl CardsSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            switch: egui::vec2(28.0, 16.0),
            max_segments: 4,
            envelope_min_stage: 0.05,
            envelope_stage_range: 0.25,
        }
    }
}

impl Default for CardsSizes {
    /// The dark theme's sizes, so theme files written before this area still load.
    fn default() -> Self {
        Self::dark()
    }
}
