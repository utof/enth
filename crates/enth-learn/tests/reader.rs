//! The `.enthpack` reader and `Pack::from_files`: every refusal of report 02 §5 and docs/learn.md ("Integrity and
//! the reader"), each with its code and a hint, and the canonical writer's round trip.

mod common;

use std::collections::BTreeMap;
use std::io::Write as _;

use enth_learn::pack::{MANIFEST_PATH, sha256};
use enth_learn::reader::{MAX_ENTRIES, extract, read_archive, write_archive};
use enth_learn::{LearnError, Pack};
use rawzip::path::EntryPath;
use rawzip::{CompressionMethod, DataDescriptorOutput, ZipArchiveWriter};

/// One entry to write as given: raw name bytes, method, payload, and the CRC and size to declare.
struct Raw {
    name: Vec<u8>,
    method: CompressionMethod,
    payload: Vec<u8>,
    crc: u32,
    size: u64,
    encrypted: bool,
}

/// A stored entry, honestly declared.
fn stored(name: &str, data: &[u8]) -> Raw {
    Raw {
        name: name.as_bytes().to_vec(),
        method: CompressionMethod::STORE,
        payload: data.to_vec(),
        crc: rawzip::crc32(data),
        size: data.len() as u64,
        encrypted: false,
    }
}

fn zip(entries: Vec<Raw>) -> Vec<u8> {
    let mut archive = ZipArchiveWriter::new(Vec::new());
    for e in entries {
        let (mut w, _) = archive
            .new_file(EntryPath::verbatim(e.name))
            .compression_method(e.method)
            .encrypted(e.encrypted)
            .start()
            .unwrap();
        w.write_all(&e.payload).unwrap();
        w.finish(DataDescriptorOutput::new(e.crc, e.size)).unwrap();
    }
    archive.finish().unwrap()
}

/// The example pack's files, every one stored, plus `extra`.
fn example_with(extra: Vec<Raw>) -> Vec<u8> {
    let files = common::example_files();
    let mut entries: Vec<Raw> = files.iter().map(|(k, v)| stored(k, v)).collect();
    entries.extend(extra);
    zip(entries)
}

fn code(r: Result<Pack, LearnError>) -> &'static str {
    let e = r.expect_err("refused");
    assert_ne!(e.hint(), "", "{} has a hint", e.code());
    assert_ne!(e.to_string(), "");
    e.code()
}

#[test]
fn the_canonical_archive_round_trips_and_is_reproducible() {
    let files = common::example_files();
    let a = write_archive(&files);
    let b = write_archive(&files);
    assert_eq!(a, b, "the same files give the same bytes");
    assert_eq!(extract(&a).unwrap(), files);
    let pack = read_archive(&a).unwrap();
    assert_eq!(pack.id().as_str(), "example");
    assert_eq!(pack.lessons().count(), 1);
    // A stored archive written by another tool reads the same.
    assert_eq!(
        read_archive(&example_with(Vec::new())).unwrap().manifest(),
        pack.manifest()
    );
}

#[test]
fn a_folder_pack_reads_through_the_same_rules() {
    let files = common::example_files();
    let pack = Pack::from_files(&files).unwrap();
    let bundle = pack.bundle(&"cutoff".parse().unwrap()).unwrap();
    assert_eq!(bundle.patches.len(), 2);
    assert_eq!(bundle.texts.len(), 3);
    assert_eq!(bundle.key().to_string(), "example/cutoff");
}

#[test]
fn entries_that_would_be_renamed_are_refused() {
    for raw in [
        "../evil.txt",
        "lessons/../CREDITS.md",
        "/CREDITS.md",
        "lessons\\cutoff\\x.md",
        "./CREDITS.md",
    ] {
        let bytes = example_with(vec![stored(raw, b"x")]);
        assert_eq!(code(read_archive(&bytes)), "renamed_entry", "{raw}");
    }
}

#[test]
fn a_local_header_that_names_another_file_is_refused() {
    let mut bytes = write_archive(&common::example_files());
    // The first occurrence of a name is its local header (the central directory comes last).
    let name = b"CREDITS.md";
    let at = bytes.windows(name.len()).position(|w| w == name).unwrap();
    bytes[at] = b'X';
    assert_eq!(code(read_archive(&bytes)), "renamed_entry");
}

#[test]
fn duplicates_unlisted_and_missing_files_are_refused() {
    let files = common::example_files();
    let credits = files["CREDITS.md"].clone();
    assert_eq!(
        code(read_archive(&example_with(vec![stored(
            "CREDITS.md",
            &credits
        )]))),
        "duplicate_entry"
    );
    assert_eq!(
        code(read_archive(&example_with(vec![stored(
            "patches/extra.patch.json",
            b"{}"
        )]))),
        "unlisted_file"
    );
    let mut without = files.clone();
    without.remove("lessons/cutoff/step-3.md");
    assert_eq!(code(read_archive(&write_archive(&without))), "missing_file");
    assert_eq!(code(Pack::from_files(&without)), "missing_file");
    let mut no_manifest = files;
    no_manifest.remove(MANIFEST_PATH);
    assert_eq!(
        code(read_archive(&write_archive(&no_manifest))),
        "missing_file"
    );
}

