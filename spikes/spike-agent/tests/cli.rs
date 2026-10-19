//! The binary end to end: JSON with a `schema` on every call, stable exit codes, errors that teach, files in and
//! out. Short clips keep the debug build quick. Nothing here plays audio.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const SHORT: &str = "note=A3,dur=1s,hold=0.7s";

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("spike-agent-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run the CLI; returns the exit code, the parsed single-line JSON object and the raw stdout.
fn run(args: &[&str]) -> (i32, Value, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_spike-agent"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        text.lines().count(),
        1,
        "one JSON object per call: {text:?} (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"));
    (out.status.code().unwrap(), v, text)
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn new_patch(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    let (code, v, _) = run(&["new", "--out", s(&p), "--perf", SHORT]);
    assert_eq!(code, 0, "{v}");
    p
}

fn assert_schema(v: &Value, name: &str) {
    assert_eq!(v["schema"], format!("enth.{name}/1"), "{v}");
}

#[test]
fn guide_is_a_one_page_workflow_in_json() {
    let (code, v, text) = run(&["guide"]);
    assert_eq!(code, 0);
    assert_schema(&v, "guide");
    assert!(v["workflow"].as_array().unwrap().len() >= 5);
    assert!(v["commands"].as_array().unwrap().len() >= 8);
    let desc = v["expectations"]["descriptors"].as_array().unwrap();
    assert_eq!(desc.len(), 11);
    for d in desc {
        assert!(d["meaning"].as_str().unwrap().len() > 20, "{d}");
        assert!(d["unit"].is_string());
    }
    for code in ["0", "1", "2", "3", "4", "5"] {
        assert!(
            v["exit_codes"][code].is_string(),
            "exit code {code} documented"
        );
    }
    assert!(
        text.len() < 9_000,
        "a guide is a page, not a manual: {} bytes",
        text.len()
    );
}

#[test]
fn params_lists_every_key_with_unit_range_and_effect() {
    let (code, v, _) = run(&["params"]);
    assert_eq!(code, 0);
    assert_schema(&v, "params");
    let params = v["params"].as_array().unwrap();
    assert_eq!(params.len(), 10, "the toy synth's ten parameters");
    let find = |key: &str| {
        params
            .iter()
            .find(|p| p["key"] == key)
            .unwrap_or_else(|| panic!("{key} listed"))
    };
    assert_eq!(find("filter.cutoff")["unit"], "Hz");
    assert_eq!(find("amp.attack")["unit"], "ms");
    assert_eq!(find("amp.sustain")["unit"], "dB");
    assert_eq!(find("osc.detune")["unit"], "ct");
    assert_eq!(find("osc.wave")["values"].as_array().unwrap().len(), 3);
    for p in params {
        assert!(p["effect"].as_str().unwrap().len() > 15, "{p}");
        assert!(p["default"].is_number() || p["default"].is_string());
    }
    assert_eq!(v["performance"].as_array().unwrap().len(), 3);
}

#[test]
fn every_command_prints_one_schema_tagged_object() {
    let dir = tmp("schemas");
    let p = new_patch(&dir, "p.json");
    let wav = dir.join("p.wav");
    let (c, v, _) = run(&["new", "--out", s(&dir.join("q.json"))]);
    assert_eq!(c, 0);
    assert_schema(&v, "new");
    let (c, v, _) = run(&[
        "apply",
        s(&p),
        "--set",
        "filter.cutoff=800Hz,amp.attack=40ms",
    ]);
    assert_eq!(c, 0);
    assert_schema(&v, "apply");
    let (c, v, _) = run(&["render", s(&p), "--out", s(&wav)]);
    assert_eq!((c, wav.exists()), (0, true));
    assert_schema(&v, "render");
    let (c, v, _) = run(&["describe", s(&p)]);
    assert_eq!(c, 0);
    assert_schema(&v, "describe");
    let (c, v, _) = run(&["check", s(&p), "--expect", "attack>20ms"]);
    assert_eq!(c, 0);
    assert_schema(&v, "check");
    let (c, v, _) = run(&[
        "step",
        s(&p),
        "--set",
        "amp.attack=60ms",
        "--expect",
        "attack>20ms",
    ]);
    assert_eq!(c, 0);
    assert_schema(&v, "step");
    let (c, v, _) = run(&["check", s(&p), "--expect", "attack<2ms"]);
    assert_eq!(c, 1);
    assert_schema(&v, "check");
    let (c, v, _) = run(&["apply", s(&p), "--set", "nope=1"]);
    assert_eq!(c, 3);
    assert_schema(&v, "error");
}

