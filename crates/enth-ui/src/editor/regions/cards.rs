//! A region of cards: the sound sources (generators), the voice effects, the effects or the modulators. The region
//! arranges its cards one way ([`Arrangement`]) inside its rectangle and scrolls on its own when they don't fit, so
//! a layout needs no minimum sizes; [`CardRegionOutput::overflow`] says by how much they didn't.

use egui::{Rect, ScrollArea, Ui, Vec2};
use serde::{Deserialize, Serialize};

use super::region_ui;
use crate::Theme;
use crate::editor::cards::{Card, CardKey, CardOutput, CardView, Intents};

/// How a region lays out its cards (a layout file names it: `"strips"`, `"lane"`, `"band"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrangement {
    /// Strips the full width of the region, top to bottom (generators: display beside one row of knobs).
    Strips,
    /// Lane cards top to bottom (voice effects, effects).
    Lane,
    /// Lane cards side by side, left to right (the modulators band).
    Band,
}

/// A region of cards.
pub struct CardRegion<'a> {
    theme: &'a Theme,
    name: &'static str,
    cards: &'a [CardView],
    arrangement: Arrangement,
}

impl<'a> CardRegion<'a> {
    /// A region named `name` (its scope and scroll state) showing `cards` in `arrangement`.
    pub fn new(
        theme: &'a Theme,
        name: &'static str,
        cards: &'a [CardView],
        arrangement: Arrangement,
    ) -> Self {
        Self {
            theme,
            name,
            cards,
            arrangement,
        }
    }

    /// The sound sources: oscillators and noise as strips.
    pub fn generators(theme: &'a Theme, cards: &'a [CardView]) -> Self {
        Self::new(theme, "generators", cards, Arrangement::Strips)
    }

    /// The per-voice effects (filters) as a lane, with the voice card if the caller adds it.
    pub fn voice_effects(theme: &'a Theme, cards: &'a [CardView]) -> Self {
        Self::new(theme, "voice_effects", cards, Arrangement::Lane)
    }

    /// The global effects as a lane, with the master card if the caller adds it.
    pub fn effects(theme: &'a Theme, cards: &'a [CardView]) -> Self {
        Self::new(theme, "effects", cards, Arrangement::Lane)
    }

    /// The envelopes and LFOs as a band.
    pub fn modulators(theme: &'a Theme, cards: &'a [CardView]) -> Self {
        Self::new(theme, "modulators", cards, Arrangement::Band)
    }

    /// Draws the cards into `rect`. Returns each card's rect and header (for a later drag-to-reorder) and how far
    /// the cards overflow the rectangle.
    pub fn show(self, ui: &mut Ui, rect: Rect, out: &mut Intents) -> CardRegionOutput {
        let t = self.theme;
        let mut ui = region_ui(ui, rect, self.name);
        let scroll = match self.arrangement {
            Arrangement::Strips | Arrangement::Lane => ScrollArea::vertical(),
            Arrangement::Band => ScrollArea::both(),
        };
        let mut shown = Vec::with_capacity(self.cards.len());
        let output = scroll
            .id_salt("scroll")
            .auto_shrink(false)
            .max_height(rect.height())
            .max_width(rect.width())
            .show(&mut ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(t.space.gap, t.space.gap);
                let mut draw = |ui: &mut Ui| {
                    for view in self.cards {
                        let card = Card::new(t, view);
                        let card = match self.arrangement {
                            Arrangement::Strips => card.strip(),
                            Arrangement::Lane | Arrangement::Band => card.stacked(),
                        };
                        shown.push((view.key.clone(), card.show(ui, out)));
                    }
                };
                match self.arrangement {
                    Arrangement::Strips | Arrangement::Lane => {
                        ui.vertical(|ui| draw(ui));
                    }
                    Arrangement::Band => {
                        ui.horizontal_top(|ui| draw(ui));
                    }
                }
            });
        let overflow = (output.content_size - output.inner_rect.size()).max(Vec2::ZERO);
        CardRegionOutput {
            cards: shown,
            overflow,
        }
    }
}

/// What a card region drew.
pub struct CardRegionOutput {
    /// Each card's key, rect and header, in drawing order.
    pub cards: Vec<(CardKey, CardOutput)>,
    /// How far the cards reach past the region, per axis (zero when they fit and nothing scrolls).
    pub overflow: Vec2,
}
