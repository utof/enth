//! enth-session (L6, see docs/architecture.md): the one single-threaded `Session` that owns the `Patch`, its
//! `History`, the engine `Link` and the `PlanMap`. The editor and the plugin shell change a patch only through it;
//! each patch mutation is an `enth_patch::Op`, and the session decides whether an op becomes a value event or a
//! recompile and plan swap.
//!
//! Like the core crates it does no I/O, spawns no threads, reads no clocks and takes no locks (the root `clippy.toml`
//! bans apply): the plugin wraps it in a `Mutex` shared by the host main thread and the GUI thread, and the audio
//! thread never touches it.
//!
//! The modules are declared up front (P1b lane S0b) so the lanes that fill them never edit this file or each other's:
//!
//! | Module | Holds | Lane |
//! | --- | --- | --- |
//! | [`model`] | `Session`, the editor's read model and edit trait, public ids and errors | S1s |
//! | [`apply`] | ops to effects: value event, depth event, recompile and swap, history and gestures | SK, S2s |
//! | [`bridge`] | the host-parameter bridge: macros and automation slots, names, rescan requests | SK, S2s |
//! | [`state`] | the saved blob (`enth.plugin-state/1`), fixtures, versions and migrations | S1s, S2s |
//! | [`link`] | delivery to the engine: plans, epochs, the return ring, who drains it | SK |
//! | [`telemetry`] | the reader types for the engine's read-only telemetry tap | S1s, ED2 |

pub mod apply;
pub mod bridge;
pub mod link;
pub mod model;
pub mod state;
pub mod telemetry;
