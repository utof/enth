//! The module catalogue: every P1a module kind with its `ParamSpec`s, plus the built-in `voice` and `master`
//! pseudo-modules, macros and route depth.
//!
//! One file per kind (`osc.rs`, `svf.rs`, …) holds that kind's parameter table, its index enum and the Rust enums
//! of its enum parameters, all declared with [`enth_params::params!`] / [`enth_params::enum_items!`] and checked at
//! compile time. A DSP lane that must change a range edits only its own kind's file (and the catalogue snapshot,
//! plus a migration if a persisted meaning changes; see below).
//!
//! **The catalogue is a persisted contract.** Patches store plain values, densely, in spec order, and route depths
//! are percentages of a destination's normalised range. So renaming a key, changing a unit, range or scale,
//! reordering a table, or changing a default needs a schema migration (design.md §2); adding a parameter needs a
//! default that reproduces the old sound. The insta snapshot `tests/snapshots/catalogue__catalogue.snap` makes every
//! such change visible in review. Module kinds and their indices are stable too: append new kinds at the end.
//!
//! | Kind | Section | Id prefix | Rate | Parameters, in spec order |
//! | --- | --- | --- | --- | --- |
//! | `osc` | `generators` | `osc` | voice | `wave`, `semi`, `fine`, `level`, `pw`, `sync`, `table`, `position`, `phase_mode`, `phase`, `seed` |
//! | `noise` | `generators` | `noise` | voice | `color`, `level` |
//! | `svf` | `voice_fx` | `filter` | voice | `mode`, `cutoff`, `resonance`, `keytrack`, `drive` |
//! | `adsr` | `modulators` | `env` | voice | `attack`, `decay`, `sustain`, `release`, `velocity` |
//! | `lfo` | `modulators` | `lfo` | voice or global (`scope`) | `shape`, `rate`, `polarity`, `scope`, `phase_mode`, `phase`, `seed` |
//! | `reverb` | `fx` | `reverb` | global | `dry`, `wet`, `room`, `decay`, `early`, `detune`, `low_cut`, `high_cut`, `low_damp`, `high_damp` |
//! | `comp` | `fx` | `comp` | global | `threshold`, `ratio`, `attack`, `release`, `knee`, `makeup` |
//! | `voice` (built-in) | — | — | voice | `mode`, `polyphony`, `glide`, `glide_mode`, `pan`; plus `amp_env`, a module id |
//! | `master` (built-in) | — | — | global | `level` |
//!
//! # Checked at compile time
//!
//! Each table is declared with `params!`, which evaluates [`enth_params::check_table`] in a `const` item. So this
//! table, written the way the kind files are, compiles:
//!
//! ```
//! use enth_params::{ParamSpec, Scale, Unit, params};
//! params! {
//!     pub enum MyFilterParam in MY_FILTER_PARAMS {
//!         Cutoff = ParamSpec::float("cutoff", "Cutoff", Unit::Hz, 20.0, 20_000.0, Scale::Log, 2_000.0)
//!             .modulatable()
//!             .basic("The corner frequency."),
//!     }
//! }
//! assert_eq!(MyFilterParam::Cutoff.spec().default, 2_000.0);
//! ```
//!
//! and the same table with a default below its range does not (`error[E0080]: evaluation panicked:
//! MY_FILTER_PARAMS.cutoff: default out of range`):
//!
//! ```compile_fail,E0080
//! use enth_params::{ParamSpec, Scale, Unit, params};
//! params! {
//!     pub enum MyFilterParam in MY_FILTER_PARAMS {
//!         Cutoff = ParamSpec::float("cutoff", "Cutoff", Unit::Hz, 20.0, 20_000.0, Scale::Log, 10.0)
//!             .modulatable()
//!             .basic("The corner frequency."),
//!     }
//! }
//! ```
//!
//! The module table itself ([`CATALOGUE`]) is checked the same way: one spec per kind in discriminant order,
//! valid and unique keys and id prefixes, no reserved prefix.

use core::fmt;

use enth_params::{LearnMeta, ParamSpec, Smoothing, Tier, enum_items, is_valid_key};

use crate::model::Section;

mod adsr;
mod builtin;
mod comp;
mod lfo;
mod noise;
mod osc;
mod reverb;
mod svf;