#[test]
fn sizes_and_hashes_must_match_the_manifest() {
    let mut files = common::example_files();
    files.get_mut("CREDITS.md").unwrap().push(b'!');
    assert_eq!(code(read_archive(&write_archive(&files))), "size_mismatch");
    let mut files = common::example_files();
    let last = files.get_mut("CREDITS.md").unwrap().last_mut().unwrap();
    *last ^= 1;
    assert_eq!(code(read_archive(&write_archive(&files))), "hash_mismatch");
    assert_eq!(code(Pack::from_files(&files)), "hash_mismatch");
}

#[test]
fn inflation_past_the_listed_size_is_refused() {
    let files = common::example_files();
    let listed = files["CREDITS.md"].len();
    // The entry declares the listed size, but its deflate stream holds far more.
    let bomb = vec![b'a'; listed * 1000];
    let mut entries: Vec<Raw> = files
        .iter()
        .filter(|(k, _)| k.as_str() != "CREDITS.md")
        .map(|(k, v)| stored(k, v))
        .collect();
    entries.push(Raw {
        name: b"CREDITS.md".to_vec(),
        method: CompressionMethod::DEFLATE,
        payload: miniz_oxide::deflate::compress_to_vec(&bomb, 6),
        crc: rawzip::crc32(&files["CREDITS.md"]),
        size: listed as u64,
        encrypted: false,
    });
    assert_eq!(code(read_archive(&zip(entries))), "inflate_overflow");
    // Stored data longer than the listed size, too.
    let mut entries: Vec<Raw> = files
        .iter()
        .filter(|(k, _)| k.as_str() != "CREDITS.md")
        .map(|(k, v)| stored(k, v))
        .collect();
    entries.push(Raw {
        size: listed as u64,
        ..stored("CREDITS.md", &bomb)
    });
    assert_eq!(code(read_archive(&zip(entries))), "inflate_overflow");
}

#[test]
fn methods_encryption_size_and_counts_are_limited() {
    let bz = Raw {
        method: CompressionMethod::BZIP2,
        ..stored("lessons/cutoff/z.md", b"x")
    };
    assert_eq!(
        code(read_archive(&example_with(vec![bz]))),
        "unsupported_compression"
    );
    let enc = Raw {
        encrypted: true,
        ..stored("lessons/cutoff/z.md", b"x")
    };
    assert_eq!(
        code(read_archive(&example_with(vec![enc]))),
        "encrypted_entry"
    );
    let big = vec![0_u8; (1 << 20) + 1];
    let huge = Raw {
        method: CompressionMethod::DEFLATE,
        payload: miniz_oxide::deflate::compress_to_vec(&big, 6),
        ..stored("lessons/cutoff/z.md", &big)
    };
    assert_eq!(code(read_archive(&example_with(vec![huge]))), "too_large");
    let many: Vec<Raw> = (0..=MAX_ENTRIES)
        .map(|i| stored(&format!("lessons/cutoff/{i}.md"), b""))
        .collect();
    assert_eq!(code(read_archive(&example_with(many))), "too_many_entries");
    assert_eq!(code(read_archive(b"not a zip")), "bad_archive");
}

#[test]
fn directory_entries_are_skipped() {
    let mut archive = ZipArchiveWriter::new(Vec::new());
    archive.new_dir("lessons/").create().unwrap();
    for (k, v) in common::example_files() {
        let (mut w, _) = archive.new_file(k.as_str()).start().unwrap();
        w.write_all(&v).unwrap();
        w.finish(DataDescriptorOutput::new(rawzip::crc32(&v), v.len() as u64))
            .unwrap();
    }
    assert!(read_archive(&archive.finish().unwrap()).is_ok());
}

/// The example's files with `pack.json` replaced by `edit` applied to its JSON (hashes recomputed for the rest).
fn with_manifest(edit: impl FnOnce(&mut serde_json::Value)) -> BTreeMap<String, Vec<u8>> {
    let mut files = common::example_files();
    let mut v: serde_json::Value = serde_json::from_slice(&files[MANIFEST_PATH]).unwrap();
    edit(&mut v);
    files.insert(MANIFEST_PATH.to_owned(), v.to_string().into_bytes());
    files
}

#[test]
fn a_newer_pack_is_refused_and_named() {
    let files = with_manifest(|v| v["schema"] = "enth.pack/2".into());
    let e = read_archive(&write_archive(&files)).unwrap_err();
    assert_eq!(e.code(), "pack_too_new");
    let LearnError::TooNew { name, found, .. } = &e else {
        panic!("{e:?}")
    };
    assert_eq!(
        (name.id.as_deref(), name.title.as_deref(), *found),
        (Some("example"), Some("Example pack"), 2)
    );
    assert!(e.to_string().contains("Example pack 0.1.0"), "{e}");
    assert!(e.hint().contains("newer enth"), "{}", e.hint());
    // A newer pack with fields this build has never heard of still names itself.
    let files = with_manifest(|v| {
        v["schema"] = "enth.pack/7".into();
        v["future"] = true.into();
    });
    assert_eq!(code(Pack::from_files(&files)), "pack_too_new");
}

