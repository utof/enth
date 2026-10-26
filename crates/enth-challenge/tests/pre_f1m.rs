//! A challenge file from before F1m (S9): the v2 patch-start challenge as it was saved when the `lfo` had no `sync`,
//! `beats` or `one_shot` (S2a lane TMP added them in the same schema). `Patch::to_json()` is dense, so every challenge
//! that embeds a patch with an `lfo` was written without the three keys, and every one of them must still load.
//!
//! `fixtures/pre-f1m/challenge-v2-patch.json` is `fixtures/challenge/v2/patch.json` as main `30c713b` had it, byte for
//! byte. It is read-only: its SHA-256 is pinned below and `fixtures.rs`'s regenerate run never touches this directory.
//! The challenge itself did not change, so the old file reads as the very same challenge as the current one, writes
//! back as the current file's bytes, and prepares to the same target.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture files; the library does no I/O"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use enth_challenge::{Challenge, prepare_challenge};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The fixture's SHA-256: a promise about old files, never regenerated.
const V2_PATCH_SHA256: &str = "57aff26da23617ac15eea5f521327eab126381178055b8e882dcbe375a418c88";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(name: &str) -> String {
    let path = fixtures().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_pre_f1m_challenge_is_the_file_it_was() {
    let text = read("pre-f1m/challenge-v2-patch.json");
    let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    let hex = digest.iter().fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    });
    assert_eq!(hex, V2_PATCH_SHA256, "the pre-F1m fixture is read-only");
    assert!(text.contains("\"enth.challenge/2\""));
    // It embeds patches with an lfo, none of which has the three keys.
    let lfos = text.matches("\"kind\": \"lfo\"").count();
    assert!(lfos >= 2, "the fixture carries lfos");
    for key in ["\"one_shot\"", "\"beats\""] {
        assert!(!text.contains(key), "{key}");
    }
}

#[test]
fn a_v2_challenge_saved_before_f1m_loads_and_is_the_current_challenge() {
    let old_text = read("pre-f1m/challenge-v2-patch.json");
    let old = Challenge::from_json(&old_text).expect("an old challenge loads");
    let current_text = read("challenge/v2/patch.json");
    let current = Challenge::from_json(&current_text).unwrap();
    assert_eq!(
        old, current,
        "the same challenge: the lfo keys took their defaults"
    );
    assert_eq!(old.version(), 2);
    // Written back, it is the current file (a one-time, lossless rewrite).
    assert_eq!(old.to_json_pretty() + "\n", current_text);
    // The embedded patches really are the old, short form on disk and the dense form in memory.
    let v: Value = serde_json::from_str(&old_text).unwrap();
    let w: Value = serde_json::from_str(&current_text).unwrap();
    assert_ne!(v, w, "the files differ by the three lfo keys");
    // It prepares to the same round and the same target as the current file.
    let a = prepare_challenge(old).expect("prepares");
    let b = prepare_challenge(current).expect("prepares");
    assert_eq!(a.fingerprint(), b.fingerprint());
}
