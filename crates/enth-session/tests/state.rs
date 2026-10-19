//! The saved state v1: fixtures, byte-identical round trips, versions, and loads that fail safely (lane S1s).
//!
//! Fixtures live in `tests/fixtures/state/v<N>/<case>.patch.json` (the `patch` field) and `<case>.enth.json` (the
//! `enth` field), exactly as the plugin persists them. They are the bytes this build writes for the cases in
//! `common::v1_cases`; a change to either format fails here. When a format changes on purpose (a new state version,
//! or a patch schema bump), add `v<N+1>/` fixtures, keep the old ones as migration inputs, and regenerate the
//! current ones with `cargo test -p enth-session --test state -- --ignored regenerate`, then read the diff.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture files and the regenerate test writes them; the library does no I/O"
)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use enth_patch::Patch;
use enth_patch::canon::{exact_hash, identical};
use enth_session::state::{
    INSTANCE_STEPS, MAX_FIELD_BYTES, STATE_VERSION, decode_patch, encode_patch, instance_version,
};
use enth_session::{InstanceState, STATE_SCHEMA, SavedState, Session, ShellRequest, StateError};

fn dir(version: u32) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/state/v{version}"))
}

fn read(version: u32, name: &str, field: &str) -> String {
    let path = dir(version).join(format!("{name}.{field}.json"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn fixture(name: &str) -> SavedState {
    SavedState {
        patch: read(STATE_VERSION, name, "patch"),
        instance: read(STATE_VERSION, name, "enth"),
    }
}

#[test]
#[ignore = "writes the fixtures: run on purpose and review the diff"]
fn regenerate() {
    let d = dir(STATE_VERSION);
    fs::create_dir_all(&d).unwrap();
    for (name, patch, instance) in common::v1_cases() {
        let saved = SavedState::encode(&patch, &instance);
        fs::write(d.join(format!("{name}.patch.json")), saved.patch).unwrap();
        fs::write(d.join(format!("{name}.enth.json")), saved.instance).unwrap();
    }
}

#[test]
fn v1_fixtures_are_what_this_build_writes() {
    for (name, patch, instance) in common::v1_cases() {
        let saved = SavedState::encode(&patch, &instance);
        let expected = fixture(name);
        assert_eq!(
            saved.patch, expected.patch,
            "{name}: the patch field changed"
        );
        assert_eq!(
            saved.instance, expected.instance,
            "{name}: the enth field changed"
        );
    }
}

#[test]
fn v1_fixtures_decode_to_the_cases() {
    for (name, patch, instance) in common::v1_cases() {
        let (loaded, decoded) = fixture(name).decode();
        let loaded = loaded.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(identical(&loaded.patch, &patch), "{name}: patch differs");
        assert_eq!(exact_hash(&loaded.patch), exact_hash(&patch), "{name}");
        assert_eq!(
            loaded.notes,
            [],
            "{name}: a current patch needs no migration"
        );
        let (state, version) = decoded.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(state, instance, "{name}");
        assert_eq!(version, STATE_VERSION);
    }
}

#[test]
fn the_custom_fixture_carries_slots_macros_and_ui_state() {
    let (loaded, decoded) = fixture("custom").decode();
    let patch = loaded.unwrap().patch;
    assert_eq!(patch.slots[0], Some(common::addr("filter1.cutoff")));
    assert_eq!(patch.slots[1], Some(common::addr("master.level")));
    assert_eq!(patch.slots[63], Some(common::addr("voice.glide")));
    assert_eq!(patch.slots.iter().flatten().count(), 3);
    assert_eq!(patch.macros[0].value, 0.25);
    assert_eq!(patch.macros[0].name.as_deref(), Some("Brightness"));
    let (state, _) = decoded.unwrap();
    assert!(state.monitor.fx_bypass);
    assert_eq!(state.ui.scale_percent, 150);
    assert!(state.ui.advanced);
}

#[test]
fn decode_then_encode_is_byte_identical() {
    for (name, ..) in common::v1_cases() {
        let saved = fixture(name);
        let (loaded, decoded) = saved.decode();
        let again = SavedState::encode(&loaded.unwrap().patch, &decoded.unwrap().0);
        assert_eq!(again, saved, "{name}");
    }
}

#[test]
fn a_session_saves_the_fixture_bytes_and_loads_them_back() {
    for (name, patch, instance) in common::v1_cases() {
        let mut session = Session::new(patch.clone());
        session.load_instance(&instance.to_json());
        let saved = session.save(None);
        assert_eq!(saved, fixture(name), "{name}");

        let mut other = Session::default();
        let report = other.load(&saved);
        assert_eq!(report.errors, [], "{name}");
        assert!(!report.fell_back);
        assert_eq!(report.patch_schema, Some(enth_patch::file::SCHEMA));
        assert_eq!(report.instance_version, Some(STATE_VERSION));
        assert!(identical(other.patch(), &patch), "{name}");
        assert_eq!(other.instance(), &instance);
        // The second save is byte-identical (clap-validator's state-reproducibility-binary).
        assert_eq!(other.save(None), saved, "{name}");
    }
}

#[test]
fn a_load_clears_the_history_and_counts_as_saved() {
    let mut session = Session::new(common::custom_patch());
    let report = session.load_patch(&encode_patch(&Patch::init()));
    assert_eq!(report.errors, []);
    let history = session.history();
    assert!(!history.can_undo() && !history.can_redo() && !history.is_dirty());
    let requests: Vec<_> = session.drain_requests().collect();
    assert_eq!(requests, vec![ShellRequest::RenameParams]);
}

#[test]
fn load_instance_reports_the_bypass_to_the_shell() {
    let mut session = Session::default();
    session.load_instance(&common::custom_instance().to_json());
    let requests: Vec<_> = session.drain_requests().collect();
    assert_eq!(requests, vec![ShellRequest::MonitorBypass(true)]);
}

#[test]
fn a_bad_patch_falls_back_to_the_init_patch() {
    for (text, inner) in [
        ("", "bad_json"),
        ("not json", "bad_json"),
        ("[]", "bad_file"),
        ("{}", "missing_schema"),
        (r#"{"schema": 99}"#, "schema_too_new"),
    ] {
        let mut session = Session::new(common::custom_patch());
        let report = session.load_patch(text);
        assert!(report.fell_back, "{text:?}");
        assert!(identical(session.patch(), &Patch::init()), "{text:?}"); // the fallback is the bare init patch
        assert_eq!(report.errors.len(), 1);
        let err = &report.errors[0];
        assert_eq!(err.code(), "bad_patch", "{text:?}");
        match err {
            StateError::Patch(invalid) => assert_eq!(invalid.codes(), vec![inner], "{text:?}"),
            other => panic!("{text:?}: {other:?}"),
        }
        assert_ne!(err.hint(), "");
    }
}

/// A patch this build cannot read (saved by a newer enth, or damaged) is written back unchanged by every save until
/// the first edit, so opening a project in an older build never destroys it (F1b review). A good load clears it;
/// an oversized field is not kept.
#[test]
fn a_patch_that_does_not_load_is_saved_back_unchanged() {
    let newer = r#"{"schema": 99, "from": "a newer enth"}"#;
    let mut session = Session::new(common::custom_patch());
    let report = session.load_patch(newer);
    assert!(report.fell_back);
    assert!(identical(session.patch(), &Patch::init()));
    let status = session.status();
    assert!(status.patch_unloaded);
    assert_eq!(
        status
            .last_error
            .as_ref()
            .map(enth_session::SessionError::code),
        Some("bad_patch"),
        "the status line shows why"
    );
    assert_eq!(session.save_patch(None), newer);
    assert_eq!(
        session.save(None).patch,
        newer,
        "every save, not just the first"
    );

    let good = encode_patch(&common::custom_patch());
    assert!(!session.load_patch(&good).fell_back);
    assert!(!session.status().patch_unloaded);
    assert_eq!(session.save_patch(None), good);

    let mut session = Session::default();
    assert!(
        session
            .load_patch(&" ".repeat(MAX_FIELD_BYTES + 1))
            .fell_back
    );
    assert!(!session.status().patch_unloaded);
    assert_eq!(session.save_patch(None), encode_patch(&Patch::init()));
}

#[test]
fn a_bad_instance_state_gives_the_defaults_and_keeps_the_patch() {
    let cases: &[(&str, &str)] = &[
        ("", "bad_json"),
        ("{", "bad_json"),
        ("[]", "bad_instance"),
        ("{}", "missing_schema"),
        (r#"{"schema": 1}"#, "bad_schema"),
        (r#"{"schema": "enth.plugin-state"}"#, "bad_schema"),
        (r#"{"schema": "enth.plugin-state/"}"#, "bad_schema"),
        (r#"{"schema": "enth.plugin-state/0"}"#, "bad_schema"),
        (r#"{"schema": "enth.plugin-state/x"}"#, "bad_schema"),
        (r#"{"schema": "enth.plugin-state/-1"}"#, "bad_schema"),
        (r#"{"schema": "enth.plugin-state/1.0"}"#, "bad_schema"),
        (r#"{"schema": "other.state/1"}"#, "bad_schema"),
        (r#"{"schema": "enth.plugin-state/2"}"#, "schema_too_new"),
        (
            r#"{"schema": "enth.plugin-state/99999999999"}"#,
            "bad_schema",
        ),
        (
            r#"{"schema": "enth.plugin-state/1", "monitor": {"fx_bypass": "yes"}}"#,
            "bad_instance",
        ),
        (
            r#"{"schema": "enth.plugin-state/1", "ui": {"scale_percent": -5}}"#,
            "bad_instance",
        ),
    ];
    for &(text, code) in cases {
        let mut session = Session::new(common::custom_patch());
        session.set_ui(common::custom_instance().ui);
        let report = session.load_instance(text);
        assert_eq!(report.errors.len(), 1, "{text:?}");
        assert_eq!(report.errors[0].code(), code, "{text:?}");
        assert_ne!(report.errors[0].hint(), "");
        assert_eq!(report.instance_version, None);
        assert_eq!(session.instance(), &InstanceState::default(), "{text:?}");
        assert!(identical(session.patch(), &common::custom_patch()));
    }
}

#[test]
fn missing_fields_default_and_unknown_fields_are_ignored() {
    let (state, version) =
        InstanceState::from_json(r#"{"schema": "enth.plugin-state/1"}"#).unwrap();
    assert_eq!(state, InstanceState::default());
    assert_eq!(version, 1);

    let text = r#"{"schema":"enth.plugin-state/1","monitor":{"fx_bypass":true,"later":1},"ui":{"advanced":true},"zoom":2}"#;
    let (state, _) = InstanceState::from_json(text).unwrap();
    assert!(state.monitor.fx_bypass);
    assert!(state.ui.advanced);
    assert_eq!(state.ui.layout, "lanes");
    assert_eq!(state.ui.scale_percent, 100);
    // Re-saving writes the current shape only.
    assert!(!state.to_json().contains("zoom"));
}

#[test]
fn the_enth_field_starts_with_its_schema() {
    let text = InstanceState::default().to_json();
    assert!(
        text.starts_with(&format!(r#"{{"schema":"{STATE_SCHEMA}","#)),
        "{text}"
    );
    assert_eq!(
        instance_version(&serde_json::from_str(&text).unwrap()).unwrap(),
        1
    );
    assert_eq!(STATE_SCHEMA, format!("enth.plugin-state/{STATE_VERSION}"));
    assert_eq!(INSTANCE_STEPS.len(), (STATE_VERSION - 1) as usize);
}

#[test]
fn oversized_fields_are_refused_before_parsing() {
    let big = " ".repeat(MAX_FIELD_BYTES + 1);
    assert_eq!(decode_patch(&big).unwrap_err().code(), "state_too_large");
    assert_eq!(
        InstanceState::from_json(&big).unwrap_err().code(),
        "state_too_large"
    );
    let mut session = Session::default();
    assert!(session.load_patch(&big).fell_back);
}

#[test]
fn a_patch_field_and_a_patch_file_are_interchangeable() {
    let patch = common::custom_patch();
    assert_eq!(encode_patch(&patch), patch.to_json());
    let loaded = enth_patch::file::load(&encode_patch(&patch)).unwrap();
    assert!(identical(&loaded.patch, &patch));
}

#[test]
fn every_state_error_code_is_distinct() {
    let errors = [
        StateError::TooLarge {
            field: "patch",
            bytes: 2,
            max: 1,
        },
        StateError::BadJson {
            field: "enth",
            message: String::new(),
        },
        StateError::MissingSchema,
        StateError::BadSchema {
            found: String::new(),
        },
        StateError::SchemaTooNew {
            found: 2,
            supported: 1,
        },
        StateError::MigrationFailed {
            from: 1,
            message: String::new(),
        },
        StateError::BadInstance {
            message: String::new(),
        },
        StateError::Patch(enth_patch::Invalid::one(
            "",
            enth_patch::PatchError::MissingSchema,
        )),
    ];
    let mut codes: Vec<_> = errors.iter().map(StateError::code).collect();
    for e in &errors {
        assert_ne!(e.to_string(), "");
    }
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), errors.len());
}

// ---------------------------------------------------------------------------------------------------------------
// Migrations and versions (lane S2s)
// ---------------------------------------------------------------------------------------------------------------

/// The synthetic schema-0 patch files (the patch format's never-released predecessor, which exists so the migration
/// framework has real steps), as a plugin state's `patch` field. `tests/fixtures/state/v0/<case>.patch.json` is what
/// an old build would have stored; `<case>.migrated.patch.json` is the v1 `patch` field the session writes after
/// loading it. (The instance state has no v0: its first schema is 1, so its walk is tested with a table of its own.)
const V0_CASES: [&str; 2] = ["lanes", "minimal"];

#[test]
#[ignore = "writes the migrated v0 fixtures: run on purpose and review the diff"]
fn regenerate_v0_expected() {
    for name in V0_CASES {
        let mut session = Session::default();
        let report = session.load_patch(&read(0, name, "patch"));
        assert_eq!(report.errors, []);
        fs::write(
            dir(0).join(format!("{name}.migrated.patch.json")),
            session.save_patch(None),
        )
        .unwrap();
    }
}

#[test]
fn a_v0_patch_field_migrates_when_the_state_loads_and_saves_as_v1() {
    for name in V0_CASES {
        let old = read(0, name, "patch");
        assert!(old.contains(r#""schema": 0"#), "{name}: a schema-0 file");
        let expected = read(0, &format!("{name}.migrated"), "patch");

        let mut session = Session::default();
        let report = session.load(&SavedState {
            patch: old.clone(),
            instance: InstanceState::default().to_json(),
        });
        assert_eq!(report.errors, [], "{name}");
        assert!(!report.fell_back);
        assert_eq!(report.patch_schema, Some(0), "{name}: the version it had");
        assert!(
            report
                .notes
                .iter()
                .all(|n| n.from == 0 && !n.message.is_empty()),
            "{name}: what the step changed is reported"
        );
        assert!(!session.status().patch_unloaded);
        // The save is the current schema, byte for byte what the fixture says.
        let saved = session.save_patch(None);
        assert_eq!(saved, expected, "{name}");
        assert!(
            saved.contains(&format!(r#""schema":{}"#, enth_patch::file::SCHEMA)),
            "{name}"
        );
        // And it is the same path as a patch file: the file layer's load gives the same patch.
        let direct = enth_patch::file::load(&old).unwrap();
        assert!(identical(session.patch(), &direct.patch), "{name}");
        assert_eq!(report.notes, direct.notes, "{name}");
        // A second load of the migrated text is current: no notes, the same bytes again.
        let mut again = Session::default();
        let second = again.load_patch(&saved);
        assert_eq!(second.patch_schema, Some(enth_patch::file::SCHEMA));
        assert_eq!(second.notes, []);
        assert_eq!(again.save_patch(None), saved, "{name}");
    }
    // The migration noted what it dropped.
    let mut session = Session::default();
    let report = session.load_patch(&read(0, "lanes", "patch"));
    assert_eq!(report.notes.len(), 3);
}

/// A test table standing in for a build whose `STATE_VERSION` is 3: v1 → v2 renames a key, v2 → v3 adds one.
fn synthetic_steps() -> [enth_session::state::InstanceStep; 2] {
    fn v1_to_v2(mut v: serde_json::Value) -> Result<serde_json::Value, String> {
        let monitor = v
            .get_mut("monitor")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("no monitor")?;
        let bypass = monitor.remove("fx_bypass").ok_or("no fx_bypass")?;
        monitor.insert("bypass_fx".to_owned(), bypass);
        Ok(v)
    }
    fn v2_to_v3(mut v: serde_json::Value) -> Result<serde_json::Value, String> {
        v.as_object_mut()
            .ok_or("not an object")?
            .insert("zoom".to_owned(), 2.into());
        Ok(v)
    }
    [v1_to_v2, v2_to_v3]
}

#[test]
fn the_instance_migration_walk_runs_every_step_in_order() {
    use enth_session::state::{migrate_instance, migrate_instance_with};
    let steps = synthetic_steps();
    let v1: serde_json::Value = serde_json::from_str(&read(1, "custom", "enth")).unwrap();

    // A v1 state walks both steps; the schema string is rewritten after each.
    let (v3, from) = migrate_instance_with(v1.clone(), 3, &steps).unwrap();
    assert_eq!(from, 1);
    assert_eq!(v3["schema"], "enth.plugin-state/3");
    assert_eq!(v3["monitor"]["bypass_fx"], true);
    assert!(v3["monitor"].get("fx_bypass").is_none());
    assert_eq!(v3["zoom"], 2);
    assert_eq!(
        v3["ui"]["scale_percent"], 150,
        "everything else is untouched"
    );

    // A v2 state walks one; a v3 state none.
    let (v2, _) = migrate_instance_with(v1.clone(), 2, &steps).unwrap();
    assert_eq!(v2["schema"], "enth.plugin-state/2");
    assert!(v2.get("zoom").is_none());
    let (same, from) = migrate_instance_with(v3.clone(), 3, &steps).unwrap();
    assert_eq!((same, from), (v3.clone(), 3));

    // Newer than the build: refused, never guessed.
    assert_eq!(
        migrate_instance_with(v3, 2, &steps).unwrap_err(),
        StateError::SchemaTooNew {
            found: 3,
            supported: 2
        }
    );
    // A missing step and a failing step are `migration_failed`, never a panic.
    let err = migrate_instance_with(v1.clone(), 4, &steps).unwrap_err();
    assert!(
        matches!(err, StateError::MigrationFailed { from: 3, .. }),
        "{err:?}"
    );
    let broken: serde_json::Value =
        serde_json::from_str(r#"{"schema":"enth.plugin-state/1","monitor":{}}"#).unwrap();
    let err = migrate_instance_with(broken, 3, &steps).unwrap_err();
    assert_eq!(err.code(), "migration_failed");
    assert!(
        matches!(err, StateError::MigrationFailed { from: 1, ref message } if message == "no fx_bypass")
    );
    // This build's table is the identity on its own version.
    let (value, from) = migrate_instance(v1.clone()).unwrap();
    assert_eq!((value, from), (v1, 1));
}

/// A state a newer build wrote is kept and saved back unchanged (lane S2s), for both fields, until the user changes
/// what it holds.
#[test]
fn a_newer_instance_state_is_kept_and_saved_back_unchanged() {
    let newer = r#"{"schema":"enth.plugin-state/2","monitor":{"fx_bypass":true},"ui":{"layout":"flow","scale_percent":150,"advanced":true},"zoom":2}"#;
    let mut session = Session::default();
    let report = session.load_instance(newer);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].code(), "schema_too_new");
    assert_eq!(report.instance_version, None);
    assert_eq!(
        report.history, None,
        "an instance load does not touch the history"
    );
    assert_eq!(
        session.instance(),
        &InstanceState::default(),
        "this build plays the defaults"
    );
    let status = session.status();
    assert!(status.instance_unloaded);
    assert_eq!(
        status
            .last_error
            .as_ref()
            .map(enth_session::SessionError::code),
        Some("schema_too_new"),
        "the status line says why"
    );
    assert_eq!(session.save_instance(), newer);
    assert_eq!(
        session.save(None).instance,
        newer,
        "every save, not just the first"
    );

    // Setting what is already set is not a change.
    session.set_ui(InstanceState::default().ui);
    session.set_monitor_bypass(false);
    assert_eq!(session.save_instance(), newer);
    // The first real change replaces it with this build's state.
    session.set_monitor_bypass(true);
    assert!(!session.status().instance_unloaded);
    assert_eq!(session.save_instance(), session.instance().to_json());
    assert_ne!(session.save_instance(), newer);

    // A UI change does too; a good load clears it; a damaged field is not kept.
    let mut session = Session::default();
    session.load_instance(newer);
    session.set_ui(enth_session::UiState {
        advanced: true,
        ..enth_session::UiState::default()
    });
    assert!(!session.status().instance_unloaded);
    session.load_instance(newer);
    session.load_instance(&common::custom_instance().to_json());
    assert!(!session.status().instance_unloaded);
    assert_eq!(session.save_instance(), common::custom_instance().to_json());
    session.load_instance("{ not json");
    assert!(!session.status().instance_unloaded);
    assert_eq!(session.save_instance(), InstanceState::default().to_json());
}

/// The same for the `patch` field, end to end through both fields: a project saved by a newer enth survives an older
/// build's open and save, byte for byte.
#[test]
fn a_project_from_a_newer_build_survives_an_older_builds_save() {
    let saved = SavedState {
        patch: r#"{"schema":2,"engine":"9.9.9","generators":["a newer shape"]}"#.to_owned(),
        instance: r#"{"schema":"enth.plugin-state/7","ui":{"layout":"deck"}}"#.to_owned(),
    };
    let mut session = Session::new(common::custom_patch());
    let report = session.load(&saved);
    assert!(report.fell_back);
    assert_eq!(
        report
            .errors
            .iter()
            .map(StateError::code)
            .collect::<Vec<_>>(),
        ["schema_too_new", "bad_patch"]
    );
    let status = session.status();
    assert!(status.patch_unloaded && status.instance_unloaded);
    assert_eq!(session.save(None), saved);
    // Saving again after the host folded values into the (init) patch does not change it either.
    assert_eq!(session.save(Some(&session.host_values())), saved);
    assert_eq!(session.save(None), saved);
}

// ---------------------------------------------------------------------------------------------------------------
// Fuzz-lite on state blobs (lane S2s): deterministic mutations, no panics, typed errors, a usable session after
// ---------------------------------------------------------------------------------------------------------------

/// xorshift64*: a fixed seed per run, no global RNG.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// One mutation of `bytes`: a flip, a truncation, a deletion, a duplicated chunk, inserted noise or a swapped pair.
fn mutate(rng: &mut Rng, bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let n = out.len().max(1);
    match rng.below(6) {
        0 => {
            let i = rng.below(n);
            if let Some(b) = out.get_mut(i) {
                *b ^= 1 << rng.below(8);
            }
        }
        1 => out.truncate(rng.below(n)),
        2 => {
            let a = rng.below(n);
            let b = (a + rng.below(16)).min(out.len());
            out.drain(a..b);
        }
        3 => {
            let a = rng.below(n);
            let b = (a + rng.below(64)).min(out.len());
            let chunk = out[a..b].to_vec();
            let at = rng.below(n);
            out.splice(at..at, chunk);
        }
        4 => {
            let at = rng.below(n);
            let noise: Vec<u8> = (0..rng.below(8)).map(|_| rng.next() as u8).collect();
            out.splice(at..at, noise);
        }
        _ => {
            let (a, b) = (rng.below(n), rng.below(n));
            if a < out.len() && b < out.len() {
                out.swap(a, b);
            }
        }
    }
    out
}

/// What a session must satisfy after any load, however bad the input: it compiles for the engine, saves, and its
/// own save loads back to the same bytes.
fn assert_usable(session: &mut Session, what: &str) {
    enth_engine::compile(session.patch(), &enth_engine::PrepareCfg::new(48_000.0))
        .unwrap_or_else(|e| panic!("{what}: the patch does not compile: {e}"));
    let first = session.save(None);
    let status = session.status();
    let mut other = Session::default();
    let report = other.load(&first);
    // Text kept as it was (a newer or damaged field) fails to load the same way again; anything else loads clean.
    let kept = usize::from(status.patch_unloaded) + usize::from(status.instance_unloaded);
    assert_eq!(report.errors.len(), kept, "{what}: {:?}", report.errors);
    assert_eq!(other.save(None), first, "{what}: a save is stable");
}

#[test]
fn mutated_state_blobs_never_panic_and_leave_a_usable_session() {
    let mut rng = Rng(0x0E17_5EED_5EED_0E17);
    let patch = read(STATE_VERSION, "custom", "patch");
    let instance = read(STATE_VERSION, "custom", "enth");
    let old = read(0, "lanes", "patch");
    let mut loaded_ok = 0;
    for round in 0..1500usize {
        let source = [&patch, &instance, &old][round % 3];
        let bytes = mutate(&mut rng, source.as_bytes());
        // The host hands over text; invalid UTF-8 is repaired the way a lossy decode would.
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut session = Session::new(common::custom_patch());
        let (as_patch, as_instance) = (round % 3 != 1, round % 3 == 1);
        if as_patch {
            let report = session.load_patch(&text);
            assert!(report.history.is_some());
            for e in &report.errors {
                assert_ne!(e.code(), "", "round {round}");
                assert_ne!(e.hint(), "", "round {round}: {e:?}");
                assert_ne!(e.to_string(), "");
            }
            if report.errors.is_empty() {
                loaded_ok += 1;
            } else {
                assert!(report.fell_back);
                assert!(identical(session.patch(), &Patch::init()), "round {round}");
            }
        }
        if as_instance {
            let report = session.load_instance(&text);
            for e in &report.errors {
                assert_ne!(e.code(), "", "round {round}");
                assert_ne!(e.hint(), "", "round {round}: {e:?}");
            }
            if !report.errors.is_empty() {
                assert_eq!(session.instance(), &InstanceState::default());
            }
        }
        assert_usable(&mut session, &format!("round {round}"));
    }
    assert!(
        loaded_ok > 0,
        "some mutations are harmless (whitespace, a duplicated key)"
    );
}

#[test]
fn a_truncated_state_blob_is_refused_at_every_offset() {
    for (name, text) in [
        ("patch", read(STATE_VERSION, "custom", "patch")),
        ("enth", read(STATE_VERSION, "custom", "enth")),
        ("v0", read(0, "lanes", "patch")),
    ] {
        // (Cutting only trailing whitespace leaves a whole file.)
        for end in (0..text.trim_end().len()).filter(|&i| text.is_char_boundary(i)) {
            let cut = &text[..end];
            let mut session = Session::new(common::custom_patch());
            let before = session.patch().clone();
            if name == "enth" {
                let report = session.load_instance(cut);
                assert_eq!(report.errors.len(), 1, "{name}[..{end}]");
                assert_eq!(session.instance(), &InstanceState::default());
                assert!(identical(session.patch(), &before));
            } else {
                let report = session.load_patch(cut);
                assert!(report.fell_back, "{name}[..{end}]");
                assert!(identical(session.patch(), &Patch::init()));
            }
        }
    }
}

#[test]
fn hostile_shapes_are_typed_errors_not_panics() {
    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    let wide = format!("{{\"schema\":1,\"x\":[{}]}}", "0,".repeat(200_000) + "0");
    let texts = [
        deep.as_str(),
        wide.as_str(),
        "\u{0}",
        "null",
        "true",
        "123",
        "\"str\"",
        r#"{"schema":null}"#,
        r#"{"schema":{"a":1}}"#,
        r#"{"schema":-1}"#,
        r#"{"schema":1.5}"#,
        r#"{"schema":4294967296}"#,
        r#"{"schema":1,"generators":null}"#,
        r#"{"schema":1,"slots":[1,2,3]}"#,
        r#"{"schema":1,"macros":{"1":{"value":NaN}}}"#,
        r#"{"schema":"enth.plugin-state/1","ui":{"scale_percent":99999999}}"#,
        r#"{"schema":"enth.plugin-state/1","ui":{"layout":null}}"#,
    ];
    for text in texts {
        let mut session = Session::default();
        let a = session.load_patch(text);
        let b = session.load_instance(text);
        // Whatever they were, neither panicked, and the session still compiles and saves.
        for e in a.errors.iter().chain(&b.errors) {
            assert_ne!(e.hint(), "", "{:?}", &text[..text.len().min(60)]);
        }
        assert_usable(&mut session, &text[..text.len().min(60)]);
    }
}
