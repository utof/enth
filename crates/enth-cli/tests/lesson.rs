//! `enth lesson show|check` (lane SKL, F2l) over the example pack (`content/lessons/example`, lesson `cutoff`): read
//! from its authoring TOML and from the built `.enthpack` the plugin bundles, which say the same. Each output is
//! deserialized back into its published type (the typed round trip). The snapshots pin the shape and the texts; the
//! check's margins are DSP measurements, masked.
//!
//! Lane EV3 adds `list`, `start`, `hint` and `closest` below (the example pack for a do step, `basics` for rounds): the
//! same typed round trips, snapshots for the shapes, and end-to-end checks that a hint's move passes the step and a
//! closest answer passes the round. The solver's numbers are DSP measurements, so a snapshot masks them and the tests
//! assert on what they mean.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, json, record, round_trip, run};
use enth_cli::schema::{
    ErrorOut, LessonCheckOut, LessonClosestOut, LessonHintOut, LessonListOut, LessonShowOut,
    LessonStartOut,
};
use insta::assert_snapshot;
use serde_json::Value;

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

// ---------------------------------------------------------------------------------------------------------------
// Lane EV3: list, start, hint, closest
// ---------------------------------------------------------------------------------------------------------------

/// The basics pack (16 lessons; its recreate steps are the rounds `hint` and `closest` work on).
fn basics() -> String {
    format!(
        "{}/../../content/lessons/basics",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The folder of packs.
fn packs_dir() -> String {
    format!("{}/../../content/lessons", env!("CARGO_MANIFEST_DIR"))
}

/// A call's JSON with the solver's measurements replaced, so a snapshot pins the shape and the words: a measured
/// move's values and ops, a closest answer's values and the numbers `mask` hides.
fn shape(o: &enth_cli::Output) -> String {
    let mut v: Value = serde_json::from_str(&o.stdout).unwrap();
    if let Some(m) = v.get_mut("measured").filter(|m| m.is_object()) {
        m["ops"] = Value::from("<ops>");
        for mv in m["moves"].as_array_mut().into_iter().flatten() {
            mv["from"] = Value::from("<from>");
            mv["to"] = Value::from("<to>");
        }
    }
    for d in v
        .get_mut("deltas")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        d["from"] = Value::from("<from>");
        d["to"] = Value::from("<to>");
    }
    common::pretty(&common::mask(&v))
}

#[test]
fn list_names_the_packs_lessons_and_steps() {
    let s = Scratch::new("lesson_list");
    let o = run(&["lesson", "list", &example()]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let out: LessonListOut = round_trip(&o);
    assert_eq!(out.schema, "enth.lesson-list/1");
    assert_eq!(out.refused, Vec::new());
    let lesson = &out.packs[0].lessons[0];
    assert_eq!(lesson.key, "example/cutoff");
    let kinds: Vec<(&str, u16)> = lesson
        .steps
        .iter()
        .map(|s| (s.kind.as_str(), s.hints))
        .collect();
    assert_eq!(kinds, [("listen", 0), ("do", 1), ("choose", 0)]);
    settings(&s).bind(|| assert_snapshot!("lesson_list", record(&s, &o, false)));

    // A built pack lists what its authoring layout lists.
    let from_pack: LessonListOut = round_trip(&run(&["lesson", "list", &built()]));
    assert_eq!(from_pack.packs[0].lessons, out.packs[0].lessons);

    // A folder of packs lists each, with the tour the basics pack carries.
    let o = run(&["lesson", "list", &packs_dir()]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let all: LessonListOut = round_trip(&o);
    let ids: Vec<&str> = all.packs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["basics", "example"]);
    assert_eq!(all.packs[0].lessons.len(), 16);
    assert_eq!(all.packs[0].tracks[0].id, "tour");
    assert_eq!(all.refused, Vec::new());
}

#[test]
fn a_folder_lists_what_it_reads_and_names_what_it_refuses() {
    let s = Scratch::new("lesson_list_folder");
    std::fs::copy(built(), s.path("example.enthpack")).unwrap();
    s.write("newer.enthpack", "not a zip");
    let o = run(&["lesson", "list", &s.path("")]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let out: LessonListOut = round_trip(&o);
    assert_eq!(out.packs.len(), 1);
    assert_eq!(out.refused.len(), 1);
    assert!(out.refused[0].path.ends_with("newer.enthpack"));
    assert_ne!(out.refused[0].hint, "");
    // The same pack named outright is an error with the reader's code, like `show`.
    let o = run(&["lesson", "list", &s.path("newer.enthpack")]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!(o.code, 3, "{}", o.stdout);
    assert_eq!(e.error, out.refused[0].code);
    // A folder with no pack, and a path that is not there.
    let empty = Scratch::new("lesson_list_empty");
    let o = run(&["lesson", "list", &empty.path("")]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (3, "not_a_pack"));
    let o = run(&["lesson", "list", &empty.path("nope")]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (5, "file_not_found"));
}

#[test]
fn start_writes_what_a_step_loads() {
    let s = Scratch::new("lesson_start");
    let out = s.path("start.patch.json");
    let o = run(&[
        "lesson",
        "start",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "try",
        "--out",
        &out,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let r: LessonStartOut = round_trip(&o);
    assert_eq!(
        (r.from.as_deref(), r.kind.as_str()),
        (Some("patches/saw.patch.json"), "do")
    );
    assert!(r.challenge.is_none());
    settings(&s).bind(|| assert_snapshot!("lesson_start", record(&s, &o, false)));
    // It is the pack's patch.
    let d = json(&run(&["diff", &out, &patch("saw")]));
    assert_eq!(d["same"], true);

    // A recreate step writes its start and its round; the round scores the start (not yet a pass).
    let (p, c) = (s.path("round.patch.json"), s.path("round.challenge.json"));
    let o = run(&[
        "lesson",
        "start",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        "--out",
        &p,
        "--challenge-out",
        &c,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let r: LessonStartOut = round_trip(&o);
    assert_eq!(
        (r.from.as_deref(), r.kind.as_str()),
        (Some("challenge"), "recreate")
    );
    assert_eq!(r.challenge.as_deref(), Some(c.as_str()));
    let o = run(&["compare", &p, "--challenge", &c]);
    assert_eq!(o.code, 1, "{}", o.stdout);
    assert_eq!(json(&o)["passed"], false);
}

/// The ear questions of a command line: `start --examples-out` writes each sound as a patch and a performance, and
/// `describe` reads them where the player would listen. The measured brightness picks the answer the pack records.
#[test]
fn start_writes_the_sounds_of_an_ear_step_and_describe_answers_the_question() {
    let s = Scratch::new("lesson_start_ears");
    let dir = s.path("ears");
    let o = run(&[
        "lesson",
        "start",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "which-darker",
        "--examples-out",
        &dir,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let r: LessonStartOut = round_trip(&o);
    assert_eq!((r.kind.as_str(), r.patch, r.from), ("choose", None, None));
    let labels: Vec<&str> = r.examples.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["A", "B"]);
    assert_eq!(r.examples[0].phrase, "held");
    settings(&s).bind(|| assert_snapshot!("lesson_start_ears", record(&s, &o, false)));

    // The pack records which option is darker; the numbers agree with it.
    let shown = json(&run(&[
        "lesson",
        "show",
        &basics(),
        "--lesson",
        "cutoff",
        "--reveal",
    ]));
    let step = shown["lessons"][0]["steps"]
        .as_array()
        .and_then(|a| a.iter().find(|st| st["id"] == "which-darker"))
        .cloned()
        .expect("the step exists");
    let darker = usize::try_from(
        step["answer"]
            .as_u64()
            .expect("a choose step has an answer"),
    )
    .expect("an index");
    let centroid = |i: usize| -> f64 {
        let e = &r.examples[i];
        let o = run(&["describe", &e.patch, "--perf", &e.perf]);
        assert_eq!(o.code, 0, "{}", o.stdout);
        let d = json(&o);
        d["spectrum"]["centroid_hz"]
            .as_f64()
            .unwrap_or_else(|| panic!("no centroid in {d}"))
    };
    let (a, b) = (centroid(0), centroid(1));
    let measured_darker = usize::from(b < a);
    assert_eq!(measured_darker, darker, "A {a} Hz, B {b} Hz");
}

#[test]
fn start_says_what_a_step_has_to_write() {
    let s = Scratch::new("lesson_start_nothing");
    // A step with neither flag, and the flag of the wrong kind of step, are usage errors that write nothing.
    let o = run(&[
        "lesson",
        "start",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "which-darker",
    ]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (2, "nothing_to_write"));
    assert!(e.hint.contains("--examples-out"), "{}", e.hint);
    let o = run(&[
        "lesson",
        "start",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "try",
        "--examples-out",
        &s.path("e"),
    ]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (2, "no_examples"));
    assert!(!std::path::Path::new(&s.path("e")).exists());
}

/// A `touched` predicate reads the editor's edit log, which a file has not: `check --from START` reads how far the
/// control moved from the start instead, so a step with one can pass on the command line.
#[test]
fn check_reads_a_touched_predicate_from_the_start_patch() {
    let s = Scratch::new("lesson_check_from");
    let start = s.path("start.patch.json");
    let o = run(&[
        "lesson",
        "start",
        &basics(),
        "--lesson",
        "how-a-lesson-works",
        "--step",
        "try",
        "--out",
        &start,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let witness = format!("{}/patches/saw-master-24.patch.json", basics());
    let pack = basics();
    let check = |extra: &[&str]| {
        let mut args = vec![
            "lesson",
            "check",
            &pack[..],
            "--lesson",
            "how-a-lesson-works",
            "--step",
            "try",
            &witness[..],
        ];
        args.extend(extra);
        let o = run(&args);
        (o.code, json(&o))
    };
    // Without the start the predicate cannot hold, and the note says what to pass.
    let (code, v) = check(&[]);
    assert_eq!((code, v["pass"].clone()), (1, Value::Bool(false)));
    assert!(
        v["note"].as_str().is_some_and(|n| n.contains("--from")),
        "{v}"
    );
    // With it, the witness moved master.level by more than the predicate asks.
    let (code, v) = check(&["--from", &start]);
    assert_eq!((code, v["pass"].clone()), (0, Value::Bool(true)), "{v}");
    // A patch that did not move from the start has not touched it.
    let o = run(&[
        "lesson",
        "check",
        &basics(),
        "--lesson",
        "how-a-lesson-works",
        "--step",
        "try",
        &start,
        "--from",
        &start,
    ]);
    assert_eq!(o.code, 1, "{}", o.stdout);
    assert_eq!(json(&o)["predicates"][0]["pass"], false);
}

#[test]
fn start_refuses_what_has_no_start_or_no_round() {
    let s = Scratch::new("lesson_start_errors");
    let out = s.path("x.patch.json");
    // The ear question carries the patch over; the hint names the step it comes from.
    let o = run(&[
        "lesson",
        "start",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "which",
        "--out",
        &out,
    ]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (2, "carried_start"));
    assert!(e.hint.contains("step 2 (try)"), "{}", e.hint);
    // A do step has no round to write.
    let o = run(&[
        "lesson",
        "start",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "try",
        "--out",
        &out,
        "--challenge-out",
        &s.path("c.json"),
    ]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (2, "no_round"));
    assert!(
        !std::path::Path::new(&out).exists(),
        "nothing is written on a refusal"
    );
}

#[test]
fn an_authored_hint_needs_no_patch_and_the_ladder_ends() {
    let s = Scratch::new("lesson_hint_authored");
    let o = run(&[
        "lesson",
        "hint",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "try",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let h: LessonHintOut = round_trip(&o);
    assert_eq!((h.n, h.authored, h.source.as_str()), (1, 1, "authored"));
    assert!(h.more, "a measured hint follows the authored one");
    assert_eq!(
        h.text,
        "The cutoff knob is on the filter. Turn it to the left."
    );
    assert!(h.measured.is_none() && h.coach.is_none() && h.patch.is_none());
    settings(&s).bind(|| assert_snapshot!("lesson_hint_authored", record(&s, &o, false)));
}

#[test]
fn a_measured_hint_is_a_move_that_passes_the_step() {
    let s = Scratch::new("lesson_hint_measured");
    let start = patch("saw");
    // The second rung is measured on the patch.
    let o = run(&[
        "lesson",
        "hint",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "try",
        "--n",
        "2",
        &start,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let h: LessonHintOut = round_trip(&o);
    assert_eq!((h.n, h.source.as_str(), h.more), (2, "measured", false));
    let m = h.measured.as_ref().expect("a measured move");
    assert_eq!(m.moves[0].param, "filter1.cutoff");
    assert_eq!(m.moves[0].direction, "down");
    assert!(m.pass_all && m.blocked.is_empty());
    assert_eq!(h.text, "Try turning Cutoff (filter1.cutoff) down.");
    settings(&s).bind(|| assert_snapshot!("lesson_hint_measured", shape(&o)));
    // Applying the move's ops is what the step asks: `lesson check` passes.
    let ops = serde_json::to_string(&m.ops).unwrap();
    let moved = s.path("moved.patch.json");
    let a = run(&["apply", &start, "--ops", &ops, "--out", &moved]);
    assert_eq!(a.code, 0, "{}", a.stdout);
    let c = run(&[
        "lesson",
        "check",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "try",
        &moved,
    ]);
    assert_eq!(c.code, 0, "{}", c.stdout);
    // The answer is the same for any number of workers.
    let again = run(&[
        "lesson",
        "hint",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "try",
        "--n",
        "2",
        &start,
        "--workers",
        "1",
    ]);
    assert_eq!(again.stdout, o.stdout);
}

#[test]
fn a_hint_says_what_it_needs_and_where_the_ladder_ends() {
    // (exit, code) of a `lesson hint` call on `pack`'s lesson `cutoff`, with a hint in every refusal.
    let refused = |pack: &str, step: &str, rest: &[&str]| {
        let mut a = vec!["lesson", "hint", pack, "--lesson", "cutoff", "--step", step];
        a.extend_from_slice(rest);
        let o = run(&a);
        let e: ErrorOut = round_trip(&o);
        assert_ne!(e.hint, "", "{}", o.stdout);
        (o.code, e.error)
    };
    let (ex, saw) = (example(), patch("saw"));
    // The measured rung needs a patch; a rung past the end, or 0, is refused.
    assert_eq!(
        refused(&ex, "try", &["--n", "2"]),
        (2, "patch_required".into())
    );
    assert_eq!(
        refused(&ex, "try", &["--n", "3", &saw]),
        (2, "no_more_hints".into())
    );
    assert_eq!(refused(&ex, "try", &["--n", "0"]), (2, "bad_n".into()));
    assert_eq!(refused(&ex, "tyr", &[]), (2, "unknown_step".into()));
    // A listen step has nothing to get wrong; an ear question with no hints has none to give.
    assert_eq!(refused(&ex, "hear", &[]), (2, "not_hintable".into()));
    assert_eq!(refused(&ex, "which", &[]), (2, "no_more_hints".into()));
    // An ear question with a hint has the authored one only.
    let o = run(&[
        "lesson",
        "hint",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "which-darker",
        "--n",
        "1",
    ]);
    let h: LessonHintOut = round_trip(&o);
    assert_eq!((h.kind.as_str(), h.more), ("choose", false));
    assert_eq!(
        refused(&basics(), "which-darker", &["--n", "2"]),
        (2, "no_more_hints".into())
    );
}

#[test]
fn a_rounds_hints_after_the_authored_ones_are_the_coachs_four_levels() {
    let s = Scratch::new("lesson_hint_coach");
    let (p, c) = (s.path("round.patch.json"), s.path("round.challenge.json"));
    let st = run(&[
        "lesson",
        "start",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        "--out",
        &p,
        "--challenge-out",
        &c,
    ]);
    assert_eq!(st.code, 0, "{}", st.stdout);
    let o = run(&[
        "lesson",
        "hint",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        "--n",
        "3",
        &p,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let h: LessonHintOut = round_trip(&o);
    assert_eq!((h.source.as_str(), h.authored, h.n), ("coach", 2, 3));
    let coach = h.coach.as_ref().expect("the coach's level 1");
    assert_eq!((coach.level, coach.levels), (1, 4));
    assert_eq!(coach.aspect.as_deref(), Some("brightness"));
    assert!(h.next.contains("--n 4"), "{}", h.next);
    assert!(
        !coach.hint.contains("--level"),
        "the lesson's advice is --n: {}",
        coach.hint
    );
    settings(&s).bind(|| assert_snapshot!("lesson_hint_coach", shape(&o)));
    // Level 4 (--n 6) is the last rung.
    let last = run(&[
        "lesson",
        "hint",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        "--n",
        "6",
        &p,
    ]);
    let h: LessonHintOut = round_trip(&last);
    assert_eq!((h.more, h.coach.map(|c| c.level)), (false, Some(4)));
    let past = run(&[
        "lesson",
        "hint",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        "--n",
        "7",
        &p,
    ]);
    let e: ErrorOut = round_trip(&past);
    assert_eq!((past.code, e.error.as_str()), (2, "no_more_hints"));
}

#[test]
fn closest_finds_a_passing_patch_next_door_and_is_the_same_for_any_workers() {
    let s = Scratch::new("lesson_closest");
    let (p, c) = (s.path("round.patch.json"), s.path("round.challenge.json"));
    let st = run(&[
        "lesson",
        "start",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        "--out",
        &p,
        "--challenge-out",
        &c,
    ]);
    assert_eq!(st.code, 0, "{}", st.stdout);
    let answer = s.path("answer.patch.json");
    let o = run(&[
        "lesson",
        "closest",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        &p,
        "--out",
        &answer,
        "--workers",
        "1",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let r: LessonClosestOut = round_trip(&o);
    assert_eq!(r.free, ["filter1.cutoff"], "the step's editable set");
    assert!(
        r.passed && r.start_percent < 90.0 && r.percent >= 90.0,
        "{r:?}"
    );
    assert!(r.renders <= r.budget && r.first_pass_renders.is_some());
    assert_eq!(r.deltas.len(), 1);
    assert!(
        r.deltas[0].moved && r.deltas[0].norm < 0.0,
        "the start is brighter than the target"
    );
    settings(&s).bind(|| assert_snapshot!("lesson_closest", shape(&o)));
    // The answer passes the round (the scorer `enth compare` runs).
    let c = run(&["compare", &answer, "--challenge", &c]);
    assert_eq!(c.code, 0, "{}", c.stdout);
    // Another number of workers gives the same answer.
    let again = run(&[
        "lesson",
        "closest",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        &p,
        "--out",
        &answer,
        "--workers",
        "3",
    ]);
    assert_eq!(again.stdout, o.stdout);
    // A budget of one render scores the start alone.
    let one = run(&[
        "lesson",
        "closest",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        &p,
        "--budget",
        "1",
    ]);
    let r: LessonClosestOut = round_trip(&one);
    assert_eq!((r.renders, r.passed), (1, false));
    // The answer, closest again, is already there.
    let done = run(&[
        "lesson",
        "closest",
        &basics(),
        "--lesson",
        "cutoff",
        "--step",
        "round-1",
        &answer,
    ]);
    let r: LessonClosestOut = round_trip(&done);
    assert!(
        r.passed && r.stop == "start_passes" && r.renders == 1,
        "{r:?}"
    );
}

#[test]
fn closest_refuses_what_is_not_a_round_or_not_a_free_set() {
    let s = Scratch::new("lesson_closest_errors");
    let p = s.path("p.patch.json");
    let st = run(&[
        "lesson",
        "start",
        &example(),
        "--lesson",
        "cutoff",
        "--step",
        "try",
        "--out",
        &p,
    ]);
    assert_eq!(st.code, 0, "{}", st.stdout);
    let code = |args: &[&str]| {
        let o = run(args);
        let e: ErrorOut = round_trip(&o);
        assert_ne!(e.hint, "", "{}", o.stdout);
        (o.code, e.error, e.did_you_mean)
    };
    let b = basics();
    assert_eq!(
        code(&[
            "lesson",
            "closest",
            &example(),
            "--lesson",
            "cutoff",
            "--step",
            "try",
            &p
        ])
        .1,
        "not_a_round"
    );
    let r1 = |extra: &[&str]| {
        let mut a = vec![
            "lesson",
            "closest",
            b.as_str(),
            "--lesson",
            "cutoff",
            "--step",
            "round-1",
            p.as_str(),
        ];
        a.extend_from_slice(extra);
        code(&a)
    };
    assert_eq!(r1(&["--budget", "0"]).1, "bad_budget");
    assert_eq!(r1(&["--workers", "0"]).1, "bad_workers");
    let (exit, error, did_you_mean) = r1(&["--free", "filter1.cutof"]);
    assert_eq!((exit, error.as_str()), (3, "unknown_param"));
    assert_eq!(
        did_you_mean.first().map(String::as_str),
        Some("filter1.cutoff")
    );
}
