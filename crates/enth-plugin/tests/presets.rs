//! The presets in `content/presets/` (lane INT): every file is a current patch document in the plugin's own
//! serialisation, so the plugin loads it as state and saves it back byte for byte (docs/plugin.md, "State").
//!
//! The files come from `cargo xtask extract-presets`, which copies the `patch` field of a REAPER project verbatim; this
//! test catches a hand edit, a pretty-printed copy or a file from an older schema, which would load but not come back
//! unchanged. Until the owner's G1 sounds are committed the directory holds no presets, so the REAPER harness's
//! round-trip fixture (the plugin's own first save of a rich patch) is checked the same way.

use std::path::{Path, PathBuf};

/// The checks: a current schema, no migration notes, and `to_json` gives the same bytes.
fn check(path: &Path, text: &str) -> Result<(), String> {
    let loaded = enth_patch::file::load(text)
        .map_err(|e| format!("{}: does not load: {e}", path.display()))?;
    if loaded.from_schema != enth_patch::file::SCHEMA || !loaded.notes.is_empty() {
        return Err(format!(
            "{}: schema {} with migration notes {:?}: extract it again from a save by the current plugin",
            path.display(),
            loaded.from_schema,
            loaded.notes
        ));
    }
    let saved = loaded.patch.to_json();
    if saved != text {
        let at = saved
            .bytes()
            .zip(text.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(saved.len().min(text.len()));
        return Err(format!(
            "{}: the plugin would save it differently from byte {at} (the file is not `Patch::to_json()`: \
             was it edited or pretty-printed? `cargo xtask extract-presets` writes it verbatim)",
            path.display()
        ));
    }
    Ok(())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `*.patch.json` under `dir`, folders included (dot files and folders are skipped, like the plugin's scan).
fn preset_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("content/presets exists")
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            preset_files(&path, out);
        } else if name.ends_with(".patch.json") {
            out.push(path);
        }
    }
}

#[test]
fn every_preset_is_a_patch_the_plugin_saves_back_unchanged() {
    let mut files = Vec::new();
    preset_files(&root().join("content/presets"), &mut files);
    let mut failures = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("a readable preset");
        if let Err(why) = check(path, &text) {
            failures.push(why);
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    println!("{} presets checked", files.len());
}

/// The factory presets are the browser's front page (docs/presets.md): one type tag, up to four character tags, all
/// from the vocabulary in `enth_session::presets::tags`; a name of their own, and no two alike ignoring case.
#[test]
fn every_factory_preset_uses_the_tag_vocabulary_and_a_name_of_its_own() {
    let mut files = Vec::new();
    preset_files(&root().join("content/presets"), &mut files);
    let mut problems = Vec::new();
    let mut names: std::collections::BTreeMap<String, PathBuf> = std::collections::BTreeMap::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("a readable preset");
        let patch = enth_patch::file::load(&text)
            .expect("checked by the test above")
            .patch;
        for why in enth_session::presets::tags::check_factory(&patch.meta) {
            problems.push(format!("{}: {why}", path.display()));
        }
        let name = patch.meta.name.trim().to_lowercase();
        if let Some(first) = names.insert(name.clone(), path.clone()) {
            problems.push(format!(
                "{}: the name {name:?} is also {}'s",
                path.display(),
                first.display()
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// What the plugin embeds is what is on disk: the build script found every file (no preset is left out of the
/// binary because a folder was added without a rebuild trigger).
#[test]
fn the_embedded_factory_presets_are_the_files_on_disk() {
    let mut files = Vec::new();
    let dir = root().join("content/presets");
    preset_files(&dir, &mut files);
    let on_disk: Vec<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(&dir)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    let embedded: Vec<String> = enth_session::presets::factory_files()
        .iter()
        .map(|f| f.key.file_name())
        .collect();
    assert_eq!(embedded, on_disk);
}

#[test]
fn the_check_accepts_the_plugins_own_save_and_refuses_an_edit() {
    let path = root().join("xtask/reaper/enth-roundtrip.patch.json");
    let text = std::fs::read_to_string(&path).expect("the REAPER round-trip fixture");
    let text = text.trim_end();
    check(&path, text).unwrap();
    // A pretty-printed copy loads, but is not what the plugin saves.
    let pretty = enth_patch::file::load(text).unwrap().patch.to_json_pretty();
    assert!(check(&path, &pretty).is_err());
}
