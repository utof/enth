//! Colour tokens (semantic tier): surfaces, lines, text, the two accents, category hues and score hues,
//! built from the private palette in `primitives.rs` (docs/design-system.md). A lane that needs a colour
//! adds it here, in `Theme::dark()`'s source for colours below, and in [`Colors::named`], and nowhere else.

use egui::Color32;
use serde::{Deserialize, Serialize};

use super::glyph;
use super::hex;
use super::primitives as p;

/// Module categories; each has a stripe colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Generator,
    Filter,
    Modulator,
    Effect,
}

/// Challenge feedback on a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Score {
    Good,
    Near,
    Bad,
}

/// Semantic colours.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Colors {
    pub surface: Surfaces,
    pub line: Lines,
    pub text: TextColors,
    pub accent: Accents,
    pub category: Categories,
    pub score: Scores,
}

impl Colors {
    /// Every colour token with its path, in declaration order (the gallery and tests walk this).
    pub fn named(&self) -> Vec<(&'static str, Color32)> {
        vec![
            ("surface.background", self.surface.background),
            ("surface.card", self.surface.card),
            ("surface.raised", self.surface.raised),
            ("surface.recessed", self.surface.recessed),
            ("line.grid", self.line.grid),
            ("text.primary", self.text.primary),
            ("text.label", self.text.label),
            ("text.dimmed", self.text.dimmed),
            ("accent.value", self.accent.value),
            ("accent.value_wash", self.accent.value_wash),
            ("accent.modulation", self.accent.modulation),
            ("accent.modulation_wash", self.accent.modulation_wash),
            ("category.generator", self.category.generator),
            ("category.filter", self.category.filter),
            ("category.modulator", self.category.modulator),
            ("category.effect", self.category.effect),
            ("score.good", self.score.good),
            ("score.near", self.score.near),
            ("score.bad", self.score.bad),
        ]
    }
}

impl Colors {
    /// The dark theme's colours, built from the private palette.
    pub(super) fn dark() -> Self {
        Self {
            surface: Surfaces {
                background: p::GRAY_900,
                card: p::GRAY_850,
                raised: p::GRAY_800,
                recessed: p::GRAY_950,
            },
            line: Lines { grid: p::GRAY_825 },
            text: TextColors {
                primary: p::GRAY_100,
                label: p::GRAY_300,
                dimmed: p::GRAY_500,
            },
            accent: Accents {
                value: p::BLUE_400,
                value_wash: p::BLUE_WASH,
                modulation: p::ORANGE_400,
                modulation_wash: p::ORANGE_WASH,
            },
            category: Categories {
                generator: p::TEAL_400,
                filter: p::YELLOW_500,
                modulator: p::SLATE_400,
                effect: p::PURPLE_400,
            },
            score: Scores {
                good: p::GREEN_400,
                near: p::AMBER_300,
                bad: p::RED_400,
            },
        }
    }
}

/// Fills, from the window down to the displays sunk into cards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Surfaces {
    /// The window behind everything.
    #[serde(with = "hex")]
    pub background: Color32,
    /// Module cards, bars and panes.
    #[serde(with = "hex")]
    pub card: Color32,
    /// Hovered and pressed controls, popups, the selected segment, knob bodies.
    #[serde(with = "hex")]
    pub raised: Color32,
    /// Displays (waveform, curve, scope), knob tracks, value fields.
    #[serde(with = "hex")]
    pub recessed: Color32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lines {
    /// Display grid lines and dividers.
    #[serde(with = "hex")]
    pub grid: Color32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextColors {
    /// Values and titles (≥ 7:1 on card).
    #[serde(with = "hex")]
    pub primary: Color32,
    /// Labels (≥ 4.5:1 on card).
    #[serde(with = "hex")]
    pub label: Color32,
    /// Disabled and de-emphasised text.
    #[serde(with = "hex")]
    pub dimmed: Color32,
}

/// The only two accents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Accents {
    /// Value arcs, selection, focus, audio displays.
    #[serde(with = "hex")]
    pub value: Color32,
    /// Faint wash of `value`: text selection, the area under an audio curve.
    #[serde(with = "hex")]
    pub value_wash: Color32,
    /// Modulation arcs, amounts, handles and modulator curves.
    #[serde(with = "hex")]
    pub modulation: Color32,
    /// Faint wash of `modulation`: the area under a modulator curve, the zero-to-amount bar track.
    #[serde(with = "hex")]
    pub modulation_wash: Color32,
}

/// Module category hues: card stripes and tile stripes only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Categories {
    #[serde(with = "hex")]
    pub generator: Color32,
    #[serde(with = "hex")]
    pub filter: Color32,
    #[serde(with = "hex")]
    pub modulator: Color32,
    #[serde(with = "hex")]
    pub effect: Color32,
}

/// Challenge feedback; always drawn together with its glyph (`super::glyph`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scores {
    #[serde(with = "hex")]
    pub good: Color32,
    #[serde(with = "hex")]
    pub near: Color32,
    #[serde(with = "hex")]
    pub bad: Color32,
}

impl Categories {
    pub fn of(&self, category: Category) -> Color32 {
        match category {
            Category::Generator => self.generator,
            Category::Filter => self.filter,
            Category::Modulator => self.modulator,
            Category::Effect => self.effect,
        }
    }
}

impl Scores {
    pub fn of(&self, score: Score) -> Color32 {
        match score {
            Score::Good => self.good,
            Score::Near => self.near,
            Score::Bad => self.bad,
        }
    }
}

impl Score {
    pub fn glyph(self) -> &'static str {
        match self {
            Score::Good => glyph::GOOD,
            Score::Near => glyph::NEAR,
            Score::Bad => glyph::BAD,
        }
    }
}
