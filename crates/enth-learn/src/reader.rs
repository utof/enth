//! **Owner: lane LM** (the lesson model, freeze point F1l). The reference is docs/learn.md, "Packs" ("Integrity and
//! the reader") and report 02 §5 of the R2 research session.
//!
//! The `.enthpack` reader and its canonical writer: pure and wasm-clean, on `rawzip` (the zip structure),
//! `miniz_oxide` (deflate, pinned so packs build reproducibly), `sha2` and `serde_json`. It reads in memory and never
//! extracts to disk. [`read_archive`] verifies everything and returns a [`Pack`]; [`extract`] stops at the verified
//! files (a folder pack goes straight to [`Pack::from_files`]).
//!
//! **Entry rules** (each a refusal with a hint):
//! - an entry whose normalised name differs from its raw name (`rawzip` silently turns `../../evil.txt` into
//!   `evil.txt`), or whose local header names another file than its central record: `renamed_entry`;
//! - two entries with one name: `duplicate_entry`; more than [`MAX_ENTRIES`]: `too_many_entries`;
//! - a file declared larger than [`MAX_FILE_BYTES`], or an archive larger than [`MAX_ARCHIVE_BYTES`]: `too_large`;
//! - a method other than stored or deflate: `unsupported_compression`; an encrypted entry: `encrypted_entry`;
//! - an entry the manifest does not list: `unlisted_file`; a listed size the entry does not declare:
//!   `size_mismatch`;
//! - inflation past the listed size: `inflate_overflow` (the inflater is capped at it, so a bomb costs nothing);
//! - a CRC that does not match: `corrupt_entry`; then [`Pack::from_files`] checks every SHA-256 and the lessons.
//!
//! Directory entries (`lessons/`) carry no data and are skipped.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;

use miniz_oxide::inflate::{TINFLStatus, decompress_to_vec_with_limit};
use rawzip::{CompressionMethod, ZipArchive, ZipArchiveWriter};

use crate::error::LearnError;
use crate::pack::{MANIFEST_PATH, MAX_FILE_BYTES, Manifest, Pack};

/// The most entries an archive may declare.
pub const MAX_ENTRIES: u64 = 1024;
/// The largest archive the reader opens, in bytes.
pub const MAX_ARCHIVE_BYTES: usize = 32 << 20;
/// The deflate level the canonical writer uses (miniz levels 0–10): fixed, so a pack builds to the same bytes.
pub const DEFLATE_LEVEL: u8 = 9;

/// Read and verify an `.enthpack`: [`extract`], then [`Pack::from_files`].
pub fn read_archive(bytes: &[u8]) -> Result<Pack, LearnError> {
    Pack::from_files(&extract(bytes)?)
}

/// One entry the central directory declares.
struct Declared {
    name: String,
    wayfinder: rawzip::ZipArchiveEntryWayfinder,
    method: CompressionMethod,
    crc: u32,
    size: u64,
}

/// Take the files out of an `.enthpack` in memory, under the entry rules in the [module docs](self): `pack.json`
/// first (it lists the sizes the rest inflate to), then every listed file. Hashes and lessons are checked by
/// [`Pack::from_files`].
pub fn extract(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, LearnError> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(LearnError::TooLarge {
            path: "*".to_owned(),
            size: bytes.len() as u64,
            max: MAX_ARCHIVE_BYTES as u64,
        });
    }
    let archive = ZipArchive::from_slice(bytes).map_err(|e| bad_archive(&e))?;
    if archive.entries_hint() > MAX_ENTRIES {
        return Err(LearnError::TooManyEntries {
            found: archive.entries_hint(),
            max: MAX_ENTRIES,
        });
    }
    let mut declared: Vec<Declared> = Vec::new();
    let mut names = BTreeSet::new();
    let mut entries = archive.entries();
    let mut count = 0_u64;
    while let Some(record) = entries.next_entry().map_err(|e| bad_archive(&e))? {
        count += 1;
        if count > MAX_ENTRIES {
            return Err(LearnError::TooManyEntries {
                found: count,
                max: MAX_ENTRIES,
            });
        }
        let raw = record.file_path().as_bytes();
        let raw_text = String::from_utf8_lossy(raw).into_owned();
        let renamed = || LearnError::RenamedEntry {
            raw: raw_text.clone(),
        };
        let normalized = record.file_path().try_normalize().map_err(|_| renamed())?;
        if normalized.as_str().as_bytes() != raw {
            return Err(renamed());
        }
        let name = normalized.as_str().to_owned();
        if record.is_dir() {
            continue;
        }
        if !names.insert(name.clone()) {
            return Err(LearnError::DuplicateEntry { path: name });
        }
        if record.flags().is_encrypted() {
            return Err(LearnError::Encrypted { path: name });
        }
        let method = record.compression_method();
        if method != CompressionMethod::STORE && method != CompressionMethod::DEFLATE {
            return Err(LearnError::Compression {
                path: name,
                method: method.as_u16(),
            });
        }
        let size = record.uncompressed_size_hint();
        if size > MAX_FILE_BYTES {
            return Err(LearnError::TooLarge {
                path: name,
                size,
                max: MAX_FILE_BYTES,
            });
        }
        declared.push(Declared {
            name,
            wayfinder: record.wayfinder(),
            method,
            crc: record.crc32(),
            size,
        });
    }

    let manifest_entry = declared
        .iter()
        .find(|d| d.name == MANIFEST_PATH)
        .ok_or_else(|| LearnError::Missing {
            path: MANIFEST_PATH.to_owned(),
        })?;
    let manifest_bytes = inflate(&archive, manifest_entry, manifest_entry.size)?;
    let manifest_text =
        std::str::from_utf8(&manifest_bytes).map_err(|_| LearnError::BadSyntax {
            doc: crate::error::Doc::Pack,
            path: Some(MANIFEST_PATH.to_owned()),
            message: "pack.json is not UTF-8".to_owned(),
        })?;
    let manifest = Manifest::from_json(manifest_text)?;

    let mut files = BTreeMap::new();
    for d in declared.iter().filter(|d| d.name != MANIFEST_PATH) {
        let listed = manifest.file(&d.name).ok_or_else(|| LearnError::Unlisted {
            path: d.name.clone(),
        })?;
        if d.size != listed.size {
            return Err(LearnError::SizeMismatch {
                path: d.name.clone(),
                listed: listed.size,
                found: d.size,
            });
        }
        files.insert(d.name.clone(), inflate(&archive, d, listed.size)?);
    }
    files.insert(MANIFEST_PATH.to_owned(), manifest_bytes);
    Ok(files)
}

