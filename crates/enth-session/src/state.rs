//! The saved state, version 1: the patch (with its slot assignments and macros) plus the per-instance state outside
//! it (monitor bypass, editor preferences), each versioned, with fixtures and a migration hook (lane S1s for the
//! types and v1, lane S2s for later versions and the host-state-load history policy).
//!
//! # Layout
//!
//! The plugin persists two nice-plug fields, both permanent names (docs/naming.md, docs/architecture.md):
//!
//! | Field | Text | Versioned by |
//! | --- | --- | --- |
//! | [`PATCH_FIELD`] `patch` | exactly `Patch::to_json()`, carried as raw JSON (never re-parsed through `serde_json::Value`, which would widen `f32`s) | the patch file's own `schema` number and `enth_patch::migrate` |
//! | [`INSTANCE_FIELD`] `enth` | an [`InstanceState`]: `{"schema":"enth.plugin-state/1","monitor":{…},"ui":{…}}` | its `schema` string, [`STATE_SCHEMA`], and [`INSTANCE_STEPS`] |
//!
//! So a plugin state's `patch` and a `.patch.json` are interchangeable, and loading one goes through
//! `enth_patch::file::load` and its migrations, the same path as `enth render`. The 64 slot assignments and the 8
//! macros (value and name) live in the patch; the 72 host values are persisted by nice-plug's own `params` map and
//! are never needed to rebuild the sound (the patch wins at load: [`crate::bridge`]).
//!
//! # Rules
//!
//! - **Byte-deterministic.** Saving the same session twice gives the same bytes (no maps in persisted types; fields
//!   in declaration order), which clap-validator's `state-reproducibility-binary` test requires.
//! - **Never panics, bounded.** Each field is at most [`MAX_FIELD_BYTES`]; bad input is a typed [`StateError`] with
//!   a code and a hint (nice-plug #85 aborted a host on a bad state length).
//! - **The patch loads independently of the instance state.** A broken `enth` field gives the default instance
//!   state; a broken `patch` field gives the init patch. Either way the load reports what happened ([`LoadReport`]).
//! - **Versions.** Adding an optional field with a default keeps schema 1 (older readers ignore unknown fields,
//!   newer readers default missing ones). Anything else bumps the version, appends a step to [`INSTANCE_STEPS`]
//!   and adds a fixture under `tests/fixtures/state/v<N>/`. A newer schema than this build knows is refused
//!   (`schema_too_new`), never guessed.
//! - nice-plug sets fields in name order, so `enth` arrives before `patch`; the two loads are independent, so the
//!   order does not matter.

use enth_patch::Patch;
use enth_patch::error::Invalid;
use enth_patch::file::Loaded;
use enth_patch::migrate::MigrationNote;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Session, ShellRequest};

/// The nice-plug field that holds the patch.
pub const PATCH_FIELD: &str = "patch";

/// The nice-plug field that holds the [`InstanceState`].
pub const INSTANCE_FIELD: &str = "enth";

/// The instance state's schema family.
pub const STATE_KIND: &str = "enth.plugin-state";

/// The instance state version this build writes.
pub const STATE_VERSION: u32 = 1;

/// The `schema` string this build writes: `enth.plugin-state/1`.
pub const STATE_SCHEMA: &str = "enth.plugin-state/1";

/// The most bytes either field may hold. A maximum patch (20 modules, 64 routes, 64 slots, names) is about 20 KB.
pub const MAX_FIELD_BYTES: usize = 1 << 20;

/// One instance-state migration step: version `i + 1` in, version `i + 2` out, as JSON. An `Err` is a plain message
/// (the input does not look like its version).
pub type InstanceStep = fn(Value) -> Result<Value, String>;

/// `INSTANCE_STEPS[i]` migrates version `i + 1` to `i + 2`. Its length is `STATE_VERSION - 1`, checked by the
/// compiler: bumping the version without a step does not build. Version 1 has none.
pub static INSTANCE_STEPS: [InstanceStep; (STATE_VERSION - 1) as usize] = [];

/// Per-instance state outside the patch: not hashed, never part of the sound, persisted in the `enth` field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanceState {
    /// Monitoring switches.
    pub monitor: MonitorState,
    /// Editor preferences (the editor is rebuilt on every open, so its view state lives here).
    pub ui: UiState,
}

