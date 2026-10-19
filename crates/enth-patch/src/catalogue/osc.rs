//! `osc`: the oscillator. Classic waves from Plaits' `VariableShapeOscillator` (polyBLEP/BLAMP, PWM, hard sync)
//! and a wavetable mode (tables generated in code, mip-mapped, Hermite-interpolated). Lane D1 owns the DSP and
//! `docs/dsp/osc.md`; this file is the parameter contract (report 02, adjusted: one kind with a wave enum that
//! includes the wavetable, a wide modulatable `semi` instead of a stepped octave switch, and a per-oscillator
//! phase mode).

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::{PHASE_MODES, PhaseMode, SMOOTH};

enum_items! {
    /// The oscillator's waveform.
    pub enum Wave in WAVES {
        /// One harmonic.
        Sine = ("sine", "Sine", "A pure tone with a single harmonic: soft and round."),
        /// Odd harmonics falling at 12 dB/octave.
        Triangle = ("triangle", "Triangle", "Odd harmonics that fade fast: mellow, flute-like."),
        /// Every harmonic falling at 6 dB/octave.
        Saw = ("saw", "Saw", "Every harmonic: bright and buzzy, the classic subtractive starting point."),
        /// A pulse whose width is `pw`; 50 % is a square (odd harmonics only).
        Pulse = ("pulse", "Pulse", "A square at 50 % width, hollow and reedy; narrower widths sound thinner."),
        /// The wavetable chosen by `table`, scanned by `position`.
        Table = ("table", "Wavetable", "Plays the chosen wavetable; position morphs through its frames."),
    }
}

enum_items! {
    /// The generated wavetables. Each frame is a formula over harmonic amplitudes, so there are no audio assets.
    pub enum Wavetable in WAVETABLES {
        /// Sine → triangle → saw → square.
        Basic = ("basic", "Basic", "Morphs sine, triangle, saw and square."),
        /// Pulse width from 50 % down to 3 %.
        Pulse = ("pulse", "Pulse", "A pulse that narrows from square to a thin buzz."),
        /// Harmonic amplitudes k^-p with p from 0.5 to 3: a smooth brightness axis.
        Tilt = ("tilt", "Tilt", "One smooth axis from very bright to nearly a sine."),
        /// Vowel formants A–E–I–O–U over a saw spectrum.
        Formant = ("formant", "Formant", "Vowel-like resonances, A to U."),
        /// Drawbar organ registrations.
        Organ = ("organ", "Organ", "Drawbar organ registrations."),
    }
}

params! {
    /// Parameters of the `osc` module, in spec order.
    pub enum OscParam in OSC_PARAMS {
        Wave = ParamSpec::enumeration("wave", "Wave", &WAVES, Wave::Saw.index())
            .basic("The waveform: sine, triangle, saw, pulse, or the wavetable chosen by table.")
            .moves("spectrum.harmonics_db"),
        Semi = ParamSpec::float("semi", "Semitones", Unit::Semitones, -48.0, 48.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Transposition in semitones from the played note (+12st is an octave up); route an envelope here for pitch sweeps.")
            .moves("pitch.f0_hz"),
        Fine = ParamSpec::float("fine", "Fine", Unit::Cents, -100.0, 100.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Fine tuning in cents; a few cents between two oscillators makes them beat and sound thicker.")
            .moves("detune.beating_hz"),
        Level = ParamSpec::float("level", "Level", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Output level of this oscillator into the voice, as a linear percentage.")
            .moves("loudness.lufs_i"),
        Pw = ParamSpec::float("pw", "Pulse width", Unit::Percent, 5.0, 95.0, Scale::Linear, 50.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Width of the pulse wave: 50 % is a square; other waves ignore it. Modulate it for PWM.")
            .moves("spectrum.odd_even_ratio"),
        Sync = ParamSpec::toggle("sync", "Hard sync", false)
            .intermediate("Restart this oscillator with each cycle of the first oscillator in generators: tune it higher for tearing, vocal tones. Ignored on that first oscillator.")
            .moves("spectrum.centroid_hz"),
        Table = ParamSpec::enumeration("table", "Wavetable", &WAVETABLES, Wavetable::Basic.index())
            .structural()
            .intermediate("Which wavetable the table wave plays; changing it rebuilds the voice plan.")
            .moves("spectrum.harmonics_db"),
        Position = ParamSpec::float("position", "Position", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Where in the wavetable to play, from its first frame to its last; only for the table wave.")
            .moves("spectrum.centroid_hz"),
        PhaseMode = ParamSpec::enumeration("phase_mode", "Phase mode", &PHASE_MODES, PhaseMode::Free.index())
            .advanced("How each note's start phase is chosen: free (random per render seed), reset (the phase knob) or seeded (from seed)."),
        Phase = ParamSpec::float("phase", "Start phase", Unit::Degrees, 0.0, 360.0, Scale::Linear, 0.0)
            .periodic()
            .advanced("The start phase in reset mode; between two oscillators at the same pitch it changes the timbre."),
        Seed = ParamSpec::int("seed", "Seed", Unit::None, 0, 65_535, 0)
            .not_automatable()
            .advanced("The random seed used in seeded phase mode; part of the patch, unlike the render seed."),
    }
}
