//! The `delay` module: a **pass-through stub** from S0d (S2a, freeze F0m), to be replaced by lane DLY.
//!
//! STUB (S0d): leaves the stereo bus unchanged. Lane DLY replaces it with a stereo delay after the voices, its time in
//! ms or synced in beats to the render's tempo; `mix` (docs/phase-s2.md; its catalogue table is lane CAT's, in
//! `enth-patch/src/catalogue/delay.rs`). Keep `Delay::new(values, cfg)` and the `GlobalProcessor` impl; the kind's
//! dispatch arms in `modules/mod.rs` are already written. The kind is hidden from the Add menu, `enth modules` and the
//! generator until lane INT-S2a clears its flag.

use crate::module::{GlobalCx, GlobalProcessor, GlobalStart, PrepareCfg};

/// The `delay` effect (S0d stub: a pass-through).
#[derive(Clone, Debug)]
pub struct Delay;

impl Delay {
    /// Prepare a `delay` effect.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl GlobalProcessor for Delay {
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
