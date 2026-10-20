//! **Owner: lane RUI.** Size tokens for the Recreate UI: the challenge bar, the results drawer and the randomizer
//! settings page (component tier). Lane RUI adds its tokens here and nowhere else (docs/design-system.md, "Token
//! files"). Every field gets a serde default (the dark theme's value), so older theme files still load.

use serde::{Deserialize, Serialize};

/// The Recreate UI: the challenge bar, the results drawer and the randomizer settings page (lane RUI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecreateSizes {
    /// One difficulty dot of the challenge bar (●, a click target before a round): five make the level picker.
    pub level_dot: f32,
    /// The challenge bar's code field ("1040-1000-0001-AA" and a little room).
    pub code_field: f32,
    /// The timer ("12:34"), fixed so the bar does not shift as the seconds change.
    pub timer: f32,
    /// The results drawer's score column (sound % against the pass mark, structure and combined %, the time).
    pub drawer_score: f32,
    /// One column of the drawer's component rows ("~ attack slightly too fast"); the components take two.
    pub drawer_component: f32,
    /// One component row, and one line of the drawer's running text.
    pub drawer_row: f32,
    /// The randomizer settings page's card.
    pub settings_width: f32,
    /// The settings page's label column ("Oscillators", "Musical").
    pub settings_label: f32,
    /// The settings page's sliders.
    pub settings_slider: f32,
    /// The "Disagree…" note's card.
    pub dispute_width: f32,
    /// The note field's rows of text.
    pub dispute_rows: f32,
}

impl RecreateSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            level_dot: 14.0,
            code_field: 132.0,
            timer: 40.0,
            drawer_score: 180.0,
            drawer_component: 188.0,
            drawer_row: 16.0,
            settings_width: 560.0,
            settings_label: 120.0,
            settings_slider: 220.0,
            dispute_width: 320.0,
            dispute_rows: 3.0,
        }
    }
}

impl Default for RecreateSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}
