//! End-to-end tests that spawn the built `enth` binary: output format by TTY (a pipe is not a terminal, so JSON),
//! stdin, exit codes, and a whole agent loop on a template: `new` -> `step` -> `check`.
#![allow(
    clippy::disallowed_methods,
    reason = "tests spawn the binary and read scratch files"
)]

mod common;

use std::process::Command;

use common::Scratch;
use enth_cli::schema::{CheckOut, ErrorOut, NewOut, StepOut};
use insta_cmd::{assert_cmd_snapshot, get_cargo_bin};

fn enth() -> Command {
    let mut c = Command::new(get_cargo_bin("enth"));
    c.env_remove("NO_COLOR");
    c
}

/// Run the binary, returning (exit code, stdout).
fn spawn(args: &[&str], stdin: Option<&str>) -> (i32, String) {
    use std::io::Write;
    let mut cmd = enth();
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
    )
}

#[test]
fn a_pipe_gets_json_and_format_table_overrides() {
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    settings.bind(|| {
        assert_cmd_snapshot!("e2e_modules_json", enth().arg("modules"));
        assert_cmd_snapshot!(
            "e2e_modules_table",
            enth().args(["modules", "--format", "table"])
        );
        assert_cmd_snapshot!("e2e_usage_error", enth().arg("chek"));
    });
}

#[test]
fn exit_codes_are_stable() {
    let s = Scratch::new("e2e_exit");
    let p = s.new_patch("init", "p.patch.json");
    let missing = s.path("missing.patch.json");
    let cases: &[(&[&str], i32)] = &[
        (&["guide"], 0),
        (&["check", &p, "--expect", "pitch.note ~ C3"], 0),
        (&["check", &p, "--expect", "brightness > 18kHz"], 1),
        (&["check", &p], 2),
        (&["check", &p, "--expect", "attack ?? 3"], 2),
        (&["apply", &p, "--set", "osc1.detune=3"], 3),
        (&["apply", &p, "--set", "osc1.fine=300"], 3),
        (&["render", &p, "--out", &s.path("no/dir/x.wav")], 4),
        (&["get", &missing], 5),
    ];
    for (args, want) in cases {
        let (code, out) = spawn(args, None);
        assert_eq!(code, *want, "{args:?}: {out}");
        let v: serde_json::Value =
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
        if *want >= 2 {
            let e: ErrorOut = serde_json::from_value(v).unwrap();
            assert_eq!(e.exit, *want);
        }
    }
}

