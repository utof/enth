//! `formant`: a formant (vowel) filter per voice: parallel resonant band-passes at the first two formants of a vowel,
//! placed anywhere in the vowel plane, plus a fixed third; `mix`. Lane FLT owns its DSP, `docs/dsp/formant.md` and its
//! golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (11) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions").
//!
//! **Why two formant frequencies and not one vowel knob:** the vowel space is two-dimensional (F1 is how open a vowel
//! is, F2 how far forward), and no single path through it makes report 04's moves: a "yoi" swings F2 from 2.4 kHz to
//! 640 Hz and back while F1 stays at 240–360 Hz (i → o → i), and a growl's "a → o" lowers both. On the usual
//! A-E-I-O-U path the second passes through "e" and "i"; on a back-to-front path the first passes through "a".
//! With `f1` and `f2` every vowel is a point and every straight move is one modulator routed to both (an MSEG to `f2`
//! at one depth and to `f1` at another). The plan's "continuous vowel path" is met as the special case. Both are in
//! Hz, real units, so the vowel table below is the knob settings (Catford 2001, male speech; report 04 §1):
//!
//! | i | e | a | ɑ | o | u |
//! | --- | --- | --- | --- | --- | --- |
//! | 240/2400 | 390/2300 | 850/1610 | 750/940 | 360/640 | 250/595 |
//!
//! `shift` moves all three formants together (a bigger or smaller voice, the vowel kept); the third formant sits at
//! 2.7 kHz before `shift` (lane FLT may refine its level and bandwidth). Coefficients are computed once per segment.
//!
//! At its defaults it is **a gentle useful setting**: the open vowel "a" (850/1610 Hz) at a moderate resonance. A vowel
//! filter fully wet is never transparent; `mix` blends it in.

use enth_params::{ParamSpec, Scale, Unit, params};

use super::SMOOTH;

params! {
    /// Parameters of the `formant` module, in spec order.
    pub enum FormantParam in FORMANT_PARAMS {
        F1 = ParamSpec::float("f1", "Formant 1", Unit::Hz, 100.0, 1_000.0, Scale::Log, 850.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("The first formant: how open the vowel is, from 'ee' and 'oo' (about 250 Hz) to 'ah' (about 850 Hz).")
            .moves("spectrum.centroid_hz"),
        F2 = ParamSpec::float("f2", "Formant 2", Unit::Hz, 400.0, 4_000.0, Scale::Log, 1_610.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("The second formant: how far forward the vowel is, from 'oo' (about 600 Hz) through 'ah' (1.6 kHz) to 'ee' (2.4 kHz); sweep it for 'yoi' and 'wow'.")
            .moves("spectrum.centroid_hz"),
        Shift = ParamSpec::float("shift", "Shift", Unit::Semitones, -24.0, 24.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Moves every formant up or down together, keeping the vowel: a smaller, brighter voice or a bigger, darker one."),
        Resonance = ParamSpec::float("resonance", "Resonance", Unit::Percent, 0.0, 100.0, Scale::Linear, 50.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How narrow and strong the formant peaks are: low is a soft, breathy vowel, high a sharp, whistling one."),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged; lower values keep some of the unfiltered body."),
    }
}
