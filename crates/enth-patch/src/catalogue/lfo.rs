//! `lfo`: a low-frequency oscillator with a u32 phase accumulator, evaluated per 32-frame segment, per voice or
//! global. Lane D2 owns the DSP and `docs/dsp/lfo.md`.
//!
//! Polarity belongs to the source (design.md §2): a bipolar LFO runs −1..1, a unipolar one 0..1, and a route only
//! carries a depth. Random shapes (`sample_hold`, `smooth_random`) draw from the render seed, or from `seed` in
//! the `seeded` phase mode; never from the patch hash.
//!
//! **Tempo sync and one-shot (S2a, lane TMP, appended at freeze F1m).** `sync`, `beats` and `one_shot` come after
//! the seven parameters layout 1 reads, with neutral defaults (sync off: `rate` in Hz as always; one-shot off), so
//! an unsynced patch renders the same bits. `beats` is a new enum, not an item of the pinned `lfo.shape`.
//! The cycle lengths in [`LfoBeats::cycle_beats`] are exact rationals of a beat; `docs/dsp/lfo.md` justifies the list.

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::{PHASE_MODES, PhaseMode, SMOOTH};

enum_items! {
    /// The LFO's waveform.
    pub enum LfoShape in LFO_SHAPES {
        /// Sine.
        Sine = ("sine", "Sine", "A smooth wave: natural vibrato and tremolo."),
        /// Triangle.
        Triangle = ("triangle", "Triangle", "Linear up and down: even sweeps."),
        /// Rising ramp.
        SawUp = ("saw_up", "Saw up", "Rises then drops: repeating swells."),
        /// Falling ramp.
        SawDown = ("saw_down", "Saw down", "Drops then rises: repeating plucks."),
        /// Square.
        Square = ("square", "Square", "Jumps between two values: trills and gating."),
        /// A new random value each cycle, held.
        SampleHold = ("sample_hold", "Sample & hold", "A new random step every cycle: the classic computer burble."),
        /// Random values joined by smooth curves.
        SmoothRandom = ("smooth_random", "Smooth random", "Wanders randomly and smoothly: slow drift and movement."),
    }
}

enum_items! {
    /// Whether the LFO's output runs −1..1 or 0..1.
    pub enum Polarity in POLARITIES {
        /// −1..1: modulates both ways around the base value.
        Bipolar = ("bipolar", "Bipolar", "Swings both above and below the destination's value."),
        /// 0..1: modulates one way from the base value.
        Unipolar = ("unipolar", "Unipolar", "Moves the destination one way only, from its value up by the route depth."),
    }
}

enum_items! {
    /// Whether one LFO runs per voice or one is shared by all voices.
    pub enum LfoScope in LFO_SCOPES {
        /// One LFO per voice, started at each note-on.
        Voice = ("voice", "Per voice", "Each note gets its own LFO, started at note-on."),
        /// One LFO for the whole engine, started at the render start; the only kind that may modulate effects.
        Global = ("global", "Global", "One LFO shared by every note, running from the start; the only kind that may modulate effects."),
    }
}

enum_items! {
    /// The cycle length of a tempo-synced LFO, as a note division (T triplet, D dotted), **sorted by length** so that
    /// a rising modulation always slows the LFO. A bar is four beats (a quarter note is one beat), whatever the
    /// host's time signature. Append-only after F1m; the answer code writes an item's index in 5 bits (22 items).
    pub enum LfoBeats in LFO_BEATS {
        /// 1/12 of a beat.
        ThirtySecondTriplet = ("thirty_second_triplet", "1/32T", "A thirty-second-note triplet: twelve cycles per beat, a very fast flutter."),
        /// 1/8 of a beat.
        ThirtySecond = ("thirty_second", "1/32", "A thirty-second note: eight cycles per beat, a buzz."),
        /// 1/6 of a beat.
        SixteenthTriplet = ("sixteenth_triplet", "1/16T", "A sixteenth-note triplet: six cycles per beat."),
        /// 3/16 of a beat.
        ThirtySecondDotted = ("thirty_second_dotted", "1/32D", "A dotted thirty-second note: sixteen cycles in three beats, an uneven flutter."),
        /// 1/4 of a beat.
        Sixteenth = ("sixteenth", "1/16", "A sixteenth note: four cycles per beat."),
        /// 1/3 of a beat.
        EighthTriplet = ("eighth_triplet", "1/8T", "An eighth-note triplet: three cycles per beat."),
        /// 3/8 of a beat.
        SixteenthDotted = ("sixteenth_dotted", "1/16D", "A dotted sixteenth note: eight cycles in three beats."),
        /// 1/2 of a beat.
        Eighth = ("eighth", "1/8", "An eighth note: two cycles per beat."),
        /// 2/3 of a beat.
        QuarterTriplet = ("quarter_triplet", "1/4T", "A quarter-note triplet: three cycles in two beats."),
        /// 3/4 of a beat.
        EighthDotted = ("eighth_dotted", "1/8D", "A dotted eighth note: four cycles in three beats, a syncopated bounce."),
        /// 1 beat.
        Quarter = ("quarter", "1/4", "A quarter note: one cycle per beat."),
        /// 4/3 of a beat.
        HalfTriplet = ("half_triplet", "1/2T", "A half-note triplet: three cycles in four beats."),
        /// 3/2 of a beat.
        QuarterDotted = ("quarter_dotted", "1/4D", "A dotted quarter note: two cycles in three beats."),
        /// 2 beats.
        Half = ("half", "1/2", "A half note: one cycle every two beats."),
        /// 8/3 of a beat.
        BarTriplet = ("bar_triplet", "1 bar T", "A whole-note triplet: three cycles in two bars."),
        /// 3 beats.
        HalfDotted = ("half_dotted", "1/2D", "A dotted half note: one cycle every three beats."),
        /// 4 beats.
        Bar = ("bar", "1 bar", "A whole note: one cycle per bar of four beats."),
        /// 16/3 of a beat.
        TwoBarsTriplet = ("two_bars_triplet", "2 bars T", "A two-bar triplet: three cycles in four bars."),
        /// 6 beats.
        BarDotted = ("bar_dotted", "1 bar D", "A dotted whole note: one cycle every six beats, a bar and a half."),
        /// 8 beats.
        TwoBars = ("two_bars", "2 bars", "One cycle every two bars: a slow swell."),
        /// 12 beats.
        TwoBarsDotted = ("two_bars_dotted", "2 bars D", "One cycle every three bars."),
        /// 16 beats.
        FourBars = ("four_bars", "4 bars", "One cycle every four bars: a whole phrase."),
    }
}

