//! The built-in pseudo-modules `voice` and `master`, and the macro knob.
//!
//! `voice` and `master` are not module instances (they have no id, cannot be added, removed or disabled), but
//! their parameters are ordinary addresses: `voice.glide`, `master.level` (design.md §2). Voices are mono: the
//! generator sum runs through `voice_fx` and the amp stage, then `voice.pan` places each voice in the stereo bus;
//! `fx` are stereo. There is no unison in P1.

use enth_params::{ParamSpec, Scale, Smoothing, Unit, enum_items, params};

use super::SMOOTH;

/// A built-in pseudo-module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Builtin {
    /// The voice manager and the amp stage: `voice.*` (per voice).
    Voice,
    /// The output stage after the effects: `master.*` (global).
    Master,
}

impl Builtin {
    /// Both built-ins.
    pub const ALL: [Builtin; 2] = [Builtin::Voice, Builtin::Master];

    /// The reserved word used in addresses (`"voice"`, `"master"`).
    pub const fn key(self) -> &'static str {
        match self {
            Builtin::Voice => "voice",
            Builtin::Master => "master",
        }
    }

    /// The parameter table, in spec order.
    pub const fn params(self) -> &'static [ParamSpec] {
        match self {
            Builtin::Voice => &VOICE_PARAMS,
            Builtin::Master => &MASTER_PARAMS,
        }
    }

    /// The index of the parameter with this key.
    pub fn param_index(self, key: &str) -> Option<usize> {
        self.params().iter().position(|p| p.key == key)
    }
}

enum_items! {
    /// How notes are assigned to voices.
    pub enum VoiceMode in VOICE_MODES {
        /// Up to `polyphony` notes at once.
        Poly = ("poly", "Poly", "Each note gets its own voice, up to the polyphony limit; chords work."),
        /// One voice; every new note retriggers the envelopes.
        Mono = ("mono", "Mono", "One note at a time; each new note restarts the envelopes."),
        /// One voice; overlapping notes change pitch without retriggering.
        Legato = ("legato", "Legato", "One note at a time; overlapping notes slide the pitch without restarting the envelopes."),
    }
}

enum_items! {
    /// When glide applies.
    pub enum GlideMode in GLIDE_MODES {
        /// Every new note glides from the previous pitch.
        Always = ("always", "Always", "Every new note glides from the previous note's pitch."),
        /// Only notes played while another is held glide.
        Legato = ("legato", "Legato", "Only notes played while another is still held glide."),
    }
}

params! {
    /// Parameters of the built-in `voice`, in spec order. `voice.amp_env` (which ADSR is the amplitude envelope)
    /// is a module reference, not a parameter: it lives in `VoiceSettings::amp_env`.
    pub enum VoiceParam in VOICE_PARAMS {
        Mode = ParamSpec::enumeration("mode", "Voice mode", &VOICE_MODES, VoiceMode::Poly.index())
            .structural()
            .basic("poly plays chords; mono and legato play one note at a time (legato slides without retriggering)."),
        Polyphony = ParamSpec::int("polyphony", "Polyphony", Unit::None, 1, 16, 8)
            .structural()
            .intermediate("How many notes can sound at once in poly mode; extra notes steal the oldest voice."),
        Glide = ParamSpec::float("glide", "Glide", Unit::Ms, 0.0, 10_000.0, Scale::Power(4.0), 0.0)
            .basic("Time to slide from the previous note's pitch to the new one; 0 is off. Linear in semitones."),
        GlideMode = ParamSpec::enumeration("glide_mode", "Glide mode", &GLIDE_MODES, GlideMode::Always.index())
            .intermediate("Glide on every note, or only on notes played legato."),
        Pan = ParamSpec::float("pan", "Pan", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Where each voice sits in the stereo field, -100 % left to +100 % right; route key or an LFO here to spread notes.")
            .moves("stereo.width"),
    }
}

params! {
    /// Parameters of the built-in `master`, in spec order.
    pub enum MasterParam in MASTER_PARAMS {
        Level = ParamSpec::float("level", "Master level", Unit::Db, -36.0, 12.0, Scale::Linear, 0.0)
            .smoothing(Smoothing::Linear { ms: 20.0 })
            .basic("Output level after the effects; turn it down to fix clipping without retuning anything else.")
            .moves("loudness.peak_dbfs"),
    }
}

/// The spec of a macro knob's value: 0–1, the knob position (macros are 1-based in text, 0-based in `Op`s). A
/// macro is a global modulation source; the host sees the eight macros as automatable parameters.
pub const MACRO_VALUE: ParamSpec =
    ParamSpec::float("value", "Macro", Unit::None, 0.0, 1.0, Scale::Linear, 0.0)
        .smoothing(SMOOTH)
        .basic("The macro knob's position, 0 to 1; routes from the macro scale it by their depth.");

const _: () = enth_params::check_table("MACRO_VALUE", &[MACRO_VALUE], &[]);
