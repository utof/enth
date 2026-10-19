//! One snapshot per error class, each error deserialized back into `enth.error/1`, and the exit code of every
//! class (docs/agent-cli.md: 2 usage, 3 validation, 4 render or analysis, 5 file not found).
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, record, round_trip, run, run_tty};
use enth_cli::schema::ErrorOut;
use insta::assert_snapshot;

/// Run, check the exit code and the error code, round-trip the error, and return the snapshot text.
fn err(s: &Scratch, args: &[&str], exit: i32, code: &str) -> String {
    let o = run(args);
    let e: ErrorOut = round_trip(&o);
    assert_eq!(
        (o.code, e.exit, e.error.as_str()),
        (exit, exit, code),
        "{}",
        o.stdout
    );
    assert_ne!(e.hint, "");
    record(s, &o, false)
}

#[test]
fn usage_errors_exit_2() {
    let s = Scratch::new("err_usage");
    let p = s.new_patch("init", "p.patch.json");
    s.settings().bind(|| {
        assert_snapshot!("err_unknown_command", err(&s, &["chek"], 2, "usage"));
        assert_snapshot!(
            "err_unknown_flag",
            err(&s, &["check", &p, "--expct", "x"], 2, "usage")
        );
        assert_snapshot!("err_missing_arg", err(&s, &["check", &p], 2, "usage"));
        assert_snapshot!(
            "err_bad_value",
            err(&s, &["render", &p, "--bits", "12"], 2, "usage")
        );
        assert_snapshot!(
            "err_nothing_to_apply",
            err(&s, &["apply", &p], 2, "nothing_to_apply")
        );
        assert_snapshot!(
            "err_bad_expect",
            err(
                &s,
                &["check", &p, "--expect", "attack << 10ms"],
                2,
                "bad_expect"
            )
        );
        assert_snapshot!(
            "err_bad_perf",
            err(
                &s,
                &[
                    "check",
                    &p,
                    "--expect",
                    "attack < 10ms",
                    "--perf",
                    "note=H9"
                ],
                2,
                "bad_perf"
            )
        );
        assert_snapshot!("err_new_without_template", err(&s, &["new"], 2, "usage"));
    });
}

#[test]
fn validation_errors_exit_3_and_suggest() {
    let s = Scratch::new("err_validation");
    let p = s.new_patch("pluck_1osc", "p.patch.json");
    s.settings().bind(|| {
        // The alias table: `detune` is `fine` on an oscillator.
        assert_snapshot!(
            "err_alias_detune",
            err(
                &s,
                &["apply", &p, "--set", "osc1.detune=5ct"],
                3,
                "unknown_param"
            )
        );
        // `res` is no key of any kind: alias and near miss.
        assert_snapshot!(
            "err_alias_res",
            err(
                &s,
                &["apply", &p, "--set", "filter1.res=40"],
                3,
                "unknown_param"
            )
        );
        assert_snapshot!(
            "err_typo_param",
            err(
                &s,
                &["apply", &p, "--set", "filter1.cutof=900Hz"],
                3,
                "unknown_param"
            )
        );
        // A module alias: `amp` is the amplitude envelope.
        assert_snapshot!(
            "err_module_alias",
            err(
                &s,
                &["apply", &p, "--set", "amp.attack=5ms"],
                3,
                "unknown_module"
            )
        );
        assert_snapshot!(
            "err_typo_module",
            err(&s, &["get", &p, "filtr1.cutoff"], 3, "unknown_module")
        );
        assert_snapshot!(
            "err_out_of_range",
            err(
                &s,
                &["apply", &p, "--set", "env1.decay=20s"],
                3,
                "out_of_range"
            )
        );
        assert_snapshot!(
            "err_wrong_unit",
            err(
                &s,
                &["apply", &p, "--set", "filter1.cutoff=40ms"],
                3,
                "wrong_unit"
            )
        );
        assert_snapshot!(
            "err_unknown_op",
            err(
                &s,
                &["apply", &p, "--ops", r#"[{"sett": "osc1.fine", "to": 1}]"#],
                3,
                "unknown_op"
            )
        );
        assert_snapshot!(
            "err_bad_ops_json",
            err(&s, &["apply", &p, "--ops", "[{"], 3, "bad_json")
        );
        assert_snapshot!(
            "err_unknown_feature",
            err(
                &s,
                &["check", &p, "--expect", "brightnes > 2kHz"],
                3,
                "unknown_feature"
            )
        );
        assert_snapshot!(
            "err_unknown_template",
            err(
                &s,
                &[
                    "new",
                    "--template",
                    "pluk",
                    "--out",
                    &s.path("x.patch.json")
                ],
                3,
                "unknown_template"
            )
        );
        assert_snapshot!(
            "err_unknown_kind",
            err(&s, &["params", "filtr"], 3, "unknown_kind")
        );
        assert_snapshot!(
            "err_unknown_topic",
            err(&s, &["guide", "--topic", "op"], 3, "unknown_topic")
        );
        assert_snapshot!(
            "err_unknown_schema",
            err(&s, &["schema", "chek"], 3, "unknown_schema")
        );
    });
}

#[test]
fn bad_patch_files_report_every_problem_with_a_path() {
    let s = Scratch::new("err_files");
    let bad = s.write(
        "bad.patch.json",
        r#"{"schema": 1, "voice": {"amp_env": "env1"},
            "generators": [{"id": "osc1", "kind": "osc", "params": {"wave": "sawtooth", "level": 150}}],
            "modulators": [{"id": "env1", "kind": "adsr"}]}"#,
    );
    let not_json = s.write("broken.patch.json", "{\"schema\": 1,");
    let wav = s.path("p.wav");
    let p = s.new_patch("init", "p.patch.json");
    assert_eq!(run(&["render", &p, "--out", &wav]).code, 0);
    s.settings().bind(|| {
        let text = err(&s, &["get", &bad], 3, "bad_value");
        assert!(
            text.contains("\"errors\""),
            "both problems are listed: {text}"
        );
        assert_snapshot!("err_bad_patch", text);
        assert_snapshot!(
            "err_patch_not_json",
            err(&s, &["get", &not_json], 3, "bad_json")
        );
        assert_snapshot!(
            "err_wav_as_patch",
            err(
                &s,
                &["apply", &wav, "--set", "osc1.fine=1"],
                3,
                "not_a_patch"
            )
        );
    });
}

