//! enth-session (L6, see docs/architecture.md and docs/session.md): the one single-threaded [`Session`] that owns the
//! `Patch`, its `History`, the engine `Link` and the `PlanMap`. The editor and the plugin shell change a patch only
//! through it; each patch mutation is an `enth_patch::Op`, and the session decides whether an op becomes a value
//! event, a depth event, a macro event or a recompile and plan swap.
//!
//! Like the core crates it does no I/O, spawns no threads, reads no clocks and takes no locks (the root `clippy.toml`
//! bans apply): the plugin wraps it in a `Mutex` shared by the host main thread and the GUI thread, and **the audio
//! thread never touches it**. The threading contract is in [`model`] and, in full, in `docs/session.md`.
//!
//! | Module | Holds | Lane |
//! | --- | --- | --- |
//! | [`model`] | [`Session`], the editor's read model ([`SessionView`]) and edit trait ([`SessionEdit`]), the shell's request outbox ([`ShellRequest`]), status and errors | S1s |
//! | [`apply`] | ops to deliveries ([`apply::route`], [`apply::delivery`]); gestures; edit, undo and redo | S1s types; S2s undo, redo and the shared delivery; SK `edit` |
//! | [`bridge`] | the 72 host parameters ([`HostParam`]), the patch-wins fold against the plan's value ([`bridge::fold_value`], [`bridge::fold_against`], [`Session::fold_host`]), host writes, names and when the host must rescan | S1s; S2s |
//! | [`state`] | the saved state v1: the `patch` field and the `enth` field ([`InstanceState`], `enth.plugin-state/1`), fixtures, migrations, the host-load history policy ([`state::HistoryFate`]) | S1s; S2s |
//! | [`link`] | activation, the pump, epochs, bounded queues, audition notes, who drains the return ring | S1s contract; SK bodies |
//! | [`telemetry`] | the tap's reader side ([`TelemetrySource`], [`TelemetryReader`], [`TelemetryMap`]) | S1s; ED2 |
//! | [`game`] | the pure Recreate round: states, stash, submit, reveal, jobs for the shell; [`GameState`] in the saved state | SG (R1/S1; pre-declared by S0r) |
//! | [`presets`] | preset format, index, tags, search, the undoable editor load; [`PresetState`] in the saved state | PBC (R1/S1; pre-declared by S0r) |
//!
//! P1b stubs (lane S1s): [`Session::activate`], [`Session::pump`]'s active case, [`Session::edit`] and the audition
//! calls return [`SessionError::Unsupported`] (or `Inactive`) until lane SK fills them in. Everything else is
//! implemented and tested: the bridge rules (folded against the plan's value), the state blob and its versions, the
//! delivery classification and delivery, gestures, undo and redo, the host-load history policy, the read model and
//! the telemetry reader. `tests/engine.rs` holds the public-API tests that need SK's bodies (ignored until then).

pub mod apply;
pub mod bridge;
pub mod game;
pub mod link;
pub mod model;
pub mod presets;
pub mod state;
pub mod telemetry;

#[cfg(test)]
mod tests;

pub use apply::{Delivery, Routing};
pub use bridge::{FoldReport, GestureStep, HostParam, HostValues, HostWrite, SlotTarget};
pub use enth_patch::history::GestureId;
pub use game::GameState;
pub use link::PumpReport;
pub use model::{
    AUDITION_PORT, Activation, Applied, HistoryView, RENDER_SEED, Session, SessionConfig,
    SessionEdit, SessionError, SessionStatus, SessionView, ShellRequest,
};
pub use presets::{
    Favorites, Library, MemStore, PresetEntry, PresetError, PresetKey, PresetLoad, PresetState,
    PresetStore, Query, RawPreset, SaveRequest, Source,
};
pub use state::{
    InstanceState, LoadReport, MonitorState, STATE_SCHEMA, SavedState, StateError, UiState,
};
pub use telemetry::{TelemetryFrame, TelemetryMap, TelemetryReader, TelemetrySource};
