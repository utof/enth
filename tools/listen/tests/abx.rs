//! The ABX tool end to end, in quick mode (0.5 s clips, a 64-evaluation search, 4 targets, 4 sessions): the design
//! is made once and copied per test. Nothing here plays audio: sessions use a recording mock, `--dry-run`, or the
//! command `true`, which plays nothing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use enth_analysis::loudness::LoudnessMeter;
use enth_listen::abx::{
    AbxArgs, Manifest, Role, SCHEMA_ANSWERS, SessionSel, Side, StopAt, TrialKind, run,
};
use enth_listen::files::read_json;
use enth_listen::session::{AnswerFile, MockPlayer, Scripted};
use enth_listen::wav::decode;

fn args(dir: &Path) -> AbxArgs {
    AbxArgs {
        dir: dir.to_path_buf(),
        session: None,
        analyze: false,
        dry_run: false,
        player: "true".into(),
        seed: 20_261_019,
        targets: 4,
        trials: 4,
        workers: 2,
        time_cap: 60.0,
        budget: 64,
        stop_at: StopAt::T1,
        alpha: 0.05,
        quick: true,
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("enth-listen-abx-{name}-{}", std::process::id()));
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

/// The shared design (made by the first test that needs it).
fn designed() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = scratch("design");
        let mut out = Vec::new();
        run(&args(&d), &mut out, &mut Scripted::default(), None).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("solving 4 targets on 2 workers") && text.contains("session 1: 0/"),
            "{text}"
        );
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
    assert_eq!((m.items.len(), m.sessions()), (4, 4));
    // every session: one real trial per target, two null, at least one obviously different
    for s in 1..=4 {
        let ts = m.session_trials(s);
        assert_eq!(ts.iter().filter(|t| t.kind == TrialKind::Real).count(), 4);
        assert_eq!(ts.iter().filter(|t| t.kind == TrialKind::Null).count(), 2);
        assert!(ts.iter().any(|t| t.kind == TrialKind::Different));
        assert_eq!(
            ts.iter().map(|t| t.n).collect::<Vec<_>>(),
            (1..=ts.len()).collect::<Vec<_>>()
        );
    }
    // per target: X is A in half the sessions, and the target is A in half
    for it in &m.items {
        let real: Vec<_> = m
            .trials
            .iter()
            .filter(|t| t.kind == TrialKind::Real && t.item == it.index)
            .collect();
        assert_eq!(real.len(), 4);
        assert_eq!(
            real.iter().filter(|t| t.x_is == Side::A).count(),
            2,
            "target {}",
            it.index
        );
        let target_in_a = real
            .iter()
            .filter(|t| m.clip(&t.a).unwrap().role == Role::Target)
            .count();
        assert_eq!(target_in_a, 2, "target {}", it.index);
        for t in &real {
            assert_ne!(t.a, t.b);
        }
    }
    for t in m.trials.iter().filter(|t| t.kind == TrialKind::Null) {
        assert_eq!(t.a, t.b);
    }
    for t in m.trials.iter().filter(|t| t.kind == TrialKind::Different) {
        assert_ne!(t.item, t.item_b.unwrap());
    }
    // blind: random, unique ids that say nothing
    let ids: HashSet<&str> = m
        .trials
        .iter()
        .map(|t| t.id.as_str())
        .chain(m.clips.iter().map(|c| c.id.as_str()))
        .collect();
    assert_eq!(ids.len(), m.trials.len() + m.clips.len());
    assert!(
        ids.iter()
            .all(|i| !i.contains("target") && !i.contains("solution") && i.len() == 7)
    );
    // every clip: a mono 24-bit file at the same loudness (within 0.1 LU) and under the peak ceiling
    let mut meter = LoudnessMeter::new(48_000);
    let mut levels = Vec::new();
    for c in &m.clips {
        let w = decode(&std::fs::read(dir.join("clips").join(format!("{}.wav", c.id))).unwrap())
            .unwrap();
        assert_eq!((w.channels.len(), w.sample_rate), (1, 48_000));
        assert!(w.peak() <= 0.98 + 1e-6);
        levels.push(meter.integrated(&w.channels[0]).expect("audible"));
    }
    let (lo, hi) = levels
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(hi - lo < 0.1, "clips differ by {} LU: {levels:?}", hi - lo);
    // the manifest records the settings of the G0 gate it was made for
    assert_eq!((m.config.workers, m.config.lambda), (2, 10));
    assert!(
        m.items
            .iter()
            .all(|i| i.wall_s <= m.config.time_cap_s && i.t1 > 0.0 && i.t3 >= i.t1)
    );
}

