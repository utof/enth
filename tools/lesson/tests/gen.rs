//! `gen` (lane DEC): `gen deconstruct` writes a lesson in the authoring layout that reads back, keeps hand-edited text
//! on a regeneration, and refuses a preset with nothing to take apart; `gen` alone prints its usage. A one-element
//! preset keeps it to a few dozen renders (the six studies are the full exercise, checked by `lesson-check`).
#![allow(
    clippy::disallowed_methods,
    reason = "the test runs the tool's own binary on files it writes"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_enth-lesson-tool"))
        .args(args)
        .output()
        .expect("the tool runs")
}

fn starter() -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../xtask/src/preset/starter.patch.json"),
    )
    .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn gen_alone_prints_its_usage() {
    let out = run(&["gen"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("gen deconstruct"));
}

#[test]
fn a_preset_with_nothing_to_take_apart_is_refused() {
    let dir = scratch("gen-nothing");
    let preset = dir.join("starter.patch.json");
    fs::write(&preset, starter()).unwrap();
    let out = run(&[
        "gen",
        "deconstruct",
        preset.to_str().unwrap(),
        "--pack",
        dir.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing_to_deconstruct"));
}

#[test]
fn deconstruct_writes_a_lesson_that_reads_back_and_keeps_edited_text() {
    let dir = scratch("gen-one");
    // The starter with a dark filter: one element.
    let text = starter()
        .replace("\"cutoff\":2000.0", "\"cutoff\":300.0")
        .replace("\"name\":\"Starter\"", "\"name\":\"Dark Saw\"");
    assert!(text.contains("\"cutoff\":300.0") && text.contains("Dark Saw"));
    let preset = dir.join("dark-saw.patch.json");
    fs::write(&preset, text).unwrap();
    let args = [
        "gen",
        "deconstruct",
        preset.to_str().unwrap(),
        "--pack",
        dir.to_str().unwrap(),
        "--no-choice",
        "--no-variation",
    ];
    let out = run(&args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lesson_dir = dir.join("lessons/dark-saw");
    let toml = fs::read_to_string(lesson_dir.join("lesson.toml")).unwrap();
    assert!(toml.starts_with("# Deconstruct: Dark Saw"), "{toml}");
    assert!(toml.contains("renders (ablation"), "the cost is recorded");
    let lesson = enth_learn::Lesson::from_toml(&toml).unwrap();
    assert!(
        lesson
            .steps
            .iter()
            .any(|s| s.id.as_str() == "build-filter1")
    );
    for p in lesson.patches() {
        let bytes = fs::read_to_string(dir.join(p.as_str())).unwrap();
        assert_eq!(
            enth_patch::file::load(&bytes).unwrap().patch.to_json(),
            bytes
        );
    }
    // A hand edit survives a regeneration, and the lesson is written again to the same bytes.
    fs::write(lesson_dir.join("intro.md"), "# Edited\n\nBy hand.\n").unwrap();
    let out = run(&args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(lesson_dir.join("intro.md")).unwrap(),
        "# Edited\n\nBy hand.\n"
    );
    assert_eq!(
        fs::read_to_string(lesson_dir.join("lesson.toml")).unwrap(),
        toml
    );
}
