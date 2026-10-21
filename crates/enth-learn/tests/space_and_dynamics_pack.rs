//! The `space-and-dynamics` pack (`content/lessons/space-and-dynamics/`, lane C3) reads with this build's reader, as `basics_pack.rs` does
//! for lane C1's pack. `cargo xtask lesson-check --cross --solve` proves the rest (the playthrough, the checks'
//! honesty, reachability, the audibility of the ear questions); this test keeps the pack's bytes in `cargo test`:
//! - every `lesson.toml` and `pack.toml` turns into canonical JSON through the same serde types (predicates written
//!   in PLY's one-line grammar read into the frozen objects), the manifest seals, and `Pack::from_files` accepts the
//!   whole pack (schemas, `requires`, views against their starts, every embedded challenge validated against its
//!   start, every patch loading), and the same files written as a canonical `.enthpack` read back through the zip
//!   reader;
//! - every patch file is exactly `Patch::to_json()`, and every one is used;
//! - every Read step (the Name text) keeps to about 80 words (report 01 §3; 90 words is the hard limit here).

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads the space-and-dynamics pack from the repository; the library does no I/O"
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use enth_learn::pack::lesson_path;
use enth_learn::reader::{read_archive, write_archive};
use enth_learn::{Lesson, Manifest, Pack, StepKind};

/// `content/lessons/space-and-dynamics/`.
fn pack_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/lessons/space-and-dynamics")
}

/// The pack's files as `cargo xtask pack` will write them (canonical JSON, texts, patches, credits, the sealed
/// `pack.json`). A pack directory holds only what the manifest lists.
fn pack_files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let text = fs::read_to_string(dir.join("pack.toml")).unwrap();
    let manifest = Manifest::from_json(&common::toml_as_json(&text))
        .unwrap_or_else(|e| panic!("pack.toml: {e}"));
    let mut files = BTreeMap::new();
    files.insert(
        "CREDITS.md".to_owned(),
        fs::read(dir.join("CREDITS.md")).unwrap(),
    );
    for entry in fs::read_dir(dir.join("patches")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        files.insert(format!("patches/{name}"), fs::read(&path).unwrap());
    }
    for id in &manifest.lessons {
        let ldir = dir.join("lessons").join(id.as_str());
        for entry in fs::read_dir(&ldir).unwrap_or_else(|e| panic!("{}: {e}", ldir.display())) {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            if name == "lesson.toml" {
                let text = fs::read_to_string(&path).unwrap();
                let lesson = Lesson::from_json(&common::toml_as_json(&text))
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                files.insert(lesson_path(id), lesson.to_json().into_bytes());
            } else {
                files.insert(format!("lessons/{id}/{name}"), fs::read(&path).unwrap());
            }
        }
    }
    common::seal(&manifest, files)
}

#[test]
fn the_space_and_dynamics_pack_reads() {
    let files = pack_files(&pack_dir());
    let pack =
        Pack::from_files(&files).unwrap_or_else(|e| panic!("the space-and-dynamics pack: {e}"));
    assert_eq!(pack.id().as_str(), "space-and-dynamics");
    assert_eq!(
        pack.lessons().count(),
        pack.manifest().lessons.len(),
        "every listed lesson reads"
    );
}

#[test]
fn the_space_and_dynamics_pack_reads_as_an_enthpack() {
    let files = pack_files(&pack_dir());
    let archive = write_archive(&files);
    assert_eq!(
        archive,
        write_archive(&files),
        "the archive is reproducible"
    );
    let pack =
        read_archive(&archive).unwrap_or_else(|e| panic!("space-and-dynamics.enthpack: {e}"));
    assert_eq!(
        pack.manifest(),
        Pack::from_files(&files).unwrap().manifest()
    );
}

#[test]
fn every_patch_is_canonical_and_used() {
    let dir = pack_dir();
    let files = pack_files(&dir);
    let pack =
        Pack::from_files(&files).unwrap_or_else(|e| panic!("the space-and-dynamics pack: {e}"));
    let used: BTreeSet<String> = pack
        .lessons()
        .flat_map(|l| {
            l.patches()
                .map(|r| r.as_str().to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    for (path, bytes) in files.iter().filter(|(p, _)| p.starts_with("patches/")) {
        let text = std::str::from_utf8(bytes).unwrap();
        let loaded = enth_patch::file::load(text).unwrap_or_else(|e| panic!("{path}: {e:?}"));
        assert!(
            loaded.patch.to_json() == text,
            "{path} is not exactly Patch::to_json()"
        );
        assert!(used.contains(path), "{path} is not used by any lesson");
    }
}

#[test]
fn name_steps_keep_to_about_eighty_words() {
    let files = pack_files(&pack_dir());
    let pack =
        Pack::from_files(&files).unwrap_or_else(|e| panic!("the space-and-dynamics pack: {e}"));
    for lesson in pack.lessons() {
        for step in lesson
            .steps
            .iter()
            .filter(|s| matches!(s.kind, StepKind::Read))
        {
            let text = step.text.as_ref().unwrap();
            let path = text.path_in(&lesson.id);
            let body = std::str::from_utf8(&files[&path]).unwrap();
            let words = body
                .lines()
                .filter(|l| !l.starts_with('#'))
                .flat_map(str::split_whitespace)
                .count();
            assert!(
                words <= 90,
                "{path}: {words} words (a Name step keeps to about 80)"
            );
        }
    }
}
