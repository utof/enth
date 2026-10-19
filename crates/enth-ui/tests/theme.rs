//! The design-system rules that can be checked without pixels: contrast, scales, fonts, serde.

use egui::{Color32, FontId};
use enth_ui::Theme;
use enth_ui::theme::glyph;

/// WCAG relative luminance of an opaque sRGB colour.
fn luminance(color: Color32) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

fn contrast(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[test]
fn text_meets_the_contrast_rules() {
    let c = Theme::dark().color;
    for (surface_name, surface) in [
        ("card", c.surface.card),
        ("background", c.surface.background),
        ("recessed", c.surface.recessed),
    ] {
        let label = contrast(c.text.label, surface);
        let value = contrast(c.text.primary, surface);
        assert!(
            label >= 4.5,
            "text.label on {surface_name}: {label:.2}:1 < 4.5:1"
        );
        assert!(
            value >= 7.0,
            "text.primary on {surface_name}: {value:.2}:1 < 7:1"
        );
    }
    // Raised surfaces carry values while hovered.
    let raised = contrast(c.text.primary, c.surface.raised);
    assert!(raised >= 7.0, "text.primary on raised: {raised:.2}:1");
}

#[test]
fn graphics_stand_out_from_their_surfaces() {
    let c = Theme::dark().color;
    // Non-text contrast (WCAG 1.4.11): arcs, stripes, rings and score glyphs ≥ 3:1 on a card.
    for (name, color) in [
        ("accent.value", c.accent.value),
        ("accent.modulation", c.accent.modulation),
        ("category.generator", c.category.generator),
        ("category.filter", c.category.filter),
        ("category.modulator", c.category.modulator),
        ("category.effect", c.category.effect),
        ("score.good", c.score.good),
        ("score.near", c.score.near),
        ("score.bad", c.score.bad),
    ] {
        let ratio = contrast(color, c.surface.card);
        assert!(ratio >= 3.0, "{name} on card: {ratio:.2}:1 < 3:1");
    }
}

#[test]
fn scales_stay_small() {
    let t = Theme::dark();
    for (name, value) in t.space.named() {
        let on_grid = value % 4.0 == 0.0 || name == "hairline";
        assert!(on_grid, "space.{name} = {value} is off the 4 px grid");
    }
    let mut sizes: Vec<u32> = t
        .text
        .named()
        .iter()
        .map(|(_, f)| f.size.to_bits())
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    assert_eq!(sizes.len(), 3, "the type scale has exactly three sizes");
    assert_ne!(t.radius.small, t.radius.medium);
    assert_ne!(t.stroke.thin, t.stroke.thick);
    assert!(t.motion.animated_hz <= t.motion.scope_hz);
}

#[test]
fn theme_round_trips_through_json_with_hex_colours() {
    let theme = Theme::dark();
    let json = serde_json::to_string_pretty(&theme).unwrap();
    assert!(json.contains("\"background\": \"#1b1d20\""), "{json}");
    // Translucent tokens keep their alpha (#rrggbbaa); egui stores them premultiplied, so the
    // colour digits of a faint wash may drift by a step, but the value survives the round trip.
    assert!(json.contains("\"value_wash\": \"#6cb"), "{json}");
    let back: Theme = serde_json::from_str(&json).unwrap();
    assert_eq!(back, theme);
}

/// Runs `f` with the theme's fonts loaded.
fn with_fonts<R>(f: impl FnOnce(&mut egui::epaint::text::FontsView<'_>, &Theme) -> R) -> R {
    let theme = Theme::dark();
    let ctx = egui::Context::default();
    theme.apply(&ctx);
    let mut out = None;
    let mut f = Some(f);
    ctx.run_ui(egui::RawInput::default(), |ui| {
        if let Some(f) = f.take() {
            out = Some(ui.ctx().fonts_mut(|fonts| f(fonts, &theme)));
        }
    })
    .drop_without_applying_deltas();
    out.unwrap()
}

#[test]
fn bundled_fonts_cover_every_glyph_we_draw() {
    let symbols = [
        glyph::GOOD,
        glyph::NEAR,
        glyph::BAD,
        glyph::HANDLE,
        glyph::PLAY,
        glyph::PATH,
        glyph::SEND,
        glyph::BULLET,
        glyph::DOT_ON,
        glyph::DOT_OFF,
    ]
    .concat();
    let text = "AaZz 0123456789 .,:;+-−%/·×±→ kHz dB ms ct st";
    with_fonts(|fonts, t| {
        for (name, font) in t.text.named() {
            for s in [symbols.as_str(), text] {
                assert!(
                    fonts.has_glyphs(font, s),
                    "text.{name} lacks a glyph in {s:?}"
                );
            }
        }
    });
}

#[test]
fn values_have_tabular_digits() {
    with_fonts(|fonts, t| {
        let mut widths = |font: &FontId| -> Vec<f32> {
            "0123456789"
                .chars()
                .map(|c| fonts.glyph_width(font, c))
                .collect()
        };
        let value = widths(&t.text.value);
        assert!(
            value.iter().all(|w| (w - value[0]).abs() < 1e-3),
            "text.value digits differ in width: {value:?}"
        );
        // Sanity: the regular face's digits are proportional, so the check above means something.
        let label = widths(&t.text.label);
        assert!(label.iter().any(|w| (w - label[0]).abs() > 1e-3));
    });
}