#[test]
fn ops_from_stdin() {
    let s = Scratch::new("e2e_stdin");
    let p = s.new_patch("init", "p.patch.json");
    let (code, out) = spawn(
        &["apply", &p, "--ops", "-", "--dry-run"],
        Some(r#"[{"set": "osc1.fine", "to": "7ct"}]"#),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("\"target\":\"osc1.fine\""), "{out}");
}

/// The agent loop on a template, through the real binary: start a pluck, take one step towards a goal with the
/// goal's performance, and have the judge confirm it; then a list that cannot pass exits 1 with a reason per item.
#[test]
fn new_step_check_on_a_template() {
    let s = Scratch::new("e2e_loop");
    let patch = s.path("pluck.patch.json");
    let (code, out) = spawn(&["new", "--template", "pluck_1osc", "--out", &patch], None);
    assert_eq!(code, 0, "{out}");
    let n: NewOut = serde_json::from_str(&out).unwrap();
    assert_eq!(n.template, "pluck_1osc");

    let perf = r#"{"notes": [{"note": "A3", "vel": 100, "on_s": 0, "off_s": 1.0}], "dur_s": 2.0}"#;
    let goal = "pitch.note ~ A3, attack < 30ms, sustain < -30dB, duration < 1500ms";
    let next = s.path("pluck2.patch.json");
    let (code, out) = spawn(
        &[
            "step",
            &patch,
            "--set",
            "env1.decay=250ms,env1.release=100ms",
            "--expect",
            goal,
            "--perf",
            perf,
            "--out",
            &next,
        ],
        None,
    );
    assert_eq!(code, 0, "{out}");
    let st: StepOut = serde_json::from_str(&out).unwrap();
    let c = st.check.unwrap();
    assert!(c.pass, "{:?}", c.results);
    assert_eq!(st.applied.len(), 2);
    assert_eq!(st.perf.notes[0].note.as_deref(), Some("A3"));
    assert!(std::path::Path::new(&st.wav).exists());

    // The judge: the same list and performance on the written patch.
    let (code, out) = spawn(&["check", &next, "--expect", goal, "--perf", perf], None);
    assert_eq!(code, 0, "{out}");
    let judged: CheckOut = serde_json::from_str(&out).unwrap();
    assert!(judged.pass);
    assert_eq!(judged.results.len(), 4);
    for (a, b) in judged.results.iter().zip(&c.results) {
        assert_eq!(
            a.got, b.got,
            "step and check measure the same: {}",
            a.expect
        );
    }

    // The WAV step wrote carries the performance, so describe reads it without being told.
    let (code, out) = spawn(&["describe", &st.wav], None);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("\"source\":\"wav\""), "{out}");

    // An impossible list: exit 1, the failing items say why or by how much.
    let (code, out) = spawn(
        &[
            "check",
            &next,
            "--expect",
            "beating in [3Hz, 9Hz], sustain > -3dB",
            "--perf",
            perf,
        ],
        None,
    );
    assert_eq!(code, 1, "{out}");
    let failed: CheckOut = serde_json::from_str(&out).unwrap();
    assert_eq!(failed.failed, 2);
    assert!(failed.results[0].why.is_some(), "no beating: why");
    assert!(failed.results[1].margin.is_some_and(|m| m < 0.0));
}

/// The loop the hints exist for, through the real binary: a `step` that fails on a goal returns a measured `fix`;
/// applying exactly its ops passes, reads what the fix said it would, and the judge agrees.
#[test]
fn a_failing_step_returns_a_fix_that_passes_when_applied() {
    let s = Scratch::new("e2e_fix");
    let patch = s.new_patch("pluck_1osc", "pluck.patch.json");
    let perf = r#"{"notes": [{"note": "A3", "vel": 100, "on_s": 0, "off_s": 1.0}], "dur_s": 2.0}"#;
    let goal = "pitch.note ~ A3, attack < 30ms, brightness < 900Hz, duration > 900ms";

    // An edit that does not reach the goal: the step fails, exit 1, and says how to get there.
    let tried = s.path("tried.patch.json");
    let (code, out) = spawn(
        &[
            "step",
            &patch,
            "--set",
            "filter1.resonance=30%",
            "--expect",
            goal,
            "--perf",
            perf,
            "--out",
            &tried,
        ],
        None,
    );
    assert_eq!(code, 1, "{out}");
    let st: StepOut = serde_json::from_str(&out).unwrap();
    let c = st.check.unwrap();
    assert!(!c.pass && c.failed == 2, "{:?}", c.results);
    let fix = c.fix.expect("the failing step returns a fix");
    assert!(fix.measured && fix.pass_all, "{fix:?}");
    assert_eq!(c.blocked.len(), 0, "{:?}", c.blocked);
    assert!(fix.renders <= 16, "inside the default budget");

    // Apply exactly those ops: the goal passes.
    let ops = serde_json::to_string(&fix.ops).unwrap();
    let fixed = s.path("fixed.patch.json");
    let (code, out) = spawn(
        &[
            "step", &tried, "--ops", &ops, "--expect", goal, "--perf", perf, "--out", &fixed,
        ],
        None,
    );
    assert_eq!(code, 0, "the fix passes when applied: {out}");
    let applied: StepOut = serde_json::from_str(&out).unwrap();
    let after = applied.check.unwrap();
    assert!(after.pass && after.fix.is_none() && after.blocked.is_empty());
    // `after` is a real render: the readings the fix promised are the ones the step measured.
    for r in &after.results {
        if let Some(promised) = fix.after.get(feature_path(&r.expect)) {
            let got = match r.got.as_ref().unwrap() {
                enth_cli::schema::Reading::Num(n) => *n,
                other => panic!("{other:?}"),
            };
            let enth_cli::schema::Reading::Num(promised) = promised else {
                panic!("{promised:?}");
            };
            assert_eq!(got, *promised, "{}", r.expect);
        }
    }

    // The judge: the same list and performance on the written patch.
    let (code, out) = spawn(&["check", &fixed, "--expect", goal, "--perf", perf], None);
    assert_eq!(code, 0, "{out}");
}

/// The canonical form starts with the feature's path (`spectrum.centroid_hz < 900`).
fn feature_path(canonical: &str) -> &str {
    canonical.split_whitespace().next().unwrap()
}
