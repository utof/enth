//! `wt`: a wavetable oscillator on phase-read waves: generated tables, phase warps, phase and amplitude modulation from
//! another generator through audio routes (`pm`, `am`; bipolar AM is ring modulation), `feedback`, and later unison on
//! `wide` lanes (lane UNI). Lane WT owns its DSP, `docs/dsp/wt.md` and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (8) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is S0d's placeholder**: lane CAT
//! writes the real one before F1m, when the S2a catalogue freezes. Nothing persisted depends on it yet (no golden,
//! fixture, preset or lesson names this kind).

use enth_params::{ParamSpec, Scale, Unit, params};

use super::SMOOTH;

params! {
    /// Parameters of the `wt` module, in spec order (a placeholder until lane CAT's table).
    pub enum WtParam in WT_PARAMS {
        Level = ParamSpec::float("level", "Level", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Output level of this oscillator into the voice, as a linear percentage."),
        Pm = ParamSpec::float("pm", "PM", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .audio()
            .advanced("Phase-modulation input: an audio route from another generator lands here, and its depth sets the index."),
        Am = ParamSpec::float("am", "AM", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .audio()
            .advanced("Amplitude-modulation input (bipolar is ring modulation): an audio route from another generator lands here."),
    }
}
