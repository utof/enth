//! The presentation table: how a card shows its parameters. UI data, not catalogue data: the catalogue says what a
//! parameter is, this says how it fits a 56 px knob cell or a lane card.
//!
//! - The control follows from the spec alone ([`control`]): a knob for numbers, a stepped knob for integers and
//!   long enums, a segmented control for short enums, a switch for toggles.
//! - Short labels where the catalogue's display name doesn't fit a cell ("Pulse width" → "Width"), and short item
//!   labels where an enum's names don't fit a segment or a knob's value line ("Low-pass" → "LP").
//! - The enum the header names after the title (an oscillator's wave, a filter's mode).
//!
//! A test checks that every entry names a key and an item that exist in the catalogue.

use enth_params::{EnumItem, Kind, ParamSpec};
use enth_patch::ModuleKind;

use super::view::CardKind;
use crate::Theme;
use crate::theme::Category;

/// Which control a parameter gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// A continuous knob (`Float`).
    Knob,
    /// A knob with this many steps (`Int`, or an enum longer than `size.cards.max_segments`): it snaps, and a slow
    /// drag accumulates until the next step.
    Stepped(u32),
    /// A segmented control (an enum of at most `size.cards.max_segments` items).
    Segmented,
    /// An on/off switch (`Toggle`).
    Toggle,
}

/// The control for `spec`.
pub fn control(theme: &Theme, spec: &ParamSpec) -> Control {
    match spec.kind {
        Kind::Float { .. } => Control::Knob,
        Kind::Enum(items) if items.len() <= usize::from(theme.size.cards.max_segments) => {
            Control::Segmented
        }
        Kind::Int { .. } | Kind::Enum(_) => Control::Stepped(spec.step_count().unwrap_or(1)),
        Kind::Toggle => Control::Toggle,
    }
}

/// Whether the knob's arc starts at the centre: a range symmetric around zero (semitones, cents, pan).
pub fn bipolar(spec: &ParamSpec) -> bool {
    matches!(spec.kind, Kind::Float { min, max, .. } if min < 0.0 && min == -max)
}

/// The key of a card kind in the tables below (a module kind's catalogue key, or `voice`, `master`).
pub fn kind_key(kind: CardKind) -> &'static str {
    match kind {
        CardKind::Module(k) => k.key(),
        CardKind::Voice => "voice",
        CardKind::Master => "master",
    }
}

/// The short name a card title starts with ("Osc 1", "Env 2").
pub fn short_name(kind: ModuleKind) -> &'static str {
    match kind {
        ModuleKind::Osc => "Osc",
        ModuleKind::Noise => "Noise",
        ModuleKind::Svf => "Filter",
        ModuleKind::Adsr => "Env",
        ModuleKind::Lfo => "LFO",
        ModuleKind::Reverb => "Reverb",
        ModuleKind::Comp => "Comp",
        // S2a's reserved kinds (hidden from the Add menu until INT-S2a; SKS and EDS own their cards).
        ModuleKind::Dist => "Dist",
        ModuleKind::Wt => "WT",
        ModuleKind::Ott => "OTT",
        ModuleKind::Comb => "Comb",
        ModuleKind::Formant => "Formant",
        ModuleKind::Mseg => "MSEG",
        ModuleKind::Chorus => "Chorus",
        ModuleKind::Phaser => "Phaser",
        ModuleKind::Eq => "EQ",
        ModuleKind::Delay => "Delay",
        ModuleKind::Ladder => "Ladder",
    }
}

/// The category stripe of a card. The built-ins take the neutral modulator grey.
pub fn category(kind: CardKind) -> Category {
    match kind {
        CardKind::Module(k) => match k.section() {
            enth_patch::Section::Generators => Category::Generator,
            enth_patch::Section::VoiceFx => Category::Filter,
            enth_patch::Section::Modulators => Category::Modulator,
            enth_patch::Section::Fx => Category::Effect,
        },
        CardKind::Voice | CardKind::Master => Category::Modulator,
    }
}

/// Short labels for parameters whose catalogue name is too long for a knob cell: (kind, key, label).
pub const LABELS: &[(&str, &str, &str)] = &[
    ("osc", "semi", "Semi"),
    ("osc", "pw", "Width"),
    ("osc", "sync", "Sync"),
    ("osc", "table", "Table"),
    ("osc", "phase_mode", "Start"),
    ("osc", "phase", "Phase"),
    ("svf", "resonance", "Reso"),
    ("svf", "keytrack", "Key track"),
    ("lfo", "phase_mode", "Start"),
    ("lfo", "phase", "Phase"),
    ("reverb", "room", "Room"),
    ("reverb", "early", "Early"),
    ("reverb", "low_damp", "Low damp"),
    ("reverb", "high_damp", "High damp"),
    ("voice", "mode", "Mode"),
    ("master", "level", "Level"),
];

