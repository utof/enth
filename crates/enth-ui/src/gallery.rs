//! The token gallery: every colour, text style and scale, and every component, on one page. The
//! snapshot test renders it to `tests/snapshots/token_gallery.png`, so a token change shows up as
//! an image diff; the dev window can show it too.

use egui::{Align2, Rect, Sense, Ui, pos2, vec2};

use crate::theme::{Category, Score, Theme, hex};
use crate::widgets::{
    Block, ChallengeBar, Display, Emphasis, Ink, Knob, LessonPane, Macro, MacroBar, MatrixGrid,
    ModuleCard, NumberField, Segmented, SourceTile, Span, StatusLine, Trace,
};

/// Window size the gallery is laid out for.
pub const GALLERY_SIZE: egui::Vec2 = egui::vec2(1100.0, 1320.0);

/// Widget values shown in the gallery (they can be dragged in the dev window).
pub struct GalleryState {
    knobs: [f32; 8],
    mode: usize,
    semitones: f64,
    voices: f64,
    enabled: [bool; 2],
    matrix: [f32; 12],
    macros: [f32; 4],
    wave: Vec<f32>,
    scope: Vec<f32>,
    spectrum: Vec<f32>,
}

impl Default for GalleryState {
    fn default() -> Self {
        Self {
            knobs: [0.62, 0.35, 0.7, 0.5, 0.8, 0.45, 0.25, 0.55],
            mode: 2,
            semitones: -12.0,
            voices: 8.0,
            enabled: [true, false],
            matrix: [
                0.35, 0.0, 0.0, 0.0, -0.2, 0.0, 0.0, 0.0, 0.6, 0.15, 0.0, 0.0,
            ],
            macros: [0.7, 0.3, 0.5, 0.0],
            wave: (0..128).map(|i| saw(i as f32 / 128.0)).collect(),
            scope: (0..2048).map(|i| demo_signal(i as f32 / 2048.0)).collect(),
            spectrum: (0..512).map(|i| demo_spectrum(i as f32 / 512.0)).collect(),
        }
    }
}

/// A band-limited saw (8 harmonics), −1..1, for display only.
pub fn saw(x: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (1..=8)
        .map(|k| (tau * k as f32 * x).sin() / k as f32)
        .sum::<f32>()
        * 0.55
}

fn demo_signal(x: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    0.6 * (tau * 6.0 * x).sin() + 0.25 * (tau * 19.0 * x).sin()
}

fn demo_spectrum(x: f32) -> f32 {
    let peak = |at: f32, width: f32| (-(x - at).powi(2) / width).exp();
    (0.8 * peak(0.02, 0.0001) + 0.5 * peak(0.06, 0.0003) + 0.3 * peak(0.15, 0.002)).min(1.0)
}

/// An ADSR-like shape, 0..1, for display only.
pub fn envelope(x: f32) -> f32 {
    match x {
        x if x < 0.1 => x / 0.1,
        x if x < 0.3 => 1.0 - (x - 0.1) / 0.2 * 0.35,
        x if x < 0.75 => 0.65,
        x => 0.65 * (1.0 - (x - 0.75) / 0.25),
    }
}

/// A sine LFO shape, 0..1.
pub fn sine_shape(x: f32) -> f32 {
    0.5 + 0.5 * (std::f32::consts::TAU * x).sin()
}

/// Shows the whole gallery.
pub fn token_gallery(ui: &mut Ui, theme: &Theme, state: &mut GalleryState) {
    let t = theme;
    egui::Frame::new()
        .fill(t.color.surface.background)
        .inner_margin(t.space.section)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(t.space.gap, t.space.gap);
            heading(ui, t, "Colour");
            colours(ui, t);
            heading(ui, t, "Type and scales");
            ui.horizontal_top(|ui| {
                type_styles(ui, t);
                ui.add_space(t.space.wide);
                scales(ui, t);
            });
            heading(ui, t, "Controls");
            controls(ui, t, state);
            heading(ui, t, "Displays and cards");
            displays(ui, t, state);
            cards(ui, t, state);
            heading(ui, t, "Bands, matrix and lesson");
            bands(ui, t, state);
        });
}

