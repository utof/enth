//! Migration fixtures (S1b). Every schema from 0 to the current one has fixtures in
//! `tests/fixtures/patch/v<N>/<name>.patch.json`; the file layer must read each one into the current form recorded
//! in `expected/<name>.patch.json` (the dense, pretty file `to_json_pretty` writes), with the notes recorded in
//! `v<N>/<name>.notes.json` (none, if the file is absent), and the hashes in `expected/<name>.hashes.txt` (the
//! persisted hash encodings are a contract: a change there needs a new magic, not a new golden).
//!
//! When a schema is added: bump `SCHEMA`, add a step to `STEPS`, add `v<new>/` fixtures, and run
//! `cargo test -p enth-patch --test migrate -- --ignored regenerate`, then read the diff of `expected/` as a
//! reviewer would. Schema 0 is synthetic (see `migrate/v0.rs`): it never existed, it exists to keep this machinery
//! honest.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture files and the regenerate test writes them; the library does no I/O"
)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use enth_patch::canon::{canonical_hash, exact_hash};
use enth_patch::file::{SCHEMA, load};
use enth_patch::migrate::{MigrationNote, STEPS, migrate, schema_of};
use serde_json::{Value, json};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/patch")
}

struct Fixture {
    version: u32,
    name: String,
    path: PathBuf,
}

impl Fixture {
    fn text(&self) -> String {
        fs::read_to_string(&self.path).unwrap()
    }

    fn notes_path(&self) -> PathBuf {
        self.path
            .with_file_name(format!("{}.notes.json", self.name))
    }

    fn expected_path(&self) -> PathBuf {
        expected_path(&self.name)
    }
}

fn expected_path(name: &str) -> PathBuf {
    root().join("expected").join(format!("{name}.patch.json"))
}

fn hashes_path(name: &str) -> PathBuf {
    root().join("expected").join(format!("{name}.hashes.txt"))
}

