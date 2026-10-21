//! `pack`, `new` and `coverage` (lane LCK): the pack builds to the same bytes every time, the bytes lane LM's fixture
//! pins for the example pack; a scaffolded lesson reads and plays, and fails only on its TODOs; coverage reports the
//! example pack's one parameter and fails on the holes.
#![allow(
    clippy::disallowed_methods,
    reason = "the test runs the tool's own binary and reads what it wrote"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tool(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_enth-lesson-tool"))
        .args(args)
        .output()
        .expect("the tool runs")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A fresh scratch directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("lesson-tool-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn pack_builds_the_example_to_the_bytes_lane_lm_pins() {
    let a = scratch("pack-a");
    let b = scratch("pack-b");
    for dir in [&a, &b] {
        let out = tool(&["pack", "--out", dir.to_str().unwrap(), "example"]);
        assert!(out.status.success(), "{}", stdout(&out));
    }
    let first = fs::read(a.join("example.enthpack")).unwrap();
    let second = fs::read(b.join("example.enthpack")).unwrap();
    assert_eq!(first, second, "two builds, the same bytes");
    let pinned = fs::read_to_string(
        repo().join("crates/enth-learn/tests/fixtures/pack/v1/example.enthpack.sha256"),
    )
    .unwrap();
    let out = tool(&["pack", "--check", "example"]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}");
    assert!(
        text.contains(pinned.trim()),
        "the SHA-256 lane LM pins, in {text}"
    );
    assert!(text.contains("(not written)"), "{text}");
}

#[test]
fn a_scaffolded_lesson_plays_and_fails_only_on_its_todos() {
    let lessons = scratch("new");
    let out = tool(&[
        "new",
        "--in",
        lessons.to_str().unwrap(),
        "drafts",
        "first-lesson",
    ]);
    assert!(out.status.success(), "{}", stdout(&out));
    for file in [
        "pack.toml",
        "CREDITS.md",
        "lessons/first-lesson/lesson.toml",
        "lessons/first-lesson/hear.md",
        "patches/first-lesson-start.patch.json",
        "patches/first-lesson-witness.patch.json",
    ] {
        assert!(lessons.join("drafts").join(file).is_file(), "{file}");
    }
    // A second lesson joins the same pack's `lessons`.
    let out = tool(&["new", "--in", lessons.to_str().unwrap(), "drafts", "second"]);
    assert!(out.status.success(), "{}", stdout(&out));
    let manifest = fs::read_to_string(lessons.join("drafts/pack.toml")).unwrap();
    assert!(
        manifest.contains(r#"lessons = ["first-lesson", "second"]"#),
        "{manifest}"
    );
    // The same id twice is refused, and nothing changes.
    let out = tool(&["new", "--in", lessons.to_str().unwrap(), "drafts", "second"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        fs::read_to_string(lessons.join("drafts/pack.toml")).unwrap(),
        manifest
    );

    let pack = lessons.join("drafts");
    let out = tool(&["check", "--solve", pack.to_str().unwrap()]);
    let text = stdout(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    let failures: Vec<&str> = text
        .lines()
        .skip_while(|l| *l != "failures:")
        .skip(1)
        .collect();
    assert!(!failures.is_empty(), "{text}");
    for f in &failures {
        assert!(f.contains("a TODO is left"), "only TODOs fail: {f}\n{text}");
    }
    assert!(
        text.contains("the check fails at the start and passes at the witness"),
        "{text}"
    );
    // Two untouched scaffolds are the same lesson twice: `--cross` says so.
    let out = tool(&["check", "--cross", pack.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(
        text.contains(
            "drafts/first-lesson#try: --cross: its check passes drafts/second#try's witness"
        ),
        "{text}"
    );
}

#[test]
fn coverage_reports_the_example_pack_and_fails_on_the_holes() {
    let out = tool(&["coverage", "example"]);
    let text = stdout(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    let row = text
        .lines()
        .find(|l| l.starts_with("svf.cutoff "))
        .unwrap_or_else(|| panic!("a row for svf.cutoff in\n{text}"));
    assert!(
        row.contains("basic") && row.contains("S E") && row.contains("example/cutoff"),
        "{row}"
    );
    assert!(
        text.contains("osc: no lesson teaches any of its parameters"),
        "{text}"
    );
    assert!(
        text.contains("svf.resonance: a Basic-tier parameter no lesson teaches"),
        "{text}"
    );
    assert!(
        !text.contains("svf: no non-patch check"),
        "the cutoff's S and E reach svf\n{text}"
    );
    // Without a pack named, the layout example is left out. Since lane C3 (2026-10-21) the real packs leave no hole,
    // so this exits 0; a content change that opens a hole makes it 1, which is not this test's subject.
    let out = tool(&["coverage"]);
    let all = stdout(&out);
    assert!(matches!(out.status.code(), Some(0 | 1)), "{all}");
    assert!(
        !all.contains("example/cutoff") || all.contains("basics"),
        "{all}"
    );
}