/// Inflate one entry, capped at `limit` bytes, and check its local header and CRC.
fn inflate(
    archive: &rawzip::ZipSliceArchive<&[u8]>,
    d: &Declared,
    limit: u64,
) -> Result<Vec<u8>, LearnError> {
    let entry = archive
        .get_entry(d.wayfinder)
        .map_err(|e| LearnError::Corrupt {
            path: d.name.clone(),
            message: e.to_string(),
        })?;
    let local = entry.local_header();
    if local.file_path().as_bytes() != d.name.as_bytes() {
        return Err(LearnError::RenamedEntry {
            raw: String::from_utf8_lossy(local.file_path().as_bytes()).into_owned(),
        });
    }
    if local.compression_method() != d.method {
        return Err(LearnError::Corrupt {
            path: d.name.clone(),
            message: "the local header and the central directory name different methods".to_owned(),
        });
    }
    let data = entry.data();
    let limit_usize = usize::try_from(limit).unwrap_or(usize::MAX);
    let out = if d.method == CompressionMethod::STORE {
        if data.len() > limit_usize {
            return Err(LearnError::InflateOverflow {
                path: d.name.clone(),
                listed: limit,
            });
        }
        data.to_vec()
    } else {
        decompress_to_vec_with_limit(data, limit_usize).map_err(|e| {
            if e.status == TINFLStatus::HasMoreOutput {
                LearnError::InflateOverflow {
                    path: d.name.clone(),
                    listed: limit,
                }
            } else {
                LearnError::Corrupt {
                    path: d.name.clone(),
                    message: format!("deflate: {e}"),
                }
            }
        })?
    };
    if out.len() as u64 != limit {
        return Err(LearnError::SizeMismatch {
            path: d.name.clone(),
            listed: limit,
            found: out.len() as u64,
        });
    }
    if rawzip::crc32(&out) != d.crc {
        return Err(LearnError::Corrupt {
            path: d.name.clone(),
            message: "CRC-32 mismatch".to_owned(),
        });
    }
    Ok(out)
}

fn bad_archive(e: &rawzip::Error) -> LearnError {
    LearnError::BadArchive {
        message: e.to_string(),
    }
}

/// The canonical `.enthpack` of `files` (every file by path, `pack.json` included): `pack.json` first, then the
/// rest in path order, each deflated at [`DEFLATE_LEVEL`] (or stored when deflate would not shrink it), with no
/// timestamps (DOS zero), no permissions and no comments, so the same files always give the same bytes (each build
/// pins the hashes of the packs it bundles). `cargo xtask pack` writes this after [`Manifest::seal`].
pub fn write_archive(files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut archive = ZipArchiveWriter::new(Vec::new());
    let order = files
        .get_key_value(MANIFEST_PATH)
        .into_iter()
        .chain(files.iter().filter(|(k, _)| k.as_str() != MANIFEST_PATH));
    for (name, bytes) in order {
        // Writing to a Vec cannot fail; an error here is a bug in the writer, and it leaves the archive invalid,
        // which `read_archive` then refuses.
        let _ = write_entry(&mut archive, name, bytes);
    }
    archive.finish().unwrap_or_default()
}

fn write_entry(
    archive: &mut ZipArchiveWriter<Vec<u8>>,
    name: &str,
    bytes: &[u8],
) -> Result<(), rawzip::Error> {
    let deflated = miniz_oxide::deflate::compress_to_vec(bytes, DEFLATE_LEVEL);
    let (method, payload) = if deflated.len() < bytes.len() {
        (CompressionMethod::DEFLATE, deflated.as_slice())
    } else {
        (CompressionMethod::STORE, bytes)
    };
    let (mut entry, _config) = archive.new_file(name).compression_method(method).start()?;
    entry.write_all(payload)?;
    // The descriptor carries the uncompressed size and CRC; rawzip fills in the compressed size.
    entry.finish(rawzip::DataDescriptorOutput::new(
        rawzip::crc32(bytes),
        bytes.len() as u64,
    ))?;
    Ok(())
}
