//! `ladder`: a four-pole ladder filter per voice (a TPT Moog ladder with one saturator at its input, report 05 §5),
//! low-pass at 24 or 12 dB/octave, or band-pass or high-pass from the stage taps; the first item on the cut list. Lane
//! FLT owns its DSP, `docs/dsp/ladder.md` and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (17) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions").
//!
//! The table mirrors `svf`'s (same keys, units, ranges and defaults where they mean the same), so a patch can swap one
//! for the other: the effective cutoff is `cutoff · 2^(keytrack · (pitch − 60) / 1200)` as in docs/dsp/svf.md;
//! `resonance` reaches self-oscillation near its top (FLT records where); `drive` is gain into the input saturator,
//! level-compensated like `svf`'s. A new `mode` (a diode ladder, say) is appended to the unpinned `mode` enum later.
//!
//! At its defaults it is **a gentle useful setting**, as a new `svf` is: a 24 dB low-pass at 2 kHz, little resonance.

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::SMOOTH;

enum_items! {
    /// The ladder's response, from its stage taps.
    pub enum LadderMode in LADDER_MODES {
        /// Four poles: 24 dB/octave.
        Lp24 = ("lp24", "Low-pass 24", "The classic ladder: a steep 24 dB/octave low-pass, fat and round."),
        /// Two poles: 12 dB/octave.
        Lp12 = ("lp12", "Low-pass 12", "A gentler 12 dB/octave low-pass with the ladder's character."),
        /// Band-pass from the taps.
        Bp = ("bp", "Band-pass", "Keeps a band around the cutoff, with the ladder's resonance: vocal and nasal."),
        /// High-pass from the taps.
        Hp = ("hp", "High-pass", "Removes the lows below the cutoff, with the ladder's resonance."),
    }
}

params! {
    /// Parameters of the `ladder` module, in spec order.
    pub enum LadderParam in LADDER_PARAMS {
        Mode = ParamSpec::enumeration("mode", "Mode", &LADDER_MODES, LadderMode::Lp24.index())
            .basic("The response: 24 or 12 dB/octave low-pass, band-pass or high-pass.")
            .moves("spectrum.centroid_hz"),
        Cutoff = ParamSpec::float("cutoff", "Cutoff", Unit::Hz, 20.0, 20_000.0, Scale::Log, 2_000.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("The corner frequency: lower is darker for a low-pass; route an envelope or LFO here for sweeps and wobbles.")
            .moves("spectrum.centroid_hz"),
        Resonance = ParamSpec::float("resonance", "Resonance", Unit::Percent, 0.0, 100.0, Scale::Linear, 10.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Emphasis at the cutoff; near the top the ladder rings on its own (self-oscillation) and the lows thin out, as on a Moog."),
        Keytrack = ParamSpec::float("keytrack", "Key tracking", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .intermediate("How much the cutoff follows the played note: 100 % moves it an octave per octave played."),
        Drive = ParamSpec::float("drive", "Drive", Unit::Db, 0.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Gain into the ladder's saturator, normalised back after: grit and a thicker, compressed tone.")
            .moves("spectrum.harmonics_db"),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged; lower values blend the dry signal back in."),
    }
}
