//! enth-patch (see docs/architecture.md).
//!
//! Every module below is pre-declared by P1a lane S0 (freeze point F0) as an empty file naming the lane that fills
//! it, so no lane edits this file. To split a module into submodules, turn `x.rs` into `x/mod.rs`: `mod x;` still
//! resolves.

pub mod addr;
pub mod canon;
pub mod catalogue;
pub mod file;
pub mod history;
pub mod migrate;
pub mod model;
pub mod ops;
pub mod validate;
pub mod wire;
