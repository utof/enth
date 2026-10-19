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
