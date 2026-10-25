//! The `mseg` module: a **stub that outputs 0** from S0d (S2a, freeze F0m), to be replaced by lane MSEG.
//!
//! STUB (S0d): writes 0 for every active voice and never ends one. Lane MSEG replaces it with a multi-segment envelope
//! with a fixed point count (lane CAT decides it), a curve per segment, a length synced in beats or in ms, loop,
//! one-shot and sustain-loop modes (docs/phase-s2.md; its catalogue table is lane CAT's, in
//! `enth-patch/src/catalogue/mseg.rs`). Keep `Mseg::new(values, cfg)` and the `VoiceModulator` impl; the kind's
//! dispatch arms in `modules/mod.rs` are already written. The kind is hidden from the Add menu, `enth modules` and the
//! generator until lane INT-S2a clears its flag.

use crate::module::{PrepareCfg, VoiceIdx, VoiceModCx, VoiceModulator, VoiceStart};

/// The `mseg` modulator (S0d stub: outputs 0).
#[derive(Clone, Debug)]
pub struct Mseg;

impl Mseg {
    /// Prepare a `mseg` modulator.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl VoiceModulator for Mseg {
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
