//! A minimal CLAP test host on `clack-host` (Phase 0 lane H, `docs/spikes/test-host.md`).
//!
//! The library holds the pieces so tests can drive them with an in-process plugin; the `enth-test-host`
//! binary is the command line around them.
#![allow(
    unsafe_code,
    reason = "a host loads a plugin library and hands it a window handle; both are unsafe in clack-host"
)]

pub mod editor;
pub mod host;
pub mod instances;
pub mod notes;
pub mod procfs;
pub mod query;
pub mod render;
pub mod wav;
pub mod xvfb;