pub use adsr::{ADSR_PARAMS, AdsrParam};
pub use builtin::{
    Builtin, GLIDE_MODES, GlideMode, MACRO_VALUE, MASTER_PARAMS, MasterParam, VOICE_MODES,
    VOICE_PARAMS, VoiceMode, VoiceParam,
};
pub use comp::{COMP_PARAMS, CompParam};
pub use lfo::{
    LFO_PARAMS, LFO_SCOPES, LFO_SHAPES, LfoParam, LfoScope, LfoShape, POLARITIES, Polarity,
};
pub use noise::{NOISE_COLORS, NOISE_PARAMS, NoiseColor, NoiseParam};
pub use osc::{OSC_PARAMS, OscParam, WAVES, WAVETABLES, Wave, Wavetable};
pub use reverb::{REVERB_PARAMS, ReverbParam};
pub use svf::{FILTER_MODES, FilterMode, SVF_PARAMS, SvfParam};

/// Smoothing for gains, mixes and other continuous controls on the audio path.
pub(crate) const SMOOTH: Smoothing = Smoothing::Linear { ms: 10.0 };

enum_items! {
    /// Where a module's start phase (and, for `seeded`, all its randomness) comes from at each note-on. Shared by
    /// `osc` and `lfo`. At 0 ct a phase *difference* between two oscillators is a timbre, so it must be explicit
    /// (design.md §2); the descriptors' noise estimate re-renders with another render seed, which moves `free`
    /// phases and nothing else.
    pub enum PhaseMode in PHASE_MODES {
        /// A start phase drawn per note from (render seed, module id, note index): like a free-running analog
        /// oscillator. A different render seed moves it.
        Free = ("free", "Free", "Each note starts at a random phase that changes with the render seed, like a free-running analog oscillator."),
        /// Every note starts at `phase`.
        Reset = ("reset", "Reset", "Every note starts at the phase knob, so attacks are identical and phase relationships are fixed."),
        /// A start phase drawn per note from the module's own `seed` parameter, so it is part of the patch and does
        /// not change with the render seed.
        Seeded = ("seeded", "Seeded", "Each note starts at a random phase drawn from this module's seed, so the patch itself fixes it."),
    }
}

/// Whether a module, source or destination runs once per voice or once for the whole engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rate {
    /// Per voice (generators, `voice_fx`, envelopes, voice LFOs, velocity, key, `voice.*`).
    Voice,
    /// Once for all voices (`fx`, global LFOs, macros, mod wheel, `master.*`).
    Global,
}

/// A module kind. The discriminant is stable (it is hashed by `canonical_hash`): append new kinds at the end.
///
/// The engine's `ModuleImpl::new` is an exhaustive `match` on this enum, so a kind without an implementation is a
/// compile error (architecture.md, rule 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ModuleKind {
    /// Oscillator: classic waves (Plaits variable-shape core with polyBLEP), hard sync, and a wavetable mode.
    Osc = 0,
    /// White or pink noise.
    Noise = 1,
    /// State-variable filter (Cytomic/Simper TPT): low-pass, band-pass, high-pass, notch.
    Svf = 2,
    /// ADSR envelope (RC-style exponential segments with exact segment times).
    Adsr = 3,
    /// Low-frequency oscillator, per voice or global.
    Lfo = 4,
    /// Stereo FDN reverb (port of Signalsmith Basics), global.
    Reverb = 5,
    /// Stereo compressor with soft knee and make-up gain, global.
    Comp = 6,
}

/// The descriptor of a module kind: identity, where it lives, and its parameter table.
#[derive(Clone, Copy, Debug)]
pub struct ModuleSpec {
    /// The kind this spec describes (`CATALOGUE[k as usize].kind == k`).
    pub kind: ModuleKind,
    /// Stable key used in files and the CLI (`"svf"`). Renaming one is a migration.
    pub key: &'static str,
    /// Display name (`"Filter"`).
    pub name: &'static str,
    /// Prefix for new module ids: `add` without an id takes the lowest free `<prefix><n>` (`filter1`), and
    /// `canonical_hash` relabels ids with it.
    pub id_prefix: &'static str,
    /// The section modules of this kind live in (in P1a each kind has exactly one).
    pub section: Section,
    /// A time-based effect (its tail outlives its input): the plugin's global FX bypass may mute it. The bypass is
    /// monitor state outside the patch, so renders and scores never see it (ADR-0006).
    pub time_based: bool,
    /// Parameters, in spec order: the order of `ModuleInstance::values`.
    pub params: &'static [ParamSpec],
    /// Tier and one plain sentence for learners and agents (`enth modules`).
    pub learn: LearnMeta,
}

