//! The `wt` module: a **silent stub** from S0d (S2a, freeze F0m), to be replaced by lane WT.
//!
//! STUB (S0d): writes zeros. Lane WT replaces it with a wavetable oscillator on phase-read waves: generated tables,
//! phase warps, phase and amplitude modulation from another generator through audio routes (`pm`, `am`; bipolar AM is
//! ring modulation), `feedback`, and later unison on `wide` lanes (lane UNI) (docs/phase-s2.md; its catalogue table is
//! lane CAT's, in `enth-patch/src/catalogue/wt.rs`). Keep `Wt::new(values, cfg)` and the `VoiceProcessor` impl; the
//! kind's dispatch arms in `modules/mod.rs` are already written. The kind is hidden from the Add menu, `enth modules`
//! and the generator until lane INT-S2a clears its flag.

use crate::module::{PrepareCfg, SEG, VoiceCx, VoiceProcessor, VoiceStart};

/// The `wt` generator (S0d stub: silent).
#[derive(Clone, Debug)]
pub struct Wt;

impl Wt {
    /// Prepare a `wt` generator.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl VoiceProcessor for Wt {
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
