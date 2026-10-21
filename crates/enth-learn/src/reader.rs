//! **Owner: lane LM** (the lesson model, freeze point F1l). Pre-declared by lane S0l (R2).
//!
//! The `.enthpack` reader (docs/learn.md, "Packs"): pure and wasm-clean, on `rawzip` (the zip structure), `miniz_oxide`
//! (deflate, pinned so `cargo xtask pack` builds reproducibly), `sha2` (SHA-256 per file) and `serde_json`. It reads
//! in memory and never extracts. It refuses an entry whose normalised name differs from its raw name (`rawzip`
//! silently turns `../../evil.txt` into `evil.txt`), duplicate entries, too many entries, oversize entries and
//! inflation past the listed size, and files the manifest does not list. LM writes all of that.
//!
//! Lane S0l left one probe, [`entry_count`]: the call that makes `rawzip` part of what `cargo xtask wasm-check` builds
//! for `wasm32-unknown-unknown` (the F0l acceptance; docs/phase-3.md). LM replaces it with the real reader.

use rawzip::{Error, ZipArchive};

/// How many entries the central directory of a zip held in memory declares: the F0l probe for `rawzip` on wasm32.
///
/// # Errors
///
/// `rawzip`'s error when `bytes` is not a zip archive.
pub fn entry_count(bytes: &[u8]) -> Result<u64, Error> {
    Ok(ZipArchive::from_slice(bytes)?.entries_hint())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use rawzip::{CompressionMethod, ZipArchiveWriter};

    use super::entry_count;

    #[test]
    fn entry_count_reads_a_stored_archive() {
        let mut archive = ZipArchiveWriter::new(Vec::new());
        for name in ["pack.json", "lessons/01-saw/step-1.md"] {
            let (mut entry, config) = archive
                .new_file(name)
                .compression_method(CompressionMethod::STORE)
                .start()
                .unwrap();
            let mut data = config.wrap(&mut entry);
            data.write_all(b"{}").unwrap();
            let (_, output) = data.finish().unwrap();
            entry.finish(output).unwrap();
        }
        let bytes = archive.finish().unwrap();
        assert_eq!(entry_count(&bytes).unwrap(), 2);
    }

    #[test]
    fn entry_count_refuses_what_is_not_a_zip() {
        assert!(entry_count(b"not a zip").is_err());
    }
}
