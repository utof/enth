//! **Owner: lane LCK.** Pre-declared by lane S0l (R2); the command is built in `tools/lesson/src/pack.rs`.
//!
//! `cargo xtask pack [--out DIR] [--check] [PACK...]`: a driver for `enth-lesson-tool pack`, which builds
//! `content/lessons/<pack>/` into a reproducible `.enthpack` and compares it with its pins (the plugin's bundled
//! archive, a release catalogue). `--out crates/enth-plugin/packs` rebuilds the bundled archives on purpose.

/// `cargo xtask pack`: runs `enth-lesson-tool pack`.
pub fn pack(args: &[String]) -> Result<(), String> {
    super::drive("pack", args)
}
