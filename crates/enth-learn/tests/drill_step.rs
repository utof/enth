//! Lane DRL's lesson step (docs/learn.md, "Ear skills, bursts and drills"): `StepKind::Drill`, an `enth.lesson/2`
//! piece (the F1e review's M1), so a pack with one says `requires.lesson = 2`. Its JSON and TOML forms, its fixture,
//! what a pack must say about it, what an R2 or R3 build answers, and how the player runs it: `Effect::StartDrill` on
//! entering, a pass on `drill_ended` whatever the score, and `NotHere` anywhere else.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads a fixture file from the repository; the library does no I/O"
)]

mod common;

use std::fs;

use enth_learn::drill::DrillStep;
use enth_learn::error::Need;
use enth_learn::lesson::{DRILL_LESSON_VERSION, LESSON_VERSION};
use enth_learn::review::Skill;
use enth_learn::{Effect, LearnError, Lesson, Pack, Player, PlayerError, StepKind};

fn lesson_at(version: u32, steps: &str) -> String {
    format!(
        r#"{{"schema":"enth.lesson/{version}","id":"ears","title":"Thirds","tier":"basic","objective":"Hear a third.","teaches":["svf.cutoff"],"steps":[{steps}]}}"#
    )
}

/// A lesson with a drill step: `enth.lesson/2`.
fn lesson_json(steps: &str) -> String {
    lesson_at(DRILL_LESSON_VERSION, steps)
}

/// A read step, a drill that bursts on a skill, and a plain drill: the forms of the step.
fn lesson_text() -> String {
    lesson_json(
        r#"{"id":"intro","kind":"read","text":"text.md"},
        {"id":"thirds","kind":"drill","text":"text.md","game":5,"level":3,"items":15,"skill":"ear.interval.M3"},
        {"id":"check","kind":"drill","game":3}"#,
    )
}

/// The sealed pack of `ears`, declaring lesson schema `lesson_version`.
fn files(lesson: &str, lesson_version: u32) -> std::collections::BTreeMap<String, Vec<u8>> {
    let files = common::test_files(&[("ears", lesson)]);
    let mut manifest = common::test_manifest(&["ears"]);
    manifest.requires.lesson = lesson_version;
    common::seal(&manifest, files)
}

fn pack() -> Pack {
    Pack::from_files(&files(&lesson_text(), DRILL_LESSON_VERSION)).unwrap()
}

fn player() -> Player {
    Player::new(pack().bundle(&"ears".parse().unwrap()).unwrap())
}

fn start_drills(effects: &[Effect]) -> Vec<&DrillStep> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::StartDrill(d) => Some(d),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// The step

