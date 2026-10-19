//! `--state`: a raw plugin state blob, or a patch document the host wraps into one.
//!
//! A nice-plug plugin saves its CLAP state as `u64 little-endian length` followed by that many bytes of JSON,
//! `{"version": "...", "params": {id: {"f32": v}, ...}, "fields": {name: "<JSON text>"}}`. The patch of the
//! spike plugin lives in the persisted field `patch`; `enth-plugin` keeps the exact `Patch::to_json()` text in
//! the same field ([design.md §4](../../../docs/design.md)), so a `*.patch.json` file and the field's contents
//! are interchangeable. This module does the wrapping, so a test can say `--state some.patch.json` and
//! never write the envelope by hand.
//!
//! What counts as what, in `--state FILE` (`--state-format auto`, the default):
//!
//! - a file whose text is one JSON object **with** `params` and `fields` members is an envelope in JSON form
//!   (a decoded blob): only the length prefix is added;
//! - any other file whose text is JSON is a **patch document**: it goes, as text, into
//!   `fields.<--state-field>` (default `patch`) of an envelope with no parameter values, so every parameter
//!   keeps its default;
//! - anything else (the binary the plugin saved) is a **blob** and goes through unchanged.
//!
//! `--state-format blob|patch|envelope` overrides the guess. A plugin that wants more than the patch field
//! (a versioned `enth` blob next to it) gets it from the blob form: save a state from the plugin, or write the
//! envelope JSON.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// The persisted field the patch goes in, for the spike plugin and for `enth-plugin`.
pub const DEFAULT_FIELD: &str = "patch";

/// How `--state FILE` is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Auto,
    Blob,
    Patch,
    Envelope,
}

impl Format {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "auto" => Ok(Self::Auto),
            "blob" => Ok(Self::Blob),
            "patch" => Ok(Self::Patch),
            "envelope" => Ok(Self::Envelope),
            other => Err(format!(
                "--state-format {other:?}: use auto, blob, patch or envelope"
            )),
        }
    }
}

/// What the host ended up loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Blob,
    Patch,
    Envelope,
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Blob => "blob",
            Kind::Patch => "patch-document",
            Kind::Envelope => "envelope-json",
        }
    }
}

/// A state ready for the plugin's `state.load`.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub bytes: Vec<u8>,
    pub kind: Kind,
    /// The field the patch went into, for [`Kind::Patch`].
    pub field: Option<String>,
}

/// The framed envelope: length prefix, then the JSON text.
pub fn frame(json_text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + json_text.len());
    bytes.extend_from_slice(&(json_text.len() as u64).to_le_bytes());
    bytes.extend_from_slice(json_text.as_bytes());
    bytes
}

/// Wraps a patch document into a state blob: `fields.<field>` holds the patch text, no parameter values.
///
/// `patch_text` is checked to be JSON, then stored as given (not re-serialised), so a plugin that compares
/// bytes sees the file's own.
pub fn wrap_patch(patch_text: &str, field: &str, version: &str) -> Result<Vec<u8>, String> {
    serde_json::from_str::<Value>(patch_text)
        .map_err(|e| format!("the patch document is not valid JSON: {e}"))?;
    let envelope = json!({
        "version": version,
        "params": {},
        "fields": { field: patch_text },
    });
    Ok(frame(&envelope.to_string()))
}

/// Splits a state blob into its envelope JSON. Only the uncompressed framing is understood (nice-plug's
/// `zstd` feature is off in this project).
pub fn unframe(blob: &[u8]) -> Result<Value, String> {
    let (prefix, rest) = blob
        .split_first_chunk::<8>()
        .ok_or("a state blob starts with an 8-byte length; this one is shorter")?;
    let length = u64::from_le_bytes(*prefix);
    if length != rest.len() as u64 {
        return Err(format!(
            "the length prefix says {length} bytes, the blob has {} after it",
            rest.len()
        ));
    }
    serde_json::from_slice(rest)
        .map_err(|e| format!("the state blob is not framed JSON (compressed?): {e}"))
}

/// The persisted fields of a state blob, as text.
pub fn fields_of(blob: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let envelope = unframe(blob)?;
    let fields = envelope
        .get("fields")
        .and_then(Value::as_object)
        .ok_or("the state has no `fields` object")?;
    Ok(fields
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
        .collect())
}

