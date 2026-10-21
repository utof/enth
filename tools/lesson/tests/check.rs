//! `check` (lane LCK's acceptance, docs/phase-3.md): it catches a seeded bad lesson of each kind (a start that passes,
//! a witness that fails or touches a locked control, a dead link, an inaudible choice, and more), each pack under
//! `tests/fixtures/` holding one fault, and the example pack passes `--cross --solve`. The fixtures are built from the
//! example pack's two patches (the two good ones, `odd-one-out` and `descriptor-audible`, also from the basics pack's),
//! so each bad one fails for its one seeded reason and nothing else.
#![allow(
    clippy::disallowed_methods,
    reason = "the test runs the tool's own binary on fixture packs"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tool(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_enth-lesson-tool"))
        .args(args)
        .output()
        .expect("the tool runs")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The failures the report lists at its end, one per line.
fn failures(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| *l != "failures:")
        .skip(1)
        .map(|l| l.trim().to_owned())
        .collect()
}

#[test]
fn every_seeded_bad_lesson_is_caught_for_its_reason() {
    // (fixture, options, the failures expected, each a fragment of one failure line)
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "start-passes",
            &[],
            &["start-passes/l#try: the start already passes the check"],
        ),
        (
            "witness-fails",
            &[],
            &[
                "witness-fails/l#try: the witness does not pass the check: filter1.cutoff < 300Hz fails",
            ],
        ),
        (
            "witness-locked",
            &[],
            &[
                "the witness touches filter1.cutoff, which the step locks (filter1.cutoff)",
                "Show me: the lesson refused: no op leads from the patch to the witness (no_witness_path",
                "the witness does not pass the check",
                "at the witness, spectrum.centroid_hz < 500 fails",
            ],
        ),
        (
            "witness-not-editable",
            &[],
            &["the witness touches filter1.cutoff, which the step's `editable` list leaves out"],
        ),
        (
            "dead-link",
            &[],
            &[
                "lessons/l/text.md:3: selector 'osc3.wave': the start patch has no such parameter (unresolved)",
                "lessons/l/text.md:3: example 2 does not exist: the step has 0 (unresolved)",
            ],
        ),
        (
            "inaudible-choice",
            &["--solve"],
            &[
                r#"inaudible-choice/l#which: --solve: the answer "B" and "A" are 0.00 JND apart under sound/1, below 2, and no descriptor tells them apart by 2 steps: an inaudible choice"#,
            ],
        ),
        (
            "cross-confused",
            &["--cross"],
            &[
                "cross-confused/l#darker: --cross: its check passes cross-confused/l#lower's witness",
                "cross-confused/l#lower: --cross: its check passes cross-confused/l#darker's witness",
            ],
        ),
        ("stray-file", &[], &["notes.txt: not a pack file"]),
    ];
    for (name, options, expected) in cases {
        let path = fixture(name);
        let mut args: Vec<&str> = vec!["check"];
        args.extend(options.iter());
        args.push(path.to_str().unwrap());
        let out = tool(&args);
        let text = stdout(&out);
        assert_eq!(out.status.code(), Some(1), "{name}: {text}");
        let found = failures(&text);
        assert_eq!(found.len(), expected.len(), "{name}: {found:#?}");
        for (want, got) in expected.iter().zip(&found) {
            assert!(got.contains(want), "{name}: wanted {want:?} in {got:?}");
        }
    }
}

#[test]
fn a_fixture_is_clean_without_the_option_that_catches_it() {
    // The inaudible choice and the confused checks are `--solve` and `--cross` findings: without them the packs pass,
    // so each fixture's fault is the only one it has.
    for name in ["inaudible-choice", "cross-confused"] {
        let path = fixture(name);
        let out = tool(&["check", path.to_str().unwrap()]);
        assert!(out.status.success(), "{name}: {}", stdout(&out));
    }
}

#[test]
fn the_audibility_gate_passes_an_odd_one_out_and_a_difference_only_descriptors_hear() {
    // Lane LCK's gate: only the answer must stand 2 JND apart from each distractor; when sound/1 is blind to the
    // difference (a beat rate), a descriptor that tells the options apart decides. Both pass, each with a note.
    for (name, note) in [
        (
            "odd-one-out",
            r#"distractors "A" and "C" are 0.00 JND apart under sound/1: only the answer must stand apart"#,
        ),
        (
            "descriptor-audible",
            "but detune.beating_hz differs by inf steps (present in one option, absent in the other): audible by its descriptors",
        ),
    ] {
        let path = fixture(name);
        let out = tool(&["check", "--solve", path.to_str().unwrap()]);
        let text = stdout(&out);
        assert!(out.status.success(), "{name}: {text}");
        assert!(text.contains(note), "{name}: {note:?} in\n{text}");
    }
}

#[test]
fn the_example_pack_passes_cross_and_solve() {
    let out = tool(&["check", "--cross", "--solve", "example"]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}");
    for line in [
        "files: ok",
        "start:   spectrum.centroid_hz < 500 got 894.",
        "witness: spectrum.centroid_hz < 500 got 321.",
        "the check fails at the start and passes at the witness",
        r#"every wrong pick leaves it open, the answer "B" passes"#,
        "try (do): the hint follower reaches the pass in 1 measured hint(s)",
        "audible (JND): A-B 13.",
        "0 failure(s)",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
}

#[test]
fn every_pack_in_content_lessons_passes() {
    // What `cargo xtask lesson-check` runs (without --cross and --solve, which lessons.yml adds): the packs on main.
    let out = tool(&["check"]);
    assert!(out.status.success(), "{}", stdout(&out));
}

#[test]
fn a_bad_option_is_refused_with_the_usage() {
    let out = tool(&["check", "--nope"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: check"));
    let out = tool(&["check", "no-such-pack"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no such pack"));
}