/// Short item labels for segments and a stepped knob's value line: (kind, key, item key, label).
pub const ITEMS: &[(&str, &str, &str, &str)] = &[
    ("osc", "wave", "table", "Table"),
    ("osc", "phase_mode", "seeded", "Seed"),
    ("svf", "mode", "lp", "LP"),
    ("svf", "mode", "bp", "BP"),
    ("svf", "mode", "hp", "HP"),
    ("lfo", "shape", "saw_up", "Saw up"),
    ("lfo", "shape", "saw_down", "Saw dn"),
    ("lfo", "shape", "sample_hold", "S & H"),
    ("lfo", "shape", "smooth_random", "Smooth"),
    ("lfo", "scope", "voice", "Voice"),
    ("lfo", "phase_mode", "seeded", "Seed"),
];

/// The enum parameter a card's header names after its title (an oscillator's wave, a filter's mode), if any.
pub const HEADER_ENUM: &[(&str, &str)] = &[
    ("osc", "wave"),
    ("noise", "color"),
    ("svf", "mode"),
    ("lfo", "shape"),
    ("voice", "mode"),
];

/// A parameter's label in a cell: the short label if it has one, else the catalogue's name.
pub fn label(kind: CardKind, spec: &ParamSpec) -> &'static str {
    let k = kind_key(kind);
    LABELS
        .iter()
        .find(|(kk, key, _)| *kk == k && *key == spec.key)
        .map_or(spec.name, |(_, _, label)| label)
}

/// An enum item's short label (segments, a stepped knob's value), else its catalogue name.
pub fn item_label(kind: CardKind, spec: &ParamSpec, item: &EnumItem) -> &'static str {
    let k = kind_key(kind);
    ITEMS
        .iter()
        .find(|(kk, key, ik, _)| *kk == k && *key == spec.key && *ik == item.key)
        .map_or(item.name, |(.., label)| label)
}

/// The key of the enum the header names, if this kind has one.
pub fn header_enum(kind: CardKind) -> Option<&'static str> {
    let k = kind_key(kind);
    HEADER_ENUM
        .iter()
        .find(|(kk, _)| *kk == k)
        .map(|(_, key)| *key)
}

#[cfg(test)]
mod tests {
    use enth_params::ParamSpec;
    use enth_patch::catalogue::Builtin;

    use super::*;

    fn params_of(kind: &str) -> &'static [ParamSpec] {
        match kind {
            "voice" => Builtin::Voice.params(),
            "master" => Builtin::Master.params(),
            k => ModuleKind::from_key(k)
                .unwrap_or_else(|| panic!("{k} is no kind"))
                .params(),
        }
    }

    fn spec(kind: &str, key: &str) -> &'static ParamSpec {
        params_of(kind)
            .iter()
            .find(|p| p.key == key)
            .unwrap_or_else(|| panic!("{kind} has no parameter {key}"))
    }

    #[test]
    fn the_table_names_only_catalogue_keys_and_items() {
        for (kind, key, _) in LABELS {
            spec(kind, key);
        }
        for (kind, key, item, _) in ITEMS {
            assert!(
                spec(kind, key).enum_index(item).is_some(),
                "{kind}.{key} has no item {item}"
            );
        }
        for (kind, key) in HEADER_ENUM {
            assert!(
                spec(kind, key).enum_items().is_some(),
                "{kind}.{key} is no enum"
            );
        }
    }

    #[test]
    fn every_parameter_of_every_kind_gets_a_control_that_fits_its_spec() {
        let theme = Theme::dark();
        let kinds = ModuleKind::ALL
            .iter()
            .map(|k| k.key())
            .chain(["voice", "master"]);
        for kind in kinds {
            for p in params_of(kind) {
                match (control(&theme, p), p.kind) {
                    (Control::Knob, Kind::Float { .. })
                    | (Control::Toggle, Kind::Toggle)
                    | (Control::Stepped(_), Kind::Int { .. }) => {}
                    (Control::Segmented, Kind::Enum(items)) => {
                        assert!(items.len() <= usize::from(theme.size.cards.max_segments));
                    }
                    (Control::Stepped(steps), Kind::Enum(items)) => {
                        assert_eq!(steps as usize, items.len() - 1, "{kind}.{}", p.key);
                    }
                    (c, k) => panic!("{kind}.{}: {c:?} for {k:?}", p.key),
                }
            }
        }
    }
}
