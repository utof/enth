//! Tier 1: the raw palette and scales. Private to `theme/`: widgets never see these names, only
//! the semantic and component tokens built from them in [`super::Theme::dark`].
//!
//! Values are the starting proposals from docs/design-system.md ("Initial values").
#![allow(
    clippy::disallowed_methods,
    reason = "the palette is the one place raw colours are constructed (docs/design-system.md)"
)]

use egui::Color32;

// Neutral greys with a cool tint, darkest first.
pub(super) const GRAY_950: Color32 = Color32::from_rgb(0x14, 0x16, 0x18);
pub(super) const GRAY_900: Color32 = Color32::from_rgb(0x1b, 0x1d, 0x20);
pub(super) const GRAY_850: Color32 = Color32::from_rgb(0x26, 0x29, 0x2d);
pub(super) const GRAY_825: Color32 = Color32::from_rgb(0x2c, 0x30, 0x36);
pub(super) const GRAY_800: Color32 = Color32::from_rgb(0x30, 0x34, 0x3a);
pub(super) const GRAY_500: Color32 = Color32::from_rgb(0x5e, 0x63, 0x6b);
pub(super) const GRAY_300: Color32 = Color32::from_rgb(0xa9, 0xae, 0xb5);
pub(super) const GRAY_100: Color32 = Color32::from_rgb(0xe8, 0xea, 0xed);

// The two accents.
pub(super) const BLUE_400: Color32 = Color32::from_rgb(0x6c, 0xb4, 0xfc);
pub(super) const ORANGE_400: Color32 = Color32::from_rgb(0xf0, 0x90, 0x4f);

// Category hues (stripes and title tints only).
pub(super) const TEAL_400: Color32 = Color32::from_rgb(0x4f, 0xb3, 0xa9);
pub(super) const YELLOW_500: Color32 = Color32::from_rgb(0xd9, 0xb4, 0x4a);
pub(super) const SLATE_400: Color32 = Color32::from_rgb(0x8a, 0x93, 0xa0);
pub(super) const PURPLE_400: Color32 = Color32::from_rgb(0xa9, 0x8b, 0xd9);

// Score hues (always shown with a glyph).
pub(super) const GREEN_400: Color32 = Color32::from_rgb(0x4c, 0xc3, 0x8a);
pub(super) const AMBER_300: Color32 = Color32::from_rgb(0xe9, 0xc4, 0x6a);
pub(super) const RED_400: Color32 = Color32::from_rgb(0xe5, 0x53, 0x4b);

/// Opacity of the faint accent washes (selection, the area under a curve), out of 255.
const WASH_ALPHA: u8 = 40;
pub(super) const BLUE_WASH: Color32 =
    Color32::from_rgba_unmultiplied_const(0x6c, 0xb4, 0xfc, WASH_ALPHA);
pub(super) const ORANGE_WASH: Color32 =
    Color32::from_rgba_unmultiplied_const(0xf0, 0x90, 0x4f, WASH_ALPHA);

/// The 4 px spacing scale (plus a 2 px hairline step). `SPACE_4` is 16 px.
pub(super) const SPACE_0: f32 = 2.0;
pub(super) const SPACE_1: f32 = 4.0;
pub(super) const SPACE_2: f32 = 8.0;
pub(super) const SPACE_3: f32 = 12.0;
pub(super) const SPACE_4: f32 = 16.0;
pub(super) const SPACE_5: f32 = 24.0;
pub(super) const SPACE_6: f32 = 32.0;

/// Two radii.
pub(super) const RADIUS_SMALL: f32 = 2.0;
pub(super) const RADIUS_MEDIUM: f32 = 4.0;

/// Two stroke widths.
pub(super) const STROKE_THIN: f32 = 1.0;
pub(super) const STROKE_THICK: f32 = 3.0;

/// Three text sizes.
pub(super) const TEXT_SMALL: f32 = 11.0;
pub(super) const TEXT_MEDIUM: f32 = 12.0;
pub(super) const TEXT_LARGE: f32 = 14.0;

/// Repaint rates (Hz).
pub(super) const RATE_ANIMATED: f32 = 30.0;
pub(super) const RATE_SCOPE: f32 = 60.0;
