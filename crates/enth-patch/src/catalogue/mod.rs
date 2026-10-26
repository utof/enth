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
//! | `comp` | `fx` | `comp` | global | `threshold`, `ratio`, `attack`, `release`, `knee`, `makeup`, `mix` |
//! | `voice` (built-in) | — | — | voice | `mode`, `polyphony`, `glide`, `glide_mode`, `pan`; plus `amp_env`, a module id |
//! | `master` (built-in) | — | — | global | `level` |
//!
//! # Reserved S2a kinds
//!
//! S2a (docs/phase-s2.md) appended its kinds at once (lane S0d, freeze F0m), so the order lanes merge in never sets
//! a persisted discriminant. Each is **hidden** ([`ModuleSpec::hidden`]): the Add menu, `enth modules`, the
//! generator and `lesson-coverage` never list it ([`ModuleKind::listed`]), while patches, the engine and the CLI
//! accept it by key, so the module lanes can build and test it. Lane INT-S2a clears the flag per kind once the kind
//! has its lesson. Until F1m each table is S0d's placeholder, which lane CAT replaces. The order follows the plan's
//! (D1), with the cut list's kinds last, so that if the answer code is not widened (D3) the two kinds past its
//! 16-kind cap are the first two cut (`ladder`, `delay`). **No reserved kind adds an item to an enum that generator
//! v1 or R3b's layout 1 pins** (S2 rule 1).
//!
//! | Discriminant | Kind | Section | Id prefix | Rate | Lane |
//! | --- | --- | --- | --- | --- | --- |
//! | 7 | `dist` | `voice_fx` | `dist` | voice | DST |
//! | 8 | `wt` | `generators` | `wt` | voice | WT |
//! | 9 | `ott` | `fx` | `ott` | global | OTT |
//! | 10 | `comb` | `voice_fx` | `comb` | voice | FLT |
//! | 11 | `formant` | `voice_fx` | `formant` | voice | FLT |
//! | 12 | `mseg` | `modulators` | `mseg` | voice | MSEG |
//! | 13 | `chorus` | `fx` | `chorus` | global | MOD |
//! | 14 | `phaser` | `fx` | `phaser` | global | MOD |
//! | 15 | `eq` | `fx` | `eq` | global | EQ |
//! | 16 | `delay` | `fx` | `delay` | global | DLY |
//! | 17 | `ladder` | `voice_fx` | `ladder` | voice | FLT |
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
mod chorus;
mod comb;
mod comp;
mod delay;
mod dist;
mod eq;
mod formant;
mod ladder;
mod lfo;
mod mseg;
mod noise;
mod osc;
mod ott;
mod phaser;
mod reverb;
mod svf;
mod wt;

pub use adsr::{ADSR_PARAMS, AdsrParam};
pub use builtin::{
    Builtin, GLIDE_MODES, GlideMode, MACRO_VALUE, MASTER_PARAMS, MasterParam, VOICE_MODES,
    VOICE_PARAMS, VoiceMode, VoiceParam,
};
pub use chorus::{CHORUS_PARAMS, ChorusParam};
pub use comb::{COMB_PARAMS, CombParam};
pub use comp::{COMP_PARAMS, CompParam};
pub use delay::{DELAY_PARAMS, DelayParam};
pub use dist::{DIST_PARAMS, DistParam};
pub use eq::{EQ_PARAMS, EqParam};
pub use formant::{FORMANT_PARAMS, FormantParam};
pub use ladder::{LADDER_PARAMS, LadderParam};
pub use lfo::{
    LFO_PARAMS, LFO_SCOPES, LFO_SHAPES, LfoParam, LfoScope, LfoShape, POLARITIES, Polarity,
};
pub use mseg::{MSEG_PARAMS, MsegParam};
pub use noise::{NOISE_COLORS, NOISE_PARAMS, NoiseColor, NoiseParam};
pub use osc::{OSC_PARAMS, OscParam, WAVES, WAVETABLES, Wave, Wavetable};
pub use ott::{OTT_PARAMS, OttParam};
pub use phaser::{PHASER_PARAMS, PhaserParam};
pub use reverb::{REVERB_PARAMS, ReverbParam};
pub use svf::{FILTER_MODES, FilterMode, SVF_PARAMS, SvfParam};
pub use wt::{WT_PARAMS, WtParam};

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

