//! Serde for colours as `"#rrggbb"` (opaque) or `"#rrggbbaa"` (unmultiplied alpha), so a theme
//! override file reads like the design-system table.
#![allow(
    clippy::disallowed_methods,
    reason = "parsing a colour token is part of the theme"
)]

use egui::Color32;
use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

pub fn serialize<S: Serializer>(color: &Color32, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&to_hex(*color))
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color32, D::Error> {
    let text = String::deserialize(deserializer)?;
    parse(&text).ok_or_else(|| D::Error::custom(format!("not a #rrggbb[aa] colour: {text:?}")))
}

pub fn to_hex(color: Color32) -> String {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    if a == u8::MAX {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

pub fn parse(text: &str) -> Option<Color32> {
    let digits = text.strip_prefix('#')?;
    if !digits.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
    match digits.len() {
        6 => Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color32::from_rgba_unmultiplied(
            byte(0)?,
            byte(2)?,
            byte(4)?,
            byte(6)?,
        )),
        _ => None,
    }
}
