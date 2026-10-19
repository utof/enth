//! The `svf` module: a **pass-through stub** from S2a, to be replaced by lane D2.
//!
//! STUB (S2a): leaves the voice bus untouched. D2 replaces it with the Cytomic/Simper TPT state-variable filter
//! (LP, BP, HP, notch) with a rational-tanh drive, `g = tan(π·fc/fs)` computed from the parameter views once per
//! segment and the coefficients `a1..a3` interpolated per sample from `prev` to `cur` (design.md §3), key tracking
//! from `NoteState::pitch_cur`, the cutoff clamped below Nyquist, and its two integrator states flushed with
//! `enth_dsp::util::flush::flush`. Keep `Svf::new(values, cfg)` and the `VoiceProcessor` impl.

use crate::module::{PrepareCfg, VoiceCx, VoiceProcessor, VoiceStart};

/// The state-variable filter (S2a stub: pass-through).
#[derive(Clone, Debug)]
pub struct Svf;

impl Svf {
    /// Prepare a filter.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl VoiceProcessor for Svf {
    fn start_voice(&mut self, _start: &VoiceStart<'_>) {}

    fn process(&mut self, _cx: &mut VoiceCx<'_>) {}

    fn copy_state_from(&mut self, _other: &Self) {}

    fn subnormals(&self) -> usize {
        0
    }
}
