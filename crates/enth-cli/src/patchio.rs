//! Reading and writing patch files, and telling a patch from a WAV.

use std::path::Path;

use enth_patch::{Patch, file};

use crate::error::{self, CliError, Ctx};

/// The text of a path for output (as given, not canonicalised).
pub fn show(path: &Path) -> String {
    path.display().to_string()
}

/// Read a file's bytes (exit 5 when it cannot be read).
pub fn read_bytes(path: &Path) -> Result<Vec<u8>, CliError> {
    std::fs::read(path).map_err(|e| error::not_found(&show(path), &e))
}

/// Read and validate a patch file: parse, migrate, resolve every value against its spec, validate. Every problem
/// is reported (exit 3); migration notes go to `notes`.
pub fn load(path: &Path) -> Result<(Patch, Vec<String>), CliError> {
    let bytes = read_bytes(path)?;
    parse(&bytes, &show(path))
}

/// [`load`] from bytes already read.
pub fn parse(bytes: &[u8], name: &str) -> Result<(Patch, Vec<String>), CliError> {
    if bytes.starts_with(b"RIFF") {
        return Err(error::invalid(
            "not_a_patch",
            format!("{name} is a WAV file, not a patch"),
            "commands that edit or render take a *.patch.json; describe and check also take a WAV",
        )
        .in_file(name));
    }
    let text = String::from_utf8_lossy(bytes);
    let loaded = file::load(&text).map_err(|inv| {
        error::from_invalid(
            &inv,
            Ctx {
                file: Some(name),
                ..Ctx::default()
            },
        )
    })?;
    let notes = loaded
        .notes
        .iter()
        .map(|n| format!("migrated from schema {}: {} {}", n.from, n.path, n.message))
        .collect();
    Ok((loaded.patch, notes))
}

/// Write a patch file (indented, dense, stamped with this build's engine version).
pub fn write(path: &Path, patch: &Patch) -> Result<(), CliError> {
    let mut p = patch.clone();
    file::ENGINE_VERSION.clone_into(&mut p.engine);
    let mut text = p.to_json_pretty();
    text.push('\n');
    std::fs::write(path, text).map_err(|e| error::write_failed(&show(path), &e))
}

/// Whether bytes are a WAV (RIFF/WAVE) rather than a patch.
pub fn is_wav(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
}

/// The path with its extension replaced: `p.patch.json` gives `p.wav`.
pub fn with_wav_extension(path: &Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name
        .strip_suffix(".patch.json")
        .or_else(|| name.strip_suffix(".json"))
        .unwrap_or(&name);
    path.with_file_name(format!("{stem}.wav"))
}
