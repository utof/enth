//! `enth-listen gr3`, end to end. **Nothing here plays audio:** sessions use a recording mock player, kits are only
//! written, and `--dry-run` never calls a player.
//!
//! - The bootstrap is checked against a Python reference (`tests/gr3/bootstrap_ref.py`, standard library only) two ways:
//!   against the numbers it printed for a committed synthetic answers file (`synthetic.json` -> `expected.json`,
//!   always run), and live, by running the script on the same file and on the judgements of a dry run (skipped with a
//!   note when `python3` is missing; set `ENTH_REQUIRE_PYTHON=1` to make that an error).
//! - A dry run designs a demo set, writes the kits, answers synthetically and reports.
//! - The owner's session resumes after `q`; the friends' flow is gated until everyone is done.

#![allow(
    clippy::disallowed_methods,
    clippy::print_stderr,
    reason = "the tests run the tool's own binary and python3, and say when they skip the live check"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use enth_listen::files::read_json;
use enth_listen::gr3::analyze::{Report, analyze, judgements, summarize};
use enth_listen::gr3::answers::{kit_answers_text, owner_responses, parse_kit_answers};
use enth_listen::gr3::audio::{decode_any, measure};
use enth_listen::gr3::boot::{Boot, Judgement};
use enth_listen::gr3::design::{Kind, Manifest, OWNER, SCHEMA_ANSWERS, manifest_path};
use enth_listen::gr3::dry::{demo_set, synthetic_choices};
use enth_listen::gr3::{self, Gr3Args, friends_dir, report_path};
use enth_listen::session::{MockPlayer, Scripted, StdinLines, load_answers};

const TESTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/gr3");

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("enth-listen-gr3-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn args(dir: &Path, set: Option<PathBuf>) -> Gr3Args {
    Gr3Args {
        set,
        bootstrap: 300,
        player: "true".into(),
        ..Gr3Args::new(dir)
    }
}

fn run_text(a: &Gr3Args, lines: &[&str], player: &mut MockPlayer) -> (Result<(), String>, String) {
    let mut out = Vec::new();
    let r =
        gr3::run(a, &mut out, &mut Scripted::new(lines), Some(player)).map_err(|e| e.to_string());
    (r, String::from_utf8(out).unwrap())
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

// ---- the bootstrap against the Python reference -------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Fixture {
    judgements: Vec<Judgement>,
}

fn close(a: f64, b: f64, what: &str) {
    assert!(
        (a - b).abs() <= 1e-12 * (1.0 + a.abs().max(b.abs())),
        "{what}: rust {a} vs python {b}"
    );
}

fn check_boot(rust: Option<&Boot>, py: &serde_json::Value, what: &str) {
    if py.is_null() {
        assert!(rust.is_none(), "{what}: python has no data");
        return;
    }
    let b = rust.unwrap_or_else(|| panic!("{what}: rust has no data"));
    for (key, got) in [
        ("n", b.n),
        ("wins", b.wins),
        ("clusters", b.clusters as u64),
        ("listeners", b.listeners as u64),
        ("replicates", b.replicates as u64),
    ] {
        assert_eq!(py[key].as_u64(), Some(got), "{what}: {key}");
    }
    for (key, got) in [
        ("p", b.p),
        ("lo", b.lo),
        ("hi", b.hi),
        ("se", b.se),
        ("lo_ref", b.lo_ref),
        ("hi_ref", b.hi_ref),
        ("se_ref", b.se_ref),
    ] {
        close(got, py[key].as_f64().unwrap(), &format!("{what}: {key}"));
    }
    for (key, got) in [("deff", b.deff), ("deff_two_stage", b.deff_two_stage)] {
        match (got, py[key].as_f64()) {
            (Some(g), Some(p)) => close(g, p, &format!("{what}: {key}")),
            (None, None) => {}
            other => panic!("{what}: {key} {other:?}"),
        }
    }
}

fn check_summary(js: &[Judgement], replicates: usize, py: &serde_json::Value) {
    let s = summarize(js, replicates);
    check_boot(s.pooled.as_ref(), &py["all"], "all");
    check_boot(s.clean.as_ref(), &py["clean"], "clean");
    check_boot(s.degraded.as_ref(), &py["degraded"], "degraded");
    assert_eq!(s.strata.len(), py["strata"].as_object().unwrap().len());
    for row in &s.strata {
        check_boot(
            row.boot.as_ref(),
            &py["strata"][&row.name],
            &format!("stratum {}", row.name),
        );
    }
}

