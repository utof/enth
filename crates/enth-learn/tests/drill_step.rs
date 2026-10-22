//! Lane DRL's lesson step (docs/learn.md, "Ear skills, bursts and drills"): `StepKind::Drill`, additive to
//! `enth.lesson/1` and declared in a pack's `requires.drill`. Its JSON and TOML forms, its fixture, what a pack must
//! say about it, and how the player runs it: `Effect::StartDrill` on entering, a pass on `drill_ended` whatever the
//! score, and `NotHere` anywhere else.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads a fixture file from the repository; the library does no I/O"
)]

mod common;

use std::fs;

use enth_learn::drill::{DRILL_VERSION, DrillStep};
use enth_learn::pack::engine_is_newer;
use enth_learn::review::Skill;
use enth_learn::{Effect, LearnError, Lesson, Pack, Player, PlayerError, StepKind};

fn lesson_json(steps: &str) -> String {
    format!(
        r#"{{"schema":"enth.lesson/1","id":"ears","title":"Thirds","tier":"basic","objective":"Hear a third.","teaches":["svf.cutoff"],"steps":[{steps}]}}"#
    )
}

/// A read step, a drill that bursts on a skill, and a plain drill: the forms of the step.
fn lesson_text() -> String {
    lesson_json(
        r#"{"id":"intro","kind":"read","text":"text.md"},
        {"id":"thirds","kind":"drill","text":"text.md","game":5,"level":3,"items":15,"skill":"ear.interval.M3"},
        {"id":"check","kind":"drill","game":3}"#,
    )
}

/// A manifest for the pack of `ears`, declaring `drill`.
fn files(lesson: &str, drill: Option<u32>) -> std::collections::BTreeMap<String, Vec<u8>> {
    let files = common::test_files(&[("ears", lesson)]);
    let mut manifest = common::test_manifest(&["ears"]);
    manifest.requires.drill = drill;
    common::seal(&manifest, files)
}

fn pack() -> Pack {
    Pack::from_files(&files(&lesson_text(), Some(DRILL_VERSION))).unwrap()
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
    assert_eq!(Lesson::from_json(&text).unwrap(), lesson);
}

#[test]
fn a_drill_step_reads_from_toml_too() {
    let toml = r#"
schema = "enth.lesson/1"
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

/// `tests/fixtures/lesson/v1/drill.json`: the lesson as this build writes it, pinned (regenerate with
/// `cargo test -p enth-learn --test drill_step -- --ignored regenerate`, then read the diff).
fn fixture_path() -> std::path::PathBuf {
    common::fixture_dir("lesson").join("drill.json")
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
fn a_pack_with_a_drill_step_declares_the_drill_schema() {
    Pack::from_files(&files(&lesson_text(), Some(DRILL_VERSION))).unwrap();
    let err = Pack::from_files(&files(&lesson_text(), None)).unwrap_err();
    assert!(err.to_string().contains("drill"), "{err}");
    // A pack without a drill step needs say nothing (and says nothing in its manifest).
    let plain = lesson_json(r#"{"id":"intro","kind":"read","text":"text.md"}"#);
    let pack = Pack::from_files(&files(&plain, None)).unwrap();
    assert!(!pack.manifest().to_json().contains("drill"));
    // The declared schema is written, and read back.
    let declared = pack.manifest().to_json();
    let with = Pack::from_files(&files(&lesson_text(), Some(DRILL_VERSION))).unwrap();
    assert!(
        with.manifest().to_json().contains(r#""drill":1"#),
        "{declared}"
    );
    assert_eq!(with.manifest().requires.drill, Some(DRILL_VERSION));
}

#[test]
fn a_drill_schema_newer_than_this_build_is_a_requirement_it_names() {
    for newer in [DRILL_VERSION + 1, 0] {
        let err = Pack::from_files(&files(&lesson_text(), Some(newer))).unwrap_err();
        assert!(matches!(err, LearnError::Requires { .. }), "{err}");
        assert!(err.to_string().contains("drill schema"), "{err}");
    }
}

#[test]
fn an_older_build_reads_a_drill_step_as_a_kind_it_lacks_and_asks_for_an_update() {
    // This build cannot be an older one; a step kind it lacks stands in for the Drill step an older build meets, and
    // takes the same path: a decode failure that becomes "needs a newer enth" when the pack's engine stamp is newer.
    let lesson = lesson_json(
        r#"{"id":"intro","kind":"read","text":"text.md"},{"id":"a","kind":"rhythm","text":"text.md","game":7}"#,
    );
    let files = common::test_files(&[("ears", &lesson)]);
    let mut manifest = common::test_manifest(&["ears"]);
    "9.0.0".clone_into(&mut manifest.engine);
    assert!(engine_is_newer(&manifest.engine));
    let err = Pack::from_files(&common::seal(&manifest, files)).unwrap_err();
    assert_eq!(err.code(), "needs_newer_enth", "{err}");
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
