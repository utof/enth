//! `phaser`: a stereo phaser after the voices: a chain of swept first-order all-pass filters (TPT, stable under fast
//! sweeps) summed with the dry signal, with feedback; `mix`. Lane MOD owns its DSP, `docs/dsp/phaser.md` and its golden
//! cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (14) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions").
//!
//! What the table's meanings assume (lane MOD implements it): `stages` all-passes (an even count gives `stages / 2`
//! notches; an odd count is allowed and adds a half), their corner swept around `center` by `depth` (in octaves: 100 %
//! is ±3 octaves) at `rate` Hz or one cycle per `beats` when `sync` is on; the right channel's sweep runs `width` × 180°
//! behind the left's.
//!
//! **The effect's own output is the dry signal plus the all-pass chain, level-matched** (that sum makes the notches):
//! `wet = chain(v)` with `v[n] = (1 − |feedback|) · x[n] + feedback · wet[n − 1]` (the feedback is taken around the whole
//! chain, and its input is scaled by `1 − |feedback|`), and the output at `mix` 100 % is `(x + wet) / 2`. So the level is
//! bounded: the chain's gain peaks at 0 dB at every `feedback` (the bare `x + feedback · wet` loop peaks at
//! `1 / (1 − |feedback|)`, +40 dB at 99 %), and where the dry and the wet are in phase the sum is 0 dB and not +6 dB.
//! A freshly added phaser changes a full-scale saw's level by at most about 3 dB. `mix` blends the input (0 %, bit for
//! bit) with that sum (100 %). A global effect: only global sources modulate it. Lane MOD measures the level and may
//! choose another normalisation that keeps the same bound.
//!
//! At its defaults it is **a gentle useful setting**: a slow six-stage sweep around 1 kHz with light feedback.

use enth_params::{ParamSpec, Scale, Unit, params};

use super::{BEATS, Beats, SMOOTH};

params! {
    /// Parameters of the `phaser` module, in spec order.
    pub enum PhaserParam in PHASER_PARAMS {
        Stages = ParamSpec::int("stages", "Stages", Unit::None, 2, 12, 6)
            .intermediate("How many all-pass filters are chained: every two make one notch, so more stages give more notches and a stronger sweep."),
        Rate = ParamSpec::float("rate", "Rate", Unit::Hz, 0.01, 20.0, Scale::Log, 0.3)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("How fast the notches sweep, in cycles per second, when tempo sync is off."),
        Sync = ParamSpec::toggle("sync", "Sync", false)
            .intermediate("On: one sweep lasts beats of the song's tempo, and rate is ignored."),
        Beats = ParamSpec::enumeration("beats", "Beats", &BEATS, Beats::Bar.index())
            .intermediate("How long one sweep lasts when tempo sync is on, as a note division."),
        Depth = ParamSpec::float("depth", "Depth", Unit::Percent, 0.0, 100.0, Scale::Linear, 50.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("How far the notches sweep around the center: 100 % is three octaves either way."),
        Center = ParamSpec::float("center", "Center", Unit::Hz, 50.0, 10_000.0, Scale::Log, 1_000.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("The frequency the notches sweep around; with depth at 0 % a static, comb-like color here.")
            .moves("spectrum.centroid_hz"),
        Feedback = ParamSpec::float("feedback", "Feedback", Unit::Percent, -99.0, 99.0, Scale::Linear, 30.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Sends the output back into the chain: sharper, ringing notches; negative moves the peaks between them."),
        Width = ParamSpec::float("width", "Width", Unit::Percent, 0.0, 100.0, Scale::Linear, 50.0)
            .intermediate("How far apart the left and right sweeps run: 0 % is mono, 100 % opposite.")
            .moves("stereo.width"),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How much of the effect is heard: at 0 % the input passes unchanged, at 100 % the full phaser (the dry sound plus the swept all-passes)."),
    }
}
