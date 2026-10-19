//! End to end through the binary: `listen --dry-run`, `solve` and `bench` on short clips. Nothing here plays audio.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_spike-metric"));
    c.env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
    c
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("spike-metric-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn listen_dry_run_works_end_to_end_without_playing() {
    let dir = tmp("listen");
    let out = bin()
        .args([
            "listen",
            "--dry-run",
            "--quick",
            "--pool",
            "60",
            "--workers",
            "4",
            "--bootstrap",
            "200",
            "--dir",
        ])
        .arg(&dir)
        // a player that would fail loudly if it were ever called
        .args(["--player", "/nonexistent/never-play"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stdout: {text}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("SYNTHETIC ANSWERS"), "{text}");
    assert!(
        text.contains("Spearman") && text.contains("Kendall"),
        "{text}"
    );
    let d = dir.join("dry-run");
    for f in [
        "manifest.json",
        "answers-session1.json",
        "answers-session2.json",
        "listen-report.json",
    ] {
        assert!(d.join(f).exists(), "missing {f}");
    }
    let wavs = |s: &str| {
        std::fs::read_dir(d.join(s))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "wav")
            })
            .count()
    };
    assert_eq!(wavs("session1"), 34);
    assert_eq!(wavs("session2"), 34);
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("listen-report.json")).unwrap())
            .unwrap();
    assert_eq!(report["schema"], "enth.spike-metric.listen-report/1");
    assert_eq!(report["synthetic"], true);
    assert_eq!(report["metrics"].as_array().unwrap().len(), 3);
    // the answer files are marked synthetic and the real directory was not touched
    let a1: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("answers-session1.json")).unwrap())
            .unwrap();
    assert_eq!(a1["synthetic"], true);
    assert!(!dir.join("manifest.json").exists());
    // a second run reuses the stimuli
    let again = bin()
        .args([
            "listen",
            "--dry-run",
            "--quick",
            "--bootstrap",
            "100",
            "--dir",
        ])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(again.status.success());
    assert!(String::from_utf8_lossy(&again.stdout).contains("using the stimuli designed earlier"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn listen_rate_fails_cleanly_with_a_missing_player() {
    let dir = tmp("noplayer");
    let out = bin()
        .args([
            "listen",
            "--quick",
            "--pool",
            "40",
            "--workers",
            "2",
            "--session",
            "1",
            "--player",
            "/nonexistent/never-play",
            "--dir",
        ])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot run"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn solve_runs_and_reports() {
    let dir = tmp("solve");
    let json = dir.join("solve.json");
    // every distance, heuristic start
    let out = bin()
        .args([
            "solve",
            "--quick",
            "--targets",
            "1",
            "--budget",
            "40",
            "--workers",
            "2",
            "--distance",
            "all",
            "--start",
            "heuristic",
            "--quiet",
            "--json",
        ])
        .arg(&json)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(text.matches("SUMMARY").count(), 3, "{text}");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(v["schema"], "enth.spike-metric.solve/1");
    assert_eq!(v["cells"].as_array().unwrap().len(), 3);
    assert_eq!(v["cells"][0]["results"].as_array().unwrap().len(), 1);
    // heuristic and random searches side by side, BIPOP-style restarts
    let out = bin()
        .args([
            "solve",
            "--quick",
            "--targets",
            "1",
            "--budget",
            "40",
            "--workers",
            "2",
            "--start",
            "both",
            "--restarts",
            "bipop",
            "--quiet",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("SUMMARY dist A start Both"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bench_reports_identical_results_across_workers() {
    let out = bin()
        .args([
            "bench",
            "--quick",
            "--reps",
            "4",
            "--evals",
            "24",
            "--workers",
            "1,2,3",
            "--batches",
            "8",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("one evaluation") && text.contains("evals/s"),
        "{text}"
    );
    let rows: Vec<&str> = text.lines().filter(|l| l.contains("(hash")).collect();
    assert_eq!(rows.len(), 3, "{text}");
    assert!(rows.iter().all(|r| r.contains("true")), "{text}");
}
