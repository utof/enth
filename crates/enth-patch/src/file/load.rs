//! `load`: text or `Value` → migrate → decode → resolve → validate.

use serde_json::Value;

use super::{PatchFile, SCHEMA};
use crate::error::{Invalid, PatchError};
use crate::migrate::{MigrationNote, migrate, schema_of};
use crate::model::Patch;

/// A patch read from a file, with what migration told the user.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    /// The valid patch.
    pub patch: Patch,
    /// What the migration steps changed that the user should know (empty for a current file).
    pub notes: Vec<MigrationNote>,
    /// The schema the file had; `SCHEMA` when no migration ran.
    pub from_schema: u32,
}

/// Read a patch file: parse, migrate to the current schema, decode, resolve every value against its
/// `ParamSpec`, and validate. Every problem is reported with a path, a code and a hint ([`Invalid`]).
///
/// A leading byte-order mark is ignored. A current-schema file is decoded straight from the text, so a repeated
/// key is an error; an older file is migrated through `serde_json::Value`, where the last duplicate wins.
pub fn load(json: &str) -> Result<Loaded, Invalid> {
    let text = json.strip_prefix('\u{feff}').unwrap_or(json);
    let value: Value = serde_json::from_str(text).map_err(|e| {
        Invalid::one(
            "",
            PatchError::BadJson {
                message: e.to_string(),
            },
        )
    })?;
    let from_schema = schema_of(&value).map_err(|e| Invalid::one("schema", e))?;
    if from_schema == SCHEMA {
        let file: PatchFile = serde_json::from_str(text).map_err(|e| {
            Invalid::one(
                "",
                PatchError::BadFile {
                    message: e.to_string(),
                },
            )
        })?;
        return Ok(Loaded {
            patch: file.to_patch()?,
            notes: Vec::new(),
            from_schema,
        });
    }
    load_value(value)
}

/// [`load`] for a value that is already parsed (migrating if its schema is older).
pub fn load_value(value: Value) -> Result<Loaded, Invalid> {
    let migrated = migrate(value).map_err(|e| Invalid::one("schema", e))?;
    let file: PatchFile = serde_json::from_value(migrated.value).map_err(|e| {
        Invalid::one(
            "",
            PatchError::BadFile {
                message: e.to_string(),
            },
        )
    })?;
    Ok(Loaded {
        patch: file.to_patch()?,
        notes: migrated.notes,
        from_schema: migrated.from,
    })
}
