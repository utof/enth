//! `dist`: distortion per voice: soft and hard clipping, an asymmetric (tube- and diode-like) curve, sine and triangle
//! folds, bitcrush and downsample, with ADAA-1 and zero-latency 2x or 4x IIR oversampling; drive, bias, tone, a clean
//! low band and `mix`. Lane DST owns its DSP, `docs/dsp/dist.md` and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (7) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions").
//!
//! The signal path, which the table's meanings assume (lane DST implements it):
//!
//! 1. `keep_lows` splits the input: the band below it bypasses the shaper and is added back after it, so a sub stays
//!    clean under a distorted mid (report 04's "keep lows", its fallback for the sub lane). At its minimum, 20 Hz, there
//!    is no split and the whole input is distorted.
//! 2. `drive` (gain) and `bias` (a DC offset) go into the curve `mode` names, oversampled by `oversample` for the
//!    clipping and folding curves; `bitcrush` uses `bits` and `downsample` uses `rate` (their aliasing is the point, so
//!    `oversample` does not apply to them). **`rate` runs up to 192 kHz so that its top is the identity at every
//!    supported sample rate** (up to 192 kHz): a downsample rate at or above the engine's sample rate holds nothing and
//!    passes the input unchanged, so the knob can always be turned fully off. A DC blocker after the curve removes
//!    what `bias` adds.
//! 3. `tone` is a low-pass after the curve.
//! 4. `mix` blends the input (0 %, returned bit for bit) with the result (100 %). **The blend is against a
//!    phase-matched dry path**: the oversampler's IIR half-bands, the `keep_lows` crossover and the `tone` low-pass all
//!    shift phase, so a plain dry copy would comb the blend wherever the curve is nearly linear (a quiet signal, or
//!    `drive` near 0 dB). The dry signal that `mix` blends is therefore the input put through the same phase response
//!    as the wet path's linear stages (for example an all-pass with that phase, or the crossover's own all-pass
//!    output), so wet and dry stay aligned at every frequency; how is lane DST's choice. At `mix` 0 % the dry path
//!    is not used and the input is returned bit for bit (the step from the matched dry path to the raw input changes
//!    phase, never level). The test: with `drive` 0 dB and a level well below the curve's
//!    knee, `mix` 50 % has a flat magnitude response, within ±0.5 dB from 50 Hz to 10 kHz.
//!
//! At its defaults it is **a gentle useful setting**: 6 dB into the soft curve, warmth on a full-scale oscillator and
//! nearly nothing on a quiet one.

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::SMOOTH;

enum_items! {
    /// The distortion curve. Ordered clipping, then folding, then digital, as on the card.
    pub enum DistMode in DIST_MODES {
        /// A smooth saturating curve (tanh-like), odd harmonics.
        Soft = ("soft", "Soft clip", "Rounds off the peaks smoothly: warm saturation that grows with drive."),
        /// Flat-topped clipping.
        Hard = ("hard", "Hard clip", "Cuts the peaks flat: bright, buzzy and aggressive."),
        /// A soft curve that treats the two halves differently, so even harmonics appear.
        Asymmetric = ("asymmetric", "Asymmetric", "Clips the top and bottom differently, like a tube or a diode: thicker, with even harmonics."),
        /// The signal folds back through a sine once it passes the limit.
        SineFold = ("sine_fold", "Sine fold", "Folds the wave back on itself in smooth curves: rich, metallic and vocal as drive rises."),
        /// The signal folds back linearly at the limits.
        TriangleFold = ("triangle_fold", "Triangle fold", "Folds the wave back at sharp corners: brighter and buzzier than the sine fold."),
        /// Fewer amplitude steps (`bits`).
        Bitcrush = ("bitcrush", "Bitcrush", "Fewer volume steps, set by bits: gritty digital noise that follows the signal."),
        /// A lower sample rate, held between samples (`rate`).
        Downsample = ("downsample", "Downsample", "Holds each sample for longer, set by rate: metallic, aliased tones and robotic vowels."),
    }
}

enum_items! {
    /// How much the clipping and folding curves are oversampled (zero-latency IIR half-bands, so nothing is reported).
    pub enum Oversample in OVERSAMPLES {
        /// No oversampling: ADAA-1 alone.
        Off = ("off", "Off", "No oversampling, the lightest on the CPU; anti-derivative anti-aliasing still applies."),
        /// 2x.
        X2 = ("x2", "2x", "Runs the curve at twice the sample rate: much less aliasing on high notes, for a little CPU."),
        /// 4x.
        X4 = ("x4", "4x", "Runs the curve at four times the sample rate: the cleanest, at about twice the cost of 2x."),
    }
}

params! {
    /// Parameters of the `dist` module, in spec order.
    pub enum DistParam in DIST_PARAMS {
        Mode = ParamSpec::enumeration("mode", "Mode", &DIST_MODES, DistMode::Soft.index())
            .basic("The distortion curve: soft or hard clipping, asymmetric, sine or triangle folding, bitcrush or downsample.")
            .moves("spectrum.harmonics_db"),
        Drive = ParamSpec::float("drive", "Drive", Unit::Db, 0.0, 48.0, Scale::Linear, 6.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Gain into the curve: more drive is more distortion; route an LFO or MSEG here for rhythmic grit.")
            .moves("spectrum.harmonics_db"),
        Bias = ParamSpec::float("bias", "Bias", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("An offset added before the curve, so the top and bottom distort differently: adds even harmonics and a thicker, rasping tone."),
        Bits = ParamSpec::float("bits", "Bits", Unit::None, 1.0, 16.0, Scale::Linear, 8.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Bit depth in bitcrush mode, fractions allowed: 16 is clean, 8 is gritty, 2 is a buzz. Other modes ignore it."),
        Rate = ParamSpec::float("rate", "Rate", Unit::Hz, 100.0, 192_000.0, Scale::Log, 6_000.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("The held sample rate in downsample mode: lower is more metallic and aliased; at the engine's rate or above it the input passes unchanged. Other modes ignore it."),
        Tone = ParamSpec::float("tone", "Tone", Unit::Hz, 100.0, 20_000.0, Scale::Log, 20_000.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("A low-pass after the curve that tames fizz: lower is darker; 20 kHz leaves it open.")
            .moves("spectrum.centroid_hz"),
        KeepLows = ParamSpec::float("keep_lows", "Keep lows", Unit::Hz, 20.0, 1_000.0, Scale::Log, 20.0)
            .intermediate("Frequencies below this bypass the distortion, so the sub stays clean and only the mids distort; at 20 Hz everything is distorted."),
        Oversample = ParamSpec::enumeration("oversample", "Oversampling", &OVERSAMPLES, Oversample::X2.index())
            .structural()
            .advanced("How much the clipping and folding curves are oversampled to cut aliasing: off, 2x or 4x (no latency either way); changing it rebuilds the plan."),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged; lower values blend in the clean signal (parallel distortion)."),
    }
}
