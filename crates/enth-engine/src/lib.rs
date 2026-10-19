//! enth-engine (see docs/architecture.md).
//!
//! Every module below is pre-declared by P1a lane S0 (freeze point F0) as an empty file naming the lane that fills
//! it, so no lane edits this file. To split a module into submodules, turn `x.rs` into `x/mod.rs`: `mod x;` still
//! resolves.

// RealtimeSanitizer (nightly workflow, `--cfg enth_rtsan`): `process` and everything it calls carry
// `#[cfg_attr(enth_rtsan, sanitize(realtime = "nonblocking"))]`. The cfg is declared in the workspace lints.
#![cfg_attr(enth_rtsan, feature(sanitize))]

pub mod modulation;
pub mod module;
pub mod modules;
pub mod plan;
pub mod seed;
pub mod segment;
pub mod swap;
pub mod voice;