#[test]
fn render_write_and_missing_file_errors() {
    let s = Scratch::new("err_io");
    let p = s.new_patch("init", "p.patch.json");
    // Modulation routes render (S2b): a patch with an LFO routed to a pitch is not an error.
    let routed = s.path("routed.patch.json");
    let add = r#"[{"add": "lfo"}, {"route": "lfo1 -> osc1.fine", "depth": 10}]"#;
    assert_eq!(run(&["apply", &p, "--ops", add, "--out", &routed]).code, 0);
    let r = run(&["render", &routed]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    let not_a_wav = s.write("noise.wav", "RIFF\u{0}\u{0}\u{0}\u{0}WAVEjunk");
    s.settings().bind(|| {
        assert_snapshot!(
            "err_write_failed",
            err(
                &s,
                &["render", &p, "--out", &s.path("no/such/dir/p.wav")],
                4,
                "write_failed"
            )
        );
        assert_snapshot!(
            "err_unsupported_wav",
            err(&s, &["describe", &not_a_wav], 4, "unsupported_wav")
        );
        assert_snapshot!(
            "err_file_not_found",
            err(
                &s,
                &[
                    "check",
                    &s.path("missing.patch.json"),
                    "--expect",
                    "attack < 10ms"
                ],
                5,
                "file_not_found"
            )
        );
        assert_snapshot!(
            "err_ops_file_not_found",
            err(
                &s,
                &["apply", &p, "--ops", &format!("@{}", s.path("ops.json"))],
                5,
                "file_not_found"
            )
        );
    });
}

#[test]
fn errors_on_a_terminal_are_plain_text_on_stderr() {
    let s = Scratch::new("err_tty");
    let p = s.new_patch("init", "p.patch.json");
    let o = run_tty(&["apply", &p, "--set", "osc1.detune=3"]);
    assert_eq!(o.code, 3);
    assert_eq!(o.stdout, "");
    s.settings()
        .bind(|| assert_snapshot!("err_table", s.clean(&o.stderr)));
}

#[test]
fn help_and_version_exit_0() {
    let h = run(&["--help"]);
    assert_eq!(h.code, 0);
    assert!(h.stdout.contains("enth guide"));
    let v = run(&["--version"]);
    assert_eq!(v.code, 0);
    assert!(v.stdout.starts_with("enth "));
    let c = run(&["check", "--help"]);
    assert_eq!(c.code, 0);
    assert!(c.stdout.contains("--expect"));
}