enum_items! {
    /// A tempo-synced length as a note division (T triplet, D dotted), **sorted by length**, so a rising modulation
    /// always lengthens it. A bar is four beats (a quarter note is one beat), whatever the host's time signature.
    /// Shared by every synced parameter (S2a: `lfo.beats` from lane TMP, `mseg` and `delay` from lane CAT), so friends
    /// see one list. Not a pinned enum, but persisted: append-only after freeze F1m. [`Beats::length`] gives each
    /// item as an exact rational number of beats.
    pub enum Beats in BEATS {
        /// 1/16 of a beat.
        SixtyFourth = ("sixty_fourth", "1/64", "A sixty-fourth note: a sixteenth of a beat, at the edge of audio rate."),
        /// 1/12 of a beat.
        ThirtySecondTriplet = ("thirty_second_triplet", "1/32T", "A thirty-second-note triplet: a twelfth of a beat."),
        /// 1/8 of a beat.
        ThirtySecond = ("thirty_second", "1/32", "A thirty-second note: an eighth of a beat."),
        /// 1/6 of a beat.
        SixteenthTriplet = ("sixteenth_triplet", "1/16T", "A sixteenth-note triplet: a sixth of a beat."),
        /// 3/16 of a beat.
        ThirtySecondDotted = ("thirty_second_dotted", "1/32D", "A dotted thirty-second note: three sixteenths of a beat."),
        /// 1/4 of a beat.
        Sixteenth = ("sixteenth", "1/16", "A sixteenth note: a quarter of a beat."),
        /// 1/3 of a beat.
        EighthTriplet = ("eighth_triplet", "1/8T", "An eighth-note triplet: a third of a beat."),
        /// 3/8 of a beat.
        SixteenthDotted = ("sixteenth_dotted", "1/16D", "A dotted sixteenth note: three eighths of a beat."),
        /// 1/2 of a beat.
        Eighth = ("eighth", "1/8", "An eighth note: half a beat."),
        /// 2/3 of a beat.
        QuarterTriplet = ("quarter_triplet", "1/4T", "A quarter-note triplet: two thirds of a beat."),
        /// 3/4 of a beat.
        EighthDotted = ("eighth_dotted", "1/8D", "A dotted eighth note: three quarters of a beat, a syncopated bounce."),
        /// 1 beat.
        Quarter = ("quarter", "1/4", "A quarter note: one beat."),
        /// 4/3 of a beat.
        HalfTriplet = ("half_triplet", "1/2T", "A half-note triplet: four thirds of a beat."),
        /// 3/2 of a beat.
        QuarterDotted = ("quarter_dotted", "1/4D", "A dotted quarter note: a beat and a half."),
        /// 2 beats.
        Half = ("half", "1/2", "A half note: two beats."),
        /// 8/3 of a beat.
        BarTriplet = ("bar_triplet", "1 bar T", "A whole-note triplet: two thirds of a bar."),
        /// 3 beats.
        HalfDotted = ("half_dotted", "1/2D", "A dotted half note: three beats."),
        /// 4 beats.
        Bar = ("bar", "1 bar", "A whole note: one bar of four beats."),
        /// 16/3 of a beat.
        TwoBarsTriplet = ("two_bars_triplet", "2 bars T", "A two-bar triplet: four thirds of a bar."),
        /// 6 beats.
        BarDotted = ("bar_dotted", "1 bar D", "A dotted whole note: a bar and a half."),
        /// 8 beats.
        TwoBars = ("two_bars", "2 bars", "Two bars: a slow swell."),
        /// 12 beats.
        TwoBarsDotted = ("two_bars_dotted", "2 bars D", "Three bars."),
        /// 16 beats.
        FourBars = ("four_bars", "4 bars", "Four bars: a whole phrase."),
        /// 32 beats.
        EightBars = ("eight_bars", "8 bars", "Eight bars: a riser or a long build."),
        /// 64 beats.
        SixteenBars = ("sixteen_bars", "16 bars", "Sixteen bars: the longest build."),
    }
}