impl LfoBeats {
    /// One cycle as a rational number of beats, `(numerator, denominator)` in lowest terms: `(1, 1)` is a quarter
    /// note, `(3, 4)` a dotted eighth, `(2, 3)` a quarter-note triplet. Strictly increasing in the item order.
    #[must_use]
    pub const fn cycle_beats(self) -> (u32, u32) {
        match self {
            LfoBeats::ThirtySecondTriplet => (1, 12),
            LfoBeats::ThirtySecond => (1, 8),
            LfoBeats::SixteenthTriplet => (1, 6),
            LfoBeats::ThirtySecondDotted => (3, 16),
            LfoBeats::Sixteenth => (1, 4),
            LfoBeats::EighthTriplet => (1, 3),
            LfoBeats::SixteenthDotted => (3, 8),
            LfoBeats::Eighth => (1, 2),
            LfoBeats::QuarterTriplet => (2, 3),
            LfoBeats::EighthDotted => (3, 4),
            LfoBeats::Quarter => (1, 1),
            LfoBeats::HalfTriplet => (4, 3),
            LfoBeats::QuarterDotted => (3, 2),
            LfoBeats::Half => (2, 1),
            LfoBeats::BarTriplet => (8, 3),
            LfoBeats::HalfDotted => (3, 1),
            LfoBeats::Bar => (4, 1),
            LfoBeats::TwoBarsTriplet => (16, 3),
            LfoBeats::BarDotted => (6, 1),
            LfoBeats::TwoBars => (8, 1),
            LfoBeats::TwoBarsDotted => (12, 1),
            LfoBeats::FourBars => (16, 1),
        }
    }
}

