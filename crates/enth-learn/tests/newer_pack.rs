//! Lane PLY, the F1l review's should-fix 6: a pack made with a newer engine that fails on something this build lacks
//! (a module kind, a parameter key, a phrase, a newer patch or inline patch schema) is refused as "needs a newer
//! enth" (`needs_newer_enth`, naming the pack and keeping the refusal it met), never as a damaged pack; the same pack
//! stamped with this build's engine keeps its plain refusal, and integrity failures are never renamed.

mod common;

use std::collections::BTreeMap;

use enth_learn::pack::engine_is_newer;
use enth_learn::{LearnError, Pack};

const NEWER: &str = "9.0.0";

fn read_step(extra: &str) -> String {
    format!(
        r#"{{"schema":"enth.lesson/1","id":"l","title":"L","tier":"basic","objective":"O.","teaches":["svf.cutoff"],
        "steps":[{{"id":"a","kind":"read","text":"text.md"}}{extra}]}}"#
    )
}

fn do_step(check: &str) -> String {
    read_step(&format!(
        r#",{{"id":"b","kind":"do","view":{{"start":"patches/saw.patch.json"}},"check":{check},"witness":"patches/dark.patch.json"}}"#
    ))
}

/// A sealed pack of one lesson, with `engine` stamped, and `patches` replaced or added.
fn pack(engine: &str, lesson: &str, patches: &[(&str, String)]) -> BTreeMap<String, Vec<u8>> {
    let mut files = common::test_files(&[("l", lesson)]);
    for (name, text) in patches {
        files.insert(format!("patches/{name}"), text.clone().into_bytes());
    }
    let mut manifest = common::test_manifest(&["l"]);
    engine.clone_into(&mut manifest.engine);
    common::seal(&manifest, files)
}

fn refusal(files: &BTreeMap<String, Vec<u8>>) -> LearnError {
    Pack::from_files(files).unwrap_err()
}

/// The saw patch with its schema raised (a newer patch file).
fn newer_patch() -> String {
    common::example_patch("saw.patch.json").replacen(r#""schema":1"#, r#""schema":2"#, 1)
}

/// What a case is: its name, the lesson, the patches it replaces, and the code this build gives it.
type Case = (
    &'static str,
    String,
    Vec<(&'static str, String)>,
    &'static str,
);

/// The content a newer build might ship, and the code this build gives it when it was made with its own engine.
fn cases() -> Vec<Case> {
    let challenge_with_inline = format!(
        r#"{{"schema":"enth.challenge/2","target":{{"kind":"patch","patch":{dark}}},"answer":"patch","phrase":1,"start":{{"kind":"patch","patch":{newer}}},"constraints":{{}},"assists":{{"live_meter":false,"dim_unused":false,"hint_budget":null}},"scoring":{{"sound":"sound/1","structure":"structure/1","combined":"combined/1","time":"time/1","sound_weight":50,"pass_mark":90}},"meta":{{"title":"","tags":[]}}}}"#,
        dark = common::example_patch("dark.patch.json").trim(),
        newer = newer_patch().trim(),
    );
    vec![
        (
            "a module kind",
            do_step(r#"{"kind":"patch","predicates":["has granular"]}"#),
            vec![],
            "bad_lesson",
        ),
        (
            "a parameter key",
            do_step(
                r#"{"kind":"patch","predicates":[{"kind":"value","knob":"filter1.formant","cmp":"<","value":"1"}]}"#,
            ),
            vec![],
            "bad_lesson",
        ),
        (
            "a phrase",
            do_step(r#"{"kind":"sound","phrase":99,"expect":["brightness<500Hz"]}"#),
            vec![],
            "bad_field",
        ),
        (
            "an expectation feature",
            do_step(r#"{"kind":"sound","phrase":1,"expect":["shimmer>3"]}"#),
            vec![],
            "bad_field",
        ),
        (
            "a newer patch file",
            do_step(r#"{"kind":"patch","predicates":["filter1.cutoff < 1kHz"]}"#),
            vec![("saw.patch.json", newer_patch())],
            "bad_content",
        ),
        (
            "an inline patch schema in a challenge",
            read_step(&format!(
                r#",{{"id":"r","kind":"recreate","challenge":{challenge_with_inline}}}"#
            )),
            vec![],
            "bad_lesson",
        ),
    ]
}

#[test]
fn a_newer_pack_that_needs_what_this_build_lacks_is_named() {
    assert!(engine_is_newer(NEWER));
    assert!(!engine_is_newer(enth_patch::file::ENGINE_VERSION));
    assert!(!engine_is_newer("not a version"));
    for (what, lesson, patches, own_code) in cases() {
        let mine = refusal(&pack(enth_patch::file::ENGINE_VERSION, &lesson, &patches));
        assert_eq!(mine.code(), own_code, "{what}, this build's engine: {mine}");
        let newer = refusal(&pack(NEWER, &lesson, &patches));
        assert_eq!(newer.code(), "needs_newer_enth", "{what}: {newer}");
        let LearnError::NeedsNewer {
            name,
            engine,
            cause,
        } = &newer
        else {
            unreachable!()
        };
        assert_eq!(engine, NEWER);
        assert_eq!(name.id.as_deref(), Some("test"));
        assert_eq!(cause.code(), own_code, "{what}: the refusal it met is kept");
        assert!(newer.hint().contains("newer enth"), "{}", newer.hint());
        assert!(newer.needs_newer_enth() && !mine.needs_newer_enth());
        assert!(newer.to_string().contains(NEWER), "{newer}");
    }
}

#[test]
fn a_newer_manifest_is_named_too() {
    let mut files = common::test_files(&[("l", &read_step(""))]);
    let mut manifest = common::test_manifest(&["l"]);
    manifest.engine = NEWER.to_owned();
    files = common::seal(&manifest, files);
    let text = String::from_utf8(files["pack.json"].clone()).unwrap();
    let with_kind = text.replacen(r#""lessons":"#, r#""covers":["granular"],"lessons":"#, 1);
    files.insert("pack.json".to_owned(), with_kind.clone().into_bytes());
    let e = refusal(&files);
    assert_eq!(e.code(), "needs_newer_enth", "{e}");
    let mine = with_kind.replacen(NEWER, enth_patch::file::ENGINE_VERSION, 1);
    files.insert("pack.json".to_owned(), mine.into_bytes());
    assert_eq!(refusal(&files).code(), "bad_pack");
}

#[test]
fn integrity_and_requires_are_never_renamed() {
    let lesson = do_step(r#"{"kind":"patch","predicates":["has granular"]}"#);
    let mut files = pack(NEWER, &lesson, &[]);
    files.insert(
        "lessons/l/text.md".to_owned(),
        b"# Changed after sealing\n".to_vec(),
    );
    let e = refusal(&files);
    assert!(matches!(e.code(), "size_mismatch" | "hash_mismatch"), "{e}");
    // A requires this build lacks keeps its own code (it already says "needs a newer enth").
    let mut manifest = common::test_manifest(&["l"]);
    manifest.engine = NEWER.to_owned();
    manifest.requires.lesson = 2;
    let files = common::seal(&manifest, common::test_files(&[("l", &read_step(""))]));
    assert_eq!(refusal(&files).code(), "requires_unmet");
    // A newer pack that this build reads loads.
    assert!(Pack::from_files(&pack(NEWER, &read_step(""), &[])).is_ok());
}
