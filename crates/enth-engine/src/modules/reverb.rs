//! The `reverb` module: a **pass-through stub** from S2a, to be replaced by lane FX1 (phase 2, the wrapper).
//!
//! STUB (S2a): leaves the stereo bus untouched. FX1 wraps `enth_dsp::reverb::Reverb` here: `Reverb::prepare(cfg
//! .sample_rate)` in `new` (its 1.64 MB is sized from the sample rate only), the parameter conversions documented in
//! `enth_patch::catalogue` (`reverb.rs`: dB → upstream percentages with −60 dB as off, `decay` RT60 ms → RT20 s,
//! the rest as is) applied with `set_params` once per segment, `process_in_place(left, right)`, and `is_silent`
//! true only when the core's state is all zeros and no fade is in flight. Keep `Reverb::new(values, cfg)` and the
//! `GlobalProcessor` impl.

use crate::module::{GlobalCx, GlobalProcessor, GlobalStart, PrepareCfg};

/// The reverb (S2a stub: pass-through).
#[derive(Clone, Debug)]
pub struct Reverb;

impl Reverb {
    /// Prepare a reverb.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl GlobalProcessor for Reverb {
    fn reset(&mut self, _start: &GlobalStart<'_>) {}

    fn process(&mut self, _cx: &mut GlobalCx<'_>) {}

    fn is_silent(&self) -> bool {
        true
    }

    fn copy_state_from(&mut self, _other: &Self) {}

    fn subnormals(&self) -> usize {
        0
    }
}
