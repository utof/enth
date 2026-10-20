//! enth-patch (L1): the patch model, the module catalogue, addresses, ops, the file layer, validation, canonical
//! forms, hashes, history and migrations. See docs/design.md §2 and docs/architecture.md.
//!
//! - [`catalogue`]: every module kind ([`ModuleKind`], [`ModuleSpec`], [`CATALOGUE`]) with its `const`
//!   `ParamSpec` table and index enum (`OscParam`, `SvfParam`, …), the built-in `voice` and `master`, the macro
//!   knob, and the Rust enums of enum parameters (`Wave`, `FilterMode`, …). Checked at compile time; snapshotted
//!   in `tests/snapshots/`.
//! - [`model`]: [`Patch`] in memory (a dense `Vec<f32>` per module, in spec order), [`ModuleInstance`], [`Route`],
//!   [`Macro`], [`VoiceSettings`], [`PatchMeta`], [`Section`] and the limits.
//! - [`addr`]: [`ModuleId`], [`ParamAddr`] (`filter1.cutoff`, `voice.glide`, `master.level`), [`ModSource`],
//!   [`RouteAddr`], [`Position`], [`MacroIndex`], [`SlotIndex`], with text parse/display and serde.
//! - [`ops`]: the edits. **Every patch mutation is an [`Op`]**; [`apply`] returns the inverse ops, [`apply_all`] is
//!   atomic, and [`ops::effect`] says whether an op needs a plan rebuild. [`history`] keeps undo and redo over
//!   them, merging a knob drag only for one address.
//! - [`error`]: one vocabulary of [`PatchError`]s (a stable code, a message, a hint, `did_you_mean`) shared by ops,
//!   validation, the file layer, migrations and the ops JSON; [`Invalid`] lists every [`Violation`] with a path.
//! - [`validate`](mod@validate): every invariant of a patch, all violations at once, in the order of a file.
//! - [`file`](mod@file): the [`PatchFile`](file::PatchFile) serde types (sparse in, dense out), [`file::load`], and
//!   [`Patch::to_json`] (written from the typed file, never through `serde_json::Value`, which widens an `f32` to
//!   `f64` text: `0.1` would print as `0.10000000149011612`).
//! - [`migrate`](mod@migrate): pure `Value -> Value` steps, one per schema version, with fixtures under
//!   `tests/fixtures/patch/`.
//! - [`modulation`](mod@modulation): [`modulation_range`], how far a parameter's routes can move it (normalised, by
//!   each source's polarity), for the editor's depth arcs and the CLI's `describe` (P1b lane ED2).
//! - [`canon`]: [`canon::exact_hash`] ("is it the same patch?") and [`canon::canonical_hash`] ("is it the same up to
//!   symmetry?"); the first enabled `osc` is the hard-sync master and keeps its place.
//! - [`wire`]: the JSON form of ops the CLI speaks (`{"set": "filter1.cutoff", "to": "1.2kHz"}`), resolved against
//!   the patch with typed, path-carrying errors.
//!
//! Module ids, the order of [`ModuleKind`] discriminants and [`Section::ALL`], and the byte encodings behind the
//! two hashes are persisted contracts: change them only with a migration or a new hash magic.

pub mod addr;
pub mod canon;
pub mod catalogue;
pub mod error;
pub mod file;
pub mod history;
pub mod migrate;
pub mod model;
pub mod modulation;
pub mod ops;
pub mod validate;
pub mod wire;

pub use addr::{
    AddrError, MacroIndex, ModSource, ModuleId, ParamAddr, Position, RouteAddr, SlotIndex, Target,
};
pub use catalogue::{Builtin, CATALOGUE, ModuleKind, ModuleSpec, Rate};
pub use error::{Invalid, OpError, PatchError, Violation};
pub use model::{Macro, ModuleInstance, Patch, PatchMeta, Route, Section, VoiceSettings};
pub use modulation::{ModulationRange, modulation_range};
pub use ops::{Op, apply, apply_all};
