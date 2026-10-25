//! The `ladder` module: a **pass-through stub** from S0d (S2a, freeze F0m), to be replaced by lane FLT.
//!
//! STUB (S0d): leaves the voice bus unchanged. Lane FLT replaces it with a four-pole ladder low-pass per voice (a TPT
//! Moog ladder with a saturator); the first item on the cut list (docs/phase-s2.md; its catalogue table is lane CAT's,
//! in `enth-patch/src/catalogue/ladder.rs`). Keep `Ladder::new(values, cfg)` and the `VoiceProcessor` impl; the kind's
//! dispatch arms in `modules/mod.rs` are already written. The kind is hidden from the Add menu, `enth modules` and the
//! generator until lane INT-S2a clears its flag.

use crate::module::{PrepareCfg, VoiceCx, VoiceProcessor, VoiceStart};

/// The `ladder` per-voice effect (S0d stub: a pass-through).
#[derive(Clone, Debug)]
pub struct Ladder;

impl Ladder {
    /// Prepare a `ladder` effect.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl VoiceProcessor for Ladder {
    fn start_voice(&mut self, _start: &VoiceStart<'_>) {}

    fn process(&mut self, _cx: &mut VoiceCx<'_>) {}

    fn copy_state_from(&mut self, _other: &Self) {}

    fn subnormals(&self) -> usize {
        0
    }
}
