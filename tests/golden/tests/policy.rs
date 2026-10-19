//! Test-suite policy that a lane could break without noticing.
#![cfg(not(target_family = "wasm"))] // a WASI sandbox gets no environment variables, so nothing sets the seed there

use proptest::test_runner::{Config, RngSeed};

/// The gate runs proptest with a fixed seed (`PROPTEST_RNG_SEED` from `.cargo/config.toml`'s `[env]`), so a red
/// or green run is the same for every agent. The nightly job overrides it with a fresh seed and a larger
/// `PROPTEST_CASES`; that is still a fixed seed per run, printed so a failure can be replayed. A random seed
/// here means the policy in `.config/nextest.toml` is not reaching the test processes.
#[test]
fn the_gate_runs_proptest_with_a_fixed_seed() {
    assert!(
        matches!(Config::default().rng_seed, RngSeed::Fixed(_)),
        "proptest would pick a random seed: PROPTEST_RNG_SEED is unset. See .cargo/config.toml and .config/nextest.toml"
    );
}
