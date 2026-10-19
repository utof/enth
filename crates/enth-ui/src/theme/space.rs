//! Spacing, corner radius and stroke tokens (semantic tier), built from the private scales in
//! `primitives.rs` (docs/design-system.md). A lane that needs a new spacing, radius or stroke adds it here.

use egui::{Color32, Stroke};
use serde::{Deserialize, Serialize};

use super::primitives as p;

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

impl Space {
    /// The dark theme's spacing scale.
    pub(super) fn dark() -> Self {
        Self {
            hairline: p::SPACE_0,
            tight: p::SPACE_1,
            gap: p::SPACE_2,
            pad: p::SPACE_3,
            section: p::SPACE_4,
            wide: p::SPACE_5,
            band: p::SPACE_6,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Radius {
    /// 2: displays, segments, badges, matrix cells.
    pub small: f32,
    /// 4: cards, bars, buttons, popups.
    pub medium: f32,
}

impl Radius {
    /// The dark theme's corner radii.
    pub(super) fn dark() -> Self {
        Self {
            small: p::RADIUS_SMALL,
            medium: p::RADIUS_MEDIUM,
        }
    }
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

impl Strokes {
    /// The dark theme's stroke widths.
    pub(super) fn dark() -> Self {
        Self {
            thin: p::STROKE_THIN,
            thick: p::STROKE_THICK,
        }
    }
}
