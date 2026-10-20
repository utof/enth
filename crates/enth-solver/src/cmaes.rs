//! **Owner: lane MT** (match). Pre-declared empty by lane S0r (R1/S1).
//!
//! CMA-ES, ported from the Phase 0 metric spike (`spikes/spike-metric/src/cmaes.rs`) into the solver (summary "Coach
//! and CLI"): `enth match` and the agent and solver floors on golden challenges. Seeded streams from `enth_dsp`'s
//! `derive`, maths through `libm`, batches through the caller's [`Evaluator`](crate::Evaluator), so a run is identical
//! for any worker count. MT reports the golden-challenge solve rate.