/// Reads `--state` bytes (already loaded from the file) into what the plugin should be given.
pub fn prepare(
    bytes: Vec<u8>,
    format: Format,
    field: &str,
    version: &str,
) -> Result<Prepared, String> {
    let json_text = std::str::from_utf8(&bytes)
        .ok()
        .map(str::trim_start)
        .filter(|t| t.starts_with('{'));
    let parsed = json_text.and_then(|t| serde_json::from_str::<Value>(t).ok());
    let kind = match format {
        Format::Blob => Kind::Blob,
        Format::Patch => Kind::Patch,
        Format::Envelope => Kind::Envelope,
        Format::Auto => match &parsed {
            Some(v) if v.get("params").is_some() && v.get("fields").is_some() => Kind::Envelope,
            Some(_) => Kind::Patch,
            None => Kind::Blob,
        },
    };
    match kind {
        Kind::Blob => Ok(Prepared {
            bytes,
            kind,
            field: None,
        }),
        Kind::Patch => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| "--state-format patch: the file is not UTF-8 text".to_owned())?;
            Ok(Prepared {
                bytes: wrap_patch(text.trim(), field, version)?,
                kind,
                field: Some(field.to_owned()),
            })
        }
        Kind::Envelope => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| "--state-format envelope: the file is not UTF-8 text".to_owned())?;
            serde_json::from_str::<Value>(text)
                .map_err(|e| format!("--state-format envelope: not valid JSON: {e}"))?;
            Ok(Prepared {
                bytes: frame(text.trim()),
                kind,
                field: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = r#"{"schema":1,"generators":[{"id":"osc1","kind":"osc"}]}"#;

    #[test]
    fn a_patch_document_is_wrapped_into_the_patch_field() {
        let blob = wrap_patch(PATCH, "patch", "1.2.3").unwrap();
        let envelope = unframe(&blob).unwrap();
        assert_eq!(envelope["version"], "1.2.3");
        assert_eq!(envelope["params"], json!({}));
        // The patch is a string inside the envelope, byte for byte what the file held.
        assert_eq!(envelope["fields"]["patch"], PATCH);
        assert_eq!(fields_of(&blob).unwrap()["patch"], PATCH);
    }

    #[test]
    fn auto_tells_patch_envelope_and_blob_apart() {
        let patch = prepare(PATCH.as_bytes().to_vec(), Format::Auto, "patch", "0").unwrap();
        assert_eq!(patch.kind, Kind::Patch);
        assert_eq!(patch.field.as_deref(), Some("patch"));

        let envelope_json = r#"{"version":"0","params":{},"fields":{"patch":"{}"}}"#;
        let envelope = prepare(
            envelope_json.as_bytes().to_vec(),
            Format::Auto,
            "patch",
            "0",
        )
        .unwrap();
        assert_eq!(envelope.kind, Kind::Envelope);
        assert_eq!(unframe(&envelope.bytes).unwrap()["fields"]["patch"], "{}");

        // A real blob (its length prefix is binary) is passed through untouched.
        let blob = wrap_patch(PATCH, "patch", "0").unwrap();
        let passed = prepare(blob.clone(), Format::Auto, "patch", "0").unwrap();
        assert_eq!(passed.kind, Kind::Blob);
        assert_eq!(passed.bytes, blob);
    }

    #[test]
    fn a_blob_whose_length_byte_is_an_open_brace_is_still_a_blob() {
        // Length 123 = 0x7b = `{`, then seven zero bytes: not JSON, so not mistaken for a patch.
        let mut blob = vec![0x7b, 0, 0, 0, 0, 0, 0, 0];
        blob.extend(std::iter::repeat_n(b'x', 123));
        let prepared = prepare(blob.clone(), Format::Auto, "patch", "0").unwrap();
        assert_eq!(prepared.kind, Kind::Blob);
        assert_eq!(prepared.bytes, blob);
    }

    #[test]
    fn bad_input_is_an_error_with_the_reason() {
        assert!(wrap_patch("{not json", "patch", "0").is_err());
        assert!(
            prepare(b"{nope".to_vec(), Format::Patch, "patch", "0")
                .unwrap_err()
                .contains("not valid JSON")
        );
        assert!(unframe(&[1, 2, 3]).is_err());
        let mut bad = frame("{}");
        bad[0] = 99;
        assert!(unframe(&bad).unwrap_err().contains("length prefix"));
        assert!(Format::parse("nope").is_err());
    }
}
