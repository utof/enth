//! enth-analysis (see docs/architecture.md).
//!
//! Every module below is pre-declared by P1a lane S0 (freeze point F0) as an empty file naming the lane that fills
//! it, so no lane edits this file. To split a module into submodules, turn `x.rs` into `x/mod.rs`: `mod x;` still
//! resolves.

pub mod beat;
pub mod descriptors;
pub mod expect;
pub mod features;
pub mod loudness;
pub mod margin;
pub mod pitch;
