//! **Owner: lane LCK.** Pre-declared by lane S0l (R2) as a stub that refuses to run.
//!
//! `cargo xtask pack [PACK...]`: builds `content/lessons/<pack>/` into a reproducible `.enthpack` (docs/learn.md,
//! "Packs"): authors write TOML, this writes canonical JSON into a zip with sorted entries, 1980 timestamps and a fixed
//! deflate level (`miniz_oxide`, pinned with `=`), the manifest `enth.pack/1` listing every file's size and SHA-256.
//! It also checks that each bundled pack's hash matches the build's pin.

/// `cargo xtask pack`: not built yet.
pub fn pack(_args: &[String]) -> Result<(), String> {
    Err("pack is not built yet (R2 lane LCK; docs/phase-3.md)".to_owned())
}