impl Beats {
    /// The length as a rational number of beats, `(numerator, denominator)` in lowest terms: `(1, 1)` is a quarter
    /// note, `(3, 4)` a dotted eighth, `(2, 3)` a quarter-note triplet. Strictly increasing in the item order.
    #[must_use]
    pub const fn length(self) -> (u32, u32) {
        match self {
            Beats::SixtyFourth => (1, 16),
            Beats::ThirtySecondTriplet => (1, 12),
            Beats::ThirtySecond => (1, 8),
            Beats::SixteenthTriplet => (1, 6),
            Beats::ThirtySecondDotted => (3, 16),
            Beats::Sixteenth => (1, 4),
            Beats::EighthTriplet => (1, 3),
            Beats::SixteenthDotted => (3, 8),
            Beats::Eighth => (1, 2),
            Beats::QuarterTriplet => (2, 3),
            Beats::EighthDotted => (3, 4),
            Beats::Quarter => (1, 1),
            Beats::HalfTriplet => (4, 3),
            Beats::QuarterDotted => (3, 2),
            Beats::Half => (2, 1),
            Beats::BarTriplet => (8, 3),
            Beats::HalfDotted => (3, 1),
            Beats::Bar => (4, 1),
            Beats::TwoBarsTriplet => (16, 3),
            Beats::BarDotted => (6, 1),
            Beats::TwoBars => (8, 1),
            Beats::TwoBarsDotted => (12, 1),
            Beats::FourBars => (16, 1),
            Beats::EightBars => (32, 1),
            Beats::SixteenBars => (64, 1),
        }
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
    // S2a's reserved kinds (lane S0d, F0m): hidden until INT-S2a clears `ModuleSpec::hidden` per kind.
    /// Distortion per voice (reserved, S2a lane DST).
    Dist = 7,
    /// Wavetable oscillator with PM, AM and ring modulation from other generators (reserved, S2a lane WT).
    Wt = 8,
    /// Three-band upward and downward compressor, global (reserved, S2a lane OTT).
    Ott = 9,
    /// Keytracked comb filter per voice (reserved, S2a lane FLT).
    Comb = 10,
    /// Formant (vowel) filter per voice (reserved, S2a lane FLT).
    Formant = 11,
    /// Multi-segment envelope with a fixed point count (reserved, S2a lane MSEG).
    Mseg = 12,
    /// Stereo chorus with a flanger mode, global (reserved, S2a lane MOD).
    Chorus = 13,
    /// Stereo phaser, global (reserved, S2a lane MOD).
    Phaser = 14,
    /// Parametric equaliser on the biquad, global (reserved, S2a lane EQ).
    Eq = 15,
    /// Stereo delay, synced in beats or in ms, global (reserved, S2a lane DLY).
    Delay = 16,
    /// Four-pole ladder low-pass per voice (reserved, S2a lane FLT; first on the cut list).
    Ladder = 17,
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
    /// A reserved kind (S2a, lane S0d): the Add menu, `enth modules`, the generator and `lesson-coverage` never list
    /// it ([`ModuleKind::listed`]), and user-facing suggestions leave it out. Patches, the engine and the CLI still
    /// accept it by key. Lane INT-S2a clears it per kind once the kind has its lesson (docs/phase-s2.md, rule 6).
    pub hidden: bool,
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
        hidden: false,
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
        hidden: false,
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
        hidden: false,
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
        hidden: false,
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
        hidden: false,
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
        hidden: false,
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
        hidden: false,
    },
    ModuleSpec {
        kind: ModuleKind::Dist,
        key: "dist",
        name: "Distortion",
        id_prefix: "dist",
        section: Section::VoiceFx,
        time_based: false,
        params: &DIST_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "Per-voice distortion: clipping, wavefolding, bitcrushing and downsampling, blended with the dry signal.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Wt,
        key: "wt",
        name: "Wavetable",
        id_prefix: "wt",
        section: Section::Generators,
        time_based: false,
        params: &WT_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A wavetable oscillator whose phase or amplitude another source can modulate: FM, PM, AM and ring modulation.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Ott,
        key: "ott",
        name: "OTT",
        id_prefix: "ott",
        section: Section::Fx,
        time_based: false,
        params: &OTT_PARAMS,
        learn: LearnMeta {
            tier: Tier::Advanced,
            desc: "Three-band upward and downward compression after the voices: the loud, dense, bright bass-music sound.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Comb,
        key: "comb",
        name: "Comb filter",
        id_prefix: "comb",
        section: Section::VoiceFx,
        time_based: false,
        params: &COMB_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A keytracked comb filter per voice: metallic, resonant tones and plucked strings.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Formant,
        key: "formant",
        name: "Formant filter",
        id_prefix: "formant",
        section: Section::VoiceFx,
        time_based: false,
        params: &FORMANT_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A vowel filter per voice that morphs between vowel shapes: talking and growling basses.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Mseg,
        key: "mseg",
        name: "MSEG",
        id_prefix: "mseg",
        section: Section::Modulators,
        time_based: false,
        params: &MSEG_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A drawn multi-segment envelope or LFO, free or tempo-synced: the wobbles and rhythmic movement of bass music.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Chorus,
        key: "chorus",
        name: "Chorus",
        id_prefix: "chorus",
        section: Section::Fx,
        time_based: true,
        params: &CHORUS_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A stereo chorus or flanger after the voices: width and movement from short modulated delays.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Phaser,
        key: "phaser",
        name: "Phaser",
        id_prefix: "phaser",
        section: Section::Fx,
        time_based: true,
        params: &PHASER_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A phaser after the voices: sweeping notches from a chain of all-pass filters.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Eq,
        key: "eq",
        name: "EQ",
        id_prefix: "eq",
        section: Section::Fx,
        time_based: false,
        params: &EQ_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A parametric equaliser after the voices: boost or cut bands of the spectrum.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Delay,
        key: "delay",
        name: "Delay",
        id_prefix: "delay",
        section: Section::Fx,
        time_based: true,
        params: &DELAY_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "Echoes after the voices, timed in milliseconds or synced to the tempo in beats.",
        },
        hidden: true,
    },
    ModuleSpec {
        kind: ModuleKind::Ladder,
        key: "ladder",
        name: "Ladder filter",
        id_prefix: "ladder",
        section: Section::VoiceFx,
        time_based: false,
        params: &LADDER_PARAMS,
        learn: LearnMeta {
            tier: Tier::Intermediate,
            desc: "A Moog-style four-pole ladder low-pass per voice: a fat, resonant sweep.",
        },
        hidden: true,
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
        ModuleKind::Dist,
        ModuleKind::Wt,
        ModuleKind::Ott,
        ModuleKind::Comb,
        ModuleKind::Formant,
        ModuleKind::Mseg,
        ModuleKind::Chorus,
        ModuleKind::Phaser,
        ModuleKind::Eq,
        ModuleKind::Delay,
        ModuleKind::Ladder,
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

    /// Whether this is a reserved kind that user-facing lists leave out ([`ModuleSpec::hidden`]).
    #[inline]
    pub const fn is_hidden(self) -> bool {
        self.spec().hidden
    }

    /// Every kind the Add menu, `enth modules`, the generator, `lesson-coverage` and user-facing suggestions may
    /// list, in discriminant order: [`ModuleKind::ALL`] without the hidden (reserved) kinds.
    pub fn listed() -> impl Iterator<Item = ModuleKind> + Clone {
        ModuleKind::ALL.iter().copied().filter(|k| !k.is_hidden())
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
            // S2a's reserved kinds run at their sections' rate (an MSEG per voice: a global twin would be another
            // kind, S2 rule 4).
            ModuleKind::Osc
            | ModuleKind::Noise
            | ModuleKind::Svf
            | ModuleKind::Adsr
            | ModuleKind::Dist
            | ModuleKind::Wt
            | ModuleKind::Comb
            | ModuleKind::Formant
            | ModuleKind::Mseg
            | ModuleKind::Ladder => Rate::Voice,
            ModuleKind::Reverb
            | ModuleKind::Comp
            | ModuleKind::Ott
            | ModuleKind::Chorus
            | ModuleKind::Phaser
            | ModuleKind::Eq
            | ModuleKind::Delay => Rate::Global,
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
        let keys: Vec<&str> = ModuleKind::listed().map(ModuleKind::key).collect();
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

    /// The shared `Beats` list: lengths strictly increase in item order (a rising modulation always lengthens) and each
    /// is in lowest terms, so two items never mean the same length.
    #[test]
    fn beats_are_sorted_by_length_in_lowest_terms() {
        const fn gcd(a: u32, b: u32) -> u32 {
            if b == 0 { a } else { gcd(b, a % b) }
        }
        let all = Beats::ALL;
        for b in all {
            let (n, d) = b.length();
            assert_eq!(gcd(n, d), 1, "{} is not in lowest terms", b.key());
        }
        for w in all.windows(2) {
            let ((n0, d0), (n1, d1)) = (w[0].length(), w[1].length());
            assert!(
                u64::from(n0) * u64::from(d1) < u64::from(n1) * u64::from(d0),
                "{} is not shorter than {}",
                w[0].key(),
                w[1].key()
            );
        }
        assert_eq!(Beats::Quarter.length(), (1, 1));
        assert_eq!(BEATS.len(), 25);
    }

    /// F0m (lane S0d): every S2a kind's discriminant, key, section and id prefix are persisted from the freeze on.
    /// If this fails, a discriminant moved: put it back (append a new kind at the end instead).
    #[test]
    fn the_reserved_s2a_kinds_keep_their_discriminants() {
        let want: [(u8, &str, Section, &str); 11] = [
            (7, "dist", Section::VoiceFx, "dist"),
            (8, "wt", Section::Generators, "wt"),
            (9, "ott", Section::Fx, "ott"),
            (10, "comb", Section::VoiceFx, "comb"),
            (11, "formant", Section::VoiceFx, "formant"),
            (12, "mseg", Section::Modulators, "mseg"),
            (13, "chorus", Section::Fx, "chorus"),
            (14, "phaser", Section::Fx, "phaser"),
            (15, "eq", Section::Fx, "eq"),
            (16, "delay", Section::Fx, "delay"),
            (17, "ladder", Section::VoiceFx, "ladder"),
        ];
        for (index, key, section, prefix) in want {
            let k = ModuleKind::from_key(key).unwrap();
            assert_eq!(k as u8, index, "{key}");
            assert_eq!(k.section(), section, "{key}");
            assert_eq!(k.spec().id_prefix, prefix, "{key}");
            assert!(k.is_hidden(), "{key} is hidden until INT-S2a");
        }
        assert_eq!(ModuleKind::ALL.len(), 18);
    }

    /// The hide flag: the listed kinds are exactly P1's seven, in discriminant order, and a reserved kind's key is
    /// still accepted (patches, the engine and the CLI take it) but left out of the unknown-kind message.
    #[test]
    fn listed_kinds_leave_out_the_reserved_ones() {
        let listed: Vec<&str> = ModuleKind::listed().map(ModuleKind::key).collect();
        assert_eq!(
            listed,
            ["osc", "noise", "svf", "adsr", "lfo", "reverb", "comp"]
        );
        assert_eq!("wt".parse::<ModuleKind>(), Ok(ModuleKind::Wt));
        let msg = UnknownKind("analog".to_owned()).to_string();
        assert_eq!(
            msg,
            "unknown module kind 'analog'; the kinds are osc, noise, svf, adsr, lfo, reverb, comp"
        );
    }

    #[test]
    fn reserved_kinds_run_at_their_sections_rate() {
        for &k in ModuleKind::ALL.iter().filter(|k| k.is_hidden()) {
            let want = match k.section() {
                Section::Fx => Rate::Global,
                Section::Generators | Section::VoiceFx | Section::Modulators => Rate::Voice,
            };
            assert_eq!(k.rate(&k.defaults()), want, "{k}");
        }
    }

    #[test]
    fn only_wt_has_audio_parameters_so_far() {
        let audio: Vec<String> = CATALOGUE
            .iter()
            .flat_map(|m| {
                m.params
                    .iter()
                    .filter(|p| p.flags.audio)
                    .map(move |p| format!("{}.{}", m.key, p.key))
            })
            .collect();
        assert_eq!(audio, ["wt.pm", "wt.am"]);
    }

    #[test]
    fn param_keys_are_found_across_kinds() {
        assert_eq!(module_param_key("cutoff"), Some("cutoff"));
        assert_eq!(module_param_key("cutof"), None);
        // Builtin keys are not module keys.
        assert_eq!(module_param_key("polyphony"), None);
    }
}
