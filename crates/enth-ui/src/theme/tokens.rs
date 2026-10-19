//! Tiers 2 and 3: the semantic and component tokens widgets read. Every field says what it is
//! for; a widget that needs a value not listed here gets a new token, not a literal.

use egui::{Color32, FontId, Stroke, Vec2};
use serde::{Deserialize, Serialize};

use super::hex;

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

impl Space {
    /// Every spacing token with its name, smallest first.
    pub fn named(&self) -> [(&'static str, f32); 7] {
        [
            ("hairline", self.hairline),
            ("tight", self.tight),
            ("gap", self.gap),
            ("pad", self.pad),
            ("section", self.section),
            ("wide", self.wide),
            ("band", self.band),
        ]
    }
}

/// Spacing on the 4 px grid, named by use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Space {
    /// 2: offsets of rings and stripes, the gap between a value and its bar.
    pub hairline: f32,
    /// 4: label to value, glyph to text, knob cells in a row.
    pub tight: f32,
    /// 8: between sibling controls; the padding inside cards and bars.
    pub gap: f32,
    /// 12: between groups inside a card.
    pub pad: f32,
    /// 16: around bands and panes.
    pub section: f32,
    /// 24: between a pane and the editor body.
    pub wide: f32,
    /// 32: large empty states.
    pub band: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Radius {
    /// 2: displays, segments, badges, matrix cells.
    pub small: f32,
    /// 4: cards, bars, buttons, popups.
    pub medium: f32,
}

/// Stroke widths. Build strokes with [`Strokes::thin`] and [`Strokes::thick`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Strokes {
    /// 1: grid lines, curves, outlines, score rings, knob pointers.
    pub thin: f32,
    /// 3: knob tracks and arcs, category stripes, the selection underline.
    pub thick: f32,
}

impl Strokes {
    /// A thin stroke in `color`.
    #[allow(
        clippy::disallowed_methods,
        reason = "strokes are built here, from the width tokens"
    )]
    pub fn thin(&self, color: Color32) -> Stroke {
        Stroke::new(self.thin, color)
    }

    /// A thick stroke in `color`.
    #[allow(
        clippy::disallowed_methods,
        reason = "strokes are built here, from the width tokens"
    )]
    pub fn thick(&self, color: Color32) -> Stroke {
        Stroke::new(self.thick, color)
    }
}

impl TextStyles {
    /// Every text style with its name.
    pub fn named(&self) -> Vec<(&'static str, &FontId)> {
        vec![
            ("label", &self.label),
            ("body", &self.body),
            ("value", &self.value),
            ("title", &self.title),
            ("strong", &self.strong),
        ]
    }
}

/// Text styles. Three sizes (11, 12, 14) in three faces (Inter Regular, Inter Semibold, Inter
/// Tabular for values).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextStyles {
    /// 11 px regular: parameter labels, column headers, status text.
    pub label: FontId,
    /// 12 px regular: running text (lessons, help).
    pub body: FontId,
    /// 12 px with tabular digits: values, so they don't jitter while dragging.
    pub value: FontId,
    /// 14 px semibold: card titles, lesson headings, the wordmark.
    pub title: FontId,
    /// 12 px semibold: bold words in running text.
    pub strong: FontId,
}

/// Component tokens: per-widget dimensions, in logical px at 100 % UI scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sizes {
    pub window: WindowSizes,
    pub bar: BarSizes,
    pub card: CardSizes,
    pub knob: KnobSizes,
    pub tile: TileSizes,
    pub matrix: MatrixSizes,
    pub pane: PaneSizes,
    pub display: DisplaySizes,
    /// Height of single-line controls: buttons, segmented controls, number fields.
    pub control_height: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSizes {
    pub default: Vec2,
    pub min: Vec2,
}

/// Horizontal bands of the editor, top to bottom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarSizes {
    pub header: f32,
    pub macros: f32,
    /// The modulator strip with its cards expanded.
    pub modulator_strip: f32,
    pub status: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardSizes {
    /// A generator strip (header plus one row of knobs beside its display).
    pub generator_height: f32,
    /// A collapsed card: the header row alone.
    pub collapsed_height: f32,
    /// Width of an effect lane and of the cards in it.
    pub lane_width: f32,
    /// Width of the display slot in a generator strip.
    pub display_width: f32,
    /// Height of the display in a stacked (lane) card.
    pub lane_display_height: f32,
    /// Side of the enable toggle square.
    pub toggle: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnobSizes {
    pub diameter: f32,
    /// Width of the track and the value arc.
    pub track: f32,
    /// Width of one knob cell (knob, label above, value below).
    pub cell_width: f32,
    /// Vertical drag distance that sweeps the full range.
    pub drag_travel: f32,
    /// Drag and wheel speed while Shift is held (fine adjustment).
    pub fine_ratio: f32,
    /// Angle swept from minimum to maximum, in degrees; the gap is at the bottom.
    pub sweep_degrees: f32,
}

/// Live modulation-source tiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileSizes {
    pub width: f32,
    pub height: f32,
    /// Diameter of the route-count badge.
    pub badge: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatrixSizes {
    pub row_height: f32,
    /// Width of the target-name column.
    pub target_width: f32,
    /// Width of one source column.
    pub cell_width: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneSizes {
    pub lesson_width: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplaySizes {
    /// Vertical grid divisions in a display (lines between them).
    pub grid_columns: u8,
    /// Horizontal grid divisions in a display.
    pub grid_rows: u8,
    /// Radius of a position marker on a curve (e.g. an LFO's phase).
    pub marker_radius: f32,
}

/// How a control or a group of controls reads in a given state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct States {
    /// Opacity of everything inside a disabled module card (title, display, knobs, values),
    /// and of egui's own disabled widgets. The controls stay editable.
    pub disabled_opacity: f32,
}

impl Default for States {
    /// The dark theme's states, so theme files written before this token still load.
    fn default() -> Self {
        super::Theme::dark().state
    }
}

/// Animation rates. Nothing animates unless it is visible and its data changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    /// Live rings and source tiles (Hz).
    pub animated_hz: f32,
    /// A visible scope with fresh data (Hz).
    pub scope_hz: f32,
    /// egui's own transitions (collapsing, scrolling), in seconds.
    pub transition: f32,
}

impl Motion {
    /// Delay between frames while something animated is visible.
    pub fn animated_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f32(1.0 / self.animated_hz)
    }

    /// Delay between frames while a scope is visible and fed.
    pub fn scope_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f32(1.0 / self.scope_hz)
    }
}