#[test]
fn a_session_resumes_and_the_analysis_reads_the_answers() {
    let dir = scratch("resume");
    copy_dir(designed(), &dir);
    let m = manifest(&dir);
    let n1 = m.session_trials(1).len();
    let mut a = args(&dir);
    a.session = Some(SessionSel::One(1));
    // answer two trials, then quit
    let mut player = MockPlayer::default();
    let mut out = Vec::new();
    run(
        &a,
        &mut out,
        &mut Scripted::new(&["a", "b", "q"]),
        Some(&mut player),
    )
    .unwrap();
    let f: AnswerFile = read_json(&dir.join("answers-session1.json")).unwrap();
    assert_eq!(
        (f.answers.len(), f.schema.as_str(), f.synthetic),
        (2, SCHEMA_ANSWERS, false)
    );
    assert_eq!(
        player.played.len(),
        3,
        "each trial plays once on presentation; the third was quit after playing"
    );
    let first_two: Vec<_> = f
        .answers
        .iter()
        .map(|x| (x.id.clone(), x.choice.clone()))
        .collect();
    // resume: the same command carries on from trial 3 and replays are counted
    let mut lines: Vec<&str> = vec!["ra", "rx", "r", "a"];
    lines.extend(std::iter::repeat_n("b", n1));
    run(
        &a,
        &mut Vec::new(),
        &mut Scripted::new(&lines),
        Some(&mut MockPlayer::default()),
    )
    .unwrap();
    let f: AnswerFile = read_json(&dir.join("answers-session1.json")).unwrap();
    assert_eq!(f.answers.len(), n1);
    assert_eq!(
        f.answers
            .iter()
            .take(2)
            .map(|x| (x.id.clone(), x.choice.clone()))
            .collect::<Vec<_>>(),
        first_two
    );
    assert_eq!(f.answers[2].replays, 3);
    assert_eq!(
        f.answers
            .iter()
            .map(|x| x.id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        n1,
        "no trial answered twice"
    );
    // `next` moves on to session 2; a session that does not exist is an error
    a.session = Some(SessionSel::Next);
    run(
        &a,
        &mut Vec::new(),
        &mut Scripted::new(&["a", "q"]),
        Some(&mut MockPlayer::default()),
    )
    .unwrap();
    assert!(dir.join("answers-session2.json").exists());
    a.session = Some(SessionSel::One(9));
    let err = run(
        &a,
        &mut Vec::new(),
        &mut Scripted::default(),
        Some(&mut MockPlayer::default()),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("session 9 does not exist"),
        "{err}"
    );
    // analysis of what is there: pending targets, an open gate, a report file
    a.session = None;
    a.analyze = true;
    let mut out = Vec::new();
    run(&a, &mut out, &mut Scripted::default(), None).unwrap();
    let text = String::from_utf8(out).unwrap();
    // four trials per target can never flag a target as different, so this small design reads INVALID, not OPEN
    assert!(
        text.contains("ABX analysis") && text.contains("pending") && text.contains("INVALID"),
        "{text}"
    );
    assert!(!text.contains("SYNTHETIC"));
    let report: serde_json::Value = read_json(&dir.join("abx-report.json")).unwrap();
    assert_eq!(report["schema"], "enth.listen.abx-report/1");
    assert_eq!(report["synthetic"], false);
}

#[test]
fn synthetic_answers_are_never_mistaken_for_the_owners() {
    let dir = scratch("synthetic");
    copy_dir(designed(), &dir);
    let m = manifest(&dir);
    let f = enth_listen::abx::synthetic_answers(&m, 1, 7);
    assert!(f.synthetic && f.answers.len() == m.session_trials(1).len());
    enth_listen::files::write_json(&dir.join("answers-session1.json"), &f).unwrap();
    let mut a = args(&dir);
    a.session = Some(SessionSel::One(1));
    let err = run(
        &a,
        &mut Vec::new(),
        &mut Scripted::default(),
        Some(&mut MockPlayer::default()),
    )
    .unwrap_err();
    assert!(err.to_string().contains("synthetic"), "{err}");
}

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_enth-listen"));
    c.env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
    c
}

#[test]
fn the_binary_dry_run_works_end_to_end_and_plays_nothing() {
    // the binary's dry run lives in DIR/dry-run; put the shared design there so it is reused, not redone
    let root = scratch("cli-dry");
    copy_dir(designed(), &root.join("dry-run"));
    let o = bin()
        .args([
            "abx",
            "--dry-run",
            "--quick",
            "--targets",
            "4",
            "--trials",
            "4",
            "--workers",
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
    assert!(
        text.contains("G0 search gate") && text.contains("controls:"),
        "{text}"
    );
    let report: serde_json::Value = read_json(&root.join("dry-run/abx-report.json")).unwrap();
    assert_eq!(report["synthetic"], true);
    assert_eq!(report["items"].as_array().unwrap().len(), 4);
}

#[test]
fn the_binary_session_uses_the_player_command_and_stdin() {
    let dir = scratch("cli-session");
    copy_dir(designed(), &dir);
    let mut child = bin()
        .args([
            "abx",
            "--quick",
            "--targets",
            "4",
            "--trials",
            "4",
            "--workers",
            "2",
            "--session",
            "1",
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
        stdin.write_all("a\nb\n".repeat(20).as_bytes()).unwrap();
    }
    let o = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        text.contains("playing with: true") && text.contains("Session 1 done"),
        "{text}"
    );
    let n1 = manifest(&dir).session_trials(1).len();
    let f: AnswerFile = read_json(&dir.join("answers-session1.json")).unwrap();
    assert_eq!(f.answers.len(), n1);
    // a missing player is a clean error with a hint, not a panic
    let o = bin()
        .args([
            "abx",
            "--quick",
            "--targets",
            "4",
            "--trials",
            "4",
            "--workers",
            "2",
            "--session",
            "2",
            "--player",
            "definitely-not-a-player-xyz",
            "--dir",
        ])
        .arg(&dir)
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
