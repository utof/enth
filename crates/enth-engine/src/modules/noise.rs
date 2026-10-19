//! The `noise` module: a **silent stub** from S2a, to be replaced by lane D1.
//!
//! STUB (S2a): writes zeros. D1 replaces it with white noise (xorshift32) and pink noise (J.O. Smith's third-order
//! pinking filter), seeded per voice from `VoiceStart::seed` (render seed, module id, note index), so every note
//! has its own stream and nothing depends on parameter values. Keep `Noise::new(values, cfg)` and the
//! `VoiceProcessor` impl.

use crate::module::{PrepareCfg, SEG, VoiceCx, VoiceProcessor, VoiceStart};

/// The noise source (S2a stub: silent).
#[derive(Clone, Debug)]
pub struct Noise;

impl Noise {
    /// Prepare a noise source.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl VoiceProcessor for Noise {
    fn start_voice(&mut self, _start: &VoiceStart<'_>) {}

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        for &v in cx.voices {
            cx.io[v] = [0.0; SEG];
        }
    }

    fn copy_state_from(&mut self, _other: &Self) {}

    fn subnormals(&self) -> usize {
        0
    }
}
