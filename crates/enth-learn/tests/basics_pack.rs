//! The `basics` pack (`content/lessons/basics/`, lane C1) reads with this build's reader, until lane LCK's
//! `cargo xtask lesson-check` exists to do it (and more: the playthrough, `--cross`, `--solve`).
//!
//! What it checks, in the authoring layout, as `common::example_files` does for the example pack:
//! - every `lesson.toml` and `pack.toml` turns into canonical JSON through the same serde types, the manifest seals,
//!   and `Pack::from_files` accepts the whole pack (schemas, `requires`, views against their starts, every embedded
//!   challenge validated against its start, every patch loading);
//! - every patch file is exactly `Patch::to_json()` (docs/learn.md, `lesson-check` step 1), and every one is used;
//! - every Read step (the Name text) keeps to about 80 words (report 01 §3; 90 words is the hard limit here).
//!
//! It neither renders nor plays: the sound checks were measured with `enth check`, and the rounds with
//! `enth compare` (lane C1's notes); the playthrough is LCK's.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads the basics pack from the repository; the library does no I/O"
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use enth_learn::pack::lesson_path;
use enth_learn::{Lesson, Manifest, Pack, StepKind};

/// `content/lessons/basics/`.
fn basics_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/lessons/basics")
}

/// The pack's files as `cargo xtask pack` will write them (canonical JSON, texts, patches, credits, the sealed
/// `pack.json`). Authoring-only files at the pack's root (`TASTE.md`, notes) are not part of the pack.
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
fn the_basics_pack_reads() {
    let files = pack_files(&basics_dir());
    let pack = Pack::from_files(&files).unwrap_or_else(|e| panic!("the basics pack: {e}"));
    assert_eq!(pack.id().as_str(), "basics");
    assert_eq!(
        pack.lessons().count(),
        pack.manifest().lessons.len(),
        "every listed lesson reads"
    );
}

#[test]
fn every_patch_is_canonical_and_used() {
    let dir = basics_dir();
    let files = pack_files(&dir);
    let pack = Pack::from_files(&files).unwrap_or_else(|e| panic!("the basics pack: {e}"));
    let used: BTreeSet<String> = pack
        .lessons()
        .flat_map(|l| l.patches().map(|r| r.as_str().to_owned()).collect::<Vec<_>>())
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
    let files = pack_files(&basics_dir());
    let pack = Pack::from_files(&files).unwrap_or_else(|e| panic!("the basics pack: {e}"));
    for lesson in pack.lessons() {
        for step in lesson.steps.iter().filter(|s| matches!(s.kind, StepKind::Read)) {
            let text = step.text.as_ref().unwrap();
            let path = text.path_in(&lesson.id);
            let body = std::str::from_utf8(&files[&path]).unwrap();
            let words = body
                .lines()
                .filter(|l| !l.starts_with('#'))
                .flat_map(str::split_whitespace)
                .count();
            assert!(words <= 90, "{path}: {words} words (a Name step keeps to about 80)");
        }
    }
}
