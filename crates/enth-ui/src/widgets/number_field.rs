//! A labelled numeric field (pitch in semitones, voice count): label above, a recessed drag field
//! below with tabular digits. Drag to change, double-click to type.

use std::ops::RangeInclusive;

use egui::{DragValue, Response, Ui, Widget};

use crate::theme::Theme;

pub struct NumberField<'a> {
    theme: &'a Theme,
    value: &'a mut f64,
    label: &'a str,
    range: RangeInclusive<f64>,
    suffix: &'a str,
    decimals: usize,
    speed: f64,
}

impl<'a> NumberField<'a> {
    pub fn new(theme: &'a Theme, value: &'a mut f64, label: &'a str) -> Self {
        Self {
            theme,
            value,
            label,
            range: f64::MIN..=f64::MAX,
            suffix: "",
            decimals: 0,
            speed: 1.0,
        }
    }

    #[must_use]
    pub fn range(mut self, range: RangeInclusive<f64>) -> Self {
        self.range = range;
        self
    }

    /// Unit after the number, e.g. `" st"`.
    #[must_use]
    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = suffix;
        self
    }

    #[must_use]
    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    /// Change per point dragged.
    #[must_use]
    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }
}

impl Widget for NumberField<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = self.theme;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = t.space.hairline;
            let label = ui.label(
                egui::RichText::new(self.label)
                    .font(t.text.label.clone())
                    .color(t.color.text.label),
            );
            let visuals = &mut ui.visuals_mut().widgets;
            for state in [
                &mut visuals.inactive,
                &mut visuals.hovered,
                &mut visuals.active,
            ] {
                state.weak_bg_fill = t.color.surface.recessed;
                state.bg_fill = t.color.surface.recessed;
                state.corner_radius = t.radius.small.into();
            }
            ui.add(
                DragValue::new(self.value)
                    .range(self.range)
                    .speed(self.speed)
                    .fixed_decimals(self.decimals)
                    .suffix(self.suffix),
            )
            .labelled_by(label.id)
        })
        .inner
    }
}