fn fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root()).unwrap() {
        let dir = entry.unwrap().path();
        let Some(dirname) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(version) = dirname
            .strip_prefix('v')
            .and_then(|v| v.parse::<u32>().ok())
        else {
            continue;
        };
        for f in fs::read_dir(&dir).unwrap() {
            let path = f.unwrap().path();
            let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(name) = file.strip_suffix(".patch.json") {
                out.push(Fixture {
                    version,
                    name: name.to_owned(),
                    path: path.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| (a.version, &a.name).cmp(&(b.version, &b.name)));
    out
}

fn notes_json(notes: &[MigrationNote]) -> String {
    let v: Vec<Value> = notes
        .iter()
        .map(|n| json!({ "from": n.from, "path": n.path, "message": n.message }))
        .collect();
    let mut s = serde_json::to_string_pretty(&v).unwrap();
    s.push('\n');
    s
}

fn pretty(patch: &enth_patch::model::Patch) -> String {
    let mut s = patch.to_json_pretty();
    s.push('\n');
    s
}

fn hashes(patch: &enth_patch::model::Patch) -> String {
    format!(
        "exact {}\ncanonical {}\n",
        exact_hash(patch),
        canonical_hash(patch)
    )
}

#[test]
fn every_schema_has_fixtures_and_they_say_which() {
    let all = fixtures();
    for n in 0..=SCHEMA {
        assert!(
            all.iter().any(|f| f.version == n),
            "no fixture for schema {n}: add tests/fixtures/patch/v{n}/"
        );
    }
    for f in &all {
        assert!(
            f.version <= SCHEMA,
            "{}: a fixture from the future",
            f.path.display()
        );
        let v: Value = serde_json::from_str(&f.text()).unwrap();
        assert_eq!(
            schema_of(&v).unwrap(),
            f.version,
            "{} says another schema than its directory",
            f.path.display()
        );
    }
    assert_eq!(STEPS.len(), SCHEMA as usize, "one step per schema version");
}

#[test]
fn every_fixture_migrates_to_its_expected_form() {
    for f in fixtures() {
        let loaded = load(&f.text()).unwrap_or_else(|e| panic!("{}: {e}", f.path.display()));
        assert_eq!(loaded.from_schema, f.version, "{}", f.path.display());
        let want = fs::read_to_string(f.expected_path()).unwrap_or_else(|_| {
            panic!(
                "{}: no expected/{}.patch.json (run the regenerate test)",
                f.path.display(),
                f.name
            )
        });
        assert_eq!(
            pretty(&loaded.patch),
            want,
            "{} migrated to something else",
            f.path.display()
        );
        let want_notes = fs::read_to_string(f.notes_path()).unwrap_or_else(|_| notes_json(&[]));
        assert_eq!(
            notes_json(&loaded.notes),
            want_notes,
            "{}: the notes differ",
            f.path.display()
        );
        if f.version == SCHEMA {
            assert!(
                loaded.notes.is_empty(),
                "a current file has nothing to migrate"
            );
        }
    }
}

#[test]
fn expected_files_are_current_fixed_points_with_their_hashes() {
    let all = fixtures();
    let mut seen = Vec::new();
    for entry in fs::read_dir(root().join("expected")).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_str().unwrap().to_owned();
        if let Some(name) = file.strip_suffix(".patch.json") {
            assert!(
                all.iter().any(|f| f.name == name),
                "expected/{file} has no fixture"
            );
            let text = fs::read_to_string(&path).unwrap();
            let loaded = load(&text).unwrap();
            assert_eq!(
                loaded.from_schema, SCHEMA,
                "{file} is not in the current schema"
            );
            assert_eq!(loaded.notes, Vec::<MigrationNote>::new());
            assert_eq!(
                pretty(&loaded.patch),
                text,
                "expected/{file} is not what the writer writes"
            );
            let v: Value = serde_json::from_str(&text).unwrap();
            let again = migrate(v.clone()).unwrap();
            assert_eq!(
                again.value, v,
                "migrate is the identity on the current schema"
            );
            assert_eq!(again.notes, Vec::<MigrationNote>::new());
            let want = fs::read_to_string(hashes_path(name))
                .unwrap_or_else(|_| panic!("no expected/{name}.hashes.txt"));
            assert_eq!(
                hashes(&loaded.patch),
                want,
                "{name}: a hash changed; the encodings are persisted, change the magic instead"
            );
            // The canonical golden is the golden of every spelling of the patch.
            for seed in [1, 2, 3] {
                let q = common::scramble(&loaded.patch, seed);
                assert_eq!(
                    canonical_hash(&q),
                    canonical_hash(&loaded.patch),
                    "{name} scrambled with seed {seed}"
                );
            }
            seen.push(name.to_owned());
        } else if let Some(name) = file.strip_suffix(".hashes.txt") {
            assert!(expected_path(name).exists(), "expected/{file} has no patch");
        } else {
            panic!("unexpected file expected/{file}");
        }
    }
    for f in &all {
        assert!(
            seen.contains(&f.name),
            "fixture {} has no expected file",
            f.name
        );
    }
}

/// The same patch written in two schemas reads to the same patch, bit for bit.
#[test]
fn old_and_new_spellings_of_one_patch_agree() {
    let all = fixtures();
    let mut checked = 0;
    for old in all.iter().filter(|f| f.version < SCHEMA) {
        for new in all
            .iter()
            .filter(|f| f.version == SCHEMA && f.name == old.name)
        {
            let a = load(&old.text()).unwrap().patch;
            let b = load(&new.text()).unwrap().patch;
            // (The old file has no engine stamp; the writer stamps it, so compare what is written, and the hashes.)
            assert_eq!(
                a.to_json(),
                b.to_json(),
                "{} (v{}) and {} (v{}) are one patch",
                old.name,
                old.version,
                new.name,
                new.version
            );
            assert_eq!(exact_hash(&a), exact_hash(&b));
            assert_eq!(
                a.generators[0]
                    .values
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                b.generators[0]
                    .values
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 1,
        "the fixtures hold no patch in both an old and the new spelling"
    );
}

/// `migrate` is `STEPS[from..]` applied in order with the schema number set after each step: pure, deterministic,
/// and idempotent on its own output.
#[test]
fn migrate_is_the_steps_in_order() {
    for f in fixtures() {
        let v: Value = serde_json::from_str(&f.text()).unwrap();
        let m = migrate(v.clone()).unwrap();
        assert_eq!(m.from, f.version);
        assert_eq!(m.value["schema"], json!(SCHEMA));
        // determinism
        assert_eq!(migrate(v.clone()).unwrap(), m);
        // by hand
        let mut by_hand = v;
        let mut notes = Vec::new();
        for step in f.version..SCHEMA {
            by_hand = STEPS[step as usize](by_hand, &mut notes).unwrap();
            by_hand["schema"] = json!(step + 1);
        }
        assert_eq!(by_hand, m.value, "{}", f.path.display());
        assert_eq!(notes, m.notes);
        // idempotence
        let again = migrate(m.value.clone()).unwrap();
        assert_eq!(again.value, m.value);
        assert_eq!(again.notes, Vec::<MigrationNote>::new());
    }
}

#[test]
fn files_from_the_future_and_files_without_a_version_are_refused() {
    let current: Value = serde_json::from_str(
        &fixtures()
            .iter()
            .find(|f| f.version == SCHEMA)
            .unwrap()
            .text(),
    )
    .unwrap();
    for newer in [SCHEMA + 1, SCHEMA + 100, u32::MAX] {
        let mut v = current.clone();
        v["schema"] = json!(newer);
        let e = migrate(v.clone()).unwrap_err();
        assert_eq!(e.code(), "schema_too_new");
        assert!(e.message().contains(&newer.to_string()) && e.hint().contains("upgrade"));
        assert_eq!(
            load(&v.to_string()).unwrap_err().codes(),
            ["schema_too_new"]
        );
    }
    let mut v = current.clone();
    v.as_object_mut().unwrap().remove("schema");
    assert_eq!(migrate(v).unwrap_err().code(), "missing_schema");
    for bad in [
        json!(-1),
        json!("1"),
        json!(1.5),
        json!(null),
        json!(u64::from(u32::MAX) + 1),
    ] {
        let mut v = current.clone();
        v["schema"] = bad.clone();
        assert_eq!(migrate(v).unwrap_err().code(), "bad_schema", "{bad}");
    }
}

/// A step that finds a file that does not look like its schema says so with `migration_failed`, not a panic.
#[test]
fn a_file_that_does_not_look_like_its_schema_fails_cleanly() {
    for text in [
        r#"{"schema":0,"lanes":3}"#,
        r#"{"schema":0,"lanes":[3]}"#,
        r#"{"schema":0,"generators":"x"}"#,
        r#"{"schema":0,"generators":[3]}"#,
        r#"{"schema":0,"generators":[{"id":"osc1","kind":"analog","params":3}]}"#,
        r#"{"schema":0,"generators":[{"id":"osc1","kind":"analog","params":{"level":"loud"}}]}"#,
        r#"{"schema":0,"meta":3,"name":"x"}"#,
        r#"{"schema":0,"routes":{"a":1}}"#,
    ] {
        let v: Value = serde_json::from_str(text).unwrap();
        let e = migrate(v).unwrap_err();
        assert_eq!(e.code(), "migration_failed", "{text}");
        assert_ne!(e.hint(), "");
    }
    assert!(migrate(json!([1])).is_err());
}

/// The engine stamp records where a patch was authored, and nothing branches on it: a file stamped with an older
/// engine version loads, keeps its stamp when written back, and has the same hashes as the same patch stamped with
/// the current version (they leave the stamp out). A patch with no stamp is written with `ENGINE_VERSION`;
/// `enth-engine`'s `tests/engine_version.rs` pins that to the engine's own version.
#[test]
fn an_older_engine_stamp_loads_and_is_kept() {
    // The dense expected file is a fixed point of the writer, so an edit of its stamp alone must round-trip.
    let current = fs::read_to_string(expected_path("pluck")).unwrap();
    let stamp = format!(r#""engine": "{}""#, enth_patch::file::ENGINE_VERSION);
    assert!(
        current.contains(&stamp),
        "the expected file carries the current stamp"
    );
    let older = current.replace(&stamp, r#""engine": "0.1.0""#);

    let loaded = load(&older).expect("an older engine stamp keeps loading");
    assert!(loaded.notes.is_empty(), "no migration, no warning");
    assert_eq!(loaded.patch.engine, "0.1.0");
    assert_eq!(
        pretty(&loaded.patch),
        older,
        "writing it back changes nothing"
    );

    let now = load(&current).unwrap().patch;
    assert_eq!(canonical_hash(&loaded.patch), canonical_hash(&now));
    assert_eq!(exact_hash(&loaded.patch), exact_hash(&now));
}

/// Writes `expected/` and the notes files from the fixtures. Run by hand when a fixture or a step changes, then
/// review the diff:
/// `cargo test -p enth-patch --test migrate -- --ignored regenerate`.
#[test]
#[ignore = "rewrites tests/fixtures/patch/expected; run by hand"]
fn regenerate_expected_files() {
    let all = fixtures();
    for f in all.iter().rev() {
        // the newest spelling of a name decides; the check test then demands the older ones agree
        let loaded = load(&f.text()).unwrap();
        let first = all.iter().rev().find(|g| g.name == f.name).unwrap();
        if std::ptr::eq(first, f) {
            fs::write(f.expected_path(), pretty(&loaded.patch)).unwrap();
            fs::write(hashes_path(&f.name), hashes(&loaded.patch)).unwrap();
        }
        if !loaded.notes.is_empty() {
            fs::write(f.notes_path(), notes_json(&loaded.notes)).unwrap();
        }
    }
}
