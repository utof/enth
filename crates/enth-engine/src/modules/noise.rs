//! The `noise` module (P1a lane D1): white (xorshift32) or pink (J. O. Smith's third-order pinking filter) noise, one
//! independent stream per voice. The DSP is in `enth_dsp::noise`; sources and the measured slope are in
//! `docs/dsp/noise.md`.
//!
//! - **Seeding.** A note's stream starts from `VoiceStart::seed`, which the engine derives from the render seed, the
//!   module id and the note index, so every note has its own stream, a render is reproducible, and no parameter
//!   (including `color` and `level`) ever reseeds anything.
//! - **Level** ramps per sample from the previous segment's value to this one's, as for `osc`.
//! - **Colour** is read from `params.base`. Switching to pink mid-note starts the filter from the state it had
//!   (cold at the note start), so the first tens of milliseconds of pink lack their lowest octaves; the filter is
//!   stable, so nothing blows up whatever the history.

use enth_dsp::noise::NoiseVoice;
use enth_patch::catalogue::{NoiseColor, NoiseParam};

use crate::module::{
    Block, MAX_VOICES, PrepareCfg, RAMP, SEG, VoiceCx, VoiceProcessor, VoiceStart,
};

/// The noise source.
#[derive(Clone, Debug)]
pub struct Noise {
    voices: [NoiseVoice; MAX_VOICES],
}

impl Noise {
    /// Prepare a noise source (no allocation).
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self {
            voices: [NoiseVoice::new(0); MAX_VOICES],
        }
    }
}

/// `out[i] *= g0 + (g1 − g0)·RAMP[i]`, skipped when the gain is exactly 1 throughout.
#[inline(always)]
fn apply_level(out: &mut Block, g0: f32, g1: f32) {
    if g0 == 1.0 && g1 == 1.0 {
        return;
    }
    let step = g1 - g0;
    for (o, r) in out.iter_mut().zip(RAMP) {
        *o *= g0 + step * r;
    }
}

impl VoiceProcessor for Noise {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        self.voices[start.voice] = NoiseVoice::new(start.seed);
    }

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        let p = cx.params;
        let pink = NoiseColor::from_value(p.base(NoiseParam::Color)) == Some(NoiseColor::Pink);
        for &v in cx.voices {
            let out: &mut [f32; SEG] = &mut cx.io[v];
            if pink {
                self.voices[v].pink(out);
            } else {
                self.voices[v].white(out);
            }
            apply_level(
                out,
                p.prev(NoiseParam::Level, v) * 0.01,
                p.cur(NoiseParam::Level, v) * 0.01,
            );
        }
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.voices = other.voices;
    }

    fn subnormals(&self) -> usize {
        self.voices.iter().filter(|v| v.has_subnormal()).count()
    }
}
