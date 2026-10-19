//! The bundled faces (assets/fonts, OFL-1.1) and the egui font families built from them.
//!
//! - Inter Regular: labels and running text (`FontFamily::Proportional`).
//! - Inter Semibold: titles and bold words.
//! - Inter Tabular: Inter Regular with `tnum` frozen into the cmap and cut down to the
//!   characters of a value (digits, `. , : + - −`, space). egui shapes text without OpenType
//!   features, so this is how values get tabular digits. Other characters in a value (units)
//!   fall back to Inter Regular. `assets/fonts/freeze_tabular.py` rebuilds it.
#![allow(
    clippy::disallowed_methods,
    reason = "font ids are built here, from the size tokens"
)]

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontId};

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
const INTER_TABULAR: &[u8] = include_bytes!("../../assets/fonts/InterTabular-Regular.ttf");

const REGULAR: &str = "Inter Regular";
const SEMIBOLD: &str = "Inter SemiBold";
const TABULAR: &str = "Inter Tabular";

/// Family name of the semibold face.
const SEMIBOLD_FAMILY: &str = "enth-semibold";
/// Family name of the tabular face.
const TABULAR_FAMILY: &str = "enth-tabular";

pub(super) fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub(super) fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD_FAMILY.into()))
}

pub(super) fn tabular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(TABULAR_FAMILY.into()))
}

/// The complete font set: three files, no egui default fonts.
pub(super) fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    for (name, bytes) in [
        (REGULAR, INTER_REGULAR),
        (SEMIBOLD, INTER_SEMIBOLD),
        (TABULAR, INTER_TABULAR),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let chain = |names: &[&str]| names.iter().map(|&n| n.to_owned()).collect::<Vec<_>>();
    fonts
        .families
        .insert(FontFamily::Proportional, chain(&[REGULAR]));
    fonts.families.insert(
        FontFamily::Name(SEMIBOLD_FAMILY.into()),
        chain(&[SEMIBOLD, REGULAR]),
    );
    // egui's Monospace family (code, its own numeric widgets) uses the tabular face too.
    for family in [
        FontFamily::Name(TABULAR_FAMILY.into()),
        FontFamily::Monospace,
    ] {
        fonts.families.insert(family, chain(&[TABULAR, REGULAR]));
    }
    fonts
}
