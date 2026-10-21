//! **Owner: lane DG** (the dataset generator, wave 6, after F3s). Pre-declared by lane S0s (R3); refuses to run until DG
//! writes it.
//!
//! `shards`: deterministic shards of clean renders and their labels (docs/r3.md, "`ds1`"): a shard is a pure function of
//! the sampler version, its stream key, its index and the render seed, so it can be re-rendered bit for bit and 5 to 10 %
//! of the shards are re-rendered per epoch instead of cached. The CPU is the bottleneck (10 to 28 ms per dry 3 s render,
//! 130 to 175 ms with the reverb); throughput is reported (DG's acceptance). Shards are files `ml/` reads; a training
//! cache stays under about 70 GB and outside the repository.

/// `shards`: not built yet.
pub fn run(_args: &[String]) -> Result<(), String> {
    Err("`shards` is not built yet (R3 lane DG)".to_owned())
}
