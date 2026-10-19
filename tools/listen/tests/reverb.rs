//! The reverb A/B tool end to end, in quick mode (16 kHz, short sources, loose calibration, 2 repeats). The design
//! (which runs fundsp unoptimised) is made once and copied per test. Nothing here plays audio.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use enth_listen::files::read_json;
use enth_listen::loudness::StereoMeter;
use enth_listen::reverb::{Candidate, Manifest, ReverbArgs, run};
use enth_listen::session::{AnswerFile, MockPlayer, Scripted};
use enth_listen::wav::decode;

/// A short decay keeps the design cheap: fundsp runs unoptimised in a test build.
const RT60: f64 = 1.0;

fn args(dir: &Path) -> ReverbArgs {
    ReverbArgs {
        dir: dir.to_path_buf(),
        analyze: false,
        dry_run: false,
        player: "true".into(),
        seed: 20_261_019,
        rt60: RT60,
        wet_db: -6.0,
        repeats: 2,
        quick: true,
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("enth-listen-reverb-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), to).unwrap();
        }
    }
}

fn designed() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = scratch("design");
        let mut out = Vec::new();
        run(&args(&d), &mut out, &mut Scripted::default(), None).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("designed 9 clips and 18 trials"), "{text}");
        d
    })
}

fn manifest(dir: &Path) -> Manifest {
    read_json(&dir.join("manifest.json")).unwrap()
}

#[test]
fn the_design_is_balanced_blind_and_level_matched() {
    let dir = designed();
    let m = manifest(dir);
    // 3 sources x 3 reverbs, and 3 sources x 3 pairs x 2 repeats
    assert_eq!((m.clips.len(), m.trials.len(), m.sources.len()), (9, 18, 3));
    let names: HashSet<&str> = m.sources.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, HashSet::from(["pad", "pluck", "transient"]));
    assert_eq!(
        m.trials.iter().map(|t| t.n).collect::<Vec<_>>(),
        (1..=18).collect::<Vec<_>>()
    );
    for s in &m.sources {
        for (x, y) in [
            (Candidate::Ours, Candidate::Reverb2),
            (Candidate::Ours, Candidate::Reverb4),
            (Candidate::Reverb2, Candidate::Reverb4),
        ] {
            let ts: Vec<_> = m
                .trials
                .iter()
                .filter(|t| {
                    t.source == s.name
                        && [t.a_candidate, t.b_candidate].contains(&x)
                        && [t.a_candidate, t.b_candidate].contains(&y)
                })
                .collect();
            assert_eq!(ts.len(), 2, "{} {x:?} {y:?}", s.name);
            assert_eq!(
                ts.iter().filter(|t| t.a_candidate == x).count(),
                1,
                "A/B order is balanced"
            );
        }
    }
    for t in &m.trials {
        assert_ne!(t.a_candidate, t.b_candidate);
        assert_eq!(
            m.clips.iter().find(|c| c.id == t.a).unwrap().candidate,
            t.a_candidate
        );
        assert_eq!(
            m.clips.iter().find(|c| c.id == t.b).unwrap().candidate,
            t.b_candidate
        );
    }
    // blind: ids say nothing about the reverb or the source
    let ids: HashSet<&str> = m
        .trials
        .iter()
        .map(|t| t.id.as_str())
        .chain(m.clips.iter().map(|c| c.id.as_str()))
        .collect();
    assert_eq!(ids.len(), m.trials.len() + m.clips.len());
    assert!(ids.iter().all(|i| {
        !["ours", "reverb", "fundsp", "pad", "pluck"]
            .iter()
            .any(|w| i.contains(w))
    }));
    // the clips: stereo 24-bit, under the peak ceiling, and the three of a source at the same loudness (within 0.1 LU
    // in the stereo BS.1770 measure; the level a source is matched to may sit under -23 LUFS if the loudest clip
    // would otherwise clip: the sources are not compared with each other)
    let mut levels = Vec::new();
    for c in &m.clips {
        let w = decode(&std::fs::read(dir.join("clips").join(format!("{}.wav", c.id))).unwrap())
            .unwrap();
        assert_eq!((w.channels.len(), w.sample_rate), (2, 16_000));
        assert!(w.peak() <= 0.98 + 1e-6);
        let mut meter = StereoMeter::new(w.sample_rate);
        levels.push((
            c.source.clone(),
            meter
                .integrated_any_level(&w.channels[0], &w.channels[1])
                .expect("audible"),
        ));
        assert!(w.channels.iter().flatten().all(|v| v.is_finite()));
    }
    for s in &m.sources {
        let of: Vec<f64> = levels
            .iter()
            .filter(|(n, _)| *n == s.name)
            .map(|(_, l)| *l)
            .collect();
        let (lo, hi) = of
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert_eq!(of.len(), 3);
        assert!(
            hi - lo < 0.1,
            "{}: clips differ by {} LU: {of:?}",
            s.name,
            hi - lo
        );
        assert!((-26.0..=-22.9).contains(&lo), "{}: {of:?}", s.name);
    }
    // all three reverbs were calibrated to the requested decay (quick mode is loose: 12 %)
    assert_eq!(m.calibration.len(), 3);
    for c in &m.calibration {
        assert!(((c.measured_rt60_s - RT60) / RT60).abs() < 0.15, "{c:?}");
    }
}

