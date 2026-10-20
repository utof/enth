//! The header band: the wordmark, the patch name, and the Undo and Redo buttons (keyboard undo waits for the
//! fork's captured-shortcut hook, report 02 D6, so the buttons are the way to undo in every host). Built-in cards a
//! layout places here (layout 1: master) draw as inline knobs at the right end, with every control's menu.
//!
//! Three hook points end [`Header::show`] (lane S0r, R1/S1): Add (`editor/add_module.rs`, feature `add-module`, lane
//! AM), Presets (`editor/browser.rs`, feature `presets`, lane PBU) and Recreate (`editor/recreate/bar.rs`, feature
//! `recreate`, lane RUI). Each is a no-op until its lane fills it.

use egui::{Align, Align2, Button, Layout, Rect, RichText, Ui, UiBuilder, pos2};

use super::region_ui;
use crate::Theme;
use crate::editor::cards::{CardView, Form, Intent, Intents, control};

/// What the header shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderView {
    /// The preset the sound came from and whether it was edited since, for the Presets hook's ◀ name ▶ (lane PBU).
    pub presets: crate::editor::browser::HeaderPresets,
    /// The patch's name; empty for an unnamed patch.
    pub patch_name: String,
    pub can_undo: bool,
    pub can_redo: bool,
    /// What Undo would undo ("Filter 1 › Cutoff"), for its tooltip.
    pub undo: Option<String>,
    /// What Redo would redo.
    pub redo: Option<String>,
    /// Every section and how full it is, for the Add menu (lane AM, the Add hook).
    pub room: crate::editor::add_module::Room,
    /// The Recreate bar's view (lane SKR's skeleton, the Recreate hook): empty, and drawn as nothing, until the
    /// shell hands the editor a round view.
    #[cfg(feature = "recreate")]
    pub recreate: crate::editor::recreate::bar::BarView,
}

/// The header band.
pub struct Header<'a> {
    theme: &'a Theme,
    view: &'a HeaderView,
    knobs: &'a [CardView],
}

impl<'a> Header<'a> {
    pub fn new(theme: &'a Theme, view: &'a HeaderView) -> Self {
        Self {
            theme,
            view,
            knobs: &[],
        }
    }

    /// Cards whose parameters draw as inline knobs at the right end (the built-ins a layout places in the header).
    #[must_use]
    pub fn knobs(mut self, cards: &'a [CardView]) -> Self {
        self.knobs = cards;
        self
    }

    /// Draws into `rect`; the buttons push [`Intent::Undo`] and [`Intent::Redo`].
    pub fn show(self, ui: &mut Ui, rect: Rect, out: &mut Intents) {
        let t = self.theme;
        let c = &t.color;
        let mut ui = region_ui(ui, rect, "header");
        ui.painter()
            .rect_filled(rect, t.radius.medium, c.surface.card);
        let inner = rect.shrink2(egui::vec2(t.space.pad, 0.0));
        ui.painter().text(
            pos2(inner.left(), inner.center().y),
            Align2::LEFT_CENTER,
            "enth",
            t.text.title.clone(),
            c.text.primary,
        );
        // With the Presets hook (lane PBU) the name is part of its ◀ name ▶, drawn by the hook.
        #[cfg(not(feature = "presets"))]
        {
            let (name, ink) = if self.view.patch_name.is_empty() {
                ("Untitled", c.text.label)
            } else {
                (self.view.patch_name.as_str(), c.text.primary)
            };
            ui.painter().text(
                inner.center(),
                Align2::CENTER_CENTER,
                name,
                t.text.body.clone(),
                ink,
            );
        }
        let mut buttons = ui.new_child(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::right_to_left(Align::Center)),
        );
        buttons.spacing_mut().item_spacing.x = t.space.tight;
        // Right to left: the knobs first (rightmost; the last card's last parameter at the edge), then Redo, Undo.
        for card in self.knobs.iter().rev() {
            for p in card.drawn().collect::<Vec<_>>().into_iter().rev() {
                control(
                    &mut buttons,
                    t,
                    card,
                    p,
                    Form::Inline(t.size.editor.header_knob),
                    out,
                );
            }
        }
        if !self.knobs.is_empty() {
            buttons.add_space(t.space.section);
        }
        let button = |text: &str| Button::new(RichText::new(text).font(t.text.body.clone()));
        // Right to left: Redo is drawn before Undo.
        let redo = buttons.add_enabled(self.view.can_redo, button("Redo"));
        let redo = match &self.view.redo {
            Some(what) => redo.on_hover_text(format!("Redo {what}")),
            None => redo,
        };
        let undo = buttons.add_enabled(self.view.can_undo, button("Undo"));
        let undo = match &self.view.undo {
            Some(what) => undo.on_hover_text(format!("Undo {what}")),
            None => undo,
        };
        if undo.clicked() {
            out.push(Intent::Undo);
        }
        if redo.clicked() {
            out.push(Intent::Redo);
        }
        // R1/S1 header hook points (lane S0r): each lane draws its header controls from its own file, behind its
        // feature, so AM, PBU and RUI never edit this one to add them. No-ops until those lanes fill them.
        #[cfg(feature = "add-module")]
        crate::editor::add_module::header(&mut buttons, t, inner, self.view, out);
        // In Recreate mode the challenge bar takes the Presets group's place (lane RUI): the group steps aside.
        #[cfg(feature = "recreate")]
        let presets = !self.view.recreate.mode;
        #[cfg(not(feature = "recreate"))]
        let presets = true;
        #[cfg(feature = "presets")]
        if presets {
            crate::editor::browser::header(&mut ui, t, inner, self.view, out);
        }
        #[cfg(not(feature = "presets"))]
        let _ = presets;
        // Lane SKR: the Recreate bar joins the right-hand row, left of Add (it fits there beside the Presets group).
        #[cfg(feature = "recreate")]
        crate::editor::recreate::bar::header(&mut buttons, t, inner, self.view, out);
    }
}
