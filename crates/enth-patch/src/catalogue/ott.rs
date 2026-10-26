//! `ott`: three-band upward and downward compression after the voices (the "OTT" sound of bass music: Ableton's
//! Multiband Dynamics preset that Xfer turned into a free plug-in), on the compressor's detector; `mix`. Lane OTT owns
//! its DSP, `docs/dsp/ott.md` and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (9) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions").
//!
//! The table is OTT's own small surface, not the full multiband compressor: the bands' thresholds, ratios and time
//! constants are lane OTT's constants (from the preset, report 04 §2, verified or measured by OTT), and the knobs
//! scale them:
//!
//! - `input` gain, then a Linkwitz–Riley split at `low_split` and `high_split` (report 05 §4);
//! - per band, downward compression above its threshold and upward compression below it (with a gain cap and a floor,
//!   so silence is not lifted), each scaled by `downward` and `upward`, and both by `depth`; `time` scales every
//!   band's attack and release;
//! - `low_gain`, `mid_gain` and `high_gain` per band, the sum, `output` gain, then `mix` against the input.
//!
//! **`depth` is intensity, not dry/wet:** Xfer's Depth knob is described as a dry/wet blend (Mixelite), and every S2a
//! effect has `mix` for that; but blending a band-split signal with its dry input notches it at the crossovers, and
//! the S2 rule wants a gentle default with `mix` at 100 %. So `depth` scales the compression amounts towards none,
//! like Ableton's Amount, and **at its defaults (depth 30 %) it is a gentle useful setting**: denser and brighter, not
//! the full effect, which is `depth` 100 %.

use enth_params::{ParamSpec, Scale, Unit, params};

use super::SMOOTH;

params! {
    /// Parameters of the `ott` module, in spec order.
    pub enum OttParam in OTT_PARAMS {
        Depth = ParamSpec::float("depth", "Depth", Unit::Percent, 0.0, 100.0, Scale::Linear, 30.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("How hard all three bands are squashed up and down: 0 % leaves the levels alone, 100 % is the full, dense OTT sound.")
            .moves("loudness.lufs_i"),
        Time = ParamSpec::float("time", "Time", Unit::Percent, 10.0, 1_000.0, Scale::Log, 100.0)
            .intermediate("Scales every band's attack and release: 100 % is the reference OTT timing, lower is faster and grittier, higher is smoother."),
        Upward = ParamSpec::float("upward", "Upward", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much quiet parts of each band are lifted (upward compression): brings out tails, noise and detail."),
        Downward = ParamSpec::float("downward", "Downward", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much loud parts of each band are pushed down (downward compression): evens out peaks."),
        Input = ParamSpec::float("input", "Input", Unit::Db, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Gain before the bands: more input drives more of the signal above the thresholds."),
        Output = ParamSpec::float("output", "Output", Unit::Db, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Gain after the bands, to match the level before and after.")
            .moves("loudness.lufs_i"),
        LowSplit = ParamSpec::float("low_split", "Low split", Unit::Hz, 40.0, 500.0, Scale::Log, 88.3)
            .advanced("The crossover between the low and mid bands."),
        HighSplit = ParamSpec::float("high_split", "High split", Unit::Hz, 500.0, 10_000.0, Scale::Log, 2_500.0)
            .advanced("The crossover between the mid and high bands."),
        LowGain = ParamSpec::float("low_gain", "Low gain", Unit::Db, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Output level of the low band after compression."),
        MidGain = ParamSpec::float("mid_gain", "Mid gain", Unit::Db, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Output level of the mid band after compression."),
        HighGain = ParamSpec::float("high_gain", "High gain", Unit::Db, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Output level of the high band after compression.")
            .moves("spectrum.centroid_hz"),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged. Prefer depth for a lighter OTT: a dry blend notches the crossovers."),
    }
}
