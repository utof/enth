//! **Owner: lane DG** (the dataset generator, wave 6, after F3s). Pre-declared by lane S0s (R3); refuses to run until DG
//! writes it.
//!
//! `exclude`: held-out hygiene (docs/r3.md, "GR3 and its hygiene"): DG asserts that no training item is on the exclusion
//! lists of `content/gr3/` (lane DS), re-run against every dev and test item after each refresh, with dedup by an exact
//! hash of the masked canonical labels and then by pooled log-mel embeddings within about one just-noticeable step. The
//! sets freeze with their hashes before any training starts.

/// `exclude`: not built yet.
pub fn run(_args: &[String]) -> Result<(), String> {
    Err("`exclude` is not built yet (R3 lane DG)".to_owned())
}
