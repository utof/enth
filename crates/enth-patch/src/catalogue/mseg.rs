//! `mseg`: a multi-segment envelope with a fixed point count (lane CAT decides it), a curve per segment, a length
//! synced in beats or in ms, loop, one-shot and sustain-loop modes. Lane MSEG owns its DSP, `docs/dsp/mseg.md`
//! and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (12) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is S0d's placeholder**: lane CAT
//! writes the real one before F1m, when the S2a catalogue freezes. Nothing persisted depends on it yet (no golden,
//! fixture, preset or lesson names this kind).

use enth_params::{ParamSpec, Scale, Unit, params};

params! {
    /// Parameters of the `mseg` module, in spec order (a placeholder until lane CAT's table).
    pub enum MsegParam in MSEG_PARAMS {
        Length = ParamSpec::float("length", "Length", Unit::Ms, 1.0, 60_000.0, Scale::Log, 1_000.0)
            .intermediate("How long one pass through the drawn shape takes."),
    }
}
