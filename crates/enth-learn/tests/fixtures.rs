//! The persisted forms of lane LM's types, pinned: `enth.pack/1`, `enth.lesson/1`, `enth.progress/1` and the
//! `LessonRef` the session saves.
//!
//! Fixtures live in `tests/fixtures/<kind>/v1/`. They are the bytes this build writes for the cases below, and they
//! must read back to those cases: a change to a format fails here. When a format changes on purpose (a new
//! version), add `v<N+1>/` fixtures, keep the old ones as migration inputs, and regenerate the current ones with
//! `cargo test -p enth-learn --test fixtures -- --ignored regenerate`, then read the diff. `progress/v1/newer.json`
//! is an input only (a file from a newer build), never regenerated.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture files and the regenerate test writes them; the library does no I/O"
)]

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;

use enth_learn::pack::{ContentId, PackVersion, StepId, sha256};
use enth_learn::player::StepRecord;
use enth_learn::reader::write_archive;
use enth_learn::{Lesson, LessonRef, Manifest, Pack, Progress, StepKind, StepProgress};

/// A lesson that uses every step kind and every optional field once.
const EVERY_FIELD: &str = r#"{"schema":"enth.lesson/1","id":"every-field","title":"Every field","tier":"advanced",
"objective":"Exercise every field of enth.lesson/1.","teaches":["svf.cutoff","osc.wave","macro.value","voice.glide"],
"steps":[
 {"id":"name","kind":"read","text":"text.md","view":{"layout":"focus","highlight":["filter1.cutoff","macro1"],"hide":["noise1.*","voice.*"]}},
 {"id":"hear","kind":"listen","text":"text.md","examples":[
   {"label":"Open","patch":"patches/saw.patch.json"},
   {"label":"Low","patch":"patches/dark.patch.json","phrase":2,"transpose":-12,"level_match":false,"seed":7,
    "curve":{"source":"modwheel","points":[{"ms":0,"pct":0},{"ms":1500,"pct":100}]}}]},
 {"id":"try","kind":"do","text":"text.md","hints":["Turn the cutoff down.","Below 1 kHz."],
  "view":{"start":"patches/saw.patch.json","editable":["filter1.cutoff"],"locked":["osc1.*","lfo1 -> filter1.cutoff","macro2"]},
  "check":{"kind":"all","checks":[
    {"kind":"patch","predicates":[
      {"kind":"value","knob":"filter1.cutoff","cmp":"<","value":"1kHz"},
      {"kind":"within","knob":"master.level","lo":"-24dB","hi":0},
      {"kind":"route","route":"macro1 -> filter1.cutoff","cmp":">=","depth":"0%"},
      {"kind":"has","module_kind":"svf"},
      {"kind":"enabled","module":"noise1","enabled":false},
      {"kind":"touched","knob":"filter1.cutoff","min_span_pct":10},
      {"kind":"value","knob":"macro1","cmp":"==","value":0}]},
    {"kind":"sound","phrase":1,"expect":["brightness<500Hz"],"macros":[{"macro":"macro1","value":"0"}]}]},
  "witness":"patches/dark.patch.json"},
 {"id":"which","kind":"choose","text":"text.md","options":[
   {"label":"A","patch":"patches/saw.patch.json"},{"label":"B","patch":"patches/dark.patch.json"}],"answer":1},
 {"id":"round","kind":"recreate","text":"text.md","view":{"highlight":["osc1.wave"]},"witness":"patches/dark.patch.json",
  "challenge":{"schema":"enth.challenge/1","target":{"kind":"seed","spec":{"generator":1,"seed":42,"difficulty":0,"amounts":[null,null,null,null,null,null],"musical":100,"template":0}},"answer":"patch","phrase":1,"start":{"kind":"template","template":0},"constraints":{},"assists":{"live_meter":false,"dim_unused":false,"hint_budget":null},"scoring":{"sound":"sound/1","structure":"structure/1","combined":"combined/1","time":"time/1","sound_weight":50,"pass_mark":90},"meta":{"title":"","tags":[]}}}
]}"#;

fn read(kind: &str, name: &str) -> String {
    let path = common::fixture_dir(kind).join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .trim_end()
        .to_owned()
}