#[test]
fn apply_parses_units_and_diffs_in_real_units() {
    let dir = tmp("apply");
    let p = new_patch(&dir, "p.json");
    let (code, v, _) = run(&[
        "apply",
        s(&p),
        "--set",
        "filter.cutoff=1.2kHz,amp.attack=0.04s,amp.sustain=-12dB,osc.detune=12ct,osc.wave=triangle,perf.note=C3",
    ]);
    assert_eq!(code, 0, "{v}");
    let changed = v["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 6);
    assert_eq!(changed[0]["param"], "filter.cutoff");
    assert_eq!(
        (changed[0]["from"].as_str(), changed[0]["to"].as_str()),
        (Some("2kHz"), Some("1.2kHz"))
    );
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(doc["params"]["filter.cutoff"], 1200.0);
    assert_eq!(doc["params"]["amp.attack"], 40.0);
    assert_eq!(doc["params"]["osc.wave"], "triangle");
    assert_eq!(doc["perf"]["note"], "C3");
    // --out keeps the original; --dry-run writes nothing
    let before = std::fs::read_to_string(&p).unwrap();
    let q = dir.join("q.json");
    let (code, _, _) = run(&[
        "apply",
        s(&p),
        "--set",
        "filter.cutoff=500Hz",
        "--out",
        s(&q),
    ]);
    assert_eq!(code, 0);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
    assert!(q.exists());
    let r = dir.join("r.json");
    let (code, v, _) = run(&[
        "apply",
        s(&p),
        "--set",
        "filter.cutoff=500Hz",
        "--out",
        s(&r),
        "--dry-run",
    ]);
    assert_eq!(code, 0);
    assert!(
        !r.exists() && v["wrote"].is_null() && v["dry_run"] == true,
        "{v}"
    );
}

#[test]
fn errors_teach_and_exit_codes_are_stable() {
    let dir = tmp("errors");
    let p = new_patch(&dir, "p.json");
    // 3: validation
    let (c, v, _) = run(&["apply", s(&p), "--set", "filter.cutof=800Hz"]);
    assert_eq!((c, v["error"].as_str()), (3, Some("unknown_param")));
    assert_eq!(v["did_you_mean"][0], "filter.cutoff");
    assert!(v["hint"].is_string());
    let (c, v, _) = run(&["apply", s(&p), "--set", "filter.cutoff=40kHz"]);
    assert_eq!((c, v["error"].as_str()), (3, Some("out_of_range")));
    assert_eq!(v["max"], "16kHz");
    let (c, v, _) = run(&["apply", s(&p), "--set", "filter.cutoff=40ms"]);
    assert_eq!((c, v["error"].as_str()), (3, Some("bad_unit")));
    let (c, v, _) = run(&["apply", s(&p), "--set", "osc.wave=sine"]);
    assert_eq!((c, v["error"].as_str()), (3, Some("bad_value")));
    assert_eq!(v["allowed"][0], "saw");
    let (c, v, _) = run(&["check", s(&p), "--expect", "brigthness>2kHz"]);
    assert_eq!((c, v["error"].as_str()), (3, Some("unknown_feature")));
    assert_eq!(v["did_you_mean"][0], "brightness");
    let bad = dir.join("bad.json");
    std::fs::write(&bad, "{not json").unwrap();
    let (c, v, _) = run(&["describe", s(&bad)]);
    assert_eq!((c, v["error"].as_str()), (3, Some("bad_patch")));
    // 2: usage
    let (c, v, _) = run(&["apply", s(&p), "--set", "filter.cutoff"]);
    assert_eq!((c, v["error"].as_str()), (2, Some("bad_set")));
    let (c, v, _) = run(&["check", s(&p), "--expect", "attack 10"]);
    assert_eq!((c, v["error"].as_str()), (2, Some("bad_expect")));
    let (c, v, _) = run(&["check", s(&p), "--expect", "attack<10Hz"]);
    assert_eq!((c, v["error"].as_str()), (2, Some("bad_expect")));
    let (c, v, _) = run(&["check", s(&p)]);
    assert_eq!((c, v["error"].as_str()), (2, Some("usage")));
    let (c, v, _) = run(&["frobnicate"]);
    assert_eq!((c, v["error"].as_str()), (2, Some("usage")));
    // 5: not found
    let (c, v, _) = run(&["describe", s(&dir.join("missing.json"))]);
    assert_eq!((c, v["error"].as_str()), (5, Some("file_not_found")));
    let (c, _, _) = run(&[
        "check",
        s(&dir.join("missing.wav")),
        "--expect",
        "attack<10ms",
    ]);
    assert_eq!(c, 5);
    // 4: cannot analyse
    let junk = dir.join("junk.wav");
    std::fs::write(&junk, b"not a wav file at all, just bytes").unwrap();
    let (c, v, _) = run(&["describe", s(&junk)]);
    assert_eq!((c, v["error"].as_str()), (4, Some("unsupported_wav")));
    // a failed apply wrote nothing
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(doc["params"]["filter.cutoff"], 2000.0);
}

#[test]
fn check_exit_codes_follow_the_results() {
    let dir = tmp("check");
    let p = new_patch(&dir, "p.json");
    let (c, v, _) = run(&["check", s(&p), "--expect", "attack<20ms, sustain>-8dB"]);
    assert_eq!(
        (c, v["pass"].as_bool(), v["passed"].as_u64()),
        (0, Some(true), Some(2))
    );
    let (c, v, _) = run(&["check", s(&p), "--expect", "attack<20ms, brightness<300Hz"]);
    assert_eq!(
        (c, v["pass"].as_bool(), v["failed"].as_u64()),
        (1, Some(false), Some(1))
    );
    let results = v["results"].as_array().unwrap();
    assert_eq!(results[0]["pass"], true);
    assert!(results[0].get("hint").is_none());
    assert_eq!(results[1]["pass"], false);
    let hint = &results[1]["hint"];
    assert_eq!(hint["param"], "filter.cutoff");
    assert_eq!(hint["measured"], true);
    assert!(results[1]["got"].as_f64().unwrap() > 300.0);
}

#[test]
fn step_applies_renders_describes_and_checks_in_one_call() {
    let dir = tmp("step");
    let p = new_patch(&dir, "p.json");
    let out = dir.join("p2.json");
    let (c, v, text) = run(&[
        "step",
        s(&p),
        "--set",
        "amp.attack=80ms,filter.cutoff=900Hz",
        "--expect",
        "attack>50ms, brightness<1kHz",
        "--out",
        s(&out),
    ]);
    assert_eq!(c, 0, "{v}");
    assert_eq!(v["check"]["pass"], true);
    assert_eq!(v["applied"].as_array().unwrap().len(), 2);
    assert!(v["describe"]["envelope"]["attack_ms"].as_f64().unwrap() > 50.0);
    assert!(
        out.exists() && out.with_extension("wav").exists(),
        "patch and WAV are written"
    );
    assert_eq!(v["wav"], s(&out.with_extension("wav")));
    // the original is untouched, and the same step twice prints the same bytes
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(doc["params"]["amp.attack"], 5.0);
    let (_, _, again) = run(&[
        "step",
        s(&p),
        "--set",
        "amp.attack=80ms,filter.cutoff=900Hz",
        "--expect",
        "attack>50ms, brightness<1kHz",
        "--out",
        s(&out),
    ]);
    assert_eq!(text, again, "deterministic output");
    // a failing step exits 1 and still prints everything
    let (c, v, _) = run(&["step", s(&out), "--expect", "attack<5ms", "--out", s(&out)]);
    assert_eq!(c, 1);
    assert_eq!(v["check"]["pass"], false);
    assert_eq!(v["check"]["results"][0]["hint"]["param"], "amp.attack");
    // step without expectations just measures
    let (c, v, _) = run(&["step", s(&out), "--set", "osc.wave=square"]);
    assert_eq!(c, 0);
    assert!(v.get("check").is_none());
}

#[test]
fn a_wav_describes_like_the_patch_it_came_from_and_gets_no_hints() {
    let dir = tmp("wav");
    let p = new_patch(&dir, "p.json");
    let wav = dir.join("p.wav");
    let (c, v, _) = run(&["render", s(&p), "--out", s(&wav)]);
    assert_eq!(c, 0);
    assert_eq!(v["sample_rate"], 48_000);
    let bytes = std::fs::read(&wav).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    let (_, mut from_patch, _) = run(&["describe", s(&p)]);
    let (_, mut from_wav, _) = run(&["describe", s(&wav), "--perf", "hold=0.7s"]);
    from_patch.as_object_mut().unwrap().remove("source");
    from_wav.as_object_mut().unwrap().remove("source");
    assert_eq!(
        from_patch, from_wav,
        "describe(patch) == describe(its WAV): the descriptors measure the file"
    );
    // without hold the note-off descriptors are unknown, the rest is there
    let (_, v, _) = run(&["describe", s(&wav)]);
    assert!(v["envelope"]["sustain_db"].is_null() && v["envelope"]["release_ms"].is_null());
    assert!(v["spectrum"]["centroid_hz"].is_number());
    let (c, v, _) = run(&["check", s(&wav), "--expect", "brightness<300Hz"]);
    assert_eq!(c, 1);
    assert!(
        v["results"][0].get("hint").is_none(),
        "a WAV has no parameters to blame"
    );
}

#[test]
fn describe_explain_adds_a_meaning_for_each_descriptor() {
    let dir = tmp("explain");
    let p = new_patch(&dir, "p.json");
    let (_, v, _) = run(&["describe", s(&p), "--explain"]);
    let meaning = v["meaning"].as_object().unwrap();
    assert_eq!(meaning.len(), 11);
    assert!(
        meaning.contains_key("spectrum.centroid_hz") && meaning.contains_key("detune.beating_hz")
    );
    let (_, plain, _) = run(&["describe", s(&p)]);
    assert!(plain.get("meaning").is_none());
    // measured values are rounded to 4 significant digits
    for c in [
        plain["spectrum"]["centroid_hz"].as_f64().unwrap(),
        plain["loudness"]["lufs_i"].as_f64().unwrap(),
        plain["envelope"]["release_ms"].as_f64().unwrap(),
    ] {
        assert_eq!(
            spike_agent::units::r4(c),
            c,
            "{c} has more than 4 significant digits"
        );
    }
}

#[test]
fn perf_override_forces_the_performance_for_judging() {
    let dir = tmp("perf");
    let p = new_patch(&dir, "p.json");
    // the patch says A3; judging with --perf note=C3 measures a C3
    let (_, v, _) = run(&["describe", s(&p), "--perf", "note=C3"]);
    assert_eq!(v["pitch"]["note"], "C3");
    let (c, _, _) = run(&[
        "check",
        s(&p),
        "--perf",
        "note=C3",
        "--expect",
        "pitch=C3±20ct",
    ]);
    assert_eq!(c, 0);
    let (c, v, _) = run(&["check", s(&p), "--expect", "pitch=C3±20ct"]);
    assert_eq!(c, 1);
    assert_eq!(v["results"][0]["hint"]["try"], "C3");
    let (c, v, _) = run(&["describe", s(&p), "--perf", "hold=5s"]);
    assert_eq!(
        (c, v["error"].as_str()),
        (3, Some("bad_perf")),
        "hold beyond the clip is refused"
    );
}
