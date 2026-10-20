//! **Owner: lane PBC** (the preset core; its format is reviewed at freeze point F1r). Pre-declared by lane S0r
//! (R1/S1), with the one type the saved state already reserves.
//!
//! Presets (report 03 §2.3, summary "Presets" and "Preset loads"): one `.patch.json` per preset, exactly
//! `Patch::to_json()`; folders, the factory tag vocabulary, the index and search; `Session::load_preset`, an editor
//! load that is one undo step, delivered like a load with no host writes, and marks the project dirty. The session
//! does no file I/O: the plugin reads and writes the files (PBC owns that side too).

use serde::{Deserialize, Serialize};

/// The instance's preset bookkeeping, as the `enth` field saves it
/// ([`InstanceState::preset`](crate::InstanceState::preset)): reserved by lane S0r, and empty until lane PBC gives it
/// fields (for example the key of the preset last loaded, for the header's ◀ name ▶).
///
/// The field is optional and absent until a preset is involved, so a state without one keeps its bytes. PBC decides
/// whether its contents stay within `enth.plugin-state/1` or need version 2 (docs/session.md).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetState {}