fn write(kind: &str, name: &str, text: &str) {
    let d = common::fixture_dir(kind);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(name), format!("{text}\n")).unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Cases

fn example_manifest() -> Manifest {
    Pack::from_files(&common::example_files())
        .unwrap()
        .manifest()
        .clone()
}

fn example_lesson() -> Lesson {
    Pack::from_files(&common::example_files())
        .unwrap()
        .lesson(&"cutoff".parse().unwrap())
        .unwrap()
        .clone()
}

fn every_field() -> Lesson {
    Lesson::from_json(EVERY_FIELD).unwrap()
}

/// A Recreate step embedding a Challenge v2 draw with `editable` and `locked` (F1l review, swap item 10).
fn recreate_v2() -> Lesson {
    Lesson::from_json(&common::recreate_v2_lesson("recreate-v2")).unwrap()
}

fn lesson_ref() -> LessonRef {
    LessonRef {
        pack: "space-and-dynamics".parse().unwrap(),
        version: PackVersion::new(1, 2, 0),
        lesson: "compressor-level".parse().unwrap(),
        step: 3,
        content: ContentId([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]),
    }
}

fn progress() -> Progress {
    let mut p = Progress::new();
    let step = |done, clean, best| StepProgress {
        done,
        clean,
        best,
        extra: BTreeMap::new(),
    };
    for (lesson, id, s) in [
        ("example/cutoff", "hear", step(true, true, None)),
        ("example/cutoff", "try", step(true, false, None)),
        ("basics/waveforms", "do-1", step(true, true, Some(96))),
    ] {
        p.record(&StepRecord {
            lesson: lesson.parse().unwrap(),
            step: StepId::new(id).unwrap(),
            progress: s,
        });
    }
    p
}

// ---------------------------------------------------------------------------------------------------------------
// The checks

#[test]
fn pack_v1_is_what_this_build_writes() {
    let m = example_manifest();
    assert_eq!(read("pack", "pack.json"), m.to_json());
    assert_eq!(Manifest::from_json(&read("pack", "pack.json")).unwrap(), m);
}

#[test]
fn the_example_archive_is_reproducible_to_the_byte() {
    // Each build pins the hashes of the packs it bundles, so the writer must give the same bytes on every platform.
    let digest = hex(&sha256(&write_archive(&common::example_files())));
    assert_eq!(read("pack", "example.enthpack.sha256"), digest);
}

#[test]
fn lesson_v1_is_what_this_build_writes() {
    for (name, lesson) in [
        ("cutoff.json", example_lesson()),
        ("every-field.json", every_field()),
        ("recreate-v2.json", recreate_v2()),
    ] {
        assert_eq!(read("lesson", name), lesson.to_json(), "{name}");
        assert_eq!(
            Lesson::from_json(&read("lesson", name)).unwrap(),
            lesson,
            "{name}"
        );
    }
}