fn heading(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add_space(t.space.tight);
    ui.label(
        egui::RichText::new(text)
            .font(t.text.title.clone())
            .color(t.color.text.primary),
    );
}

fn colours(ui: &mut Ui, t: &Theme) {
    let swatch = vec2(t.size.card.display_width, t.size.control_height);
    let per_row = 7;
    for row in t.color.named().chunks(per_row) {
        ui.horizontal_top(|ui| {
            for &(name, color) in row {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = t.space.hairline;
                    let (rect, _) = ui.allocate_exact_size(swatch, Sense::hover());
                    // Washes are translucent: show them over the card they are used on.
                    ui.painter()
                        .rect_filled(rect, t.radius.small, t.color.surface.card);
                    ui.painter().rect_filled(rect, t.radius.small, color);
                    ui.painter().rect_stroke(
                        rect,
                        t.radius.small,
                        t.stroke.thin(t.color.line.grid),
                        egui::StrokeKind::Inside,
                    );
                    ui.label(
                        egui::RichText::new(name)
                            .font(t.text.label.clone())
                            .color(t.color.text.label),
                    );
                    ui.label(
                        egui::RichText::new(hex::to_hex(color))
                            .font(t.text.value.clone())
                            .color(t.color.text.primary),
                    );
                });
            }
        });
    }
}

fn type_styles(ui: &mut Ui, t: &Theme) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = t.space.tight;
        for (name, font) in t.text.named() {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(
                    vec2(t.size.matrix.target_width, font.size),
                    Sense::hover(),
                );
                ui.painter().text(
                    rect.left_center(),
                    Align2::LEFT_CENTER,
                    format!("{name} {}", font.size),
                    t.text.label.clone(),
                    t.color.text.label,
                );
                ui.label(
                    egui::RichText::new("Cutoff 1.20 kHz −12.5 dB 0123456789")
                        .font(font.clone())
                        .color(t.color.text.primary),
                );
            });
        }
        ui.horizontal(|ui| {
            for (text, ink) in [
                ("primary", t.color.text.primary),
                ("label", t.color.text.label),
                ("dimmed", t.color.text.dimmed),
            ] {
                ui.label(
                    egui::RichText::new(text)
                        .font(t.text.body.clone())
                        .color(ink),
                );
            }
        });
    });
}

fn scales(ui: &mut Ui, t: &Theme) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = t.space.hairline;
        for (name, value) in t.space.named() {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(
                    vec2(t.size.tile.width, t.size.tile.badge),
                    Sense::hover(),
                );
                ui.painter().text(
                    rect.left_center(),
                    Align2::LEFT_CENTER,
                    format!("{name} {value}"),
                    t.text.label.clone(),
                    t.color.text.label,
                );
                let (bar, _) =
                    ui.allocate_exact_size(vec2(value, t.size.tile.badge), Sense::hover());
                ui.painter().rect_filled(bar, 0.0, t.color.accent.value);
            });
        }
        ui.horizontal(|ui| {
            for (name, radius) in [("small", t.radius.small), ("medium", t.radius.medium)] {
                let (rect, _) = ui.allocate_exact_size(
                    vec2(t.size.control_height, t.size.control_height),
                    Sense::hover(),
                );
                ui.painter()
                    .rect_filled(rect, radius, t.color.surface.raised);
                ui.label(
                    egui::RichText::new(format!("radius {name}"))
                        .font(t.text.label.clone())
                        .color(t.color.text.label),
                );
            }
            for (name, stroke) in [
                ("thin", t.stroke.thin(t.color.text.primary)),
                ("thick", t.stroke.thick(t.color.text.primary)),
            ] {
                let (rect, _) = ui.allocate_exact_size(
                    vec2(t.size.control_height, t.size.control_height),
                    Sense::hover(),
                );
                ui.painter().hline(rect.x_range(), rect.center().y, stroke);
                ui.label(
                    egui::RichText::new(format!("stroke {name}"))
                        .font(t.text.label.clone())
                        .color(t.color.text.label),
                );
            }
        });
    });
}

