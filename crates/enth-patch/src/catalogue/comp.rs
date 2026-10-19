//! `comp`: a stereo compressor, global, after the voices. J.O. Smith's `compression_gain_mono` structure with a
//! Giannoulis 2012 soft knee, make-up gain, a stereo link by `max(|L|, |R|)` and no look-ahead (zero latency).
//! Lane FX2 owns the DSP and `docs/dsp/compressor.md`.
//!
//! `attack` and `release` are the detector's time constants; the effective 63 % times are longer because two
//! smoothers are cascaded (JOS's design; FX2 documents the measured times). `threshold`, `ratio` and `knee` move
//! the target of the core's gain smoother, so they glide anyway; `makeup` and `mix` are applied directly and
//! would click on a step, so the engine smooths them (FX2's request). `mix` maps onto the core's `mix_pct` as is.

use enth_params::{ParamSpec, Scale, Unit, params};

use super::SMOOTH;

params! {
    /// Parameters of the `comp` module, in spec order.
    pub enum CompParam in COMP_PARAMS {
        Threshold = ParamSpec::float("threshold", "Threshold", Unit::Db, -60.0, 0.0, Scale::Linear, -18.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Level above which the compressor turns the signal down; lower compresses more.")
            .moves("loudness.peak_dbfs"),
        Ratio = ParamSpec::float("ratio", "Ratio", Unit::Ratio, 1.0, 20.0, Scale::Log, 4.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("How hard levels above the threshold are turned down: 4:1 lets 4 dB in become 1 dB out."),
        Attack = ParamSpec::float("attack", "Attack", Unit::Ms, 0.1, 100.0, Scale::Log, 10.0)
            .intermediate("How fast the compressor reacts to a rise in level; slower lets transients through."),
        Release = ParamSpec::float("release", "Release", Unit::Ms, 10.0, 2_000.0, Scale::Log, 200.0)
            .intermediate("How fast the compressor lets go after the level falls."),
        Knee = ParamSpec::float("knee", "Knee", Unit::Db, 0.0, 24.0, Scale::Linear, 6.0)
            .modulatable()
            .smoothing(SMOOTH)
            .advanced("Width of the soft transition around the threshold; 0 dB is a hard knee."),
        Makeup = ParamSpec::float("makeup", "Make-up", Unit::Db, 0.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Gain added after compression to bring the level back up.")
            .moves("loudness.lufs_i"),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the compressed signal is heard: 100 % is fully compressed, lower blends in the dry signal (parallel compression)."),
    }
}