#[test]
fn every_field_holds_every_kind_and_defaults_are_not_written() {
    let l = every_field();
    let kinds: Vec<&str> = l.steps.iter().map(|s| s.kind.name()).collect();
    assert_eq!(kinds, ["read", "listen", "do", "choose", "recreate"]);
    let StepKind::Listen { examples } = &l.steps[1].kind else {
        panic!()
    };
    assert!(examples[0].level_match && examples[0].phrase.0 == 1);
    let json = l.to_json();
    assert!(!json.contains(r#""level_match":true"#) && !json.contains(r#""phrase":1,"transpose""#));
    assert!(json.contains(r#""hi":"0""#), "a number reads as its text");
}

/// The v2 challenge a Recreate step embeds keeps its version and its constraints through the lesson's bytes.
#[test]
fn a_recreate_step_embeds_a_v2_challenge_with_its_constraints() {
    let l = recreate_v2();
    let StepKind::Recreate { challenge, .. } = &l.steps[0].kind else {
        panic!("a recreate step")
    };
    assert_eq!(challenge.version(), 2);
    assert_eq!(**challenge, common::draw_challenge());
    let json = l.to_json();
    assert!(
        json.contains(r#""challenge":{"schema":"enth.challenge/2","target":{"kind":"draw""#),
        "{json}"
    );
    assert!(json.contains(
        r#""constraints":{"editable":["filter1.cutoff"],"locked":["osc1.*","filter1.resonance"]}"#
    ));
}

#[test]
fn lesson_ref_v1_is_what_this_build_writes_and_stays_small() {
    let r = lesson_ref();
    let text = serde_json::to_string(&r).unwrap();
    assert_eq!(read("lesson-ref", "ref.json"), text);
    assert_eq!(serde_json::from_str::<LessonRef>(&text).unwrap(), r);
    // The saved state's budget (the `enth` field is 1 MiB): the reference is tiny even with the longest ids.
    let longest = LessonRef {
        pack: "p".repeat(32).parse().unwrap(),
        lesson: "l".repeat(48).parse().unwrap(),
        step: u16::MAX,
        ..r
    };
    assert!(serde_json::to_string(&longest).unwrap().len() < 200);
    // Saved state is read tolerantly: a newer build's extra field is ignored.
    let newer = text.replace(r#""step":3"#, r#""step":3,"future":true"#);
    assert_eq!(
        serde_json::from_str::<LessonRef>(&newer).unwrap(),
        lesson_ref()
    );
}

#[test]
fn progress_v1_is_what_this_build_writes() {
    let p = progress();
    assert_eq!(read("progress", "progress.json"), p.to_json());
    assert_eq!(
        Progress::from_json(&read("progress", "progress.json")).unwrap(),
        p
    );
}

#[test]
fn a_newer_progress_file_is_read_leniently_and_kept_whole() {
    let newer = read("progress", "newer.json");
    let p = Progress::from_json(&newer).unwrap();
    let key = "example/cutoff".parse().unwrap();
    assert!(p.step(&key, &StepId::new("try").unwrap()).unwrap().done);
    assert!(
        p.review.skills.is_empty() && p.review.extra.contains_key("boxes"),
        "fields this build does not know are kept, inside `review` (lane REV's field) too"
    );
    assert_eq!(read("progress", "newer.rewritten.json"), p.to_json());
    // Merged and written by this build, nothing of the newer file is lost.
    let mut merged = progress();
    merged.merge(&p);
    let back = Progress::from_json(&merged.to_json()).unwrap();
    assert_eq!(back.extra, p.extra);
    assert_eq!(back.review, p.review);
    assert!(back.unread.contains_key("Example/Odd"));
    // Where this build can read an entry the newer file could not, the readable one wins.
    assert!(!back.unread.contains_key("basics/waveforms"));
    assert!(back.lesson(&"basics/waveforms".parse().unwrap()).is_some());
}

#[cfg(feature = "toml")]
#[test]
fn the_toml_and_json_readers_agree() {
    let dir = common::example_dir();
    let toml_text = fs::read_to_string(dir.join("lessons/cutoff/lesson.toml")).unwrap();
    assert_eq!(
        Lesson::from_toml(&toml_text).unwrap(),
        Lesson::from_json(&common::toml_as_json(&toml_text)).unwrap()
    );
    let pack_text = fs::read_to_string(dir.join("pack.toml")).unwrap();
    assert_eq!(
        Manifest::from_toml(&pack_text).unwrap(),
        common::example_manifest()
    );
    let with_files = format!(
        "{pack_text}\n[[files]]\npath = \"CREDITS.md\"\nsize = 1\nsha256 = \"{}\"\n",
        "0".repeat(64)
    );
    assert_eq!(
        Manifest::from_toml(&with_files).unwrap_err().code(),
        "bad_field"
    );
}

#[test]
#[ignore = "writes the fixtures: run on purpose and read the diff"]
fn regenerate() {
    write("pack", "pack.json", &example_manifest().to_json());
    write(
        "pack",
        "example.enthpack.sha256",
        &hex(&sha256(&write_archive(&common::example_files()))),
    );
    write("lesson", "cutoff.json", &example_lesson().to_json());
    write("lesson", "every-field.json", &every_field().to_json());
    write("lesson", "recreate-v2.json", &recreate_v2().to_json());
    write(
        "lesson-ref",
        "ref.json",
        &serde_json::to_string(&lesson_ref()).unwrap(),
    );
    write("progress", "progress.json", &progress().to_json());
    let newer = Progress::from_json(&read("progress", "newer.json")).unwrap();
    write("progress", "newer.rewritten.json", &newer.to_json());
}