#[test]
fn requires_must_be_met_and_must_not_lie() {
    for (field, value, want) in [
        ("lesson", serde_json::json!(2), "requires_unmet"),
        ("patch", serde_json::json!(9), "requires_unmet"),
        ("challenge", serde_json::json!(99), "requires_unmet"),
        ("generator", serde_json::json!(99), "requires_unmet"),
        ("scorers", serde_json::json!(["sound/9"]), "requires_unmet"),
    ] {
        let files = with_manifest(|v| v["requires"][field] = value);
        assert_eq!(code(Pack::from_files(&files)), want, "{field}");
    }
    let unknown = with_manifest(|v| v["requires"]["engine"] = "0.3.0".into());
    assert_eq!(code(Pack::from_files(&unknown)), "bad_pack");
}

#[test]
fn unknown_fields_and_bad_ids_are_refused() {
    let files = with_manifest(|v| v["extra"] = 1.into());
    assert_eq!(code(Pack::from_files(&files)), "bad_pack");
    let files = with_manifest(|v| v["id"] = "Example".into());
    assert_eq!(code(Pack::from_files(&files)), "bad_pack");
    let files = with_manifest(|v| v["version"] = "1.0".into());
    assert_eq!(code(Pack::from_files(&files)), "bad_pack");
    let files = with_manifest(|v| v["files"][0]["path"] = "../x.md".into());
    assert_eq!(code(Pack::from_files(&files)), "bad_pack");
}

/// An edit of a lesson's JSON.
type Edit = dyn Fn(&mut serde_json::Value);

/// The example's files with the lesson replaced by `json` and the manifest resealed.
fn with_lesson(json: &str) -> BTreeMap<String, Vec<u8>> {
    let mut files = common::example_files();
    files.insert(
        "lessons/cutoff/lesson.json".to_owned(),
        json.as_bytes().to_vec(),
    );
    common::seal(&common::example_manifest(), files)
}

#[test]
fn lessons_are_strict_inputs_too() {
    let base: serde_json::Value =
        serde_json::from_slice(&common::example_files()["lessons/cutoff/lesson.json"]).unwrap();
    let edit = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut v = base.clone();
        f(&mut v);
        v.to_string()
    };
    // A newer lesson is refused and names its pack.
    let e = Pack::from_files(&with_lesson(&edit(&|v| {
        v["schema"] = "enth.lesson/2".into();
    })))
    .unwrap_err();
    assert_eq!(e.code(), "lesson_too_new");
    assert!(e.to_string().contains("Example pack"), "{e}");
    let cases: [(&str, &Edit, &str); 7] = [
        (
            "unknown field",
            &|v| v["steps"][0]["extra"] = 1.into(),
            "bad_lesson",
        ),
        (
            "a field of another kind",
            &|v| v["steps"][0]["answer"] = 0.into(),
            "bad_field",
        ),
        (
            "an answer past the options",
            &|v| v["steps"][2]["answer"] = 5.into(),
            "bad_field",
        ),
        (
            "a duplicate step id",
            &|v| v["steps"][1]["id"] = "hear".into(),
            "bad_field",
        ),
        (
            "an unlisted patch",
            &|v| v["steps"][1]["witness"] = "patches/none.patch.json".into(),
            "bad_field",
        ),
        (
            "a bad expectation",
            &|v| v["steps"][1]["check"]["expect"][0] = "brightnes<1kHz".into(),
            "bad_field",
        ),
        (
            "another id than its directory",
            &|v| v["id"] = "other".into(),
            "bad_field",
        ),
    ];
    for (what, f, want) in cases {
        assert_eq!(
            code(Pack::from_files(&with_lesson(&edit(f)))),
            want,
            "{what}"
        );
    }
}

#[test]
fn a_patch_that_does_not_load_is_refused() {
    let mut files = common::example_files();
    files.insert(
        "patches/dark.patch.json".to_owned(),
        br#"{"schema":1,"bogus":true}"#.to_vec(),
    );
    let files = common::seal(&common::example_manifest(), files);
    assert_eq!(code(Pack::from_files(&files)), "bad_content");
}

#[test]
fn sealing_lists_every_file_with_its_hash() {
    let files = common::example_files();
    let manifest = Pack::from_files(&files).unwrap().manifest().clone();
    assert_eq!(manifest.files.len(), files.len() - 1);
    for f in &manifest.files {
        assert_eq!(f.sha256.0, sha256(&files[f.path.as_str()]));
    }
    assert_eq!(manifest.engine, enth_patch::file::ENGINE_VERSION);
}
