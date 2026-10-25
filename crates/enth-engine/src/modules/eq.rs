//! The `eq` module: a **pass-through stub** from S0d (S2a, freeze F0m), to be replaced by lane EQ.
//!
//! STUB (S0d): leaves the stereo bus unchanged. Lane EQ replaces it with a parametric equaliser after the voices,
//! wrapping `enth-dsp`'s biquad (lane BQ's file), with per-segment coefficient ramps; `mix` (docs/phase-s2.md; its
//! catalogue table is lane CAT's, in `enth-patch/src/catalogue/eq.rs`). Keep `Eq::new(values, cfg)` and the
//! `GlobalProcessor` impl; the kind's dispatch arms in `modules/mod.rs` are already written. The kind is hidden from
//! the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.

use crate::module::{GlobalCx, GlobalProcessor, GlobalStart, PrepareCfg};

/// The `eq` effect (S0d stub: a pass-through).
#[derive(Clone, Debug)]
pub struct Eq;

impl Eq {
    /// Prepare a `eq` effect.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl GlobalProcessor for Eq {
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
