//! **Owner: lane DG** (the dataset generator, wave 6, after F3s). Pre-declared by lane S0s (R3); refuses to run until DG
//! writes it.
//!
//! `ds1` (docs/r3.md, "`ds1`: training data from our engine"): generator v1 is unfit as training data (level 1 has about
//! 12 sounds, every phrase starts on C3, `amounts` below 100 pull values towards the start), so a new versioned sampler
//! reuses v1's keyed draws, priors and `observable()` mask from `enth-challenge` and adds a mixture (60 % v1 priors at
//! `amounts` 100 and level 5, 25 % perturbed preset candidates **from the anchor half only**, 15 % uniform over the
//! safe ranges), a phrase sampler, labels in the `ParamSpec` normalised domain with a loss mask, filters (renders below
//! -60 LUFS rejected; clipped renders kept; **never a filter by duration**) and splits by stream key (`r3/train`,
//! `r3/dev`, `r3/test`), never by index ranges. Where `ds1` lives and whether features reach `ml/` through `enth-py` or
//! files this package writes are DG's open checks at F3s; the default is here, with files.

/// `ds1`: not built yet.
pub fn run(_args: &[String]) -> Result<(), String> {
    Err("`ds1` is not built yet (R3 lane DG)".to_owned())
}
