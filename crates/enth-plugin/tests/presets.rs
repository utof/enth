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

#[test]
fn every_preset_is_a_patch_the_plugin_saves_back_unchanged() {
    let dir = root().join("content/presets");
    let mut checked = 0;
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("content/presets exists") {
        let path = entry.expect("a directory entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".patch.json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a readable preset");
        if let Err(why) = check(&path, &text) {
            failures.push(why);
        }
        checked += 1;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    println!("{checked} presets checked");
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