fn controls(ui: &mut Ui, t: &Theme, s: &mut GalleryState) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = t.space.tight;
        let [a, b, c, d, e, f, g, h] = &mut s.knobs;
        ui.add(Knob::new(t, a, "Plain").value_text("1.20 kHz"));
        ui.add(
            Knob::new(t, b, "Modulated")
                .modulated(0.6)
                .value_text("35 %"),
        );
        ui.add(
            Knob::new(t, c, "Bipolar")
                .bipolar(true)
                .value_text("+40 ct"),
        );
        ui.add(Knob::new(t, d, "Good").score(Some(Score::Good)));
        ui.add(Knob::new(t, e, "Near").score(Some(Score::Near)));
        ui.add(Knob::new(t, f, "Bad").score(Some(Score::Bad)));
        ui.add(Knob::new(t, g, "Dimmed").emphasis(Emphasis::Dimmed));
        ui.add_space(t.space.wide);
        ui.add(
            Knob::new(t, h, "Macro 1")
                .value_text("55 %")
                .inline(t.size.knob.cell_width * 2.0),
        );
        ui.add_space(t.space.wide);
        ui.vertical(|ui| {
            ui.add(Segmented::new(
                t,
                "gallery-mode",
                &mut s.mode,
                &["Play", "Recreate", "Learn", "From sound"],
            ));
            ui.horizontal(|ui| {
                ui.add(
                    NumberField::new(t, &mut s.semitones, "Semi")
                        .range(-48.0..=48.0)
                        .suffix(" st"),
                );
                ui.add(NumberField::new(t, &mut s.voices, "Voices").range(1.0..=16.0));
                let _ = ui.button("Button");
            });
        });
    });
}

fn displays(ui: &mut Ui, t: &Theme, s: &GalleryState) {
    let size = vec2(t.size.card.display_width * 1.5, t.size.tile.height);
    ui.horizontal(|ui| {
        ui.add(Display::new(t, Trace::Wave(&s.wave), size).caption("Wave"));
        ui.add(
            Display::new(t, Trace::Curve(&envelope), size)
                .ink(Ink::Modulation)
                .caption("Curve"),
        );
        ui.add(
            Display::new(t, Trace::Curve(&sine_shape), size)
                .ink(Ink::Modulation)
                .marker(0.3)
                .caption("Marker"),
        );
        ui.add(Display::new(t, Trace::Scope(&s.scope), size).caption("Scope"));
        ui.add(Display::new(t, Trace::Spectrum(&s.spectrum), size).caption("Spectrum"));
    });
}

fn cards(ui: &mut Ui, t: &Theme, s: &mut GalleryState) {
    ui.horizontal_top(|ui| {
        let [on, off] = &mut s.enabled;
        let strip_width = t.size.card.display_width + 6.0 * t.size.knob.cell_width;
        ui.vertical(|ui| {
            ui.set_width(strip_width);
            let [a, b, c, d, ..] = &mut s.knobs;
            ModuleCard::new(t, "gallery-osc", "Osc 1", Category::Generator, on)
                .kind("Saw")
                .show(
                    ui,
                    |ui, rect| Display::new(t, Trace::Wave(&s.wave), rect.size()).paint(ui, rect),
                    |ui| {
                        ui.add(Knob::new(t, a, "Level"));
                        ui.add(Knob::new(t, b, "Detune").bipolar(true));
                        ui.add(Knob::new(t, c, "Shape").modulated(0.9));
                        ui.add(Knob::new(t, d, "Pan").bipolar(true));
                    },
                );
            ModuleCard::new(t, "gallery-filter", "Filter", Category::Filter, off)
                .kind("Low-pass 12")
                .show(
                    ui,
                    |ui, rect| {
                        Display::new(
                            t,
                            Trace::Curve(&|x| 0.8 / (1.0 + (x * 3.0).powi(6))),
                            rect.size(),
                        )
                        .paint(ui, rect);
                    },
                    |_| {},
                );
        });
        let [_, _, _, _, e, f, ..] = &mut s.knobs;
        let mut lane_on = true;
        ModuleCard::new(
            t,
            "gallery-reverb",
            "Reverb",
            Category::Effect,
            &mut lane_on,
        )
        .stacked(t.size.tile.height)
        .show(
            ui,
            |ui, rect| Display::new(t, Trace::Curve(&envelope), rect.size()).paint(ui, rect),
            |ui| {
                ui.add(Knob::new(t, e, "Size"));
                ui.add(Knob::new(t, f, "Mix"));
            },
        );
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                SourceTile::new(t, "gallery-lfo1", "LFO 1", &sine_shape)
                    .phase(Some(0.3))
                    .routes(3)
                    .show(ui);
                SourceTile::new(t, "gallery-env2", "Env 2", &envelope)
                    .routes(1)
                    .show(ui);
            });
            ui.horizontal(|ui| {
                SourceTile::new(t, "gallery-lfo2", "LFO 2", &sine_shape).show(ui);
                let (rect, _) = ui.allocate_exact_size(
                    vec2(t.size.tile.width, t.size.tile.height),
                    Sense::hover(),
                );
                category_legend(ui, t, rect);
            });
        });
    });
}

