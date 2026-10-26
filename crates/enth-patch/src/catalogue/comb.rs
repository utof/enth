//! `comb`: a keytracked feedback comb filter per voice: teeth at the harmonics of a pitch that follows the played note;
//! `mix`. Lane FLT owns its DSP, `docs/dsp/comb.md` and its golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (10) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions").
//!
//! The comb's pitch, which the table's meanings assume (lane FLT implements it): its frequency is
//! `f = max(20 Hz, 261.63 Hz · 2^((semi + keytrack · (pitch − 60)) / 12))`, `pitch` the voice's pitch in semitones and
//! `keytrack` a fraction, so at `keytrack` 100 % and `semi` 0 the teeth sit on the note's harmonics and follow glides
//! (svf's key 60 reference), and at 0 % the comb stays at `semi` from middle C. **The frequency is clamped to at least
//! 20 Hz**: without it `semi` −48 at key 0 is 0.5 Hz and a 2 s buffer per voice, with it the period is at most 50 ms
//! (2,400 samples at 48 kHz), which bounds each voice's delay line. The delay is one period of that frequency.
//!
//! **The level is bounded.** The output is `y[n] = (1 − |feedback|) · x[n] + feedback · lowpass(y[n − period])`: the
//! loop's input is scaled by `1 − |feedback|`, so the gain at the teeth peaks at 0 dB at every `feedback` (the bare
//! `x + feedback · lowpass(y[n − period])` peaks at `1 / (1 − |feedback|)`: +6 dB at the default 50 % and +40 dB at 99 %,
//! on every harmonic of a keytracked note), and the `damping` low-pass can only lower it. A negative `feedback` puts the
//! teeth between the harmonics, an octave lower and hollow. `mix` blends the input (0 %, bit for bit) with `y`
//! (100 %). With a short noise burst into it and `feedback` near 100 %, it rings as a plucked string
//! (Karplus–Strong), though a quiet one, since the burst is scaled by `1 − |feedback|`; with an LFO on `semi`, it
//! flanges.
//!
//! At its defaults it is **a gentle useful setting**: a soft metallic resonance on the note (`feedback` 50 %), no louder
//! than its input.

use enth_params::{ParamSpec, Scale, Unit, params};

use super::SMOOTH;

params! {
    /// Parameters of the `comb` module, in spec order.
    pub enum CombParam in COMB_PARAMS {
        Semi = ParamSpec::float("semi", "Semitones", Unit::Semitones, -48.0, 48.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("The comb's pitch in semitones from the played note (from middle C when keytrack is 0 %): +12st puts the teeth an octave up; an LFO here flanges.")
            .moves("spectrum.harmonics_db"),
        Keytrack = ParamSpec::float("keytrack", "Key tracking", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .intermediate("How much the comb's pitch follows the played note: 100 % keeps the teeth on the note's harmonics, 0 % holds a fixed pitch."),
        Feedback = ParamSpec::float("feedback", "Feedback", Unit::Percent, -99.0, 99.0, Scale::Linear, 50.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("How strongly the comb rings: higher is more metallic and longer; negative puts the resonances between the harmonics, hollow and an octave down.")
            .moves("spectrum.harmonics_db"),
        Damping = ParamSpec::float("damping", "Damping", Unit::Hz, 500.0, 20_000.0, Scale::Log, 10_000.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("A low-pass inside the feedback loop: lower makes the high teeth die faster, for a softer, plucked ring; 20 kHz leaves it bright.")
            .moves("spectrum.centroid_hz"),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged; around 50 % the dry signal adds notches between the teeth."),
    }
}
