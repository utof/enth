//! `eq`: a six-band equaliser after the voices: a low cut, a low shelf, two bells, a high shelf and a high cut, on
//! `enth-dsp`'s biquad (lane BQ's file) with per-segment coefficient ramps; `mix`. Lane EQ owns its DSP,
//! `docs/dsp/eq.md` and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (15) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions").
//!
//! The bands run in series in table order. The cuts are 12 dB/octave Butterworth sections that **stay in the signal
//! path at every value**: nothing switches them in or out at the ends of their ranges (`low_cut` 10 Hz, `high_cut`
//! 20 kHz), because both are modulatable and a stage that appears at a range end would click as a source crosses it.
//! At the end they are out of the way, not bypassed: `low_cut` at 10 Hz is −0.3 dB at 20 Hz and under 0.01 dB from
//! 50 Hz up, and `high_cut` at 20 kHz is −0.3 dB at 10 kHz and −3 dB at 20 kHz itself; `mix` is the bypass. Shelves and
//! bells are the cookbook's (EQ's acceptance: within 0.1 dB of it), a band at 0 dB does nothing. Bells at a high Q and
//! −24 dB are notches. A global effect: only global sources modulate it.
//!
//! At its defaults it is **near-transparent**: every gain at 0 dB and both cuts at the ends of their ranges, out of the
//! way.

use enth_params::{ParamSpec, Scale, Unit, params};

use super::SMOOTH;

params! {
    /// Parameters of the `eq` module, in spec order.
    pub enum EqParam in EQ_PARAMS {
        LowCut = ParamSpec::float("low_cut", "Low cut", Unit::Hz, 10.0, 1_000.0, Scale::Log, 10.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Removes everything below this frequency (12 dB/octave), clearing mud or the sub from effects; at 10 Hz it is out of the way."),
        LowFreq = ParamSpec::float("low_freq", "Low shelf", Unit::Hz, 20.0, 1_000.0, Scale::Log, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("The corner of the low shelf."),
        LowGain = ParamSpec::float("low_gain", "Low gain", Unit::Db, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Boosts or cuts everything below the low shelf's corner: weight and warmth.")
            .moves("spectrum.tilt_db_oct"),
        Bell1Freq = ParamSpec::float("bell1_freq", "Bell 1", Unit::Hz, 20.0, 20_000.0, Scale::Log, 400.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("The center of the first bell."),
        Bell1Gain = ParamSpec::float("bell1_gain", "Bell 1 gain", Unit::Db, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Boosts or cuts a band around the first bell's center: a cut near 300-500 Hz cleans boxiness."),
        Bell1Q = ParamSpec::float("bell1_q", "Bell 1 Q", Unit::None, 0.1, 20.0, Scale::Log, 1.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How narrow the first bell is: 0.7 is broad, 10 a narrow notch or peak."),
        Bell2Freq = ParamSpec::float("bell2_freq", "Bell 2", Unit::Hz, 20.0, 20_000.0, Scale::Log, 2_500.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("The center of the second bell."),
        Bell2Gain = ParamSpec::float("bell2_gain", "Bell 2 gain", Unit::Db, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Boosts or cuts a band around the second bell's center: presence and bite around 2-5 kHz."),
        Bell2Q = ParamSpec::float("bell2_q", "Bell 2 Q", Unit::None, 0.1, 20.0, Scale::Log, 1.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How narrow the second bell is: 0.7 is broad, 10 a narrow notch or peak."),
        HighFreq = ParamSpec::float("high_freq", "High shelf", Unit::Hz, 1_000.0, 20_000.0, Scale::Log, 8_000.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("The corner of the high shelf."),
        HighGain = ParamSpec::float("high_gain", "High gain", Unit::Db, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Boosts or cuts everything above the high shelf's corner: air and sparkle, or a darker top.")
            .moves("spectrum.centroid_hz"),
        HighCut = ParamSpec::float("high_cut", "High cut", Unit::Hz, 1_000.0, 20_000.0, Scale::Log, 20_000.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Removes everything above this frequency (12 dB/octave); at 20 kHz it is out of the way.")
            .moves("spectrum.centroid_hz"),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged."),
    }
}
