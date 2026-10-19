//! The `lfo` module: a **stub** from S2a that outputs 0, to be replaced by lane D2.
//!
//! One type serves both roles: a voice-scope LFO is a `VoiceModulator` (started at each note-on), a global-scope one
//! a `GlobalModulator` (started at engine start); `ModuleImpl::new` picks the role from the structural `scope`
//! parameter.
//!
//! STUB (S2a): every output is 0 (no modulation). D2 replaces it with a u32 phase accumulator advanced once per
//! segment, the shapes, bipolar/unipolar polarity, and sample-and-hold / smooth random drawn from the seed passed in
//! (`VoiceStart::seed` or `GlobalStart::seed`, or the module's own `seed` parameter in the `seeded` phase mode),
//! never from the patch. Keep `Lfo::new(values, cfg)` and both trait impls.

use crate::module::{
    GlobalModCx, GlobalModulator, GlobalStart, PrepareCfg, VoiceIdx, VoiceModCx, VoiceModulator,
    VoiceStart,
};

/// The LFO (S2a stub: outputs 0).
#[derive(Clone, Debug)]
pub struct Lfo;

impl Lfo {
    /// Prepare an LFO.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl VoiceModulator for Lfo {
    fn start_voice(&mut self, _start: &VoiceStart<'_>) {}

    fn tick(&mut self, cx: &mut VoiceModCx<'_>) {
        for &v in cx.voices {
            cx.out[v] = 0.0;
        }
    }

    fn voice_finished(&self, _v: VoiceIdx) -> bool {
        false
    }

    fn copy_state_from(&mut self, _other: &Self) {}

    fn subnormals(&self) -> usize {
        0
    }
}

impl GlobalModulator for Lfo {
    fn reset(&mut self, _start: &GlobalStart<'_>) {}

    fn tick(&mut self, _cx: &mut GlobalModCx<'_>) -> f32 {
        0.0
    }

    fn copy_state_from(&mut self, _other: &Self) {}

    fn subnormals(&self) -> usize {
        0
    }
}