/// The numbers the Python script printed for the committed synthetic answers (10,000 replicates), pinned: the Rust
/// bootstrap reproduces them to 1e-12. Always runs.
#[test]
fn the_bootstrap_reproduces_the_pinned_python_numbers() {
    let f: Fixture = read_json(&Path::new(TESTS).join("synthetic.json")).unwrap();
    assert_eq!(f.judgements.len(), 120);
    let expected: serde_json::Value = read_json(&Path::new(TESTS).join("expected.json")).unwrap();
    check_summary(&f.judgements, 10_000, &expected);
    // the interval the rule uses, spelled out for a reader of this file
    let all = &expected["all"];
    assert_eq!(all["n"], 120);
    assert_eq!(
        all["clusters"], 40,
        "60 items, 20 degraded copies sharing their clean original's cluster"
    );
    assert_eq!(all["listeners"], 3);
}

/// The python3 on this machine, if any.
fn python() -> Option<&'static str> {
    let ok = Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        assert!(
            std::env::var_os("ENTH_REQUIRE_PYTHON").is_none(),
            "ENTH_REQUIRE_PYTHON is set but python3 does not run"
        );
        eprintln!(
            "note: python3 is not available; the live cross-check was skipped (the pinned one ran)"
        );
        return None;
    }
    Some("python3")
}

