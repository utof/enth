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
//! - The modulated taps sweep around `delay` by `depth`, at `rate` Hz, or at one cycle per `beats` of the render's tempo
//!   when `sync` is on; the right channel's sweep runs `width` × 180° behind the left's. `ensemble` runs three taps per
//!   channel on two sweeps (Plaits' ensemble, a slow and a fast one at a fixed ratio, the slow one at `rate`).
//! - **The effect's own output is the dry signal plus the delayed taps**, as a chorus pedal's is, so `mix` 100 % is the
//!   full chorus (or flange) and `mix` scales the taps against an unchanged dry signal; `mix` 0 % returns the input bit
//!   for bit. `feedback` returns the taps into the delay (the flanger's resonance).
//! - A global effect: only global sources (a global LFO or MSEG, macros, the mod wheel) modulate it. Latency stays 0.
//!   A through-zero flanger (a fixed dry delay) is not a mode yet; it can be appended to `mode` later.
//!
//! At its defaults it is **a gentle useful setting**: a slow, wide chorus (0.5 Hz, 7 ms, depth 50 %).

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
        Sync = ParamSpec::toggle("sync", "Tempo sync", false)
            .intermediate("On: one sweep lasts beats of the song's tempo, and rate is ignored."),
        Beats = ParamSpec::enumeration("beats", "Beats", &BEATS, Beats::Bar.index())
            .intermediate("How long one sweep lasts when tempo sync is on, as a note division."),
        Depth = ParamSpec::float("depth", "Depth", Unit::Percent, 0.0, 100.0, Scale::Linear, 50.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("How far the delay sweeps: more is a deeper wobble in pitch (chorus) or a wider sweep of the notches (flanger)."),
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