#[test]
fn a_session_resumes_and_the_analysis_reads_the_answers() {
    let dir = scratch("resume");
    copy_dir(designed(), &dir);
    let a = args(&dir);
    // two answers (one of them "no preference"), then quit
    let mut player = MockPlayer::default();
    let mut out = Vec::new();
    run(
        &a,
        &mut out,
        &mut Scripted::new(&["a", "=", "q"]),
        Some(&mut player),
    )
    .unwrap();
    let f: AnswerFile = read_json(&dir.join("answers-session1.json")).unwrap();
    assert_eq!((f.answers.len(), f.synthetic), (2, false));
    assert_eq!(f.answers[1].choice, "=");
    assert!(String::from_utf8(out).unwrap().contains("saved 2 answers"));
    // resume with a replay on the third trial, then finish
    let mut lines: Vec<&str> = vec!["rb", "b"];
    lines.extend(std::iter::repeat_n("n", 15));
    run(
        &a,
        &mut Vec::new(),
        &mut Scripted::new(&lines),
        Some(&mut MockPlayer::default()),
    )
    .unwrap();
    let f: AnswerFile = read_json(&dir.join("answers-session1.json")).unwrap();
    assert_eq!(f.answers.len(), 18);
    assert_eq!(f.answers[2].replays, 1);
    assert_eq!(f.answers[3].choice, "=", "n is a synonym of =");
    assert_eq!(
        f.answers
            .iter()
            .map(|x| x.id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        18
    );
    // analysis: one report, honest about answers that were not a preference
    let mut b = args(&dir);
    b.analyze = true;
    let mut out = Vec::new();
    run(&b, &mut out, &mut Scripted::default(), None).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("reverb A/B analysis") && !text.contains("SYNTHETIC"),
        "{text}"
    );
    let report: serde_json::Value = read_json(&dir.join("reverb-report.json")).unwrap();
    assert_eq!(report["schema"], "enth.listen.reverb-report/1");
    assert_eq!(report["synthetic"], false);
}

#[test]
fn analysing_before_listening_is_a_clear_error() {
    let dir = scratch("noanswers");
    copy_dir(designed(), &dir);
    let mut a = args(&dir);
    a.analyze = true;
    let err = run(&a, &mut Vec::new(), &mut Scripted::default(), None).unwrap_err();
    assert!(err.to_string().contains("no answers yet"), "{err}");
}

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_enth-listen"));
    c.env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
    c
}

#[test]
fn the_binary_dry_run_works_end_to_end_and_plays_nothing() {
    let root = scratch("cli-dry");
    copy_dir(designed(), &root.join("dry-run"));
    let o = bin()
        .args([
            "reverb-ab",
            "--dry-run",
            "--quick",
            "--repeats",
            "2",
            "--player",
            "definitely-not-a-player-xyz",
            "--dir",
        ])
        .arg(&root)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        text.contains("using the stimuli designed earlier") && text.contains("SYNTHETIC ANSWERS"),
        "{text}"
    );
    let report: serde_json::Value = read_json(&root.join("dry-run/reverb-report.json")).unwrap();
    assert_eq!(report["synthetic"], true);
}

#[test]
fn the_binary_session_uses_the_player_command_and_stdin() {
    let dir = scratch("cli-session");
    copy_dir(designed(), &dir);
    let mut child = bin()
        .args([
            "reverb-ab",
            "--quick",
            "--repeats",
            "2",
            "--player",
            "true",
            "--dir",
        ])
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all("a\nb\n".repeat(10).as_bytes()).unwrap();
    }
    let o = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        text.contains("playing with: true") && text.contains("Done: 18 answers"),
        "{text}"
    );
    // a missing player is a clean error naming --player
    let dir2 = scratch("cli-noplayer");
    copy_dir(designed(), &dir2);
    let o = bin()
        .args([
            "reverb-ab",
            "--quick",
            "--repeats",
            "2",
            "--player",
            "definitely-not-a-player-xyz",
            "--dir",
        ])
        .arg(&dir2)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("--player"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}
