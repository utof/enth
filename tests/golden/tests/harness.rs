//! The harness itself, end to end on a scratch directory: every failing case is reported in one run, `--update`
//! lists what it changed, and nothing is updated under `CI`.
#![cfg(not(target_family = "wasm"))] // a WASI sandbox only sees the preopened golden directory

use std::path::{Path, PathBuf};

use enth_golden::{Options, Status, platform_key, run};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("enth-golden-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("cases")).expect("create cases dir");
    std::fs::create_dir_all(dir.join("expected")).expect("create expected dir");
    dir
}

fn sine_case(dir: &Path, name: &str, hz: f64) {
    let text = format!(
        "description = \"scratch\"\nsample_rate = 48000\nseconds = 0.25\n\n[source]\nkind = \"sine\"\nfrequency_hz = {hz:?}\namplitude = 0.5\n"
    );
    std::fs::write(dir.join("cases").join(format!("{name}.toml")), text).expect("write case");
}

fn options(dir: &Path, update: bool) -> Options {
    Options {
        dir: dir.to_owned(),
        update,
        platform: platform_key(),
    }
}

fn statuses(report: &enth_golden::Report) -> Vec<(&str, &str)> {
    report
        .cases
        .iter()
        .map(|c| {
            let s = match c.status {
                Status::Ok => "ok",
                Status::Created => "created",
                Status::Updated => "updated",
                Status::Failed(_) => "failed",
            };
            (c.name.as_str(), s)
        })
        .collect()
}

#[test]
fn every_failing_case_is_reported_in_one_run() {
    if std::env::var_os("CI").is_some() {
        // Creating the expected files needs `--update`, which CI refuses by design (checked below).
        let dir = scratch("refuse");
        sine_case(&dir, "a", 440.0);
        let why = run(&options(&dir, true)).expect_err("CI must refuse --update");
        assert!(why.contains("refusing to update goldens under CI"), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let dir = scratch("report");
    for (name, hz) in [("a-good", 440.0), ("b-moves", 440.0), ("c-no-file", 440.0)] {
        sine_case(&dir, name, hz);
    }
    let created = run(&options(&dir, true)).expect("update runs");
    assert_eq!(
        statuses(&created),
        [
            ("a-good", "created"),
            ("b-moves", "created"),
            ("c-no-file", "created")
        ]
    );
    assert_eq!(created.changed(), ["a-good", "b-moves", "c-no-file"]);
    assert!(
        created.render().contains("  - b-moves"),
        "{}",
        created.render()
    );

    // An unchanged tree changes nothing.
    let again = run(&options(&dir, true)).expect("update runs");
    assert_eq!(again.changed(), Vec::<&str>::new());

    // Break two cases in different ways, and leave a stale expected file behind.
    sine_case(&dir, "b-moves", 880.0);
    std::fs::remove_file(dir.join("expected/c-no-file.toml")).expect("remove expected");
    std::fs::copy(
        dir.join("expected/a-good.toml"),
        dir.join("expected/d-stale.toml"),
    )
    .expect("copy expected");
    let report = run(&options(&dir, false)).expect("check runs");
    assert!(!report.is_ok());
    assert_eq!(
        statuses(&report),
        [
            ("a-good", "ok"),
            ("b-moves", "failed"),
            ("c-no-file", "failed")
        ]
    );
    assert_eq!(report.stale, ["d-stale"]);
    let text = report.render();
    assert!(text.contains("hash differs on"), "{text}");
    assert!(text.contains("the sound changed"), "{text}");
    assert!(text.contains("centroid_hz"), "{text}");
    assert!(text.contains("c-no-file.toml is missing"), "{text}");
    assert!(text.contains("STALE"), "{text}");
    assert!(text.contains("2 of 3 case(s) failed"), "{text}");

    // Updating fixes the moved case, creates the missing one and removes the stale file.
    let fixed = run(&options(&dir, true)).expect("update runs");
    assert_eq!(fixed.changed(), ["b-moves", "c-no-file"]);
    assert_eq!(fixed.stale, ["d-stale"]);
    assert!(!dir.join("expected/d-stale.toml").exists());
    assert!(run(&options(&dir, false)).expect("check runs").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_directory_without_cases_is_an_error_not_a_pass() {
    let dir = scratch("empty");
    let why = run(&options(&dir, false)).expect_err("no cases must not pass");
    assert!(why.contains("no golden cases"), "{why}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_malformed_case_fails_that_case_only() {
    let dir = scratch("malformed");
    sine_case(&dir, "a-good", 440.0);
    std::fs::write(dir.join("cases/b-bad.toml"), "sample_rate = \"fast\"\n").expect("write case");
    let report = run(&options(&dir, false)).expect("check runs");
    let bad = report
        .cases
        .iter()
        .find(|c| c.name == "b-bad")
        .expect("b-bad is reported");
    assert!(matches!(bad.status, Status::Failed(_)), "{bad:?}");
    assert!(report.render().contains("FAILED   b-bad"));
    let _ = std::fs::remove_dir_all(&dir);
}
