//! `adsr`: an envelope per note with RC-style exponential segments whose overshoot targets make every segment end
//! in exactly its set time, from any start level (segment-duration semantics), evaluated per 32-frame segment.
//! Lane D2 owns the DSP and `docs/dsp/envelope.md`.
//!
//! Output runs 0–1 (envelopes are unipolar sources). One ADSR is the voice's amplitude envelope (`voice.amp_env`);
//! the others are routed like any modulator. Times start at 0 (the engine's shortest segment is one 32-frame step,
//! 0.67 ms at 48 kHz) and use `Power(4)`, which puts 625 ms at the middle of a 0–10 s knob.

use enth_params::{ParamSpec, Scale, Unit, params};

use super::SMOOTH;

const TIME: Scale = Scale::Power(4.0);

params! {
    /// Parameters of the `adsr` module, in spec order.
    pub enum AdsrParam in ADSR_PARAMS {
        Attack = ParamSpec::float("attack", "Attack", Unit::Ms, 0.0, 10_000.0, TIME, 5.0)
            .modulatable()
            .basic("Time to rise from the current level to full at note-on; under 10 ms sounds plucky, over 200 ms swells.")
            .moves("envelope.attack_ms"),
        Decay = ParamSpec::float("decay", "Decay", Unit::Ms, 0.0, 10_000.0, TIME, 300.0)
            .modulatable()
            .basic("Time to fall from full to the sustain level.")
            .moves("envelope.decay_ms"),
        Sustain = ParamSpec::float("sustain", "Sustain", Unit::Percent, 0.0, 100.0, Scale::Linear, 70.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("The level held while the note is down, as a percentage of full; 0 % makes a pluck.")
            .moves("envelope.sustain_db"),
        Release = ParamSpec::float("release", "Release", Unit::Ms, 0.0, 20_000.0, TIME, 400.0)
            .modulatable()
            .basic("Time to fall to silence after note-off.")
            .moves("envelope.release_ms"),
        Velocity = ParamSpec::float("velocity", "Velocity", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .intermediate("How much note velocity scales this envelope's output: 0 % ignores velocity, 100 % follows it fully."),
    }
}
