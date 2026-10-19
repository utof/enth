//! The editor's horizontal bands: the macro bar, the challenge bar that replaces it in Recreate,
//! and the status line.

use egui::{Align, Align2, Id, Layout, Rect, Response, Sense, Ui, UiBuilder, pos2, vec2};

use super::knob::Knob;
use super::paint;
use crate::theme::{Score, Theme, glyph};

/// One macro: a host parameter with a name.
pub struct Macro<'a> {
    pub name: &'a str,
    pub value: &'a mut f32,
    pub value_text: String,
}

/// The 8 macros, always visible.
pub struct MacroBar<'a> {
    theme: &'a Theme,
    id: Option<Id>,
}

impl<'a> MacroBar<'a> {
    pub fn new(theme: &'a Theme) -> Self {
        Self { theme, id: None }
    }

    /// Give macro `i`'s knob the global id `id.with(i)` instead of an automatic one, so its drag
    /// survives a re-layout.
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// One response per macro, in order (drag start/stop map to host gestures).
    pub fn show(self, ui: &mut Ui, macros: &mut [Macro<'_>]) -> Vec<Response> {
        let t = self.theme;
        let rect = band(ui, t, t.size.bar.macros);
        let inner = rect.shrink2(vec2(t.space.pad, 0.0));
        let count = macros.len().max(1) as f32;
        let slot = (inner.width() - t.space.gap * (count - 1.0)) / count;
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
        );
        child.spacing_mut().item_spacing.x = t.space.gap;
        macros
            .iter_mut()
            .enumerate()
            .map(|(i, m)| {
                let mut knob = Knob::new(t, m.value, m.name)
                    .value_text(m.value_text.clone())
                    .inline(slot);
                if let Some(id) = self.id {
                    knob = knob.id(id.with(i));
                }
                child.add(knob)
            })
            .collect()
    }
}

/// Recreate mode's bar (placeholder): difficulty, audition, timer, live scores, actions.
pub struct ChallengeBar<'a> {
    theme: &'a Theme,
    difficulty: (usize, usize),
    timer: &'a str,
    scores: [(&'a str, u8, Score); 2],
}

impl<'a> ChallengeBar<'a> {
    pub fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            difficulty: (3, 5),
            timer: "02:41",
            scores: [("sound", 72, Score::Near), ("structure", 58, Score::Bad)],
        }
    }

    /// `level` filled dots out of `of`.
    #[must_use]
    pub fn difficulty(mut self, level: usize, of: usize) -> Self {
        self.difficulty = (level, of);
        self
    }

    #[must_use]
    pub fn timer(mut self, timer: &'a str) -> Self {
        self.timer = timer;
        self
    }

    #[must_use]
    pub fn scores(mut self, sound: (u8, Score), structure: (u8, Score)) -> Self {
        self.scores = [
            ("sound", sound.0, sound.1),
            ("structure", structure.0, structure.1),
        ];
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let t = self.theme;
        let c = &t.color;
        let rect = band(ui, t, t.size.bar.macros);
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(t.space.pad, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = t.space.gap;
        ui.label(
            egui::RichText::new("Recreate")
                .font(t.text.title.clone())
                .color(c.text.primary),
        );
        let (level, of) = self.difficulty;
        let dots: String = (0..of)
            .map(|i| {
                if i < level {
                    glyph::DOT_ON
                } else {
                    glyph::DOT_OFF
                }
            })
            .collect();
        ui.label(
            egui::RichText::new(dots)
                .font(t.text.label.clone())
                .color(c.accent.value),
        );
        for name in [
            format!("{} Target", glyph::PLAY),
            format!("{} Mine", glyph::PLAY),
            "A/B".to_owned(),
        ] {
            let _ = ui.button(name);
        }
        ui.label(
            egui::RichText::new(self.timer)
                .font(t.text.value.clone())
                .color(c.text.primary),
        );
        for (name, value, score) in self.scores {
            ui.label(
                egui::RichText::new(name)
                    .font(t.text.label.clone())
                    .color(c.text.label),
            );
            ui.label(
                egui::RichText::new(format!("{value} {}", score.glyph()))
                    .font(t.text.value.clone())
                    .color(c.score.of(score)),
            );
        }
        let mut submit = None;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            submit = Some(ui.button("Submit"));
            let _ = ui.button("Hint");
        });
        submit.unwrap_or_else(|| ui.label(""))
    }
}

/// The help and status line at the bottom.
pub struct StatusLine<'a> {
    theme: &'a Theme,
    path: &'a str,
    value: &'a str,
    modulation: &'a str,
    right: &'a str,
}

impl<'a> StatusLine<'a> {
    pub fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            path: "",
            value: "",
            modulation: "",
            right: "",
        }
    }

    /// The hovered control ("Filter › Cutoff").
    #[must_use]
    pub fn path(mut self, path: &'a str) -> Self {
        self.path = path;
        self
    }

    #[must_use]
    pub fn value(mut self, value: &'a str) -> Self {
        self.value = value;
        self
    }

    /// Its modulations ("LFO 1 +35 %").
    #[must_use]
    pub fn modulation(mut self, modulation: &'a str) -> Self {
        self.modulation = modulation;
        self
    }

    /// Right-aligned, de-emphasised text.
    #[must_use]
    pub fn right(mut self, right: &'a str) -> Self {
        self.right = right;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let t = self.theme;
        let c = &t.color;
        let rect = band(ui, t, t.size.bar.status);
        let response = ui.interact(rect, ui.make_persistent_id("status"), Sense::hover());
        let painter = ui.painter();
        let y = rect.center().y;
        let mut x = rect.left() + t.space.pad;
        for (text, font, ink) in [
            (self.path, &t.text.label, c.text.label),
            (self.value, &t.text.value, c.text.primary),
            (self.modulation, &t.text.label, c.accent.modulation),
        ] {
            if !text.is_empty() {
                let drawn = paint::text_in(
                    ui,
                    painter,
                    pos2(x, y),
                    Align2::LEFT_CENTER,
                    text,
                    font,
                    ink,
                    (rect.right() - x).max(0.0),
                );
                x = drawn.right() + t.space.gap;
            }
        }
        painter.text(
            pos2(rect.right() - t.space.pad, y),
            Align2::RIGHT_CENTER,
            self.right,
            t.text.label.clone(),
            c.text.dimmed,
        );
        response
    }
}

/// Allocates a full-width band of `height` with the card fill and returns its rect.
fn band(ui: &mut Ui, theme: &Theme, height: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter()
        .rect_filled(rect, theme.radius.medium, theme.color.surface.card);
    rect
}
