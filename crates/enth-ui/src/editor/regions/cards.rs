//! A region of cards: the sound sources (generators), the voice effects, the effects or the modulators. The region
//! arranges its cards one way ([`Arrangement`]) inside its rectangle and scrolls on its own when they don't fit, so
//! a layout needs no minimum sizes; [`CardRegionOutput::overflow`] says by how much they didn't.
//!
//! Given the sections it hosts ([`CardRegion::room`], lane AM), a region also offers to add modules: a right-click on
//! its empty space opens the Add menu, and a dashed "+ Add" slot follows its cards when the region has room for it
//! (the slot never makes it scroll) and one of its sections is under its limit (`editor/add_module.rs`).

use egui::{Rect, ScrollArea, Sense, Ui, Vec2, vec2};
use serde::{Deserialize, Serialize};

use super::region_ui;
use crate::Theme;
use crate::editor::add_module::{self, Room};
use crate::editor::cards::{Card, CardKey, CardOutput, CardView, Intents, ids};

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
    /// The sections modules can be added to here (lane AM); none: no add menu, no slot.
    room: Option<&'a Room>,
    /// Strips wrap their knob cells by their width, not at the constant (a dense layout, lane EDS).
    fit_rows: bool,
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
            room: None,
            fit_rows: false,
        }
    }

    /// Strips wrap their knob cells by the width they are given (a dense layout, lane EDS), not at layout 1's constant.
    #[must_use]
    pub fn fit_rows(mut self, fit: bool) -> Self {
        self.fit_rows = fit;
        self
    }

    /// The sections this region shows and how full they are (lane AM): a right-click on empty space opens their Add
    /// menu, and the dashed "+ Add" slot follows the cards where there is room.
    #[must_use]
    pub fn room(mut self, room: &'a Room) -> Self {
        self.room = (!room.is_empty()).then_some(room);
        self
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
        // Under the cards, so they and their controls take their own clicks: the region's empty space (lane AM).
        let background = self
            .room
            .map(|_| ui.interact(rect, ids::region(self.name).with("add"), Sense::click()));
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
                    let start = ui.cursor().min;
                    for view in self.cards {
                        let card = Card::new(t, view)
                            .along_x(self.arrangement == Arrangement::Band)
                            .fit_rows(self.fit_rows);
                        let card = match self.arrangement {
                            Arrangement::Strips => card.strip(),
                            Arrangement::Lane | Arrangement::Band => card.stacked(),
                        };
                        shown.push((view.key.clone(), card.show(ui, out)));
                    }
                    if let Some(room) = self.room.filter(|r| r.has_room()) {
                        self.slot(ui, rect, start, room, out);
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
        if let (Some(background), Some(room)) = (&background, self.room) {
            let at = background.interact_pointer_pos();
            let over_card = at.is_some_and(|at| shown.iter().any(|(_, c)| c.rect.contains(at)));
            add_module::region_menu(t, background, over_card, room, out);
        }
        CardRegionOutput {
            cards: shown,
            overflow,
        }
    }

    /// The dashed "+ Add" slot after the cards, if the region (`rect`, its cards from `start`) has room for it; it
    /// never makes the region scroll.
    fn slot(&self, ui: &mut Ui, rect: Rect, start: egui::Pos2, room: &Room, out: &mut Intents) {
        let t = self.theme;
        let a = &t.size.add_module;
        let gap = if self.cards.is_empty() {
            0.0
        } else {
            t.space.gap
        };
        let used = ui.min_rect();
        let size = match self.arrangement {
            Arrangement::Band => {
                let left = rect.right() - used.right().max(start.x) - gap;
                (left >= a.slot_width).then(|| vec2(a.slot_width, rect.height()))
            }
            Arrangement::Strips | Arrangement::Lane => {
                let left = rect.bottom() - used.bottom().max(start.y) - gap;
                let width = match self.arrangement {
                    Arrangement::Lane => t.size.card.lane_width,
                    _ => rect.width(),
                };
                (left >= a.slot_height).then(|| vec2(width, a.slot_height))
            }
        };
        let Some(size) = size else {
            return;
        };
        let (slot, _) = ui.allocate_exact_size(size, Sense::hover());
        add_module::slot(ui, t, slot, ids::region(self.name).with("slot"), room, out);
    }
}

/// What a card region drew.
pub struct CardRegionOutput {
    /// Each card's key, rect and header, in drawing order.
    pub cards: Vec<(CardKey, CardOutput)>,
    /// How far the cards reach past the region, per axis (zero when they fit and nothing scrolls).
    pub overflow: Vec2,
}
