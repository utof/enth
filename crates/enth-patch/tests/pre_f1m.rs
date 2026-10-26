//! A stored patch from before F1m (S9): a dense file written by a build whose `lfo` had no `sync`, `beats` or
//! `one_shot` (S2a lane TMP added them to the catalogue in the same schema, appended).
//!
//! `fixtures/pre-f1m/sync-lead.patch.json` is `expected/sync-lead.patch.json` as main `30c713b` had it, byte for
//! byte, and it is read-only: its SHA-256 is pinned below, and the regenerate run of `migrate.rs` never touches this
//! directory (it only reads `fixtures/patch/`). It stays as it is so the day a catalogue edit breaks the reading of
//! old files, this test says so. Every patch saved with an `lfo` before F1m looks like it, so what it proves is what
//! a player's saved patches rely on: the three keys that are missing take their defaults, and nothing else moves.
//!
//! Hashes: the exact and canonical hashes of this patch moved at F1m (`exact faa381a3…`, `canonical 0c3dd745…`
//! before; `expected/sync-lead.hashes.txt` has the new ones), because the patch now has three more parameters per
//! `lfo`. They are the hashes of the loaded patch, so an old file hashes like the new file; a hash stored *in*
//! some other file from before F1m names a patch this build no longer computes.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture files; the library does no I/O"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use enth_patch::canon::{canonical_hash, exact_hash, identical};
use enth_patch::catalogue::{Beats, LfoParam, ModuleKind};
use enth_patch::file::{SCHEMA, load};
use enth_patch::validate::validate;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The fixture's SHA-256: it is a promise about old files, so it is never regenerated.
const SYNC_LEAD_SHA256: &str = "b06035b27d18f601f2cb3235d5cd7c751b99ae7b4bc2e6097a6d7cdf3dc902db";

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(path: &str) -> String {
    let p = tests_dir().join(path);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn the_pre_f1m_fixture_is_the_file_it_was() {
    let text = read("pre-f1m/sync-lead.patch.json");
    let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    let hex = digest.iter().fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    });
    assert_eq!(hex, SYNC_LEAD_SHA256, "the pre-F1m fixture is read-only");
    // Its lfos have no sync, beats or one_shot key, which is the point of keeping it.
    let v: Value = serde_json::from_str(&text).unwrap();
    let lfos: Vec<&Value> = v["modulators"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == "lfo")
        .collect();
    assert_eq!(lfos.len(), 2);
    for lfo in lfos {
        for key in ["sync", "beats", "one_shot"] {
            assert!(lfo["params"].get(key).is_none(), "{key} in {lfo}");
        }
        assert!(lfo["params"].get("rate").is_some());
    }
}

#[test]
fn a_patch_saved_before_f1m_loads_with_the_new_lfo_keys_at_their_defaults() {
    let loaded = load(&read("pre-f1m/sync-lead.patch.json")).expect("an old file loads");
    assert_eq!(
        loaded.from_schema, SCHEMA,
        "no schema step: the keys are appended"
    );
    assert!(
        loaded.notes.is_empty(),
        "nothing to report: {:?}",
        loaded.notes
    );
    validate(&loaded.patch).expect("and is valid");
    let lfos: Vec<_> = loaded
        .patch
        .modulators
        .iter()
        .filter(|m| m.kind == ModuleKind::Lfo)
        .collect();
    assert_eq!(lfos.len(), 2);
    for lfo in lfos {
        assert_eq!(lfo.values.len(), ModuleKind::Lfo.params().len());
        assert_eq!(lfo.values[LfoParam::Sync.index()], 0.0, "sync is off");
        assert_eq!(
            lfo.values[LfoParam::Beats.index()],
            Beats::Quarter.index() as f32,
            "beats is a quarter"
        );
        assert_eq!(
            lfo.values[LfoParam::OneShot.index()],
            0.0,
            "one_shot is off"
        );
    }
    // The old values it did have are still its values.
    let lfo1 = &loaded.patch.modulators[1];
    assert_eq!(lfo1.values[LfoParam::Rate.index()], 5.0);
}

#[test]
fn an_old_file_is_the_current_file_and_hashes_like_it() {
    let old = load(&read("pre-f1m/sync-lead.patch.json")).unwrap().patch;
    let current_text = read("patch/expected/sync-lead.patch.json");
    let current = load(&current_text).unwrap().patch;
    assert!(identical(&old, &current), "same patch, bit for bit");
    // Writing it back gives the current dense file, so re-saving an old patch is a one-time, lossless rewrite.
    assert_eq!(old.to_json_pretty() + "\n", current_text);
    let hashes = read("patch/expected/sync-lead.hashes.txt");
    assert_eq!(
        format!(
            "exact {}\ncanonical {}\n",
            exact_hash(&old),
            canonical_hash(&old)
        ),
        hashes
    );
}
