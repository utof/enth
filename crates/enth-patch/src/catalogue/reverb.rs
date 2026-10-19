//! `reverb`: a port of Signalsmith Basics' FDN reverb (4 diffusers, 8-channel feedback, Householder mixing,
//! Lagrange-7 detune, shelving damping), global and stereo. Lane FX1 owns the DSP and `docs/dsp/reverb.md`.
//!
//! The port fades `dry`, `wet`, `room`, `decay`, `early` and `detune` internally over 20 ms (upstream's STFX A/B
//! fade), so the engine does not smooth them (a second ramp would only add lag). `room` and `decay` are not
//! modulatable: they re-derive delay taps and filters and would restart the fade every segment (report 02). The
//! cut and damping filters are **not** faded: the port redesigns them at the start of the next block, and a large
//! jump steps the output (measured by FX1: a 12 to 3 kHz `high_cut` jump gives 7× the steady second difference).
//! So `low_cut`, `high_cut`, `low_damp` and `high_damp` get the engine's smoother, over the same 20 ms.
//!
//! Conversions into `enth_dsp::reverb::ReverbParams` (the wrapper's job; their arithmetic is an engine-version
//! matter, their meaning is this contract):
//!
//! - `decay` is the RT60 in ms; upstream's RT20 in seconds is `rt20_s = decay / 3000` (a decay that is linear in
//!   dB falls 60 dB in three times its 20 dB time);
//! - `dry` and `wet` are gains in dB: `pct = 100 · 10^(dB / 20)`, except that the minimum, −60 dB, is **off**
//!   (gain 0, as a fader at the bottom), so a fully wet or fully dry setting exists;
//! - `early` is upstream's percentage as is; `detune`, the cuts and the damping rates are upstream's values as is.

use enth_params::{ParamSpec, Scale, Smoothing, Unit, params};

/// The engine's smoothing for the four parameters the port does not fade: as long as its own fade.
const FILTERS: Smoothing = Smoothing::Linear { ms: 20.0 };

params! {
    /// Parameters of the `reverb` module, in spec order.
    pub enum ReverbParam in REVERB_PARAMS {
        Dry = ParamSpec::float("dry", "Dry", Unit::Db, -60.0, 6.0, Scale::Linear, 0.0)
            .modulatable()
            .intermediate("Level of the untouched signal passing through; -60 dB is off, for a fully wet sound.")
            .moves("loudness.lufs_i"),
        Wet = ParamSpec::float("wet", "Wet", Unit::Db, -60.0, 6.0, Scale::Linear, -6.0)
            .modulatable()
            .basic("Level of the reverb tail: more wet sounds further away; -60 dB is off.")
            .moves("stereo.width"),
        Room = ParamSpec::float("room", "Room size", Unit::Ms, 10.0, 200.0, Scale::Linear, 80.0)
            .basic("Size of the space, as a delay time (1 ms is about a foot): small rooms to large halls."),
        Decay = ParamSpec::float("decay", "Decay", Unit::Ms, 100.0, 30_000.0, Scale::Log, 3_000.0)
            .basic("How long the tail takes to fall by 60 dB (RT60).")
            .moves("envelope.release_ms"),
        Early = ParamSpec::float("early", "Early reflections", Unit::Percent, 0.0, 250.0, Scale::Linear, 150.0)
            .modulatable()
            .advanced("Level of the first reflections, which tell the ear the size of the room."),
        Detune = ParamSpec::float("detune", "Detune", Unit::None, 0.0, 50.0, Scale::Power(3.0), 2.0)
            .modulatable()
            .advanced("How much the pitch wobbles inside the tail (about cents per pass at a 1 s room): smears metallic ringing, chorus-like when high."),
        LowCut = ParamSpec::float("low_cut", "Low cut", Unit::Hz, 10.0, 500.0, Scale::Log, 80.0)
            .modulatable()
            .smoothing(FILTERS)
            .intermediate("Removes lows below this frequency from the reverb, keeping bass clean."),
        HighCut = ParamSpec::float("high_cut", "High cut", Unit::Hz, 1_000.0, 20_000.0, Scale::Log, 12_000.0)
            .modulatable()
            .smoothing(FILTERS)
            .intermediate("Removes highs above this frequency from the reverb: lower sounds darker and softer."),
        LowDamp = ParamSpec::float("low_damp", "Low damping", Unit::None, 1.0, 10.0, Scale::Log, 1.5)
            .modulatable()
            .smoothing(FILTERS)
            .advanced("How much faster low frequencies die away than the decay time (1 = the same)."),
        HighDamp = ParamSpec::float("high_damp", "High damping", Unit::None, 1.0, 10.0, Scale::Log, 2.5)
            .modulatable()
            .smoothing(FILTERS)
            .advanced("How much faster high frequencies die away than the decay time (1 = the same): higher is darker."),
    }
}
