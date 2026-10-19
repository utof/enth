//! `noise`: white noise (xorshift32) or pink noise (J.O. Smith's 3rd-order pinking filter), seeded per voice from
//! (render seed, module id, note index). Lane D1 owns the DSP and `docs/dsp/noise.md`.

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::SMOOTH;

enum_items! {
    /// The noise colour.
    pub enum NoiseColor in NOISE_COLORS {
        /// Flat spectrum.
        White = ("white", "White", "Equal energy at every frequency: bright hiss."),
        /// −3 dB/octave (−10 dB/decade).
        Pink = ("pink", "Pink", "Energy falls 3 dB per octave: softer, more natural, like rain."),
    }
}

params! {
    /// Parameters of the `noise` module, in spec order.
    pub enum NoiseParam in NOISE_PARAMS {
        Color = ParamSpec::enumeration("color", "Color", &NOISE_COLORS, NoiseColor::White.index())
            .basic("White is bright hiss; pink is darker and more natural.")
            .moves("spectrum.tilt_db_oct"),
        Level = ParamSpec::float("level", "Level", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Output level of the noise into the voice, as a linear percentage.")
            .moves("noise_ratio"),
    }
}