params! {
    /// Parameters of the `lfo` module, in spec order.
    pub enum LfoParam in LFO_PARAMS {
        Shape = ParamSpec::enumeration("shape", "Shape", &LFO_SHAPES, LfoShape::Sine.index())
            .basic("The waveform of the modulation."),
        Rate = ParamSpec::float("rate", "Rate", Unit::Hz, 0.01, 50.0, Scale::Log, 2.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Cycles per second: about 5 Hz for vibrato, under 0.5 Hz for slow sweeps."),
        Polarity = ParamSpec::enumeration("polarity", "Polarity", &POLARITIES, Polarity::Bipolar.index())
            .intermediate("Bipolar swings around the destination's value; unipolar only adds to it."),
        Scope = ParamSpec::enumeration("scope", "Scope", &LFO_SCOPES, LfoScope::Voice.index())
            .structural()
            .intermediate("One LFO per voice or one global LFO; only a global LFO can modulate effects. Changing it rebuilds the plan."),
        PhaseMode = ParamSpec::enumeration("phase_mode", "Phase mode", &PHASE_MODES, PhaseMode::Reset.index())
            .advanced("How each cycle start is chosen: reset (the phase knob at each note-on), free (random per render seed) or seeded (from seed)."),
        Phase = ParamSpec::float("phase", "Start phase", Unit::Degrees, 0.0, 360.0, Scale::Linear, 0.0)
            .periodic()
            .advanced("Where in its cycle the LFO starts in reset mode."),
        Seed = ParamSpec::int("seed", "Seed", Unit::None, 0, 65_535, 0)
            .not_automatable()
            .advanced("The random seed used in seeded phase mode, including the random shapes."),
        Sync = ParamSpec::toggle("sync", "Tempo sync", false)
            .intermediate("Off: rate sets the speed in Hz. On: the speed comes from beats and the tempo (140 BPM unless the host or the render says otherwise), and rate is ignored."),
        Beats = ParamSpec::enumeration("beats", "Beats", &LFO_BEATS, LfoBeats::Quarter.index())
            .modulatable()
            .intermediate("The cycle length when sync is on, as a note division (T triplet, D dotted). Modulating it steps through the divisions in order, shortest first."),
        OneShot = ParamSpec::toggle("one_shot", "One shot", false)
            .advanced("On: run one cycle from the start phase at each note-on (the render start for a global LFO), then hold the last value."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }

    #[test]
    fn the_divisions_are_sorted_by_length_and_in_lowest_terms() {
        let mut last = 0.0f64;
        for d in LfoBeats::ALL {
            let (n, m) = d.cycle_beats();
            assert!(n > 0 && m > 0, "{}", d.key());
            assert_eq!(gcd(n, m), 1, "{} is not in lowest terms", d.key());
            let beats = f64::from(n) / f64::from(m);
            assert!(
                beats > last,
                "{} breaks the order: {beats} after {last}",
                d.key()
            );
            last = beats;
        }
        assert_eq!(LfoBeats::COUNT, 22);
    }

    #[test]
    fn the_divisions_cover_what_the_brief_asks() {
        let beats = |d: LfoBeats| d.cycle_beats();
        // 1/32 to 4 bars: the shortest straight division is an eighth of a beat, the longest is four bars.
        assert_eq!(beats(LfoBeats::ThirtySecond), (1, 8));
        assert_eq!(beats(LfoBeats::FourBars), (16, 1));
        assert_eq!(beats(LfoBeats::Quarter), (1, 1));
        // Triplets are 2/3 of the straight length, dotted ones 3/2.
        let straight = |s: LfoBeats| {
            let (n, m) = s.cycle_beats();
            (f64::from(n), f64::from(m))
        };
        for (s, t, d) in [
            (
                LfoBeats::ThirtySecond,
                LfoBeats::ThirtySecondTriplet,
                LfoBeats::ThirtySecondDotted,
            ),
            (
                LfoBeats::Sixteenth,
                LfoBeats::SixteenthTriplet,
                LfoBeats::SixteenthDotted,
            ),
            (
                LfoBeats::Eighth,
                LfoBeats::EighthTriplet,
                LfoBeats::EighthDotted,
            ),
            (
                LfoBeats::Quarter,
                LfoBeats::QuarterTriplet,
                LfoBeats::QuarterDotted,
            ),
            (LfoBeats::Half, LfoBeats::HalfTriplet, LfoBeats::HalfDotted),
            (LfoBeats::Bar, LfoBeats::BarTriplet, LfoBeats::BarDotted),
            (
                LfoBeats::TwoBars,
                LfoBeats::TwoBarsTriplet,
                LfoBeats::TwoBarsDotted,
            ),
        ] {
            let (sn, sm) = straight(s);
            let (tn, tm) = straight(t);
            let (dn, dm) = straight(d);
            assert!((tn / tm - sn / sm * 2.0 / 3.0).abs() < 1e-12, "{}", t.key());
            assert!((dn / dm - sn / sm * 1.5).abs() < 1e-12, "{}", d.key());
        }
    }

    #[test]
    fn the_new_parameters_are_neutral_and_appended() {
        let n = LfoParam::COUNT;
        assert_eq!(n, 10);
        // Layout 1 reads the first seven in this order: nothing moved.
        for (i, key) in [
            "shape",
            "rate",
            "polarity",
            "scope",
            "phase_mode",
            "phase",
            "seed",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(LFO_PARAMS[i].key, *key);
        }
        assert_eq!(LfoParam::Sync.index(), 7);
        assert_eq!(LfoParam::Beats.index(), 8);
        assert_eq!(LfoParam::OneShot.index(), 9);
        assert_eq!(LfoParam::Sync.spec().default, 0.0);
        assert_eq!(LfoParam::OneShot.spec().default, 0.0);
        assert_eq!(
            LfoParam::Beats.spec().default,
            LfoBeats::Quarter.index() as f32
        );
        // `beats` is the one modulatable enum; the shape stays a plain, unmodulatable enum.
        assert!(LfoParam::Beats.spec().flags.modulatable);
        assert!(!LfoParam::Shape.spec().flags.modulatable);
        assert_eq!(
            LfoShape::COUNT,
            7,
            "no item was added to the pinned lfo.shape"
        );
    }
}
