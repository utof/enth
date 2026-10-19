//! Schema migrations: pure `serde_json::Value -> Value` steps, one per version, with fixtures (P1a lane S1b).
//!
//! A patch file says which schema it is (`"schema": N`). Reading goes: parse to a `Value`, **migrate** it step by
//! step to [`SCHEMA`], then decode it into the typed [`PatchFile`](crate::file::PatchFile). Migrations work on
//! `Value`, not on a Rust struct per version, because what changes between versions is the *catalogue* inside a
//! stable shape: a renamed key, a rescaled range, a split module, a changed default. Those are data rewrites. (A
//! struct per version would freeze the whole catalogue per version; the crates that do that fit neither
//! catalogue-driven parameters nor agent-written files. Reading through `Value` is exact: with `float_roundtrip`
//! every finite `f32` text survives.)
//!
//! # Rules
//!
//! - `STEPS[i]` migrates schema `i` to `i + 1`. Its length is `SCHEMA`, checked by the compiler: bumping `SCHEMA`
//!   without a step does not build.
//! - Steps are pure and forward-only. [`migrate`] sets `schema` after each step, so a step cannot forget.
//! - A step that changes the meaning of the file tells the user with a [`MigrationNote`] (`"osc1.level: …"`); the
//!   CLI prints them.
//! - **What a catalogue change triggers:** adding a parameter needs a default that reproduces the old sound (or a
//!   step that writes the old value); renaming a key is a step; a change of unit, range or scale is a step that
//!   rescales the values *and the depths of routes targeting them*; removing a parameter is a step that drops it
//!   with a note; changing a default is a step that writes the old default where a file omits the key. DSP changes
//!   bump the engine version and the goldens, not the schema. The catalogue snapshot test fails on any spec change,
//!   so the author must bump the schema with a step or say in the snapshot why none is needed.
//! - **Every schema from 0 to `SCHEMA` has a fixture** in `tests/fixtures/patch/v<N>/` and the expected current form
//!   in `tests/fixtures/patch/expected/`; `tests/migrate.rs` checks them all, `migrate` is the identity on the
//!   current version, and a newer schema gives `schema_too_new`.
//!
//! # Schemas
//!
//! | Schema | What |
//! | --- | --- |
//! | 1 | Patch v1 (docs/design.md §2): sections, built-in `voice` and `master`, dense files |
//! | 0 | **Synthetic**, never released: a made-up predecessor that exists so the framework has a real step and a real fixture ([`v0`]) |

use serde_json::Value;

use crate::error::PatchError;
use crate::file::SCHEMA;

mod v0;

/// A user-facing remark about something a migration changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationNote {
    /// The schema version the step migrated from.
    pub from: u32,
    /// Where in the file (`generators[0].params.level`), or empty.
    pub path: String,
    /// What was done and why.
    pub message: String,
}

/// One migration step: the file at schema `i`, the file at schema `i + 1`. An `Err` is a plain message (the file
/// does not look like its schema); [`migrate`] wraps it as `migration_failed`.
pub type Step = fn(Value, &mut Vec<MigrationNote>) -> Result<Value, String>;

/// `STEPS[i]` migrates schema `i` to `i + 1`.
pub static STEPS: [Step; SCHEMA as usize] = [v0::migrate];

/// A migrated file.
#[derive(Clone, Debug, PartialEq)]
pub struct Migrated {
    /// The file at the current [`SCHEMA`].
    pub value: Value,
    /// What the steps changed that the user should know.
    pub notes: Vec<MigrationNote>,
    /// The schema the input had.
    pub from: u32,
}

/// The schema number of a file: `missing_schema` if absent, `bad_schema` if it is not a non-negative whole number,
/// `bad_file` if the file is not an object.
pub fn schema_of(v: &Value) -> Result<u32, PatchError> {
    let Some(obj) = v.as_object() else {
        return Err(PatchError::BadFile {
            message: "the top level must be an object".to_owned(),
        });
    };
    let Some(s) = obj.get("schema") else {
        return Err(PatchError::MissingSchema);
    };
    s.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| PatchError::BadSchema {
            found: s.to_string(),
        })
}

/// Bring a file up to the current [`SCHEMA`]. The identity on a current file; `schema_too_new` on a newer one.
pub fn migrate(v: Value) -> Result<Migrated, PatchError> {
    let from = schema_of(&v)?;
    if from > SCHEMA {
        return Err(PatchError::SchemaTooNew {
            found: from,
            supported: SCHEMA,
        });
    }
    let mut value = v;
    let mut notes = Vec::new();
    for step in from..SCHEMA {
        value = STEPS[step as usize](value, &mut notes).map_err(|message| {
            PatchError::MigrationFailed {
                from: step,
                message,
            }
        })?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert("schema".to_owned(), Value::from(step + 1));
        }
    }
    Ok(Migrated { value, notes, from })
}