/// Monitoring switches (ADR-0006).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitorState {
    /// The global FX bypass: mutes the `time_based` effects (the reverb) with a 20 ms fade. Saved with the plugin's
    /// state, outside the patch; renders and scores never see it.
    pub fx_bypass: bool,
}

/// Editor preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// The layout variant's name (`"lanes"`, layout 1). The editor falls back to its default for a name it does not
    /// know, so a state from a build with more layouts still opens.
    pub layout: String,
    /// The UI scale in percent (100 = 1.0). An integer, so the saved bytes never depend on float formatting; the
    /// editor clamps it to the range it supports.
    pub scale_percent: u16,
    /// Show advanced-tier parameters.
    pub advanced: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            layout: "lanes".to_owned(),
            scale_percent: 100,
            advanced: false,
        }
    }
}

/// The on-disk shape: the schema first, then the state.
#[derive(Serialize)]
struct InstanceFile<'a> {
    schema: &'static str,
    #[serde(flatten)]
    state: &'a InstanceState,
}

impl InstanceState {
    /// The `enth` field's text: compact JSON, schema first, byte-deterministic.
    pub fn to_json(&self) -> String {
        let file = InstanceFile {
            schema: STATE_SCHEMA,
            state: self,
        };
        // Only strings, bools and integers: serialising cannot fail.
        serde_json::to_string(&file).unwrap_or_default()
    }

    /// Read an `enth` field: size check, parse, schema check, migrate to [`STATE_VERSION`], decode. Unknown fields
    /// are ignored and missing ones take their defaults. Returns the state and the version it had.
    pub fn from_json(text: &str) -> Result<(InstanceState, u32), StateError> {
        check_size(INSTANCE_FIELD, text)?;
        let value: Value = serde_json::from_str(text).map_err(|e| StateError::BadJson {
            field: INSTANCE_FIELD,
            message: e.to_string(),
        })?;
        let (value, from) = migrate_instance(value)?;
        let state = serde_json::from_value(value).map_err(|e| StateError::BadInstance {
            message: e.to_string(),
        })?;
        Ok((state, from))
    }
}

/// The version of an instance state: its `schema` must be `enth.plugin-state/<N>` with `N ≥ 1`.
pub fn instance_version(value: &Value) -> Result<u32, StateError> {
    let Some(obj) = value.as_object() else {
        return Err(StateError::BadInstance {
            message: "the top level must be an object".to_owned(),
        });
    };
    let Some(schema) = obj.get("schema") else {
        return Err(StateError::MissingSchema);
    };
    let bad = || StateError::BadSchema {
        found: schema.to_string(),
    };
    let text = schema.as_str().ok_or_else(bad)?;
    let (kind, version) = text.split_once('/').ok_or_else(bad)?;
    if kind != STATE_KIND || version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    match version.parse::<u32>() {
        Ok(v) if v >= 1 => Ok(v),
        _ => Err(bad()),
    }
}

/// Bring an instance state up to [`STATE_VERSION`] through [`INSTANCE_STEPS`]; the identity on a current one,
/// `schema_too_new` on a newer one. Returns the migrated value (its `schema` rewritten) and the version it had.
pub fn migrate_instance(value: Value) -> Result<(Value, u32), StateError> {
    let from = instance_version(&value)?;
    if from > STATE_VERSION {
        return Err(StateError::SchemaTooNew {
            found: from,
            supported: STATE_VERSION,
        });
    }
    let mut value = value;
    for version in from..STATE_VERSION {
        let step = INSTANCE_STEPS[(version - 1) as usize];
        value = step(value).map_err(|message| StateError::MigrationFailed {
            from: version,
            message,
        })?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "schema".to_owned(),
                Value::from(format!("{STATE_KIND}/{}", version + 1)),
            );
        }
    }
    Ok((value, from))
}

/// The `patch` field's text: exactly [`Patch::to_json`].
pub fn encode_patch(patch: &Patch) -> String {
    patch.to_json()
}

/// Read a `patch` field: size check, then `enth_patch::file::load` (parse, migrate, resolve, validate).
pub fn decode_patch(text: &str) -> Result<Loaded, StateError> {
    check_size(PATCH_FIELD, text)?;
    enth_patch::file::load(text).map_err(StateError::Patch)
}