/// Every module kind, indexed by `ModuleKind as usize`.
pub static CATALOGUE: [ModuleSpec; ModuleKind::ALL.len()] = [
    ModuleSpec {
        kind: ModuleKind::Osc,
        key: "osc",
        name: "Oscillator",
        id_prefix: "osc",
        section: Section::Generators,
        time_based: false,
        params: &OSC_PARAMS,
        learn: LearnMeta {
            tier: Tier::Basic,
            desc: "A pitched tone: sine, triangle, saw, pulse with width, or a wavetable, with hard sync to the first oscillator.",
        },
    },
    ModuleSpec {
        kind: ModuleKind::Noise,
        key: "noise",
        name: "Noise",
        id_prefix: "noise",
        section: Section::Generators,
        time_based: false,
        params: &NOISE_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "Unpitched hiss, white or pink, for breath, attack transients and texture.",
        },
    },
    ModuleSpec {
        kind: ModuleKind::Svf,
        key: "svf",
        name: "Filter",
        id_prefix: "filter",
        section: Section::VoiceFx,
        time_based: false,
        params: &SVF_PARAMS,
        learn: LearnMeta {
            tier: Tier::Basic,
            desc: "A resonant multimode filter per voice: shapes brightness, usually swept by an envelope.",
        },
    },
    ModuleSpec {
        kind: ModuleKind::Adsr,
        key: "adsr",
        name: "Envelope",
        id_prefix: "env",
        section: Section::Modulators,
        time_based: false,
        params: &ADSR_PARAMS,
        learn: LearnMeta {
            tier: Tier::Basic,
            desc: "An attack-decay-sustain-release envelope per note, 0 to 1; one of them is the voice's amplitude envelope.",
        },
    },
    ModuleSpec {
        kind: ModuleKind::Lfo,
        key: "lfo",
        name: "LFO",
        id_prefix: "lfo",
        section: Section::Modulators,
        time_based: false,
        params: &LFO_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A slow repeating modulator for vibrato, tremolo and sweeps, per voice or shared by all voices.",
        },
    },
    ModuleSpec {
        kind: ModuleKind::Reverb,
        key: "reverb",
        name: "Reverb",
        id_prefix: "reverb",
        section: Section::Fx,
        time_based: true,
        params: &REVERB_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A stereo room or hall after the voices: adds space, width and a tail.",
        },
    },
    ModuleSpec {
        kind: ModuleKind::Comp,
        key: "comp",
        name: "Compressor",
        id_prefix: "comp",
        section: Section::Fx,
        time_based: false,
        params: &COMP_PARAMS,
        learn: LearnMeta {
            tier: Tier::Advanced,
            desc: "Evens out loudness after the voices: turns loud parts down above a threshold, then makes up the gain.",
        },
    },
];

impl ModuleKind {
    /// Every kind, in discriminant order.
    pub const ALL: &'static [ModuleKind] = &[
        ModuleKind::Osc,
        ModuleKind::Noise,
        ModuleKind::Svf,
        ModuleKind::Adsr,
        ModuleKind::Lfo,
        ModuleKind::Reverb,
        ModuleKind::Comp,
    ];

    /// The descriptor of this kind.
    #[inline]
    pub const fn spec(self) -> &'static ModuleSpec {
        &CATALOGUE[self as usize]
    }

    /// The stable key (`"svf"`).
    #[inline]
    pub const fn key(self) -> &'static str {
        self.spec().key
    }

    /// The parameter table, in spec order.
    #[inline]
    pub const fn params(self) -> &'static [ParamSpec] {
        self.spec().params
    }

    /// The section modules of this kind live in.
    #[inline]
    pub const fn section(self) -> Section {
        self.spec().section
    }

    /// The kind with this key (exact match: keys are lowercase).
    pub fn from_key(key: &str) -> Option<ModuleKind> {
        ModuleKind::ALL.iter().copied().find(|k| k.key() == key)
    }

    /// The index of the parameter with this key.
    pub fn param_index(self, key: &str) -> Option<usize> {
        self.params().iter().position(|p| p.key == key)
    }

    /// The spec of the parameter with this key.
    pub fn param(self, key: &str) -> Option<&'static ParamSpec> {
        self.params().iter().find(|p| p.key == key)
    }

    /// Every parameter's default, in spec order: the values of a new module.
    pub fn defaults(self) -> Vec<f32> {
        self.params().iter().map(|p| p.default).collect()
    }

    /// Whether a module of this kind with these `values` runs per voice or globally: generators, `voice_fx` and
    /// envelopes are per voice, effects are global, and an LFO follows its `scope` parameter (which is
    /// structural, so the rate of a module only changes through a recompile).
    pub fn rate(self, values: &[f32]) -> Rate {
        match self {
            ModuleKind::Osc | ModuleKind::Noise | ModuleKind::Svf | ModuleKind::Adsr => Rate::Voice,
            ModuleKind::Reverb | ModuleKind::Comp => Rate::Global,
            ModuleKind::Lfo => match values
                .get(LfoParam::Scope.index())
                .copied()
                .and_then(LfoScope::from_value)
            {
                Some(LfoScope::Global) => Rate::Global,
                _ => Rate::Voice,
            },
        }
    }
}

