//! `ladder`: a four-pole ladder low-pass per voice (a TPT Moog ladder with a saturator); the first item on the cut
//! list. Lane FLT owns its DSP, `docs/dsp/ladder.md` and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (17) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is S0d's placeholder**: lane CAT
//! writes the real one before F1m, when the S2a catalogue freezes. Nothing persisted depends on it yet (no golden,
//! fixture, preset or lesson names this kind).

use enth_params::{ParamSpec, Scale, Unit, params};

use super::SMOOTH;

params! {
    /// Parameters of the `ladder` module, in spec order (a placeholder until lane CAT's table).
    pub enum LadderParam in LADDER_PARAMS {
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged."),
    }
}