fn check_size(field: &'static str, text: &str) -> Result<(), StateError> {
    if text.len() > MAX_FIELD_BYTES {
        return Err(StateError::TooLarge {
            field,
            bytes: text.len(),
            max: MAX_FIELD_BYTES,
        });
    }
    Ok(())
}

/// Both fields of a saved state, as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedState {
    /// The `patch` field.
    pub patch: String,
    /// The `enth` field.
    pub instance: String,
}

impl SavedState {
    /// Encode a patch and an instance state.
    pub fn encode(patch: &Patch, instance: &InstanceState) -> SavedState {
        SavedState {
            patch: encode_patch(patch),
            instance: instance.to_json(),
        }
    }

    /// Decode both fields, independently.
    pub fn decode(
        &self,
    ) -> (
        Result<Loaded, StateError>,
        Result<(InstanceState, u32), StateError>,
    ) {
        (
            decode_patch(&self.patch),
            InstanceState::from_json(&self.instance),
        )
    }
}

/// Why a saved state did not load. Codes are stable snake case.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StateError {
    /// A field is larger than [`MAX_FIELD_BYTES`].
    TooLarge {
        /// The field.
        field: &'static str,
        /// Its size.
        bytes: usize,
        /// The limit.
        max: usize,
    },
    /// A field is not JSON.
    BadJson {
        /// The field.
        field: &'static str,
        /// The parser's message.
        message: String,
    },
    /// The instance state has no `schema`.
    MissingSchema,
    /// The instance state's `schema` is not `enth.plugin-state/<N>`.
    BadSchema {
        /// What it was, as JSON.
        found: String,
    },
    /// The instance state is newer than this build.
    SchemaTooNew {
        /// Its version.
        found: u32,
        /// The newest this build reads.
        supported: u32,
    },
    /// An instance-state migration step failed.
    MigrationFailed {
        /// The version it migrated from.
        from: u32,
        /// Why.
        message: String,
    },
    /// The instance state has the right schema but a field of the wrong type.
    BadInstance {
        /// The decoder's message.
        message: String,
    },
    /// The patch did not load (its own errors, each with a path, a code and a hint).
    Patch(Invalid),
}

impl StateError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            StateError::TooLarge { .. } => "state_too_large",
            StateError::BadJson { .. } => "bad_json",
            StateError::MissingSchema => "missing_schema",
            StateError::BadSchema { .. } => "bad_schema",
            StateError::SchemaTooNew { .. } => "schema_too_new",
            StateError::MigrationFailed { .. } => "migration_failed",
            StateError::BadInstance { .. } => "bad_instance",
            StateError::Patch(_) => "bad_patch",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            StateError::TooLarge { .. } => {
                "this is not an enth state: it is far too large".to_owned()
            }
            StateError::BadJson { .. } | StateError::BadInstance { .. } => {
                "the saved state is damaged; the defaults were used".to_owned()
            }
            StateError::MissingSchema | StateError::BadSchema { .. } => {
                format!("an instance state starts with \"schema\": \"{STATE_SCHEMA}\"")
            }
            StateError::SchemaTooNew { .. } => {
                "saved by a newer enth: update enth to open it".to_owned()
            }
            StateError::MigrationFailed { .. } => {
                "report this with the saved state: a migration step failed".to_owned()
            }
            StateError::Patch(invalid) => invalid
                .0
                .first()
                .map_or_else(String::new, |v| v.error.hint()),
        }
    }
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::TooLarge { field, bytes, max } => {
                write!(f, "field '{field}' holds {bytes} bytes, more than {max}")
            }
            StateError::BadJson { field, message } => {
                write!(f, "field '{field}' is not JSON: {message}")
            }
            StateError::MissingSchema => f.write_str("the instance state has no schema"),
            StateError::BadSchema { found } => write!(f, "unknown instance state schema {found}"),
            StateError::SchemaTooNew { found, supported } => write!(
                f,
                "instance state version {found} is newer than this build's {supported}"
            ),
            StateError::MigrationFailed { from, message } => {
                write!(
                    f,
                    "migrating instance state version {from} failed: {message}"
                )
            }
            StateError::BadInstance { message } => write!(f, "bad instance state: {message}"),
            StateError::Patch(invalid) => write!(f, "bad patch: {invalid}"),
        }
    }
}

