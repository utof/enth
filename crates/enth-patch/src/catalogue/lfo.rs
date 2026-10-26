//! `lfo`: a low-frequency oscillator with a u32 phase accumulator, evaluated per 32-frame segment, per voice or
//! global. Lane D2 owns the DSP and `docs/dsp/lfo.md`.
//!
//! Polarity belongs to the source (design.md §2): a bipolar LFO runs −1..1, a unipolar one 0..1, and a route only
//! carries a depth. Random shapes (`sample_hold`, `smooth_random`) draw from the render seed, or from `seed` in
//! the `seeded` phase mode; never from the patch hash.
//!
//! **Tempo sync and one-shot (S2a, lane TMP, appended at freeze F1m).** `sync`, `beats` and `one_shot` come after
//! the seven parameters layout 1 reads, with neutral defaults (sync off: `rate` in Hz as always; one-shot off), so
//! an unsynced patch renders the same bits. `beats` is the note-division enum shared with the other synced
//! parameters ([`Beats`], in `catalogue/mod.rs`; its lengths are exact rationals of a beat, [`Beats::length`]), not
//! an item of the pinned `lfo.shape`; `docs/dsp/lfo.md` justifies the list.

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::{BEATS, Beats, PHASE_MODES, PhaseMode, SMOOTH};

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
        Beats = ParamSpec::enumeration("beats", "Beats", &BEATS, Beats::Quarter.index())
            .modulatable()
            .intermediate("The cycle length when sync is on, as a note division (T triplet, D dotted). Modulating it steps through the divisions in order, shortest first."),
        OneShot = ParamSpec::toggle("one_shot", "One shot", false)
            .advanced("On: run one cycle from the start phase at each note-on (the render start for a global LFO), then hold the last value."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_divisions_cover_what_the_lfo_needs() {
        // 1/32 to 4 bars, with the triplet and dotted neighbours, are all in the shared list.
        let all = Beats::ALL;
        for key in [
            "thirty_second",
            "sixteenth_triplet",
            "eighth_triplet",
            "eighth_dotted",
            "quarter_triplet",
            "quarter_dotted",
            "four_bars",
        ] {
            assert!(all.iter().any(|b| b.key() == key), "{key} is missing");
        }
        assert_eq!(Beats::ThirtySecond.length(), (1, 8));
        assert_eq!(Beats::FourBars.length(), (16, 1));
        assert_eq!(Beats::Quarter.length(), (1, 1));
        // Triplets are 2/3 of the straight length, dotted ones 3/2.
        let value = |b: Beats| {
            let (n, m) = b.length();
            f64::from(n) / f64::from(m)
        };
        for (s, t, d) in [
            (
                Beats::ThirtySecond,
                Beats::ThirtySecondTriplet,
                Beats::ThirtySecondDotted,
            ),
            (
                Beats::Sixteenth,
                Beats::SixteenthTriplet,
                Beats::SixteenthDotted,
            ),
            (Beats::Eighth, Beats::EighthTriplet, Beats::EighthDotted),
            (Beats::Quarter, Beats::QuarterTriplet, Beats::QuarterDotted),
            (Beats::Half, Beats::HalfTriplet, Beats::HalfDotted),
            (Beats::Bar, Beats::BarTriplet, Beats::BarDotted),
            (Beats::TwoBars, Beats::TwoBarsTriplet, Beats::TwoBarsDotted),
        ] {
            assert!(
                (value(t) - value(s) * 2.0 / 3.0).abs() < 1e-12,
                "{}",
                t.key()
            );
            assert!((value(d) - value(s) * 1.5).abs() < 1e-12, "{}", d.key());
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
            Beats::Quarter.index() as f32
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
