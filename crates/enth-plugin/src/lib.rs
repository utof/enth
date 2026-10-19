//! enth-plugin (L8, see docs/architecture.md): the CLAP and VST3 shell around `enth-session`, `enth-engine` and
//! `enth-ui`, built on nice-plug (docs/adr/0001-plugin-shell.md). It translates between the host and the product
//! and holds no product rules: parameters, notes, audio blocks, state and the editor window.
//!
//! Threads: the audio thread owns the `Engine` and never locks; the host main thread and baseview's GUI thread
//! share the `Session` behind one `Mutex`. Never call `GuiContext::set_state` (nice-plug would load the state on
//! the audio thread) and never schedule GUI work from `process()`.
//!
//! The modules are declared up front (P1b lane S0b) so the lanes that fill them never edit this file or each
//! other's:
//!
//! | Module | Holds | Lane |
//! | --- | --- | --- |
//! | [`params`] | the 72 host parameters (8 macros, 64 automation slots), stable ids, value text | SK |
//! | [`process`] | the audio block: events and notes in, the engine, audio out | SK, PL |
//! | [`notes`] | the note tuple, `NOTE_END`, voice info, polyphonic modulation with host voice ids | PL |
//! | [`state`] | the persistent fields bridging nice-plug's state and the session blob | SK |
//! | [`editor`] | the `NiceEguiApp` window around the `enth-ui` editor | SK, ED1 |
#![allow(
    unsafe_code,
    reason = "nice_export_clap! and nice_export_vst3! expand to extern \"C\" entry points"
)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "shell crate: it owns the plugin's threads, locks, clocks and I/O (docs/architecture.md rule 2 binds the core crates, L0-L5)"
)]

pub mod editor;
pub mod notes;
pub mod params;
pub mod process;
pub mod state;
