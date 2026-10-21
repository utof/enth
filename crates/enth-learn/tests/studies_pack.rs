//! The `studies` pack (`content/lessons/studies/`, lane DEC) reads with this build's reader and keeps the promises
//! its generator made, until lane LCK's `cargo xtask lesson-check` exists to do it (and more: the playthrough through a
//! headless `Session`, `--cross`, `--solve`).
//!
//! What it checks, in the authoring layout, as `basics_pack.rs` does for the basics pack:
//! - every `lesson.toml` and `pack.toml` turns into canonical JSON through the same serde types, the manifest seals,
//!   `Pack::from_files` accepts the whole pack, and the canonical `.enthpack` reads back;
//! - every patch file is exactly `Patch::to_json()` and is used, and each study's full preset is a byte-for-byte copy
//!   of its candidate in `content/presets-candidates/` (so the GR3 exclusion by file hash covers it);
//! - every text lints clean (`markdown::lint`, the 120-word step budget) and its links resolve against its step's start;
//! - the first half of the honesty lesson-check proves: each Do step's check fails at its start and passes for its
//!   witness (its sound items rendered once each, without the nuisance renders), and each Recreate step's target
//!   differs from its start, with something editable, its start below the pass mark under `sound/1` and its target
//!   passing.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads the studies pack from the repository; the library does no I/O"
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use enth_learn::check::{TouchLog, evaluate_sounds};
use enth_learn::markdown::{STEP_WORD_BUDGET, lint, parse};
use enth_learn::pack::lesson_path;
use enth_learn::reader::{read_archive, write_archive};
use enth_learn::{Check, Lesson, Manifest, Pack, StepKind};

/// `content/lessons/studies/`.
fn studies_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/lessons/studies")
}

/// The pack's files as `cargo xtask pack` will write them.
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

fn pack() -> (Pack, BTreeMap<String, Vec<u8>>) {
    let files = pack_files(&studies_dir());
    let pack = Pack::from_files(&files).unwrap_or_else(|e| panic!("the studies pack: {e}"));
    (pack, files)
}

#[test]
fn the_studies_pack_reads_and_builds_reproducibly() {
    let (pack, files) = pack();
    assert_eq!(pack.id().as_str(), "studies");
    assert_eq!(pack.lessons().count(), 6, "PLAN.md's six studies");
    let archive = write_archive(&files);
    assert_eq!(
        archive,
        write_archive(&files),
        "the archive is reproducible"
    );
    let back = read_archive(&archive).unwrap_or_else(|e| panic!("studies.enthpack: {e}"));
    assert_eq!(back.manifest(), pack.manifest());
}

#[test]
fn every_patch_is_canonical_used_and_each_preset_is_its_candidate() {
    let (pack, files) = pack();
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
    let candidates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/presets-candidates");
    for (lesson, candidate) in [
        ("bass", "bass/acid-snap"),
        ("plucks", "pluck/glass-pluck"),
        ("pads", "pad/velvet-pad"),
        ("leads", "lead/breath-flute"),
        ("percussion", "perc/thump-kick"),
        ("textures", "texture/wind-tunnel"),
    ] {
        let ours = &files[&format!("patches/{lesson}.patch.json")];
        let theirs = fs::read(candidates.join(format!("{candidate}.patch.json"))).unwrap();
        assert!(
            ours == &theirs,
            "patches/{lesson}.patch.json is not a byte copy of {candidate}"
        );
    }
}

