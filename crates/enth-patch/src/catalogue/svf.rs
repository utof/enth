//! `svf`: the Cytomic/Simper TPT state-variable filter, per voice, with a soft-saturating drive before it
//! (`x / sqrt(b + c·x²)`, an algebraic sigmoid with make-up gain, not a tanh: the level holds as drive rises, since
//! engine 0.3.0) and coefficients interpolated per sample across each 32-frame segment. Lane D2 owns the DSP and
//! `docs/dsp/svf.md`.
//!
//! Envelope and LFO sweeps go through modulation routes on `cutoff` (Log scale, so a route depth is a fixed
//! number of octaves: 10.034 % per octave).

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::SMOOTH;

enum_items! {
    /// The filter response.
    pub enum FilterMode in FILTER_MODES {
        /// Low-pass, 12 dB/octave.
        Lp = ("lp", "Low-pass", "Keeps frequencies below the cutoff: darker as the cutoff falls."),
        /// Band-pass, unity peak.
        Bp = ("bp", "Band-pass", "Keeps a band around the cutoff: thin, nasal, telephone-like."),
        /// High-pass, 12 dB/octave.
        Hp = ("hp", "High-pass", "Keeps frequencies above the cutoff: removes body and bass."),
        /// Notch (low-pass plus high-pass).
        Notch = ("notch", "Notch", "Removes a band around the cutoff: a hollow, phaser-like dip."),
    }
}

params! {
    /// Parameters of the `svf` module, in spec order.
    pub enum SvfParam in SVF_PARAMS {
        Mode = ParamSpec::enumeration("mode", "Mode", &FILTER_MODES, FilterMode::Lp.index())
            .basic("The filter response: lp, bp, hp or notch.")
            .moves("spectrum.centroid_hz"),
        Cutoff = ParamSpec::float("cutoff", "Cutoff", Unit::Hz, 20.0, 20_000.0, Scale::Log, 2_000.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("The corner frequency: lower is darker for a low-pass; route an envelope here for plucks and sweeps.")
            .moves("spectrum.centroid_hz"),
        Resonance = ParamSpec::float("resonance", "Resonance", Unit::Percent, 0.0, 100.0, Scale::Linear, 10.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Emphasis at the cutoff, from none (Q 0.5) to a sharp whistling peak (Q 25)."),
        Keytrack = ParamSpec::float("keytrack", "Key tracking", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .intermediate("How much the cutoff follows the played note: 100 % moves it an octave per octave played."),
        Drive = ParamSpec::float("drive", "Drive", Unit::Db, 0.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Gain into a soft saturator before the filter, normalised back after: adds grit and harmonics.")
            .moves("spectrum.harmonics_db"),
    }
}