impl fmt::Display for ModuleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl core::str::FromStr for ModuleKind {
    type Err = UnknownKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ModuleKind::from_key(s).ok_or_else(|| UnknownKind(s.to_owned()))
    }
}

/// A module kind key that is not in the catalogue. Code `unknown_kind`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownKind(pub String);

impl UnknownKind {
    /// Stable code: `unknown_kind`.
    pub const fn code(&self) -> &'static str {
        "unknown_kind"
    }
}

impl fmt::Display for UnknownKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let keys: Vec<&str> = ModuleKind::ALL.iter().map(|k| k.key()).collect();
        write!(
            f,
            "unknown module kind '{}'; the kinds are {}",
            self.0,
            keys.join(", ")
        )
    }
}

impl std::error::Error for UnknownKind {}

impl serde::Serialize for ModuleKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.key())
    }
}

impl<'de> serde::Deserialize<'de> for ModuleKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The static key of a module parameter, if any module kind has a parameter with this key. Lets an address
/// such as `filter1.cutoff` be parsed without the patch; whether `filter1`'s kind has `cutoff` is checked when
/// the address is resolved against a patch.
pub fn module_param_key(key: &str) -> Option<&'static str> {
    CATALOGUE
        .iter()
        .flat_map(|m| m.params.iter())
        .find(|p| p.key == key)
        .map(|p| p.key)
}

// ---------------------------------------------------------------------------------------------------------------
// Compile-time checks of the module table (each parameter table checks itself in its params! invocation)
// ---------------------------------------------------------------------------------------------------------------

const fn check_catalogue(c: &[ModuleSpec]) {
    assert!(
        c.len() == ModuleKind::ALL.len(),
        "CATALOGUE must list every ModuleKind"
    );
    let mut i = 0;
    while i < c.len() {
        let m = &c[i];
        assert!(
            m.kind as usize == i && ModuleKind::ALL[i] as usize == i,
            "CATALOGUE and ModuleKind::ALL must be in discriminant order"
        );
        assert!(
            is_valid_key(m.key) && is_valid_key(m.id_prefix) && m.id_prefix.len() <= 12,
            "a module key or id prefix is not a valid key (prefixes at most 12 characters)"
        );
        assert!(
            !m.name.is_empty() && !m.learn.desc.is_empty() && !m.params.is_empty(),
            "a module spec needs a name, a description and parameters"
        );
        assert!(
            !crate::addr::is_reserved_id(m.id_prefix),
            "an id prefix is a reserved word"
        );
        let mut j = i + 1;
        while j < c.len() {
            assert!(
                !enth_params::spec::str_eq(m.key, c[j].key)
                    && !enth_params::spec::str_eq(m.id_prefix, c[j].id_prefix),
                "duplicate module key or id prefix"
            );
            j += 1;
        }
        i += 1;
    }
}

const _: () = check_catalogue(&CATALOGUE);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_keys() {
        for &k in ModuleKind::ALL {
            assert_eq!(ModuleKind::from_key(k.key()), Some(k));
            assert_eq!(k.key().parse::<ModuleKind>(), Ok(k));
            assert_eq!(k.spec().kind, k);
            assert_eq!(k.defaults().len(), k.params().len());
        }
        assert_eq!(
            "analog".parse::<ModuleKind>().unwrap_err().code(),
            "unknown_kind"
        );
    }

    #[test]
    fn lfo_rate_follows_scope() {
        let mut v = ModuleKind::Lfo.defaults();
        assert_eq!(ModuleKind::Lfo.rate(&v), Rate::Voice);
        v[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
        assert_eq!(ModuleKind::Lfo.rate(&v), Rate::Global);
        assert_eq!(
            ModuleKind::Reverb.rate(&ModuleKind::Reverb.defaults()),
            Rate::Global
        );
    }

    #[test]
    fn param_keys_are_found_across_kinds() {
        assert_eq!(module_param_key("cutoff"), Some("cutoff"));
        assert_eq!(module_param_key("cutof"), None);
        // Builtin keys are not module keys.
        assert_eq!(module_param_key("polyphony"), None);
    }
}
