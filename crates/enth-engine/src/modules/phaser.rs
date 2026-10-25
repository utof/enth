//! The `phaser` module: a **pass-through stub** from S0d (S2a, freeze F0m), to be replaced by lane MOD.
//!
//! STUB (S0d): leaves the stereo bus unchanged. Lane MOD replaces it with a stereo phaser (a chain of modulated
//! all-pass filters) on MOD's modulated-delay core; `mix` (docs/phase-s2.md; its catalogue table is lane CAT's, in
//! `enth-patch/src/catalogue/phaser.rs`). Keep `Phaser::new(values, cfg)` and the `GlobalProcessor` impl; the kind's
//! dispatch arms in `modules/mod.rs` are already written. The kind is hidden from the Add menu, `enth modules` and the
//! generator until lane INT-S2a clears its flag.

use crate::module::{GlobalCx, GlobalProcessor, GlobalStart, PrepareCfg};

/// The `phaser` effect (S0d stub: a pass-through).
#[derive(Clone, Debug)]
pub struct Phaser;

impl Phaser {
    /// Prepare a `phaser` effect.
    #[must_use]
    pub fn new(_values: &[f32], _cfg: &PrepareCfg) -> Self {
        Self
    }
}

impl GlobalProcessor for Phaser {
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
