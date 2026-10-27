//! Size tokens for dense cards (component tier, lane EDS, S2a): the knob, cell, header and strip sizes a dense layout
//! draws its cards with, and [`Theme::dense`], the theme a dense layout hands to its card regions.
//!
//! Layout 1's cards are sized for a P1 patch: three 96 px generator strips, a filter lane and an effects lane that each
//! hold a few 220 px cards. An S2a patch (a unison `wt`, `dist` into `ladder` into `formant`, an MSEG, `ott`, chorus and
//! a delay) holds twice the modules and about twice the controls, and every region scrolls at 1200 × 800
//! (docs/editor.md, "Dense cards"). A dense layout draws the *same* cards with smaller tokens: the same widgets, the same
//! controls and ids, only the sizes differ, because every widget reads its sizes from the theme it is given. Nothing in
//! a dense layout is a different component, so drag-to-modulate, depth handles, rings, overlays and menus work as
//! they do in layout 1.
//!
//! Every field has a serde default (the dark theme's value), so older theme files still load.

use serde::{Deserialize, Serialize};

use crate::theme::Theme;
use crate::theme::fonts;
use crate::theme::primitives as p;

/// Dense cards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DenseSizes {
    /// The knob's diameter (layout 1: 36).
    pub knob_diameter: f32,
    /// A knob cell's width: four per 220 px lane card, with the tight gap between them (layout 1: 56, three per card).
    pub cell_width: f32,
    /// The title row of a card (layout 1: `size.card.collapsed_height`, 24).
    pub header_height: f32,
    /// A generator strip with one row of knob cells (layout 1: 96).
    pub strip_height: f32,
    /// The display in a strip (layout 1: 128).
    pub display_width: f32,
    /// The display in a lane card (layout 1: 40).
    pub display_height: f32,
    /// A band of lane cards in a dense layout (the modulators): the tallest card of an S2a patch's modulators with
    /// "More" closed.
    pub band_height: f32,
    /// The macro bar in a dense layout (layout 1: `size.bar.macros`, 56): its knobs are dense too.
    pub macro_height: f32,
    /// A single-line control: a segmented control, a button (layout 1: `size.control_height`, 24).
    pub control_height: f32,
}

impl DenseSizes {
    /// The dark theme's sizes.
    pub(super) fn dark() -> Self {
        Self {
            knob_diameter: 28.0,
            cell_width: 48.0,
            header_height: 20.0,
            strip_height: 84.0,
            display_width: 96.0,
            display_height: 28.0,
            band_height: 224.0,
            macro_height: 44.0,
            control_height: 20.0,
        }
    }
}

impl Default for DenseSizes {
    /// The dark theme's sizes, so theme files written before a token still load.
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// This theme with the dense cards' sizes (`size.dense.*`) in the places a card reads them: the knob, the cell, the
    /// title row, the strip and the displays, and the value text one step smaller so a value fits the narrower cell.
    /// Everything else (colours, strokes, spacing, the other sizes) is this theme's own.
    #[must_use]
    pub fn dense(&self) -> Theme {
        let d = &self.size.dense;
        let mut t = self.clone();
        t.size.knob.diameter = d.knob_diameter;
        t.size.knob.cell_width = d.cell_width;
        t.size.card.collapsed_height = d.header_height;
        t.size.card.generator_height = d.strip_height;
        t.size.card.display_width = d.display_width;
        t.size.card.lane_display_height = d.display_height;
        t.size.bar.macros = d.macro_height;
        t.size.control_height = d.control_height;
        t.text.value = fonts::tabular(p::TEXT_SMALL);
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dense_theme_is_smaller_where_it_says_and_the_same_everywhere_else() {
        let t = Theme::dark();
        let d = t.dense();
        assert!(d.size.knob.diameter < t.size.knob.diameter);
        assert!(d.size.knob.cell_width < t.size.knob.cell_width);
        assert!(d.size.card.collapsed_height < t.size.card.collapsed_height);
        assert!(d.size.card.generator_height < t.size.card.generator_height);
        assert!(d.size.bar.macros < t.size.bar.macros);
        assert!(d.size.control_height < t.size.control_height);
        // The rest is the theme's own: colours, strokes, spacing, the lane width the layouts are built on.
        assert_eq!(d.color, t.color);
        assert_eq!(d.space, t.space);
        assert_eq!(d.size.card.lane_width, t.size.card.lane_width);
        assert_eq!(d.size.window, t.size.window);
        // A dense theme of a dense theme is itself: the tokens are absolute, not factors.
        assert_eq!(d.dense(), d);
    }

    #[test]
    fn a_theme_file_without_the_dense_tokens_still_loads() {
        let mut value = serde_json::to_value(Theme::dark()).unwrap();
        value["size"].as_object_mut().unwrap().remove("dense");
        let theme: Theme = serde_json::from_value(value).unwrap();
        assert_eq!(theme.size.dense, DenseSizes::dark());
    }
}
