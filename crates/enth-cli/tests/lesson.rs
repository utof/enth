//! `enth lesson show|check` (lane SKL, F2l) over the example pack (`content/lessons/example`, lesson `cutoff`): read
//! from its authoring TOML and from the built `.enthpack` the plugin bundles, which say the same. Each output is
//! deserialized back into its published type (the typed round trip). The snapshots pin the shape and the texts; the
//! check's margins are DSP measurements, masked.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, record, round_trip, run};
use enth_cli::schema::{ErrorOut, LessonCheckOut, LessonShowOut};
use insta::assert_snapshot;

/// The example pack in its authoring layout.
fn example() -> String {
    format!(
        "{}/../../content/lessons/example",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The example pack as the plugin bundles it.
fn built() -> String {
    format!(
        "{}/../enth-plugin/packs/example.enthpack",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn patch(name: &str) -> String {
    format!("{}/patches/{name}.patch.json", example())
}

/// Snapshot settings: the scratch directory, the repository's path and the engine version filtered.
fn settings(s: &Scratch) -> insta::Settings {
    let mut settings = s.settings();
    let root = format!("{}/../..", env!("CARGO_MANIFEST_DIR"));
    settings.add_filter(&regex_escape(&root), "[repo]");
    settings.add_filter(r#""engine": "[0-9.]+""#, r#""engine": "[engine]""#);
    settings
}

/// `text` with the regex metacharacters escaped (insta's filters are regexes).
fn regex_escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if r"\.+*?()|[]{}^$#&-~".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

#[test]
fn show_prints_the_steps_without_answers() {
    let s = Scratch::new("lesson_show");
    let o = run(&["lesson", "show", &example(), "--lesson", "cutoff"]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let out: LessonShowOut = round_trip(&o);
    assert_eq!(out.schema, "enth.lesson-show/1");
    assert_eq!(out.pack.id, "example");
    let lesson = &out.lessons[0];
    assert_eq!(lesson.key, "example/cutoff");
    let kinds: Vec<&str> = lesson.steps.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(kinds, ["listen", "do", "choose"]);
    assert_eq!(lesson.steps[1].locked, ["osc1.*"]);
    assert!(lesson.steps[1].check.is_some());
    assert!(
        lesson
            .steps
            .iter()
            .all(|s| s.answer.is_none() && s.witness.is_none()),
        "no answers without --reveal"
    );
    settings(&s).bind(|| assert_snapshot!("lesson_show", record(&s, &o, false)));
}

#[test]
fn the_built_pack_says_what_the_authoring_layout_says_and_reveal_adds_the_answers() {
    let from_toml: LessonShowOut = round_trip(&run(&["lesson", "show", &example()]));
    let from_pack: LessonShowOut = round_trip(&run(&["lesson", "show", &built()]));
    assert_eq!(from_toml.lessons, from_pack.lessons);
    assert_eq!(from_toml.pack.version, from_pack.pack.version);
    let revealed: LessonShowOut = round_trip(&run(&["lesson", "show", &built(), "--reveal"]));
    let steps = &revealed.lessons[0].steps;
    assert_eq!(steps[1].witness.as_deref(), Some("patches/dark.patch.json"));
    assert_eq!(steps[2].answer, Some(1));
}

#[test]
fn check_runs_a_do_steps_sound_check_on_a_patch() {
    let s = Scratch::new("lesson_check");
    // The witness passes (exit 0), by number or by id.
    let o = run(&[
        "lesson",
        "check",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "2",
        &patch("dark"),
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let pass: LessonCheckOut = round_trip(&o);
    assert!(pass.pass);
    assert_eq!((pass.step.as_str(), pass.number), ("try", 2));
    assert_eq!(pass.sounds.len(), 1);
    assert!(
        pass.sounds[0].margin.unwrap() > 1e-3,
        "a margin, as lesson-check asks"
    );
    settings(&s).bind(|| assert_snapshot!("lesson_check_pass", record(&s, &o, true)));
    // The start fails (exit 1).
    let o = run(&[
        "lesson",
        "check",
        &built(),
        "--lesson",
        "cutoff",
        "--step",
        "try",
        &patch("saw"),
    ]);
    assert_eq!(o.code, 1, "{}", o.stdout);
    let fail: LessonCheckOut = round_trip(&o);
    assert!(!fail.pass && !fail.sounds[0].pass);
    settings(&s).bind(|| assert_snapshot!("lesson_check_fail", record(&s, &o, true)));
}

#[test]
fn refusals_name_what_exists() {
    let s = Scratch::new("lesson_errors");
    let o = run(&["lesson", "show", &example(), "--lesson", "cutof"]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (2, "unknown_lesson"));
    assert_eq!(e.did_you_mean, ["cutoff"]);
    let o = run(&[
        "lesson",
        "check",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "tyr",
        &patch("dark"),
    ]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (2, "unknown_step"));
    assert_eq!(e.did_you_mean, ["try"]);
    let o = run(&[
        "lesson",
        "check",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "1",
        &patch("dark"),
    ]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (2, "not_checkable"));
    settings(&s).bind(|| assert_snapshot!("err_lesson_not_checkable", record(&s, &o, false)));
    let o = run(&["lesson", "show", &s.path("")]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (3, "not_a_pack"));
    let bad = s.write("bad.enthpack", "not a zip");
    let o = run(&["lesson", "show", &bad]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!(o.code, 3, "{}", o.stdout);
    assert_ne!(e.hint, "");
}