fn run_python(py: &str, fixture: &Path, replicates: usize) -> serde_json::Value {
    let out = Command::new(py)
        .arg("-I")
        .arg(Path::new(TESTS).join("bootstrap_ref.py"))
        .arg("boot")
        .arg(fixture)
        .args(["--replicates", &replicates.to_string()])
        .output()
        .expect("python3 runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// The same check, live: run the script on the committed file with fewer replicates, and on the judgements of a
/// dry run of the whole pipeline.
#[test]
fn the_bootstrap_agrees_with_the_python_script_run_now() {
    let Some(py) = python() else { return };
    let fixture = Path::new(TESTS).join("synthetic.json");
    let f: Fixture = read_json(&fixture).unwrap();
    check_summary(&f.judgements, 600, &run_python(py, &fixture, 600));

    // the judgements of a dry run: what the production path hands the bootstrap
    let d = scratch("pycheck");
    let set = demo_set(&d.join("set"), 24).unwrap();
    let a = Gr3Args {
        dry_run: true,
        ..args(&d.join("out"), Some(set))
    };
    let (r, _) = run_text(&a, &[], &mut MockPlayer::default());
    r.unwrap();
    let dir = d.join("out").join("dry-run");
    let m: Manifest = read_json(&manifest_path(&dir)).unwrap();
    let mut responses = vec![
        owner_responses(
            &m,
            &load_answers(&dir.join("answers.json"), SCHEMA_ANSWERS, 1)
                .unwrap()
                .unwrap(),
        )
        .unwrap(),
    ];
    for k in ["kit1", "kit2"] {
        let text = std::fs::read_to_string(friends_dir(&dir).join(format!("{k}.txt"))).unwrap();
        responses.push(parse_kit_answers(k, &text, &m).unwrap());
    }
    let report = analyze(&m, &responses, 300);
    let js = judgements(&m, &responses, &report.listeners);
    assert!(js.len() > 24, "{} judgements", js.len());
    let jf = d.join("dry-judgements.json");
    std::fs::write(
        &jf,
        serde_json::to_string(&serde_json::json!({ "judgements": js })).unwrap(),
    )
    .unwrap();
    let py_report = run_python(py, &jf, 300);
    check_boot(report.pooled.as_ref(), &py_report["all"], "dry run, pooled");
    check_boot(
        report.degraded.as_ref(),
        &py_report["degraded"],
        "dry run, degraded",
    );
    std::fs::remove_dir_all(&d).unwrap();
}

// ---- the dry run ----------------------------------------------------------------------------------------------

#[test]
fn a_dry_run_designs_exports_answers_and_reports() {
    let d = scratch("dry");
    let set = demo_set(&d.join("set"), 24).unwrap();
    let a = Gr3Args {
        dry_run: true,
        ..args(&d.join("out"), Some(set))
    };
    let mut player = MockPlayer::default();
    let (r, out) = run_text(&a, &[], &mut player);
    r.unwrap();
    assert!(player.played.is_empty(), "a dry run never plays");
    for want in [
        "SYNTHETIC",
        "Decision:",
        "Strata",
        "design effect",
        "EXCLUDED",
        "lower bound",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    let dir = d.join("out").join("dry-run");
    let report: Report = read_json(&report_path(&dir)).unwrap();
    assert!(report.synthetic && report.complete && report.missing.is_empty());
    assert_eq!(report.listeners.len(), 3);
    assert!(
        report.listeners.iter().any(|l| l.excluded),
        "kit2 misses an attention trial on purpose"
    );
    assert_eq!(report.seed, 0x4752_3342_4f4f_5453);

    // the kits: 8 references per friend plus 6 catch trials, three clips each, random names, no key
    let m: Manifest = read_json(&manifest_path(&dir)).unwrap();
    assert_eq!(m.trials_of(OWNER).len(), 24 + 6);
    for k in ["kit1", "kit2"] {
        let kit = dir.join("kits").join(k);
        let names = names_in(&kit);
        assert!(
            names.contains(&"answers.txt".to_string()) && names.contains(&"README.txt".to_string())
        );
        assert!(!names.iter().any(|n| n.contains("manifest")));
        let wavs: Vec<&String> = names
            .iter()
            .filter(|n| Path::new(n).extension().is_some_and(|e| e == "wav"))
            .collect();
        assert_eq!(wavs.len(), 3 * (12 + 6), "{k}: {names:?}");
        for n in &wavs {
            // NN-K-xxxxxx.wav
            let p: Vec<&str> = n.trim_end_matches(".wav").split('-').collect();
            assert!(
                p.len() == 3
                    && p[0].len() == 2
                    && ["1", "2", "3"].contains(&p[1])
                    && p[2].len() == 6,
                "{n}"
            );
        }
        // blind: no item id, stratum or arm name in any file name or text
        let texts =
            ["answers.txt", "README.txt"].map(|f| std::fs::read_to_string(kit.join(f)).unwrap());
        for it in &m.items {
            for s in [&it.id, &it.stratum] {
                assert!(
                    !texts.iter().any(|t| t.contains(s.as_str())),
                    "{s} leaks into {k}"
                );
                assert!(
                    !names.iter().any(|n| n.contains(s.as_str())),
                    "{s} leaks into a name in {k}"
                );
            }
        }
        assert!(
            !texts
                .iter()
                .any(|t| t.contains("demo arm") || t.contains("manifest"))
        );
        // every clip is at -23 LUFS, or lower by the trial's own amount
        for t in m.trials_of(k).iter().take(4) {
            for (c, name) in t.clips.iter().enumerate() {
                let w =
                    decode_any(&std::fs::read(kit.join(format!("{name}.wav"))).unwrap()).unwrap();
                let lufs = measure(&w).unwrap().lufs;
                assert!(
                    (lufs + 23.0 + t.levels.lowered_db).abs() < 0.05,
                    "{k} trial {} clip {c}: {lufs}",
                    t.n
                );
            }
        }
    }
    // the same dry run again gives the same report
    let (r2, out2) = run_text(&a, &[], &mut MockPlayer::default());
    r2.unwrap();
    assert_eq!(out, out2);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_dry_run_without_a_set_uses_the_demo_set() {
    let d = scratch("demo");
    let a = Gr3Args {
        dry_run: true,
        friends: Some(0),
        ..args(&d, None)
    };
    let (r, out) = run_text(&a, &[], &mut MockPlayer::default());
    r.unwrap();
    let dir = d.join("dry-run");
    let m: Manifest = read_json(&manifest_path(&dir)).unwrap();
    assert_eq!(
        (m.items.len(), m.kits, m.trials_of(OWNER).len()),
        (60, 0, 66)
    );
    assert!(
        out.contains("66 trials") && out.contains("40 clean, 20 degraded"),
        "{out}"
    );
    assert!(!dir.join("kits").exists());
    std::fs::remove_dir_all(&d).unwrap();
}

// ---- the owner's session --------------------------------------------------------------------------------------

#[test]
fn the_owners_session_saves_after_each_answer_and_resumes() {
    let d = scratch("session");
    let set = demo_set(&d.join("set"), 12).unwrap();
    let a = args(&d.join("run"), Some(set));
    let mut player = MockPlayer::default();
    // two answers (with a replay of everything and of B), then quit
    let (r, out) = run_text(&a, &["r", "rb", "RR", "a", "b", "q"], &mut player);
    r.unwrap();
    assert!(out.contains("GR3 listening: 18 trials"), "{out}");
    assert!(
        out.contains("[1/18]") && out.contains("[2/18]") && out.contains("saved 2 answers"),
        "{out}"
    );
    // blind: nothing in the session names a reference, stratum or arm
    for w in ["ref-0", "deg-0", "subtractive", "demo arm"] {
        assert!(!out.contains(w), "{w:?} shown during the session:\n{out}");
    }
    let m: Manifest = read_json(&manifest_path(&d.join("run"))).unwrap();
    let first = m.trials_of(OWNER)[0];
    // trial 1 plays R, A, B once and 0.8 s gaps; then the replays (all, B, R)
    let frames = |k: usize| (first.seconds[k] * 48_000.0).round() as u64;
    let all = 44 + (frames(0) + frames(1) + frames(2) + 2 * 38_400) * 6;
    assert_eq!(player.played[0], all);
    assert_eq!(player.played[1], all, "r replays all three");
    assert_eq!(player.played[2], 44 + frames(2) * 6, "rb replays B");
    assert_eq!(
        player.played[3],
        44 + frames(0) * 6,
        "rr replays the reference"
    );
    let file = load_answers(&d.join("run").join("answers.json"), SCHEMA_ANSWERS, 1)
        .unwrap()
        .unwrap();
    assert_eq!(file.answers.len(), 2);
    assert_eq!(file.answers[0].replays, 3);
    // resume with the rest
    let rest: Vec<&str> = (0..16)
        .map(|i| if i % 3 == 0 { "b" } else { "a" })
        .collect();
    let (r, out) = run_text(&a, &rest, &mut player);
    r.unwrap();
    assert!(
        out.contains("Resuming after 2 answered trials") && out.contains("Done: 18 answers"),
        "{out}"
    );
    assert!(out.contains("--export-kit") && out.contains("--analyze"));
    let file = load_answers(&d.join("run").join("answers.json"), SCHEMA_ANSWERS, 1)
        .unwrap()
        .unwrap();
    assert_eq!(file.answers.len(), 18);
    assert_eq!(
        file.answers
            .iter()
            .map(|x| x.n)
            .collect::<BTreeSet<_>>()
            .len(),
        18
    );
    // a third run has nothing left to ask
    let (r, out) = run_text(&a, &[], &mut player);
    r.unwrap();
    assert!(out.contains("Done: 18 answers"), "{out}");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn an_unknown_answer_is_asked_again_and_the_stdin_reader_exists() {
    let d = scratch("typo");
    let set = demo_set(&d.join("set"), 12).unwrap();
    let a = args(&d.join("run"), Some(set));
    let (r, out) = run_text(&a, &["x", "a", "q"], &mut MockPlayer::default());
    r.unwrap();
    assert!(
        out.contains("type a, b, r, one of the r-commands, or q"),
        "{out}"
    );
    let _ = StdinLines; // the binary reads stdin through it
    std::fs::remove_dir_all(&d).unwrap();
}

// ---- mixed sample rates -----------------------------------------------------------------------------------------

fn wav_bytes(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels * bits / 8)).to_le_bytes());
    out.extend_from_slice(&(channels * bits / 8).to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// A degraded upload and its clean original rarely share a sample rate: here a 44.1 kHz mono reference and a 96 kHz
/// candidate sit among 48 kHz clips. Every clip the trial plays is 48 kHz stereo, the same length, and level-matched.
#[test]
fn clips_at_other_sample_rates_are_played_at_48k() {
    let d = scratch("rates");
    let set = demo_set(&d.join("set"), 12).unwrap();
    let audio = set.parent().unwrap().join("audio");
    let tone = |rate: u32, secs: f64| -> Vec<f32> {
        (0..(secs * f64::from(rate)) as usize)
            .map(|i| {
                (0.3 * libm::sin(2.0 * core::f64::consts::PI * 440.0 * i as f64 / f64::from(rate)))
                    as f32
            })
            .collect()
    };
    // ref-00: 44.1 kHz mono, 16-bit
    let mono: Vec<u8> = tone(44_100, 0.5)
        .iter()
        .flat_map(|v| ((f64::from(*v) * 32_767.0) as i16).to_le_bytes())
        .collect();
    std::fs::write(audio.join("ref-00.wav"), wav_bytes(1, 1, 44_100, 16, &mono)).unwrap();
    // ref-00's arm B: 96 kHz stereo float
    let st: Vec<u8> = tone(96_000, 0.5)
        .iter()
        .flat_map(|v| [v.to_le_bytes(), v.to_le_bytes()].concat())
        .collect();
    std::fs::write(audio.join("ref-00-b.wav"), wav_bytes(3, 2, 96_000, 32, &st)).unwrap();
    let a = args(&d.join("run"), Some(set));
    let (r, _) = run_text(&a, &[], &mut MockPlayer::default());
    r.unwrap();
    let m: Manifest = read_json(&manifest_path(&d.join("run"))).unwrap();
    let t = m
        .trials
        .iter()
        .find(|t| t.audience == OWNER && t.item == "ref-00" && t.kind == Kind::Main)
        .unwrap();
    let b_pos = if t.pos_a == enth_listen::gr3::design::Source::ArmB {
        1
    } else {
        2
    };
    assert_eq!(t.rates_in[0], 44_100);
    assert_eq!(t.rates_in[b_pos], 96_000);
    let mut frames = Vec::new();
    for c in &t.clips {
        let w = enth_listen::wav::decode(
            &std::fs::read(d.join("run").join("clips").join(format!("{c}.wav"))).unwrap(),
        )
        .unwrap();
        assert_eq!((w.sample_rate, w.channels.len()), (48_000, 2), "{c}");
        let l = measure(&w).unwrap().lufs;
        assert!((l + 23.0 + t.levels.lowered_db).abs() < 0.05, "{c}: {l}");
        frames.push(w.frames());
    }
    // 0.5 s at 44.1 kHz and at 96 kHz both come out as 24,000 frames; the 48 kHz candidate is 0.4 s long
    assert_eq!(frames[0], 24_000);
    assert_eq!(frames[b_pos], 24_000);
    std::fs::remove_dir_all(&d).unwrap();
}

// ---- the friends' flow ----------------------------------------------------------------------------------------

#[test]
fn the_friends_flow_is_gated_until_everyone_is_done() {
    let d = scratch("friends");
    let set = demo_set(&d.join("set"), 24).unwrap();
    let dir = d.join("run");
    let a = args(&dir, Some(set));
    // the owner answers everything (all 30 trials) with a fixed pattern
    let owner: Vec<&str> = (0..30)
        .map(|i| if i % 2 == 0 { "a" } else { "b" })
        .collect();
    let (r, _) = run_text(&a, &owner, &mut MockPlayer::default());
    r.unwrap();
    // kits go out; the set is found again through the key
    let kits = d.join("kits");
    let b = Gr3Args {
        export_kit: Some(kits.clone()),
        set: None,
        ..args(&dir, None)
    };
    let (r, out) = run_text(&b, &[], &mut MockPlayer::default());
    r.unwrap();
    assert!(
        out.contains("kit1") && out.contains("kit2") && out.contains("manifest.json"),
        "{out}"
    );
    assert!(kits.join("kit1").join("answers.txt").exists());
    let m: Manifest = read_json(&manifest_path(&dir)).unwrap();
    // only the owner has answered: the analysis shows progress and nothing else
    let an = Gr3Args {
        analyze: true,
        ..args(&dir, None)
    };
    let (r, out) = run_text(&an, &[], &mut MockPlayer::default());
    let err = r.unwrap_err();
    assert!(
        err.contains("stays closed") && err.contains("--partial"),
        "{err}"
    );
    assert!(
        out.contains("owner") && out.contains("kit1") && out.contains("no answers yet"),
        "{out}"
    );
    assert!(
        !out.contains("Decision") && !out.contains("arm B closer"),
        "no result before everyone is done:\n{out}"
    );
    // a friend returns a file: still gated; --partial shows a provisional result
    std::fs::create_dir_all(friends_dir(&dir)).unwrap();
    let fill = |k: &str| {
        kit_answers_text(k, &m.trials_of(k), Some(&synthetic_choices(&m, k, 5)))
            .replace("# SYNTHETIC", "# a note")
    };
    std::fs::write(friends_dir(&dir).join("one.txt"), fill("kit1")).unwrap();
    let (r, _) = run_text(&an, &[], &mut MockPlayer::default());
    assert!(r.is_err());
    let part = Gr3Args {
        partial: true,
        ..an.clone()
    };
    let (r, out) = run_text(&part, &[], &mut MockPlayer::default());
    r.unwrap();
    assert!(
        out.contains("PROVISIONAL") && out.contains("Decision:"),
        "{out}"
    );
    assert!(
        !out.contains("SYNTHETIC"),
        "these are not synthetic answers"
    );
    // the second friend finishes: final
    std::fs::write(friends_dir(&dir).join("two.txt"), fill("kit2")).unwrap();
    let (r, out) = run_text(&an, &[], &mut MockPlayer::default());
    r.unwrap();
    assert!(
        out.contains("Decision:") && !out.contains("PROVISIONAL"),
        "{out}"
    );
    let report: Report = read_json(&report_path(&dir)).unwrap();
    assert!(report.complete);
    assert_eq!(
        report
            .listeners
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>(),
        vec!["owner", "one", "two"]
    );
    // catch trials never enter the preference: judgements are main trials only
    let n_main = m.trials.iter().filter(|t| t.kind == Kind::Main).count();
    assert_eq!(n_main, 24 + 12 + 12);
    assert!(report.pooled.as_ref().unwrap().n as usize <= n_main);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn mistakes_are_reported_with_what_to_do() {
    let d = scratch("errors");
    let (r, _) = run_text(
        &args(&d.join("none"), None),
        &[],
        &mut MockPlayer::default(),
    );
    assert!(r.unwrap_err().contains("--set"));
    let (r, _) = run_text(
        &Gr3Args {
            analyze: true,
            ..args(&d.join("none"), None)
        },
        &[],
        &mut MockPlayer::default(),
    );
    assert!(r.unwrap_err().contains("no design"));
    // a design is not reused for another number of kits
    let set = demo_set(&d.join("set"), 12).unwrap();
    let dir = d.join("run");
    let (r, _) = run_text(
        &Gr3Args {
            export_kit: Some(d.join("k")),
            ..args(&dir, Some(set.clone()))
        },
        &[],
        &mut MockPlayer::default(),
    );
    r.unwrap();
    let (r, _) = run_text(
        &Gr3Args {
            friends: Some(3),
            ..args(&dir, None)
        },
        &[],
        &mut MockPlayer::default(),
    );
    assert!(r.unwrap_err().contains("2 kit(s)"));
    // a changed input is caught
    let wav = set.parent().unwrap().join("audio").join("ref-00-a.wav");
    let mut bytes = std::fs::read(&wav).unwrap();
    bytes.push(0);
    bytes.push(0);
    std::fs::write(&wav, bytes).unwrap();
    let (r, _) = run_text(&args(&dir, Some(set)), &[], &mut MockPlayer::default());
    assert!(
        r.unwrap_err().contains("ref-00/a"),
        "a changed candidate is named"
    );
    // a friend's typo is reported with its line
    let m: Manifest = read_json(&manifest_path(&dir)).unwrap();
    assert!(m.trials_of("kit1").len() > 3);
    std::fs::create_dir_all(friends_dir(&dir)).unwrap();
    std::fs::write(friends_dir(&dir).join("x.txt"), "kit: kit1\n01: maybe\n").unwrap();
    let (r, _) = run_text(
        &Gr3Args {
            analyze: true,
            partial: true,
            ..args(&dir, None)
        },
        &[],
        &mut MockPlayer::default(),
    );
    let e = r.unwrap_err();
    assert!(e.contains("x, line 2") && e.contains("a or b"), "{e}");
    std::fs::remove_dir_all(&d).unwrap();
}

// ---- the binary -----------------------------------------------------------------------------------------------

fn bin(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_enth-listen"))
        .args(args)
        .output()
        .expect("the tool runs")
}

#[test]
fn gr3_is_listed_in_help_and_its_own_help_names_the_modes() {
    let help = String::from_utf8_lossy(&bin(&["--help"]).stdout).to_string();
    assert!(help.contains("gr3"), "{help}");
    let own = String::from_utf8_lossy(&bin(&["gr3", "--help"]).stdout).to_string();
    for flag in [
        "--set",
        "--export-kit",
        "--analyze",
        "--partial",
        "--dry-run",
        "--friends",
        "--dir",
    ] {
        assert!(own.contains(flag), "{flag} missing:\n{own}");
    }
    assert!(
        !own.contains("--seed 0x"),
        "the bootstrap seed is not a flag"
    );
    assert!(!own.contains("bootstrap-seed"));
}

#[test]
fn the_binary_dry_runs_and_refuses_without_a_set() {
    let d = scratch("bin");
    let set = demo_set(&d.join("set"), 12).unwrap();
    let out = bin(&[
        "gr3",
        "--dry-run",
        "--set",
        set.to_str().unwrap(),
        "--dir",
        d.join("o").to_str().unwrap(),
        "--bootstrap",
        "200",
    ]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert!(
        text.contains("Decision:") && text.contains("SYNTHETIC"),
        "{text}"
    );
    let out = bin(&["gr3", "--dir", d.join("nothing").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--set"));
    std::fs::remove_dir_all(&d).unwrap();
}
