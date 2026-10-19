//! `lfo`: a low-frequency oscillator with a u32 phase accumulator, evaluated per 32-frame segment, per voice or
//! global. Lane D2 owns the DSP and `docs/dsp/lfo.md`.
//!
//! Polarity belongs to the source (design.md §2): a bipolar LFO runs −1..1, a unipolar one 0..1, and a route only
//! carries a depth. Random shapes (`sample_hold`, `smooth_random`) draw from the render seed, or from `seed` in
//! the `seeded` phase mode; never from the patch hash.

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
    }
}