/// The four category stripes with their names.
fn category_legend(ui: &Ui, t: &Theme, rect: Rect) {
    let row = rect.height() / 4.0;
    for (i, (name, category)) in [
        ("Generator", Category::Generator),
        ("Filter", Category::Filter),
        ("Modulator", Category::Modulator),
        ("Effect", Category::Effect),
    ]
    .into_iter()
    .enumerate()
    {
        let y = rect.top() + row * (i as f32 + 0.5);
        ui.painter().rect_filled(
            Rect::from_center_size(
                pos2(rect.left() + t.space.tight, y),
                vec2(t.stroke.thick, row - t.space.hairline),
            ),
            0.0,
            t.color.category.of(category),
        );
        ui.painter().text(
            pos2(rect.left() + t.space.gap + t.space.tight, y),
            Align2::LEFT_CENTER,
            name,
            t.text.label.clone(),
            t.color.text.label,
        );
    }
}

fn bands(ui: &mut Ui, t: &Theme, s: &mut GalleryState) {
    let [m1, m2, m3, m4] = &mut s.macros;
    let mut macros = [
        Macro {
            name: "Bright",
            value: m1,
            value_text: "70 %".into(),
        },
        Macro {
            name: "Pluck",
            value: m2,
            value_text: "30 %".into(),
        },
        Macro {
            name: "Space",
            value: m3,
            value_text: "50 %".into(),
        },
        Macro {
            name: "Macro 4",
            value: m4,
            value_text: "0 %".into(),
        },
    ];
    MacroBar::new(t).show(ui, &mut macros);
    ChallengeBar::new(t).show(ui);
    StatusLine::new(t)
        .path("Filter › Cutoff")
        .value("1.20 kHz")
        .modulation("LFO 1 +35 %, M1 +20 %")
        .right("idle · 0 fps")
        .show(ui);
    ui.horizontal_top(|ui| {
        MatrixGrid::new(
            t,
            "gallery-matrix",
            &["Env 2", "LFO 1", "Vel"],
            &["Cutoff", "Resonance", "Osc 2 pitch", "Reverb mix"],
            &mut s.matrix,
        )
        .show(ui);
        ui.add_space(t.space.wide);
        let (rect, _) = ui.allocate_exact_size(
            vec2(t.size.pane.lesson_width, t.size.bar.modulator_strip * 1.5),
            Sense::hover(),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let blocks = [
            Block::Heading("Filters shape brightness"),
            Block::Paragraph(&[
                Span { text: "A ", strong: false },
                Span { text: "low-pass filter", strong: true },
                Span { text: " removes highs above its cutoff. Turn it down and the sound gets darker.", strong: false },
            ]),
            Block::Bullets(&["Lower the cutoff", "Raise the resonance"]),
        ];
        LessonPane::new(t, "gallery-lesson", "Lesson 3 of 12", &blocks)
            .action("Start challenge")
            .show(&mut child);
    });
}
