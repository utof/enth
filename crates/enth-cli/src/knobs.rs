//! How a knob's value shows in the descriptor it moves, where the two differ (eval finding 1, `docs/p1a-g1.md`):
//! agents set `env1.release = 1s`, measured `envelope.release_ms` at about 500 and concluded the knob was off by
//! two. It is not: the engine's envelope segments are RC curves that end 80 dB down (`enth-dsp`'s
//! `MAX_CURVE_DB`), and the descriptor stops at 40 dB down, which such a curve reaches at 0.499 of its length.
//! Neither side changes (the knob is the segment time, the descriptor is a perceptual threshold); `enth params`
//! states the relation in `reads_as`, and `tests/knobs.rs` renders through the engine and holds these numbers to
//! the measurement.

/// The relation between a parameter of `kind` and the descriptor it moves, when that is not one to one.
#[must_use]
pub fn reads_as(kind: &str, key: &str) -> Option<&'static str> {
    match (kind, key) {
        ("adsr", "attack") => Some(
            "envelope.attack_ms reads about 0.72 x this above 20 ms (as the amplitude envelope): the knob is the \
             whole rise, the descriptor stops 1 dB short of the peak",
        ),
        ("adsr", "decay") => Some(
            "envelope.decay_ms reads well under this (as the amplitude envelope): about 0.4 x at sustain 0%, 0.28 x \
             at 20%, 0.22 x at 50%, 0.07 x at 80%; the knob is the whole fall to the sustain level, the descriptor \
             stops once the level settles near it",
        ),
        ("adsr", "sustain") => Some(
            "envelope.sustain_db is about 20 log10(sustain / 100%) (as the amplitude envelope): 50% reads -6 dB, 20% \
             -14 dB, 0% -80 (silent); a decay under 300 ms after a click attack reads 1 to 3 dB higher (the 8 ms \
             window blunts the peak)",
        ),
        ("adsr", "release") => Some(
            "envelope.release_ms reads about 0.5 x this (as the amplitude envelope): the knob is the whole fall, to \
             80 dB down; the descriptor stops at 40 dB down. For a measured release of R ms, set about 2R",
        ),
        _ => None,
    }
}
