//! The `dist` module: a **pass-through stub** from S0d (S2a, freeze F0m), to be replaced by lane DST.
//!
//! STUB (S0d): leaves the voice bus unchanged. Lane DST replaces it with distortion per voice: soft and hard clipping,
//! sine and triangle folds, asymmetric shaping, bitcrush and downsample, with ADAA-1 and zero-latency 2x or 4x IIR
//! oversampling; drive, bias, tone and `mix` (docs/phase-s2.md; its catalogue table is lane CAT's, in
//! `enth-patch/src/catalogue/dist.rs`). Keep `Dist::new(values, cfg)` and the `VoiceProcessor` impl; the kind's
//! dispatch arms in `modules/mod.rs` are already written. The kind is hidden from the Add menu, `enth modules` and the
//! generator until lane INT-S2a clears its flag.

use crate::module::{PrepareCfg, VoiceCx, VoiceProcessor, VoiceStart};

/// The `dist` per-voice effect (S0d stub: a pass-through).
#[derive(Clone, Debug)]
pub struct Dist;

impl Dist {
    /// Prepare a `dist` effect.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl VoiceProcessor for Dist {
    fn start_voice(&mut self, _start: &VoiceStart<'_>) {}

    fn process(&mut self, _cx: &mut VoiceCx<'_>) {}

    fn copy_state_from(&mut self, _other: &Self) {}

    fn subnormals(&self) -> usize {
        0
    }
}
