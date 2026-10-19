//! `reverb`: a port of Signalsmith Basics' FDN reverb (4 diffusers, 8-channel feedback, Householder mixing,
//! Lagrange-7 detune, shelving damping), global and stereo. Lane FX1 owns the DSP and `docs/dsp/reverb.md`.
//!
//! The port fades all of its parameters internally over 20 ms (upstream's STFX A/B fade), so none is smoothed by
//! the engine, and `room` and `decay` are not modulatable: they re-derive delay taps and filters and would
//! retrigger the fade every segment (report 02). Upstream's `rt20` becomes `decay` as an RT60 in ms
//! (`rt20 = decay / 3000` s); upstream's gains `dry`/`wet` are exposed in dB.

use enth_params::{ParamSpec, Scale, Unit, params};

params! {
    /// Parameters of the `reverb` module, in spec order.
    pub enum ReverbParam in REVERB_PARAMS {
        Dry = ParamSpec::float("dry", "Dry", Unit::Db, -60.0, 6.0, Scale::Linear, 0.0)
            .modulatable()
            .intermediate("Level of the untouched signal passing through.")
            .moves("loudness.lufs_i"),
        Wet = ParamSpec::float("wet", "Wet", Unit::Db, -60.0, 6.0, Scale::Linear, -6.0)
            .modulatable()
            .basic("Level of the reverb tail: more wet sounds further away.")
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
            .advanced("Rate of the slow pitch wobble inside the tail: smears metallic ringing, chorus-like when high."),
        LowCut = ParamSpec::float("low_cut", "Low cut", Unit::Hz, 10.0, 500.0, Scale::Log, 80.0)
            .modulatable()
            .intermediate("Removes lows below this frequency from the reverb, keeping bass clean."),
        HighCut = ParamSpec::float("high_cut", "High cut", Unit::Hz, 1_000.0, 20_000.0, Scale::Log, 12_000.0)
            .modulatable()
            .intermediate("Removes highs above this frequency from the reverb: lower sounds darker and softer."),
        LowDamp = ParamSpec::float("low_damp", "Low damping", Unit::None, 1.0, 10.0, Scale::Log, 1.5)
            .modulatable()
            .advanced("How much faster low frequencies die away than the decay time (1 = the same)."),
        HighDamp = ParamSpec::float("high_damp", "High damping", Unit::None, 1.0, 10.0, Scale::Log, 2.5)
            .modulatable()
            .advanced("How much faster high frequencies die away than the decay time (1 = the same): higher is darker."),
    }
}
