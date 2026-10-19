//! enth-patch (L1): the patch model, the module catalogue, addresses, and (lane S1b) ops, the file layer,
//! validation, canonical forms, hashes, history and migrations. See docs/design.md §2 and docs/architecture.md.
//!
//! - [`catalogue`]: every module kind ([`ModuleKind`], [`ModuleSpec`], [`CATALOGUE`]) with its `const`
//!   `ParamSpec` table and index enum (`OscParam`, `SvfParam`, …), the built-in `voice` and `master`, the macro
//!   knob, and the Rust enums of enum parameters (`Wave`, `FilterMode`, …). Checked at compile time; snapshotted
//!   in `tests/snapshots/`.
//! - [`model`]: [`Patch`] in memory (a dense `Vec<f32>` per module, in spec order), [`ModuleInstance`], [`Route`],
//!   [`Macro`], [`VoiceSettings`], [`PatchMeta`], [`Section`] and the limits.
//! - [`addr`]: [`ModuleId`], [`ParamAddr`] (`filter1.cutoff`, `voice.glide`, `master.level`), [`ModSource`],
//!   [`RouteAddr`], [`Position`], [`MacroIndex`], [`SlotIndex`], with text parse/display and serde.
//! - [`file`](mod@file): the [`PatchFile`](file::PatchFile) serde types (sparse in, dense out).
//!
//! Every patch mutation is an `Op` from [`ops`]; there is no other way to change a patch in product code.
//!
//! Hooks for lane S1b (the remaining modules are S0 stubs until then):
//!
//! - `validate`: the invariants are listed in the [`model`] docs; errors reuse the codes of
//!   [`enth_params::ValueError`] and [`AddrError`] and add their own (`unknown_module`, `duplicate_id`,
//!   `wrong_section`, `not_modulatable`, `route_rate_mismatch` via [`Patch::source_rate`]/[`Patch::target_rate`],
//!   `too_many` via [`Section::max_len`] and [`model::MAX_ROUTES`], `in_use_as_amp_env`).
//! - `ops`: `AddModule { at: Position, module: ModuleInstance }` carries the full instance; positions, macros and
//!   slots have their own index types; [`Patch::section_mut`] and the public fields are for ops and the file layer.
//!   `effect(op)` reads `ParamSpec::flags.structural`.
//! - `wire`: values go through [`enth_params::ParamSpec::parse`], depths through
//!   [`enth_params::ParamSpec::depth_from_text`]; routes, positions, sources and addresses through the [`addr`]
//!   parsers (macros and slots 1-based in text). `add` without an id uses [`ModuleSpec::id_prefix`] and
//!   [`ModuleId::numbered`].
//! - `canon`: [`ModuleKind`] discriminants and [`Section::ALL`] order are stable, and so is the derived order of
//!   [`ModSource`] and [`Target`] (new variants are appended); meta, labels, slots and macro names do not change
//!   the sound. Generators are commutative **except** that the first enabled `osc` is the hard-sync master (see
//!   the `osc` catalogue docs).
//! - `file`: the [`PatchFile`](file::PatchFile) types and [`file::SCHEMA`] are here; the conversions are S1b's.
//!   Write through the typed `PatchFile`, never through `serde_json::Value`, which widens an `f32` to `f64` text
//!   (`0.1` prints as `0.10000000149011612`); reading through `Value` (migrations) is exact.
//!
//! Module files are pre-declared by P1a lane S0 (freeze point F0). To split a module into submodules, turn `x.rs`
//! into `x/mod.rs`: `mod x;` still resolves.

pub mod addr;
pub mod canon;
pub mod catalogue;
pub mod error;
pub mod file;
pub mod history;
pub mod migrate;
pub mod model;
pub mod ops;
pub mod validate;
pub mod wire;

pub use addr::{
    AddrError, MacroIndex, ModSource, ModuleId, ParamAddr, Position, RouteAddr, SlotIndex, Target,
};
pub use catalogue::{Builtin, CATALOGUE, ModuleKind, ModuleSpec, Rate};
pub use error::{Invalid, OpError, PatchError, Violation};
pub use model::{Macro, ModuleInstance, Patch, PatchMeta, Route, Section, VoiceSettings};
pub use ops::{Op, apply, apply_all};
