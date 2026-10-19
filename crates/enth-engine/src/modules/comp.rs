//! The `comp` module: a **pass-through stub** from S2a, to be replaced by lane FX2 (phase 2, the wrapper).
//!
//! STUB (S2a): leaves the stereo bus untouched. FX2 wraps `enth_dsp::compressor::Compressor` here: `prepare(cfg
//! .sample_rate)` in `new`, `set_params` from the parameter views once per segment (`mix` maps onto `mix_pct` as
//! is), `process_in_place(left, right)`, and `is_silent` true only when processing silence would leave the state
//! unchanged. Keep `Comp::new(values, cfg)` and the `GlobalProcessor` impl.

use crate::module::{GlobalCx, GlobalProcessor, GlobalStart, PrepareCfg};

/// The compressor (S2a stub: pass-through).
#[derive(Clone, Debug)]
pub struct Comp;

impl Comp {
    /// Prepare a compressor.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl GlobalProcessor for Comp {
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
