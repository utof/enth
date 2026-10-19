//! Text-style tokens (semantic tier): sizes and faces by use, built from the fonts in `fonts.rs` and the size
//! scale in `primitives.rs` (docs/design-system.md). A lane that needs a new text style adds it here.

use egui::FontId;
use serde::{Deserialize, Serialize};

use super::fonts;
use super::primitives as p;

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

impl TextStyles {
    /// The dark theme's text styles: three sizes in Inter Regular, Semibold and Tabular.
    pub(super) fn dark() -> Self {
        Self {
            label: fonts::regular(p::TEXT_SMALL),
            body: fonts::regular(p::TEXT_MEDIUM),
            value: fonts::tabular(p::TEXT_MEDIUM),
            title: fonts::semibold(p::TEXT_LARGE),
            strong: fonts::semibold(p::TEXT_MEDIUM),
        }
    }
}