#[test]
fn every_text_lints_clean_and_resolves() {
    let (pack, files) = pack();
    for lesson in pack.lessons() {
        for step in &lesson.steps {
            let Some(text) = &step.text else { continue };
            let path = text.path_in(&lesson.id);
            let body = std::str::from_utf8(&files[&path]).unwrap();
            let issues = lint(body, STEP_WORD_BUDGET);
            assert!(issues.is_empty(), "{path}: {issues:?}");
            let start = match (&step.view.start, &step.kind) {
                (Some(r), _) => Some(pack.patch(r).unwrap().as_ref().clone()),
                (None, StepKind::Recreate { challenge, .. }) => {
                    Some(challenge.start_patch().unwrap())
                }
                _ => None,
            };
            let examples = match &step.kind {
                StepKind::Listen { examples } => examples.len(),
                StepKind::Choose { options, .. } => options.len(),
                _ => 0,
            };
            let unresolved = parse(body).resolve(start.as_ref(), examples);
            assert!(unresolved.is_empty(), "{path}: {unresolved:?}");
        }
    }
}

/// The patch half and the sound half of a Do step's check.
fn halves(check: &Check) -> (Vec<&enth_learn::Predicate>, Vec<enth_learn::SoundCheck>) {
    let preds = match check {
        Check::Patch { predicates } => predicates.iter().collect(),
        Check::All { checks } => checks
            .iter()
            .flat_map(|c| match c {
                Check::Patch { predicates } => predicates.iter().collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect(),
        Check::Sound { .. } => Vec::new(),
    };
    (preds, check.sounds())
}

#[test]
fn do_steps_fail_at_their_start_and_pass_for_their_witness() {
    let (pack, _) = pack();
    let none = TouchLog::default();
    let mut checked = 0;
    for lesson in pack.lessons() {
        for step in &lesson.steps {
            let StepKind::Do { check, witness } = &step.kind else {
                continue;
            };
            let start = pack
                .patch(step.view.start.as_ref().expect("a Do step has a start"))
                .unwrap();
            let witness = pack.patch(witness).unwrap();
            let (preds, sounds) = halves(check);
            for p in &preds {
                assert!(
                    p.holds(witness, &none),
                    "{}/{}: the witness fails {p}",
                    lesson.id,
                    step.id
                );
            }
            if !sounds.is_empty() {
                let w = evaluate_sounds(&sounds, witness).unwrap();
                assert!(
                    w.passed,
                    "{}/{}: the witness fails {:?}",
                    lesson.id, step.id, w.items
                );
            }
            // The start fails the check: a predicate fails, or the sound does (context predicates hold at the start on
            // purpose, so a step with sound items fails on its sound).
            let patch_holds = preds.iter().all(|p| p.holds(start, &none));
            let sound_holds = sounds.is_empty() || evaluate_sounds(&sounds, start).unwrap().passed;
            assert!(
                !(patch_holds && sound_holds),
                "{}/{}: the start already passes its check",
                lesson.id,
                step.id
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 6,
        "every study has its variation goal and most have Do steps ({checked})"
    );
}

#[test]
fn recreate_steps_start_below_the_pass_mark_with_something_to_change() {
    let (pack, _) = pack();
    for lesson in pack.lessons() {
        for step in &lesson.steps {
            let StepKind::Recreate { challenge, .. } = &step.kind else {
                continue;
            };
            let enth_challenge::Target::Patch(target) = &challenge.target else {
                panic!(
                    "{}/{}: a study round has a patch target",
                    lesson.id, step.id
                );
            };
            let start = challenge.start_patch().unwrap();
            assert_ne!(target.patch(), &start, "{}/{}", lesson.id, step.id);
            assert!(
                !challenge.constraints.editable.is_empty(),
                "{}/{}",
                lesson.id,
                step.id
            );
            let round = enth_challenge::prepare_challenge(challenge.as_ref().clone()).unwrap();
            let s = enth_challenge::score_round(&round, &start).unwrap();
            assert!(
                !s.passed,
                "{}/{}: the start passes at {:.1} %",
                lesson.id, step.id, s.sound.percent
            );
            let w = enth_challenge::score_round(&round, target.patch()).unwrap();
            assert!(
                w.passed,
                "{}/{}: the target scores {:.1} %",
                lesson.id, step.id, w.sound.percent
            );
        }
    }
}
