//! `delay`: a stereo delay after the voices, its time in ms or synced in beats to the render's tempo; `mix`. Lane DLY
//! owns its DSP, `docs/dsp/delay.md` and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (16) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is S0d's placeholder**: lane CAT
//! writes the real one before F1m, when the S2a catalogue freezes. Nothing persisted depends on it yet (no golden,
//! fixture, preset or lesson names this kind).

use enth_params::{ParamSpec, Scale, Unit, params};

use super::SMOOTH;

params! {
    /// Parameters of the `delay` module, in spec order (a placeholder until lane CAT's table).
    pub enum DelayParam in DELAY_PARAMS {
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged."),
    }
}
