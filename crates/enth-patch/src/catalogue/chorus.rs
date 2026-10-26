//! `chorus`: a stereo chorus, ensemble or flanger after the voices on one modulated-delay core (report 05 §3); `mix`.
//! Lane MOD owns its DSP, `docs/dsp/chorus.md` and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (13) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions").
//!
//! What the table's meanings assume (lane MOD implements it):
//!
//! - A tap's delay is `delay · (1 + depth/100 · s(t))`, `s` the sweep (−1…1) at `rate` Hz, or at one cycle per `beats`
//!   of the render's tempo when `sync` is on. **`depth` is the sweep's peak distance from `delay`, in percent of
//!   `delay`** (a ratio, so a flanger at 1 ms and a chorus at 20 ms sweep in the same proportion), and 0–100 % is the
//!   physical range: at 100 % the delay swings between 0 and twice `delay` (clamped to at least one sample), and a
//!   larger depth could only push it below zero. In milliseconds at the default: 50 % of 7 ms is ±3.5 ms. The delay
//!   line holds twice `delay`'s maximum (60 ms) and its interpolation margin. The pitch swing is about
//!   `2π · rate · depth · delay` of the pitch: ±19 cents at the defaults (0.5 Hz, 50 % of 7 ms), and at the extreme
//!   (20 Hz, 100 % of 30 ms) the read head runs backwards, a warble and no instability. The right channel's sweep runs
//!   `width` × 180° behind the left's. `ensemble` runs three taps per channel on two sweeps (Plaits' ensemble, a slow
//!   and a fast one at a fixed ratio, the slow one at `rate`), each sweeping by `depth`.
//! - **The effect's own output is the dry signal plus the delayed taps, level-matched**: with `wet` the mean of the
//!   taps (each channel's taps weighted to unity gain together) the output at `mix` 100 % is `(x + wet) / 2`, so where
//!   the dry and the wet are in phase the sum is 0 dB and not +6 dB, and a freshly added chorus changes a full-scale
//!   saw's level by at most about 3 dB; `mix` blends the input (0 %, bit for bit) with that sum (100 %). `feedback`
//!   returns the taps into the delay (the flanger's resonance) with **the delay's input scaled by `1 − |feedback|`**,
//!   `v[n] = (1 − |feedback|) · x[n] + feedback · tap[n]`, so the loop's gain peaks at 0 dB at every `feedback` (the
//!   bare loop peaks at `1 / (1 − |feedback|)`, +40 dB at ±99 %). Lane MOD measures the level and may choose another
//!   normalisation that keeps the same bound.
//! - A global effect: only global sources (a global LFO or MSEG, macros, the mod wheel) modulate it. Latency stays 0.
//!   A through-zero flanger (a fixed dry delay) is not a mode yet; it can be appended to `mode` later.
//!
//! At its defaults it is **a gentle useful setting**: a slow, wide chorus (0.5 Hz, 7 ms, depth 50 % = ±3.5 ms).

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::{BEATS, Beats, SMOOTH};

enum_items! {
    /// The modulated-delay algorithm.
    pub enum ChorusMode in CHORUS_MODES {
        /// Two swept taps (one per channel).
        Chorus = ("chorus", "Chorus", "A few slightly delayed, slowly wobbling copies beside the dry sound: width and shimmer."),
        /// Three taps per channel on a slow and a fast sweep.
        Ensemble = ("ensemble", "Ensemble", "A string-machine ensemble: three wobbling copies per side, lush and wide, for pads and big basses."),
        /// A short swept delay with feedback.
        Flanger = ("flanger", "Flanger", "A very short delay swept against the dry sound: the jet-plane whoosh of moving comb notches."),
    }
}

params! {
    /// Parameters of the `chorus` module, in spec order.
    pub enum ChorusParam in CHORUS_PARAMS {
        Mode = ParamSpec::enumeration("mode", "Mode", &CHORUS_MODES, ChorusMode::Chorus.index())
            .basic("The algorithm: chorus, ensemble or flanger."),
        Rate = ParamSpec::float("rate", "Rate", Unit::Hz, 0.01, 20.0, Scale::Log, 0.5)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("How fast the delay sweeps, in cycles per second, when tempo sync is off: slow is a gentle shimmer, fast a warble."),
        Sync = ParamSpec::toggle("sync", "Sync", false)
            .intermediate("On: one sweep lasts beats of the song's tempo, and rate is ignored."),
        Beats = ParamSpec::enumeration("beats", "Beats", &BEATS, Beats::Bar.index())
            .intermediate("How long one sweep lasts when tempo sync is on, as a note division."),
        Depth = ParamSpec::float("depth", "Depth", Unit::Percent, 0.0, 100.0, Scale::Linear, 50.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("How far the delay sweeps either side of the delay time, as a share of it: 50 % of 7 ms is plus or minus 3.5 ms, and 100 % sweeps between none and double. More is a deeper wobble in pitch (chorus) or a wider sweep of the notches (flanger)."),
        Delay = ParamSpec::float("delay", "Delay", Unit::Ms, 0.1, 30.0, Scale::Log, 7.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("The delay the sweep moves around: about 5-20 ms for a chorus, 0.5-5 ms for a flanger; an LFO here is a manual flange.")
            .moves("spectrum.centroid_hz"),
        Feedback = ParamSpec::float("feedback", "Feedback", Unit::Percent, -99.0, 99.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Sends the delayed sound back into the delay: a ringing, metallic flange; negative gives hollower notches."),
        Width = ParamSpec::float("width", "Width", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .intermediate("How far apart the left and right sweeps run: 0 % is mono, 100 % opposite, the widest.")
            .moves("stereo.width"),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged, at 100 % the full chorus (the dry sound plus the wobbling copies)."),
    }
}
