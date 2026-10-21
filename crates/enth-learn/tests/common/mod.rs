//! Shared by the integration tests: the example pack in its shipped form, and small packs built in code.
//!
//! [`example_files`] does what `cargo xtask pack` will (lane LCK): read `content/lessons/example/`, turn each TOML
//! file into canonical JSON through the same serde types (TOML → JSON value → `from_json`, so the test runs with or
//! without the `toml` feature), copy the texts, patches and credits, and seal the manifest.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads the example pack and its fixture files; the library does no I/O"
)]
#![allow(dead_code, reason = "each test binary uses some of these helpers")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use enth_learn::pack::{MANIFEST_PATH, lesson_path};
use enth_learn::{Lesson, Manifest};

/// `content/lessons/example/`.
pub fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/lessons/example")
}

/// A TOML file as the JSON text the same serde types read.
pub fn toml_as_json(text: &str) -> String {
    let value: serde_json::Value = toml::from_str(text).expect("valid TOML");
    value.to_string()
}

/// The example pack's authoring manifest (no files yet).
pub fn example_manifest() -> Manifest {
    let text = fs::read_to_string(example_dir().join("pack.toml")).unwrap();
    Manifest::from_json(&toml_as_json(&text)).unwrap()
}

/// The example pack's files as `cargo xtask pack` writes them: canonical JSON, texts, patches, credits, and the
/// sealed `pack.json`.
pub fn example_files() -> BTreeMap<String, Vec<u8>> {
    let dir = example_dir();
    let manifest = example_manifest();
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
        for entry in fs::read_dir(&ldir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            if name == "lesson.toml" {
                let lesson =
                    Lesson::from_json(&toml_as_json(&fs::read_to_string(&path).unwrap())).unwrap();
                files.insert(lesson_path(id), lesson.to_json().into_bytes());
            } else {
                files.insert(format!("lessons/{id}/{name}"), fs::read(&path).unwrap());
            }
        }
    }
    seal(&manifest, files)
}

/// `files` plus the sealed `pack.json` of `manifest`.
pub fn seal(
    manifest: &Manifest,
    mut files: BTreeMap<String, Vec<u8>>,
) -> BTreeMap<String, Vec<u8>> {
    files.remove(MANIFEST_PATH);
    let sealed = manifest.seal(&files).unwrap();
    files.insert(MANIFEST_PATH.to_owned(), sealed.to_json().into_bytes());
    files
}

/// A patch of the example pack, as text.
pub fn example_patch(name: &str) -> String {
    fs::read_to_string(example_dir().join("patches").join(name)).unwrap()
}

/// A manifest for a pack built in code: id `test`, one lesson per id, requiring everything this build has.
pub fn test_manifest(lessons: &[&str]) -> Manifest {
    let list: Vec<String> = lessons.iter().map(|l| format!("{l:?}")).collect();
    Manifest::from_json(&format!(
        r#"{{"schema":"enth.pack/1","id":"test","version":"1.0.0","title":"Test pack","authors":["enth contributors"],
        "license":{{"text":"CC-BY-4.0","patches":"CC0-1.0"}},
        "requires":{{"lesson":1,"challenge":2,"patch":1,"generator":1,"scorers":["combined/1","sound/1","structure/1","time/1"]}},
        "lessons":[{}]}}"#,
        list.join(",")
    ))
    .unwrap()
}

/// A sealed pack built in code: the given lessons (id, JSON), the example's two patches, credits.
pub fn test_files(lessons: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
    let ids: Vec<&str> = lessons.iter().map(|(id, _)| *id).collect();
    let mut files = BTreeMap::new();
    files.insert("CREDITS.md".to_owned(), b"# Credits\n".to_vec());
    for name in ["saw.patch.json", "dark.patch.json"] {
        files.insert(format!("patches/{name}"), example_patch(name).into_bytes());
    }
    for (id, json) in lessons {
        files.insert(
            format!("lessons/{id}/lesson.json"),
            json.as_bytes().to_vec(),
        );
        files.insert(format!("lessons/{id}/text.md"), b"# A step\n".to_vec());
    }
    seal(&test_manifest(&ids), files)
}

/// The fixture directory of a kind (`pack`, `lesson`, `progress`, `lesson-ref`), version 1.
pub fn fixture_dir(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{kind}/v1"))
}

/// A Challenge v2 draw over the starter template, with a control to change and locks (F1l review M4): the cutoff
/// from a set, `filter1.cutoff` editable, the oscillator and the resonance locked. Its JSON, as a lesson embeds it.
pub fn draw_challenge() -> enth_challenge::Challenge {
    use enth_challenge::model::{Start, Target};
    use enth_challenge::v2::{Constraints, Draw, DrawAt, DrawItem, DrawValue, DrawValues};
    use enth_challenge::{PhraseId, ScorerId, ScoringSpec};
    let mut c = enth_challenge::Challenge::new(
        Target::Draw(Draw {
            seed: 7,
            items: vec![DrawItem {
                at: DrawAt::Param("filter1.cutoff".parse().unwrap()),
                values: DrawValues::Set(vec![
                    DrawValue::Number(300.0),
                    DrawValue::Number(1200.0),
                    DrawValue::Text("4kHz".to_owned()),
                ]),
            }],
        }),
        Start::Template { template: 0 },
        PhraseId(1),
        ScoringSpec::with_scorers(
            0,
            ScorerId::SOUND_V1,
            ScorerId::STRUCTURE_V1,
            ScorerId::COMBINED_V1,
            ScorerId::TIME_V1,
        ),
    );
    c.constraints = Constraints {
        editable: vec!["filter1.cutoff".parse().unwrap()],
        locked: vec![
            "osc1.*".parse().unwrap(),
            "filter1.resonance".parse().unwrap(),
        ],
    };
    c
}

/// A lesson whose one step is a Recreate round on [`draw_challenge`] (`enth.challenge/2` embedded).
pub fn recreate_v2_lesson(id: &str) -> String {
    format!(
        r#"{{"schema":"enth.lesson/1","id":"{id}","title":"Recreate a drawn cutoff","tier":"basic","objective":"Find the cutoff by ear.","teaches":["svf.cutoff"],"steps":[{{"id":"round","kind":"recreate","text":"text.md","view":{{"highlight":["filter1.cutoff"],"hide":["noise1.*"]}},"challenge":{}}}]}}"#,
        draw_challenge().to_json()
    )
}
