//! `enth create` (lane SK3, freeze point F2s; the flags, the typed schema and the snapshots are lane CLC's): a sound
//! file in, K candidates and an `enth.create/1` record out. The budgets are tiny (a debug build renders slowly): these
//! tests check the plumbing and pin the output's shape; `enth-solver`'s tests check the search.
//!
//! The snapshots mask what the DSP decides (distances, patches, which start won, the intake's numbers) and keep what
//! the flags and the schema decide, as the other commands' do; see [`mask_create`].
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, json, pretty, round_trip, run, run_tty};
use enth_cli::Output;
use enth_cli::schema::{self, CreateOut, CreateProvenanceOut, CreateStopOut, ErrorOut};
use enth_solver::create::record::CreateRecord;
use insta::assert_snapshot;
use serde_json::Value;

fn fixture(name: &str) -> String {
    format!(
        "{}/../enth-plugin/tests/fixtures/create/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// `tone.wav` (SK3's fixture: 0.6 s of A3, 16-bit mono at 48 kHz, the intake keeps 0.375 s of it) copied into the
/// scratch directory, so a snapshot shows `[TMP]/tone.wav` on every machine.
fn tone(s: &Scratch) -> String {
    let to = s.path("tone.wav");
    std::fs::copy(fixture("tone.wav"), &to).unwrap();
    to
}

/// A pluck at A3 rendered to a WAV (24-bit).
fn pluck_wav(s: &Scratch) -> String {
    let patch = s.new_patch("pluck_1osc", "pluck.patch.json");
    let wav = s.path("pluck.wav");
    let o = run(&[
        "render", &patch, "--note", "A3", "--hold", "0.3s", "--dur", "1s", "--bits", "24", "--out",
        &wav,
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

/// Numbers a snapshot keeps: the settings the flags and the schema decide, and identifiers. Every other number is the
/// DSP's (distances, times, counts that follow the pruning) and reads `"<num>"`.
const KEEP: &[&str] = &[
    "k",
    "budget",
    "seed",
    "neighbours",
    "min_spread",
    "lambda",
    "first_rung",
    "eta",
    "finalists",
    "sample_rate",
    "render_seed",
    "channels",
    "version",
    "cap_s",
    "score_s",
    "stop_at",
    "index",
    "exit",
];

/// The output with the DSP-dependent parts masked: the patches (`"<patch>"`), the hashes, which start each candidate
/// came from and how its search ended, the numbers outside [`KEEP`], and the closest candidate in the hint.
fn mask_create(v: &Value) -> Value {
    fn go(parent: &str, key: &str, v: &Value) -> Value {
        match v {
            _ if key == "patch" => Value::from("<patch>"),
            _ if key.ends_with("_sha256") => Value::from("<sha256>"),
            _ if parent == "candidates" && matches!(key, "provenance" | "state") => {
                Value::from(format!("<{key}>"))
            }
            Value::Number(_) if !KEEP.contains(&key) => Value::from("<num>"),
            Value::String(t) if key == "hint" => Value::from(hide_closest(t)),
            Value::Array(a) => Value::Array(a.iter().map(|x| go(parent, key, x)).collect()),
            Value::Object(m) => {
                Value::Object(m.iter().map(|(k, x)| (k.clone(), go(key, k, x))).collect())
            }
            other => other.clone(),
        }
    }
    go("", "", v)
}

/// `the closest is candidates[1] (template pluck_1osc, distance 0.0227), written to …` without which one it was.
fn hide_closest(hint: &str) -> String {
    const HEAD: &str = "the closest is candidates[";
    match (
        hint.find(HEAD),
        hint.find("), ").or_else(|| hint.find(");")),
    ) {
        (Some(0), Some(end)) => format!(
            "the closest is candidates[N] (<start>, distance <d>){}",
            &hint[end + 1..]
        ),
        _ => hint.to_owned(),
    }
}

/// What a snapshot records for a successful call: the exit code, the masked JSON and the notes on stderr.
fn record_create(s: &Scratch, o: &Output) -> String {
    s.clean(&format!(
        "exit: {}\n--- stdout\n{}\n--- stderr\n{}",
        o.code,
        pretty(&mask_create(&json(o))),
        o.stderr
    ))
}

/// Run `enth create` on `args`, expecting success; the typed object.
fn create(args: &[&str]) -> (Output, CreateOut) {
    let mut a = vec!["create"];
    a.extend_from_slice(args);
    let o = run(&a);
    assert_eq!(o.code, 0, "{args:?}: {}\n{}", o.stdout, o.stderr);
    let v = json(&o);
    let out: CreateOut = serde_json::from_value(v.clone())
        .unwrap_or_else(|e| panic!("{args:?}: does not match CreateOut: {e}\n{}", o.stdout));
    assert_eq!(
        serde_json::to_value(&out).unwrap(),
        v,
        "{args:?}: the typed object changed the output"
    );
    (o, out)
}

#[test]
fn a_wav_becomes_k_candidates_and_an_enth_create_record() {
    let s = Scratch::new("create_wav");
    let wav = pluck_wav(&s);
    let record = s.path("run.json");
    let best = s.path("best.patch.json");
    let (o, out) = create(&[
        "--target",
        &wav,
        "--candidates",
        "3",
        "--budget",
        "60",
        "--record",
        &record,
        "--out",
        &best,
    ]);
    let v = json(&o);
    assert_eq!(v["schema"], "enth.create/1");
    assert_eq!(out.schema, schema::ids::CREATE);
    assert_eq!(
        schema::ids::CREATE,
        enth_solver::create::SCHEMA,
        "the CLI and the solver's record agree on the version"
    );
    assert_eq!(v["decoded"], "WAV, 24-bit, 48000 Hz, stereo");
    let r: CreateRecord = serde_json::from_value(v.clone()).expect("the record reads back");
    assert_eq!(r.candidates.len(), 3);
    assert!(r.renders <= 60, "{}", r.renders);
    assert_eq!(serde_json::to_value(r.stop).unwrap(), "budget");
    assert!(
        r.starts.len() >= 4,
        "the heuristic and three templates: {}",
        r.starts.len()
    );
    assert_eq!(
        r.starts.len(),
        r.starts().unwrap().starts.len(),
        "every start is a patch"
    );
    assert_eq!(r.settings.k, 3);
    assert_eq!(r.settings.objective, "logmel-pool85-robust/1");
    assert!(r.settings.bank.is_none());
    assert_eq!(r.input.audio_sha256.len(), 64);
    assert_eq!(r.input.perf.key, 57, "A3");
    let intake = r.intake.as_ref().expect("the intake report, whole");
    assert_eq!(
        serde_json::to_value(intake.cfg.codec).unwrap()["codec"],
        "pcm",
        "the decoder's hint"
    );
    assert!(
        r.rounds.iter().all(|r| r.seconds.is_some()),
        "every round stamped"
    );
    assert!(
        r.notes.iter().any(|n| n.contains("no --bank")),
        "{:?}",
        r.notes
    );
    assert!(o.stderr.contains("no --bank"), "{}", o.stderr);
    // The shell's fields.
    assert_eq!(out.score_s, 3.0);
    assert_eq!(out.cap_s, 60.0);
    assert_eq!(out.wrote, [best.clone(), record.clone()]);
    assert_eq!(out.played.schema.as_deref(), Some("enth.perf/1"));
    assert_eq!(out.played.notes.len(), 1);
    assert_eq!(out.played.notes[0].vel, 100);
    assert!(
        (out.played.dur_s - out.input.perf.duration_s).abs() < 1e-12,
        "{:?}",
        out.played
    );
    let b = out.best.expect("a closest candidate");
    assert_eq!(out.candidates[b].branch, b, "`best` indexes `candidates`");
    assert!(
        out.hint.contains(&format!("candidates[{b}]")) && out.hint.contains("written to"),
        "{}",
        out.hint
    );
    // `played` is a performance the other commands take as --perf.
    let played = serde_json::to_string(&out.played).unwrap();
    let rendered = run(&[
        "render",
        &best,
        "--perf",
        &played,
        "--out",
        &s.path("again.wav"),
    ]);
    assert_eq!(rendered.code, 0, "{}", rendered.stdout);
    // The record file is the printed record.
    let file: Value = serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    assert_eq!(file, v);
    // The best patch, named after the file.
    let patch: Value = serde_json::from_str(&std::fs::read_to_string(&best).unwrap()).unwrap();
    assert_eq!(patch["meta"]["name"], "pluck", "{patch}");
    // A terminal gets the table.
    let o = run_tty(&[
        "create",
        "--target",
        &wav,
        "--candidates",
        "2",
        "--budget",
        "20",
    ]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("rank  branch"), "{}", o.stdout);
    assert!(o.stdout.contains("note A3"), "{}", o.stdout);
    assert!(o.stdout.contains("hint: the closest is"), "{}", o.stdout);
}

#[test]
fn the_json_output_is_pinned() {
    let s = Scratch::new("create_snapshot");
    let wav = tone(&s);
    let o = run(&[
        "create",
        "--target",
        &wav,
        "--candidates",
        "2",
        "--budget",
        "60",
        "--seed",
        "1",
        "--workers",
        "2",
        "--out",
        &s.path("best.patch.json"),
        "--record",
        &s.path("run.json"),
    ]);
    assert_eq!(o.code, 0, "{}\n{}", o.stdout, o.stderr);
    let out: CreateOut = serde_json::from_value(json(&o)).unwrap();
    assert!(matches!(
        out.starts[0].provenance,
        CreateProvenanceOut::Heuristic
    ));
    assert_eq!(out.stop, CreateStopOut::Budget);
    s.settings()
        .bind(|| assert_snapshot!("create_json", record_create(&s, &o)));
}

#[test]
fn the_result_is_the_same_for_any_worker_count() {
    let s = Scratch::new("create_workers");
    let wav = pluck_wav(&s);
    let go = |w: &str| {
        let o = run(&[
            "create",
            "--target",
            &wav,
            "--candidates",
            "3",
            "--budget",
            "50",
            "--workers",
            w,
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
            "create",
            "--target",
            &fixture(name),
            "--candidates",
            "2",
            "--budget",
            "12",
        ]);
        assert_eq!(o.code, 0, "{name}: {}", o.stdout);
        let v = json(&o);
        assert!(
            v["decoded"].as_str().unwrap().starts_with(decoded),
            "{name}: {}",
            v["decoded"]
        );
        assert_eq!(v["intake"]["cfg"]["codec"]["lossy"], lossy, "{name}");
        assert_eq!(v["input"]["perf"]["key"], 57, "{name}: A3");
    }
}

#[test]
fn the_flags_override_the_inferred_key_velocity_and_hold() {
    let s = Scratch::new("create_perf");
    let wav = tone(&s);
    let go = |extra: &[&str]| {
        let mut a = vec![
            "--target",
            wav.as_str(),
            "--candidates",
            "2",
            "--budget",
            "8",
        ];
        a.extend_from_slice(extra);
        create(&a).1
    };
    let inferred = go(&[]);
    assert_eq!(inferred.input.perf.key, 57);
    let hold = inferred.input.perf.hold_s;
    // One flag each: the others stay the intake's.
    let o = go(&["--note", "C4", "--vel", "64", "--hold", "0.2s"]);
    assert_eq!(o.input.perf.key, 60);
    assert!((f64::from(o.input.perf.velocity) - 64.0 / 127.0).abs() < 1e-6);
    assert!((o.input.perf.hold_s - 0.2).abs() < 1e-9);
    assert_eq!(
        (o.input.perf.on_s, o.input.perf.duration_s),
        (inferred.input.perf.on_s, inferred.input.perf.duration_s),
        "the onset and the clip stay the intake's"
    );
    let intake = o.intake.as_ref().unwrap();
    assert_eq!(
        intake["perf"]["key"], 57,
        "the intake's own reading stays in its report"
    );
    assert!(
        o.notes.iter().any(|n| n.contains("performance overridden")),
        "{:?}",
        o.notes
    );
    assert_eq!(o.played.notes[0].vel, 64);
    // `--perf` in its short form names only what it changes.
    let o = go(&["--perf", "note=D3"]);
    assert_eq!(o.input.perf.key, 50, "D3");
    assert!((o.input.perf.hold_s - hold).abs() < 1e-12, "the hold stays");
    assert!(
        (f64::from(o.input.perf.velocity) - f64::from(inferred.input.perf.velocity)).abs() < 1e-9
    );
    // A whole performance (inline JSON, here) replaces all three, and its other times are not used.
    let perf = r#"{"notes":[{"note":"E3","vel":90,"on_s":0.5,"off_s":0.9}],"dur_s":2}"#;
    let o = go(&["--perf", perf]);
    assert_eq!(o.input.perf.key, 52, "E3");
    assert!((f64::from(o.input.perf.velocity) - 90.0 / 127.0).abs() < 1e-6);
    assert!((o.input.perf.hold_s - 0.4).abs() < 1e-9, "off minus on");
    assert_eq!(o.input.perf.on_s, inferred.input.perf.on_s);
    assert!(
        o.notes.iter().any(|n| n.contains("not used")),
        "{:?}",
        o.notes
    );
    // A flag after `--perf` wins for its field.
    let o = go(&["--perf", perf, "--note", "G3"]);
    assert_eq!(o.input.perf.key, 55, "G3");
    // Several notes: the first by note-on is played, and it says so.
    let two = r#"{"notes":[{"note":"E3","vel":90,"on_s":0.5,"off_s":0.9},{"note":"G3","vel":80,"on_s":0.1,"off_s":0.3}],"dur_s":2}"#;
    let o = go(&["--perf", two]);
    assert_eq!(o.input.perf.key, 55);
    assert!(
        o.notes.iter().any(|n| n.contains("2 notes")),
        "{:?}",
        o.notes
    );
}

#[test]
fn the_scored_window_cuts_the_clip() {
    let s = Scratch::new("create_window");
    let wav = tone(&s);
    let go = |w: &str| {
        create(&[
            "--target",
            &wav,
            "--candidates",
            "2",
            "--budget",
            "8",
            "--score-seconds",
            w,
        ])
        .1
    };
    let whole = go("5");
    let default = create(&["--target", &wav, "--candidates", "2", "--budget", "8"]).1;
    assert_eq!(whole.score_s, 5.0);
    assert_eq!(default.score_s, 3.0, "the product's window");
    assert_eq!(
        whole.input.audio_sha256, default.input.audio_sha256,
        "a window longer than the clip scores the clip, as the default does"
    );
    assert!(
        !whole.notes.iter().any(|n| n.contains("scored over")),
        "{:?}",
        whole.notes
    );
    // 0.2 s of the 0.375 s clip.
    let cut = go("0.2");
    assert_eq!(cut.score_s, 0.2);
    assert_eq!(cut.input.frames, 9_600, "0.2 s at 48 kHz");
    assert!((cut.input.perf.duration_s - 0.2).abs() < 1e-9);
    assert!((cut.played.dur_s - 0.2).abs() < 1e-9);
    assert_ne!(cut.input.audio_sha256, whole.input.audio_sha256);
    assert!(
        cut.notes
            .iter()
            .any(|n| n.starts_with("scored over the first 0.20 s")),
        "{:?}",
        cut.notes
    );
    assert_eq!(
        cut.intake.as_ref().unwrap()["perf"]["duration_s"],
        whole.input.perf.duration_s,
        "the intake's report is of the whole clip"
    );
}

#[test]
fn stop_at_ends_the_job_when_a_candidate_is_close_enough() {
    let s = Scratch::new("create_stop_at");
    let wav = tone(&s);
    let go = |budget: &str, extra: &[&str]| {
        let mut a = vec![
            "--target",
            wav.as_str(),
            "--candidates",
            "2",
            "--budget",
            budget,
        ];
        a.extend_from_slice(extra);
        create(&a).1
    };
    let goal = go("200", &["--stop-at", "0.5"]);
    assert_eq!(goal.stop, CreateStopOut::Goal);
    assert_eq!(goal.settings.stop_at, Some(0.5));
    assert!(goal.renders < 200, "{}", goal.renders);
    let b = goal.best.unwrap();
    assert!(goal.candidates[b].distance.unwrap() <= 0.5);
    assert_eq!(go("8", &[]).settings.stop_at, None);
}

#[test]
fn fewer_starts_than_k_say_so() {
    let s = Scratch::new("create_k");
    let wav = tone(&s);
    let (_, out) = create(&["--target", &wav, "--candidates", "16", "--budget", "8"]);
    assert_eq!(out.settings.k, 16);
    assert!(out.candidates.len() < 16, "{}", out.candidates.len());
    let note = out
        .notes
        .iter()
        .find(|n| n.starts_with("K is 16"))
        .unwrap_or_else(|| panic!("{:?}", out.notes));
    assert!(note.contains("--bank adds up to 16"), "{note}");
}

#[test]
fn a_bank_adds_neighbours_and_the_schema_describes_them() {
    let s = Scratch::new("create_bank");
    let wav = tone(&s);
    let bank = s.path("enth-bank-v1.bin");
    std::fs::write(&bank, small_bank().to_bytes()).unwrap();
    let (_, out) = create(&[
        "--target",
        &wav,
        "--candidates",
        "3",
        "--budget",
        "30",
        "--bank",
        &bank,
    ]);
    let b = out.settings.bank.as_ref().expect("the bank is recorded");
    assert_eq!((b.entries, b.generator, b.sha256.len()), (5, 1, 64));
    assert!(
        out.starts
            .iter()
            .any(|s| matches!(s.provenance, CreateProvenanceOut::Knn { .. })),
        "{:?}",
        out.starts.iter().map(|s| &s.provenance).collect::<Vec<_>>()
    );
    assert!(
        !out.notes.iter().any(|n| n.contains("no --bank")),
        "{:?}",
        out.notes
    );
    assert_eq!(out.settings.embedding, "knn-logmel-rel/1");
}

#[test]
fn the_seconds_cap_stops_the_job_and_says_so() {
    let s = Scratch::new("create_cap");
    let wav = pluck_wav(&s);
    let o = run(&[
        "create",
        "--target",
        &wav,
        "--candidates",
        "8",
        "--budget",
        "1000000",
        "--seconds",
        "1",
    ]);
    // A cap that lets the first round finish is a result; on a loaded machine it may not, and that is `time_cap`.
    if o.code == 4 {
        assert_eq!(json(&o)["error"], "time_cap", "{}", o.stdout);
        return;
    }
    assert_eq!(o.code, 0, "{}", o.stdout);
    let v = json(&o);
    assert_eq!(v["stop"], "cancelled");
    assert!(
        v["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("--seconds cap")),
        "{}",
        v["notes"]
    );
    let elapsed = v["elapsed_s"].as_f64().unwrap();
    // The cap plus about one render: seconds, under the other tests' load, not the hours a million renders take.
    assert!(elapsed < 30.0, "{elapsed}");
}

/// `--target FILE` and then `extra`.
fn flags<'a>(target: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec!["--target", target];
    a.extend_from_slice(extra);
    a
}

/// Run, check the exit code and the error code, read the error back as `enth.error/1`, and return the snapshot text.
fn err(s: &Scratch, args: &[&str], exit: i32, code: &str) -> String {
    let mut a = vec!["create"];
    a.extend_from_slice(args);
    let o = run(&a);
    let e: ErrorOut = round_trip(&o);
    assert_eq!(
        (o.code, e.exit, e.error.as_str()),
        (exit, exit, code),
        "{args:?}: {}",
        o.stdout
    );
    assert_ne!(e.hint, "", "{args:?}");
    common::record(s, &o, false)
}

#[test]
fn usage_errors_exit_2_with_a_hint() {
    let s = Scratch::new("create_err_usage");
    let wav = tone(&s);
    s.settings().bind(|| {
        assert_snapshot!("err_create_missing_target", err(&s, &[], 2, "usage"));
        assert_snapshot!(
            "err_create_bad_candidates",
            err(
                &s,
                &flags(&wav, &["--candidates", "0"]),
                2,
                "bad_candidates"
            )
        );
        assert_snapshot!(
            "err_create_bad_seconds",
            err(&s, &flags(&wav, &["--seconds", "301"]), 2, "bad_seconds")
        );
        assert_snapshot!(
            "err_create_bad_budget",
            err(&s, &flags(&wav, &["--budget", "0"]), 2, "bad_budget")
        );
        assert_snapshot!(
            "err_create_bad_score_seconds",
            err(
                &s,
                &flags(&wav, &["--score-seconds", "9"]),
                2,
                "bad_score_seconds"
            )
        );
        assert_snapshot!(
            "err_create_bad_stop_at",
            err(&s, &flags(&wav, &["--stop-at", "-1"]), 2, "bad_stop_at")
        );
        assert_snapshot!(
            "err_create_bad_workers",
            err(&s, &flags(&wav, &["--workers", "0"]), 2, "bad_workers")
        );
        assert_snapshot!(
            "err_create_bad_perf",
            err(&s, &flags(&wav, &["--perf", "note=H9"]), 2, "bad_perf")
        );
        assert_snapshot!(
            "err_create_dur_fixed",
            err(&s, &flags(&wav, &["--dur", "2s"]), 2, "dur_fixed")
        );
        assert_snapshot!(
            "err_create_out_is_the_sound",
            err(&s, &flags(&wav, &["--out", &wav]), 2, "bad_out")
        );
        let same = s.path("both.json");
        assert_snapshot!(
            "err_create_out_is_the_record",
            err(
                &s,
                &flags(&wav, &["--out", &same, "--record", &same]),
                2,
                "bad_out"
            )
        );
    });
    // Every flag is checked before the file is read.
    let o = run(&["create", "--target", "/no/such.wav", "--budget", "0"]);
    assert_eq!(json(&o)["error"], "bad_budget");
}

#[test]
fn file_and_bank_errors_say_what_to_do() {
    let s = Scratch::new("create_err_files");
    let wav = tone(&s);
    let junk = s.write("junk.wav", "not a sound");
    let patch = s.new_patch("init", "p.patch.json");
    let truncated = s.path("cut.wav");
    std::fs::write(&truncated, &std::fs::read(&wav).unwrap()[..100]).unwrap();
    let not_a_bank = s.write("bank.bin", "ENTHBANK? no");
    s.settings().bind(|| {
        assert_snapshot!(
            "err_create_file_not_found",
            err(
                &s,
                &["--target", &s.path("missing.wav")],
                5,
                "file_not_found"
            )
        );
        assert_snapshot!(
            "err_create_unknown_format",
            err(&s, &["--target", &junk], 3, "unknown_format")
        );
        assert_snapshot!(
            "err_create_a_patch",
            err(&s, &["--target", &patch], 3, "unknown_format")
        );
        assert_snapshot!(
            "err_create_bad_file",
            err(&s, &["--target", &truncated], 3, "bad_file")
        );
        assert_snapshot!(
            "err_create_bank_not_found",
            err(
                &s,
                &["--target", &wav, "--bank", &s.path("missing.bin")],
                5,
                "file_not_found"
            )
        );
        assert_snapshot!(
            "err_create_not_a_bank",
            err(
                &s,
                &["--target", &wav, "--bank", &not_a_bank],
                3,
                "not_a_bank"
            )
        );
        // A cap no job can meet: the decode or the first round passes it, and either way there is no candidate.
        assert_snapshot!(
            "err_create_time_cap",
            err(&s, &["--target", &wav, "--seconds", "0.001"], 4, "time_cap")
        );
    });
    // The bank's hint is the bank's, not a patch's.
    let o = run(&["create", "--target", &wav, "--bank", &s.path("missing.bin")]);
    assert!(
        json(&o)["hint"]
            .as_str()
            .unwrap()
            .contains("cargo xtask bank")
    );
}

#[test]
fn create_is_listed_in_help_and_its_schema_in_enth_schema() {
    let help = run(&["--help"]).stdout;
    assert!(help.contains("create"), "{help}");
    let create_help = run(&["create", "--help"]).stdout;
    for flag in [
        "--score-seconds",
        "--stop-at",
        "--perf",
        "--bank",
        "--record",
    ] {
        assert!(create_help.contains(flag), "{flag}: {create_help}");
    }
    // From the usage line on: the one-line description above it is `enth --help`'s too.
    let flags = create_help
        .split_once("Usage:")
        .map_or(create_help.as_str(), |(_, f)| f);
    Scratch::new("create_help")
        .settings()
        .bind(|| assert_snapshot!("create_help", flags));
    let e = schema::entry("create").expect("`enth schema create`");
    assert_eq!(e.id, enth_solver::create::SCHEMA);
    let o = run(&["schema", "create"]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let v = json(&o);
    assert_eq!(v["$id"], "enth.create/1");
    for field in [
        "input",
        "settings",
        "starts",
        "candidates",
        "played",
        "hint",
    ] {
        assert!(v["properties"][field].is_object(), "{field}: {v}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// A small k-NN bank
// ---------------------------------------------------------------------------------------------------------------

/// Five generator-v1 patches (levels 2 and 5) rendered at the bank's reference note: what `cargo xtask bank` makes,
/// small (the solver's own tests build theirs the same way).
fn small_bank() -> enth_solver::nearest::Bank {
    use enth_analysis::descriptors::Want;
    use enth_analysis::features::{FeatureCfg, FeatureExtractor};
    use enth_analysis::{Describer, Descriptors};
    use enth_challenge::random;
    use enth_challenge::spec::PRESETS;
    use enth_render::{Performance, RenderSpec, render};
    use enth_solver::nearest::{
        BANK_FORMAT, Bank, BankEntry, BankHeader, EMBEDDING_VERSION, REF_NOTE, UNPITCHED, embed,
        reference_hz,
    };

    const SR: u32 = 22_050;
    let perf = Performance::note(REF_NOTE, 0.8, 0.6, 1.0);
    let spec = RenderSpec::new(SR, 3);
    let cfg = FeatureCfg {
        sample_rate: f64::from(SR),
        ..FeatureCfg::default()
    };
    let mut fx = FeatureExtractor::new(cfg);
    let mut describer = Describer::new(f64::from(SR));
    let mut entries = Vec::new();
    for (i, preset) in [1usize, 4, 1, 4, 4].into_iter().enumerate() {
        let rspec = PRESETS[preset].spec(1, 0x434c_0000 + i as u64);
        let g = random::target(&rspec).unwrap();
        let mid = render(&g.patch, &perf, &spec).unwrap().mid();
        let f = fx.extract(&mid);
        let desc: Option<Descriptors> = describer
            .describe_some(&mid, enth_solver::note_timing(&perf), Want::CORE)
            .ok();
        entries.push(BankEntry {
            seed: rspec.seed,
            difficulty: rspec.difficulty,
            musical: rspec.musical,
            pitch_cents: UNPITCHED,
            fingerprint_prefix: g.fingerprint.prefix().0,
            embedding: embed(
                &f,
                &cfg,
                0.0,
                reference_hz(desc.as_ref()),
                f64::from(SR) / 2.0,
            ),
        });
    }
    Bank::new(
        BankHeader {
            format: BANK_FORMAT,
            generator: 1,
            template: 0,
            embedding: EMBEDDING_VERSION,
            note: REF_NOTE,
            velocity: 0.8,
            hold_s: 0.6,
            duration_s: 1.0,
            sample_rate: SR,
            render_seed: 3,
            exclusions_sha256: [0; 32],
        },
        entries,
    )
}