#[test]
fn a_drill_step_reads_and_writes_with_its_defaults_left_out() {
    let lesson = Lesson::from_json(&lesson_text()).unwrap();
    let StepKind::Drill(thirds) = &lesson.steps[1].kind else {
        panic!("a drill step");
    };
    assert_eq!(
        thirds,
        &DrillStep {
            game: 5,
            level: 3,
            items: 15,
            skill: Some(Skill::parse("ear.interval.M3").unwrap()),
        }
    );
    let StepKind::Drill(plain) = &lesson.steps[2].kind else {
        panic!("a drill step");
    };
    assert_eq!(plain, &DrillStep::new(3), "level 1, ten items, no skill");
    assert_eq!(lesson.steps[2].kind.name(), "drill");
    let text = lesson.to_json();
    assert!(
        text.contains(r#"{"id":"check","kind":"drill","game":3}"#),
        "{text}"
    );
    assert!(text.starts_with(r#"{"schema":"enth.lesson/2","#), "{text}");
    assert_eq!(Lesson::from_json(&text).unwrap(), lesson);
}

#[test]
fn a_lesson_is_written_at_the_lowest_version_that_holds_it() {
    assert_eq!((LESSON_VERSION, DRILL_LESSON_VERSION), (2, 2));
    let drill = Lesson::from_json(&lesson_text()).unwrap();
    assert_eq!(
        (drill.version(), drill.schema().as_str()),
        (2, "enth.lesson/2")
    );
    // No drill step: still `enth.lesson/1`, byte for byte (the v1 fixtures in fixtures.rs pin the rest).
    let plain_text = lesson_at(1, r#"{"id":"intro","kind":"read","text":"text.md"}"#);
    let plain = Lesson::from_json(&plain_text).unwrap();
    assert_eq!(plain.version(), 1);
    assert_eq!(plain.to_json(), plain_text);
    // A v2 file with no drill step reads, and is written back at v1.
    let v2_plain = lesson_at(2, r#"{"id":"intro","kind":"read","text":"text.md"}"#);
    assert_eq!(Lesson::from_json(&v2_plain).unwrap().to_json(), plain_text);
}

#[test]
fn a_v1_file_holding_a_drill_step_is_refused_naming_the_step() {
    let text = lesson_at(
        1,
        r#"{"id":"intro","kind":"read","text":"text.md"},{"id":"a","kind":"drill","game":5}"#,
    );
    let err = Lesson::from_json(&text).unwrap_err();
    assert_eq!(err.code(), "bad_field", "{err}");
    assert!(err.to_string().contains("steps[1].kind"), "{err}");
    assert!(err.to_string().contains("enth.lesson/2"), "{err}");
    // In a pack too, before any requirement is compared.
    let err = Pack::from_files(&files(&text, 1)).unwrap_err();
    assert!(err.to_string().contains("enth.lesson/2"), "{err}");
    // A newer schema still names itself too new.
    let err =
        Lesson::from_json(&lesson_at(3, r#"{"id":"a","kind":"drill","game":5}"#)).unwrap_err();
    assert_eq!(err.code(), "lesson_too_new", "{err}");
}

#[test]
fn a_drill_step_reads_from_toml_too() {
    let toml = r#"
schema = "enth.lesson/2"
id = "ears"
title = "Thirds"
tier = "basic"
objective = "Hear a third."
teaches = ["svf.cutoff"]

[[steps]]
id = "thirds"
kind = "drill"
text = "text.md"
game = 5
items = 15
skill = "ear.interval.M3"
"#;
    let lesson = Lesson::from_json(&common::toml_as_json(toml)).unwrap();
    let StepKind::Drill(d) = &lesson.steps[0].kind else {
        panic!("a drill step");
    };
    assert_eq!((d.game, d.level, d.items), (5, 1, 15));
    assert_eq!(d.skill.as_ref().map(Skill::as_str), Some("ear.interval.M3"));
}

#[test]
fn a_drill_step_that_does_not_fit_is_refused_naming_the_field() {
    let cases = [
        (r#"{"id":"a","kind":"drill"}"#, "game"),
        (r#"{"id":"a","kind":"drill","game":0}"#, "game"),
        (r#"{"id":"a","kind":"drill","game":11}"#, "game"),
        (r#"{"id":"a","kind":"drill","game":5,"level":0}"#, "level"),
        (r#"{"id":"a","kind":"drill","game":5,"level":33}"#, "level"),
        (r#"{"id":"a","kind":"drill","game":5,"items":0}"#, "items"),
        (r#"{"id":"a","kind":"drill","game":5,"items":65}"#, "items"),
        (
            r#"{"id":"a","kind":"drill","game":5,"items":4,"skill":"ear.interval.M3"}"#,
            "items",
        ),
        (
            r#"{"id":"a","kind":"drill","game":5,"skill":"Not A Skill"}"#,
            "skill",
        ),
        (
            r#"{"id":"a","kind":"drill","game":5,"skill":"svf.nosuchparam"}"#,
            "skill",
        ),
        (r#"{"id":"a","kind":"drill","game":5,"answer":1}"#, "answer"),
        (r#"{"id":"a","kind":"read","game":5}"#, "game"),
    ];
    for (step, field) in cases {
        let err = Lesson::from_json(&lesson_json(step)).unwrap_err();
        assert!(
            err.to_string().contains(field),
            "{step}: the refusal names {field}: {err}"
        );
    }
    // Both kinds of skill fit: a parameter and an ear skill.
    for skill in ["svf.cutoff", "ear.chord.min"] {
        let step = format!(r#"{{"id":"a","kind":"drill","game":2,"items":10,"skill":"{skill}"}}"#);
        Lesson::from_json(&lesson_json(&step)).unwrap();
    }
}

/// `tests/fixtures/lesson/v2/drill.json`: the lesson as this build writes it, pinned (regenerate with
/// `cargo test -p enth-learn --test drill_step -- --ignored regenerate`, then read the diff). It lives under `v2`
/// because a drill step is an `enth.lesson/2` piece; every `v1` fixture keeps its bytes.
fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lesson/v2/drill.json")
}

#[test]
fn the_drill_lesson_is_what_this_build_writes() {
    let text = fs::read_to_string(fixture_path()).unwrap();
    let lesson = Lesson::from_json(&lesson_text()).unwrap();
    assert_eq!(text.trim_end(), lesson.to_json());
    assert_eq!(Lesson::from_json(&text).unwrap(), lesson);
}

#[test]
#[ignore = "rewrites the fixture; run by hand, then read the diff"]
fn regenerate() {
    let lesson = Lesson::from_json(&lesson_text()).unwrap();
    fs::write(fixture_path(), lesson.to_json() + "\n").unwrap();
}

// ---------------------------------------------------------------------------------------------------------------
// The pack says so

#[test]
fn a_pack_with_a_drill_step_declares_lesson_2() {
    let pack = Pack::from_files(&files(&lesson_text(), 2)).unwrap();
    assert_eq!(pack.manifest().requires.lesson, 2);
    assert!(!pack.manifest().to_json().contains("drill"), "no drill key");
    // Under-declared: refused as a pack whose `requires` is wrong.
    let err = Pack::from_files(&files(&lesson_text(), 1)).unwrap_err();
    assert_eq!(err.code(), "bad_field", "{err}");
    assert!(
        err.to_string().contains("says lesson 1, a lesson uses 2"),
        "{err}"
    );
    // A pack without a drill step needs only lesson 1.
    let plain = lesson_at(1, r#"{"id":"intro","kind":"read","text":"text.md"}"#);
    let pack = Pack::from_files(&files(&plain, 1)).unwrap();
    assert_eq!(pack.manifest().requires.lesson, 1);
}

#[test]
fn a_lesson_schema_newer_than_this_build_is_a_requirement_it_names() {
    for newer in [LESSON_VERSION + 1, 0] {
        let err = Pack::from_files(&files(&lesson_text(), newer)).unwrap_err();
        assert!(
            matches!(err, LearnError::Requires { need: Need::Lesson(v), .. } if v == newer),
            "{err}"
        );
        assert_eq!(err.code(), "requires_unmet", "{err}");
    }
    // `requires.drill` is gone: a manifest that says it is refused as an unknown field, as any other.
    let mut text = common::test_manifest(&["ears"]).to_json();
    text = text.replacen(r#""requires":{"#, r#""requires":{"drill":1,"#, 1);
    assert!(
        enth_learn::pack::Manifest::from_json(&text).is_err(),
        "{text}"
    );
}

/// What an R2 or R3 build answers a drill pack (the F1e review's M1). This build cannot be an older one, so the test
/// mirrors their `Requires` exactly (`deny_unknown_fields`, no `drill` key, `LESSON_VERSION` 1) and their check: the
/// manifest decodes, and `requires.lesson = 2` is unmet, which those builds name "needs a newer enth"
/// (`requires_unmet`) before they read a lesson. Under DRL's `requires.drill` the same manifest failed to decode, and
/// the pack was listed as damaged.
#[test]
fn an_r3_build_reads_a_drill_pack_as_needing_a_newer_enth() {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(
        dead_code,
        reason = "the fields are read only by serde: the shape is the test"
    )]
    struct R3Requires {
        lesson: u32,
        #[serde(default)]
        challenge: Option<u32>,
        patch: u32,
        #[serde(default)]
        generator: Option<u16>,
        #[serde(default)]
        scorers: Vec<String>,
    }
    const R3_LESSON_VERSION: u32 = 1;
    let pack = pack();
    let manifest: serde_json::Value = serde_json::from_str(&pack.manifest().to_json()).unwrap();
    let r3: R3Requires = serde_json::from_value(manifest["requires"].clone())
        .expect("an R3 build decodes a drill pack's requires");
    assert!(
        r3.lesson == 0 || r3.lesson > R3_LESSON_VERSION,
        "R3's Requires::unmet gives Need::Lesson({})",
        r3.lesson
    );
    // The same refusal on this build, for a pack that asks more than it reads.
    let err = Pack::from_files(&files(&lesson_text(), LESSON_VERSION + 1)).unwrap_err();
    assert_eq!(err.code(), "requires_unmet");
    assert_eq!(
        err.hint(),
        "this pack needs a newer enth: update enth to open it"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The player

#[test]
fn entering_a_drill_step_starts_its_drill_and_playing_it_to_the_end_passes_it() {
    let mut p = player();
    let on_start = p.start().unwrap();
    assert!(start_drills(&on_start).is_empty(), "the read step first");
    assert_eq!(p.drill_step(), None);
    assert!(matches!(
        p.drill_ended(),
        Err(PlayerError::NotHere {
            call: "drill_ended",
            kind: "read"
        })
    ));
    let entered = p.next().unwrap();
    let drills = start_drills(&entered);
    assert_eq!(drills.len(), 1, "{entered:?}");
    assert_eq!(drills[0].game, 5);
    assert_eq!(p.drill_step().as_ref(), Some(drills[0]));
    // The drill's own score never reaches the player: it ends, and the step passes.
    let ended = p.drill_ended().unwrap();
    assert!(
        ended.iter().any(|e| matches!(e, Effect::Passed { .. })),
        "{ended:?}"
    );
    assert!(
        ended
            .iter()
            .any(|e| matches!(e, Effect::Record(r) if r.progress.done)),
        "{ended:?}"
    );
    // Ending it again changes nothing.
    assert_eq!(p.drill_ended().unwrap(), Vec::<Effect>::new());
    // The next step is the plain drill.
    let next = p.next().unwrap();
    let plain = start_drills(&next);
    assert_eq!(
        (plain.len(), plain[0].game, plain[0].skill.clone()),
        (1, 3, None)
    );
}

#[test]
fn leaving_a_drill_step_unplayed_skips_it_like_any_unpassed_step() {
    let mut p = player();
    p.start().unwrap();
    p.next().unwrap();
    let left = p.next().unwrap();
    assert!(
        !left.iter().any(|e| matches!(e, Effect::Passed { .. })),
        "an unplayed drill is skipped, not passed: {left:?}"
    );
}
