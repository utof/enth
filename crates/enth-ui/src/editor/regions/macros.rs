//! The macro bar: the eight macro knobs with their names. A drag is one gesture (`Begin`, `SetMacro`…, `End`), like a
//! parameter knob's; the host sees the macros as its first eight parameters.

use egui::{Rect, Ui};
use enth_patch::model::MACROS;
use enth_patch::{MacroIndex, Patch};

use super::region_ui;
use crate::Theme;
use crate::editor::cards::{Grip, Hovered, Intent, Intents, ids};
use crate::theme::glyph;
use crate::widgets::{Macro, MacroBar};

/// One macro.
#[derive(Clone, Debug, PartialEq)]
pub struct MacroView {
    pub index: MacroIndex,
    /// The macro's name, else "Macro N".
    pub name: String,
    /// Knob position, 0–1.
    pub value: f32,
}

impl MacroView {
    /// The patch's eight macros.
    pub fn all(patch: &Patch) -> Vec<MacroView> {
        (0..MACROS)
            .filter_map(MacroIndex::new)
            .map(|index| {
                let m = &patch.macros[index.get()];
                MacroView {
                    index,
                    name: m
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("Macro {}", index.one_based())),
                    value: m.value,
                }
            })
            .collect()
    }

    /// The position as the bar shows it ("55 %").
    pub fn value_text(&self) -> String {
        format!("{:.0} %", self.value * 100.0)
    }
}

/// The macro bar.
pub struct Macros<'a> {
    theme: &'a Theme,
    macros: &'a [MacroView],
}

impl<'a> Macros<'a> {
    pub fn new(theme: &'a Theme, macros: &'a [MacroView]) -> Self {
        Self { theme, macros }
    }

    /// Draws into `rect` (one `bar.macros` band at its top).
    pub fn show(self, ui: &mut Ui, rect: Rect, out: &mut Intents) {
        let mut ui = region_ui(ui, rect, "macros");
        let mut values: Vec<f32> = self.macros.iter().map(|m| m.value).collect();
        let texts: Vec<String> = self.macros.iter().map(MacroView::value_text).collect();
        let mut bar: Vec<Macro<'_>> = self
            .macros
            .iter()
            .zip(values.iter_mut())
            .zip(texts)
            .map(|((m, value), value_text)| Macro {
                name: &m.name,
                value,
                value_text,
            })
            .collect();
        let responses = MacroBar::new(self.theme)
            .id(ids::macros())
            .show(&mut ui, &mut bar);
        drop(bar);
        for ((m, response), value) in self.macros.iter().zip(&responses).zip(&values) {
            let grip = || Grip::Macro(m.index);
            if response.drag_started() {
                out.push(Intent::Begin(grip()));
            }
            if response.changed() && value.to_bits() != m.value.to_bits() {
                out.push(Intent::SetMacro {
                    index: m.index,
                    value: *value,
                });
            }
            if response.drag_stopped() {
                out.push(Intent::End(grip()));
            }
            if response.hovered() || response.dragged() {
                out.hovered = Some(Hovered {
                    path: format!("Macros {} {}", glyph::PATH, m.name),
                    value: m.value_text(),
                    modulation: String::new(),
                });
            }
        }
    }
}
