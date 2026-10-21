//! `enth create` (lane SK3, freeze point F2s): a sound file in, K candidates and an `enth.create/1` record out. The
//! budgets are tiny (a debug build renders slowly): these tests check the plumbing; `enth-solver`'s tests check the
//! search, and lane CLC adds the schema snapshots.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, json, run, run_tty};
use enth_solver::create::record::CreateRecord;
use serde_json::Value;

fn fixture(name: &str) -> String {
    format!(
        "{}/../enth-plugin/tests/fixtures/create/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// A pluck at A3 rendered to a WAV (24-bit).
fn pluck_wav(s: &Scratch) -> String {
    let patch = s.new_patch("pluck_1osc", "pluck.patch.json");
    let wav = s.path("pluck.wav");
    let o = run(&[
        "render", &patch, "--note", "A3", "--hold", "0.3s", "--dur", "1s", "--bits", "24", "--out", &wav,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    wav
}

/// The record without what depends on the clock.
fn timeless(v: &Value) -> Value {
    let mut v = v.clone();
    if let Value::Object(m) = &mut v {
        m.remove("elapsed_s");
        if let Some(Value::Array(rounds)) = m.get_mut("rounds") {
            for r in rounds {
                if let Value::Object(r) = r {
                    r.remove("seconds");
                }
            }
        }
    }
    v
}

#[test]
fn a_wav_becomes_k_candidates_and_an_enth_create_record() {
    let s = Scratch::new("create_wav");
    let wav = pluck_wav(&s);
    let record = s.path("run.json");
    let best = s.path("best.patch.json");
    let o = run(&[
        "create", "--target", &wav, "--candidates", "3", "--budget", "60", "--record", &record, "--out", &best,
    ]);
    assert_eq!(o.code, 0, "{}\n{}", o.stdout, o.stderr);
    let v = json(&o);
    assert_eq!(v["schema"], "enth.create/1");
    assert_eq!(v["decoded"], "WAV, 24-bit, 48000 Hz, stereo");
    let r: CreateRecord = serde_json::from_value(v.clone()).expect("the record reads back");
    assert_eq!(r.candidates.len(), 3);
    assert!(r.renders <= 60, "{}", r.renders);
    assert_eq!(serde_json::to_value(r.stop).unwrap(), "budget");
    assert!(r.starts.len() >= 4, "the heuristic and three templates: {}", r.starts.len());
    assert_eq!(r.starts.len(), r.starts().unwrap().starts.len(), "every start is a patch");
    assert_eq!(r.settings.k, 3);
    assert_eq!(r.settings.objective, "logmel-l1/1");
    assert!(r.settings.bank.is_none());
    assert_eq!(r.input.audio_sha256.len(), 64);
    assert_eq!(r.input.perf.key, 57, "A3");
    let intake = r.intake.as_ref().expect("the intake report, whole");
    assert_eq!(
        serde_json::to_value(intake.cfg.codec).unwrap()["codec"],
        "pcm",
        "the decoder's hint"
    );
    assert!(r.rounds.iter().all(|r| r.seconds.is_some()), "every round stamped");
    assert!(r.notes.iter().any(|n| n.contains("no --bank")), "{:?}", r.notes);
    assert!(o.stderr.contains("no --bank"), "{}", o.stderr);
    // The record file is the printed record.
    let file: Value = serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    assert_eq!(file, v);
    // The best patch, named after the file.
    let patch: Value = serde_json::from_str(&std::fs::read_to_string(&best).unwrap()).unwrap();
    assert_eq!(patch["meta"]["name"], "pluck", "{patch}");
    // A terminal gets the table.
    let o = run_tty(&["create", "--target", &wav, "--candidates", "2", "--budget", "20"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("rank  branch"), "{}", o.stdout);
    assert!(o.stdout.contains("note A3"), "{}", o.stdout);
}

#[test]
fn the_result_is_the_same_for_any_worker_count() {
    let s = Scratch::new("create_workers");
    let wav = pluck_wav(&s);
    let go = |w: &str| {
        let o = run(&[
            "create", "--target", &wav, "--candidates", "3", "--budget", "50", "--workers", w,
        ]);
        assert_eq!(o.code, 0, "{}", o.stdout);
        timeless(&json(&o))
    };
    assert_eq!(go("1"), go("3"));
}

#[test]
fn flac_ogg_and_mp3_read_through_the_plugins_decoder() {
    for (name, decoded, lossy) in [
        ("tone.flac", "FLAC, 16-bit", false),
        ("tone.ogg", "Ogg Vorbis", true),
        ("tone.mp3", "MP3", true),
    ] {
        let o = run(&[
            "create", "--target", &fixture(name), "--candidates", "2", "--budget", "12",
        ]);
        assert_eq!(o.code, 0, "{name}: {}", o.stdout);
        let v = json(&o);
        assert!(v["decoded"].as_str().unwrap().starts_with(decoded), "{name}: {}", v["decoded"]);
        assert_eq!(v["intake"]["cfg"]["codec"]["lossy"], lossy, "{name}");
        assert_eq!(v["input"]["perf"]["key"], 57, "{name}: A3");
    }
}

#[test]
fn the_flags_override_the_inferred_key_velocity_and_hold() {
    let s = Scratch::new("create_perf");
    let wav = pluck_wav(&s);
    let o = run(&[
        "create", "--target", &wav, "--candidates", "2", "--budget", "12", "--note", "C4", "--vel", "64", "--hold",
        "0.2s",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let r: CreateRecord = serde_json::from_value(json(&o)).unwrap();
    assert_eq!(r.input.perf.key, 60);
    assert!((r.input.perf.velocity - 64.0 / 127.0).abs() < 1e-6);
    assert!((r.input.perf.hold_s - 0.2).abs() < 1e-9);
    assert_eq!(r.intake.unwrap().perf.key, 57, "the intake's own reading stays in its report");
    assert!(r.notes.iter().any(|n| n.contains("performance overridden")), "{:?}", r.notes);
}

#[test]
fn the_seconds_cap_stops_the_job_and_says_so() {
    let s = Scratch::new("create_cap");
    let wav = pluck_wav(&s);
    let o = run(&[
        "create", "--target", &wav, "--candidates", "8", "--budget", "1000000", "--seconds", "1",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let v = json(&o);
    assert_eq!(v["stop"], "cancelled");
    assert!(
        v["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("--seconds cap")),
        "{}",
        v["notes"]
    );
    let elapsed = v["elapsed_s"].as_f64().unwrap();
    assert!(elapsed < 2.5, "the cap plus about one render: {elapsed}");
}

#[test]
fn errors_carry_a_code_and_a_hint() {
    let s = Scratch::new("create_errors");
    let junk = s.write("junk.wav", "not a sound");
    let wav = pluck_wav(&s);
    let bad_bank = s.write("bank.bin", "ENTHBANK? no");
    for (args, exit, code) in [
        (vec!["create", "--target", "/no/such/file.wav"], 5, "file_not_found"),
        (vec!["create", "--target", junk.as_str()], 3, "unknown_format"),
        (vec!["create", "--target", wav.as_str(), "--candidates", "0"], 2, "bad_candidates"),
        (vec!["create", "--target", wav.as_str(), "--seconds", "301"], 2, "bad_seconds"),
        (vec!["create", "--target", wav.as_str(), "--budget", "0"], 2, "bad_budget"),
        (vec!["create", "--target", wav.as_str(), "--workers", "0"], 2, "bad_workers"),
    ] {
        let o = run(&args);
        let v = json(&o);
        assert_eq!((o.code, v["error"].as_str().unwrap()), (exit, code), "{args:?}: {}", o.stdout);
        assert_ne!(v["hint"].as_str().unwrap(), "", "{args:?}");
    }
    let o = run(&["create", "--target", &wav, "--bank", &bad_bank]);
    assert_eq!(o.code, 3, "{}", o.stdout);
    assert_ne!(json(&o)["hint"].as_str().unwrap(), "");
}

#[test]
fn create_is_listed_in_help() {
    let help = run(&["--help"]).stdout;
    assert!(help.contains("create"), "{help}");
}