impl std::error::Error for StateError {}

/// What a load did. Build one with `Default` (it is `non_exhaustive`).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LoadReport {
    /// The patch did not load, so the session now holds [`Patch::init`].
    pub fell_back: bool,
    /// The schema the patch had (`None` if it did not load).
    pub patch_schema: Option<u32>,
    /// What patch migrations changed that the user should know.
    pub notes: Vec<MigrationNote>,
    /// The version the instance state had (`None` if it did not load).
    pub instance_version: Option<u32>,
    /// Everything that went wrong, in field order.
    pub errors: Vec<StateError>,
}

impl Session {
    /// The `patch` field for a save: folds `host` first when given (docs/design.md §4: the session folds host values
    /// before every save), then [`Patch::to_json`]. A host project save does not touch the undo history or the dirty
    /// flag (only an editor preset save would mark it saved).
    pub fn save_patch(&mut self, host: Option<&crate::bridge::HostValues>) -> String {
        if let Some(h) = host {
            self.fold_host(h);
        }
        encode_patch(&self.patch)
    }

    /// The `enth` field for a save.
    pub fn save_instance(&self) -> String {
        self.instance.to_json()
    }

    /// Both fields ([`Session::save_patch`], [`Session::save_instance`]).
    pub fn save(&mut self, host: Option<&crate::bridge::HostValues>) -> SavedState {
        SavedState {
            patch: self.save_patch(host),
            instance: self.save_instance(),
        }
    }

    /// Load an `enth` field: on error the defaults, reported. Pushes [`ShellRequest::MonitorBypass`] with the
    /// loaded value. Never touches the patch or the history. **Host main thread** (nice-plug's field `set`), never
    /// the audio thread: the editor never calls `GuiContext::set_state`.
    pub fn load_instance(&mut self, text: &str) -> LoadReport {
        let mut report = LoadReport::default();
        self.instance = match InstanceState::from_json(text) {
            Ok((state, version)) => {
                report.instance_version = Some(version);
                state
            }
            Err(e) => {
                report.errors.push(e);
                InstanceState::default()
            }
        };
        self.requests
            .push(ShellRequest::MonitorBypass(self.instance.monitor.fx_bypass));
        report
    }

    /// Load a `patch` field (invariant I7): **the patch wins.** On success the session holds the loaded patch; on
    /// error it holds [`Patch::init`] and the report says why (it never panics). Either way: the history is cleared
    /// and the new state counts as saved, an open gesture leaves the history (the host gestures it opened stay open
    /// until the editor's next `end_gesture`, because host writes are only produced and forwarded on the GUI
    /// thread), the host baseline is reset (the next fold only records), audition notes are released, host
    /// parameter names are refreshed ([`ShellRequest::RenameParams`]), and, while active, a recompile is marked
    /// pending for the next pump.
    ///
    /// **Host main thread** for a host state load (nice-plug's field `set`), or the GUI thread for an editor preset
    /// load, under the session lock; never the audio thread. The history policy for a host load mid-session (REAPER
    /// undo loads state) is lane S2s's to revisit; the default is the plain reset above.
    pub fn load_patch(&mut self, text: &str) -> LoadReport {
        let mut report = LoadReport::default();
        self.patch = match decode_patch(text) {
            Ok(loaded) => {
                report.patch_schema = Some(loaded.from_schema);
                report.notes = loaded.notes;
                loaded.patch
            }
            Err(e) => {
                report.fell_back = true;
                report.errors.push(e);
                Patch::init()
            }
        };
        self.gestures.open = None;
        self.history.clear();
        self.reset_host_baseline();
        // Releasing audition notes needs an active engine; detached there is nothing to release.
        let _ = self.audition_all_off();
        if let Some(port) = &mut self.port {
            port.compile_pending = true;
        }
        self.requests.push(ShellRequest::RenameParams);
        report
    }

    /// Load both fields of a saved state ([`Session::load_instance`], then [`Session::load_patch`]); one report.
    pub fn load(&mut self, saved: &SavedState) -> LoadReport {
        let instance = self.load_instance(&saved.instance);
        let mut report = self.load_patch(&saved.patch);
        report.instance_version = instance.instance_version;
        let mut errors = instance.errors;
        errors.append(&mut report.errors);
        report.errors = errors;
        report
    }
}
