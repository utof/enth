//! `cargo xtask leak-test`: the plugin's leak gates (P1b lane INT; docs/phase-1b.md, "The plugin-side G1 checks").
//!
//! Two runs of `enth-test-host`, both under Xvfb (`xvfb-run` is started here, with no display variable passed on, so
//! nothing can reach the real display; AGENTS.md, "UI"):
//!
//! - **`editor-cycles`**: 200 open/close cycles of the editor of a release bundle built with the plugin's
//!   `editor-counts` feature, twice, against the test host's no-editor baseline (each in its own process). It passes
//!   when the anonymous PSS growth after close (the worst editor run's least-squares slope after the warm-up, minus
//!   the baseline runs' mean) is at most 26 KB per cycle (docs/benchmarks.md; lane FK measured 14.8 to 18.3 KB with
//!   the vendored baseview's xkb fix, about 84 without it), the thread count after every close is one value, every
//!   open drew its first frame in time, and the plugin counted exactly one `editor_closed` call per `build()`, one per
//!   cycle (the vendored nice-plug-egui's patch 2; the counts come from `$ENTH_EDITOR_COUNTS`, which the plugin
//!   appends to when it is dropped). The anonymous part is what a leak adds: the test host's headline `pss_kb` also
//!   counts the shared libraries' pages (Mesa, llvmpipe), which a loaded machine reclaims and re-faults (lane FK saw
//!   108 KB per cycle in `pss_kb` against 17.8 in the anonymous part in one run). Both are printed.
//! - **`churn`**: 200 whole instance lives (instantiate, load a patch, activate, play, deactivate, drop) must leave
//!   PSS flat (at most 1 KB per cycle, the test host's default) and no thread or descriptor behind. The test host's
//!   exit status is the verdict.
//!
//! Reports: `target/leak-test/` (`editor-cycles.json`, `editor-counts.jsonl`, `churn.json`, `leak-test.json`). The
//! exit status is the verdict. A run takes several minutes (about 2 × 200 cycles of 0.15 s or more per mode, plus the
//! builds); run it in the foreground with a timeout above that, under `flock -s /tmp/enth-quiet.lock`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::{bundle, plugin};

const USAGE: &str = "usage: cargo xtask leak-test [--package <pkg>] [--cycles 200] [--repeat 2] [--churn-cycles 200]
       [--only editor|churn] [--no-build] [--bundle-dir <dir>] [--max-kb-per-cycle 26]";

/// The editor gate's limit, KB of anonymous PSS per open/close cycle (docs/benchmarks.md).
const MAX_EDITOR_KB_PER_CYCLE: f64 = 26.0;
/// Cycles left out of the growth fit (the first opens load Mesa and warm caches); the test host's default.
const WARMUP: usize = 10;
/// The Xvfb screen every window here is opened on.
const XVFB_SCREEN: &str = "-screen 0 1920x1080x24";
/// The patch each churn cycle loads (the plugin-level golden's: two oscillators, a filter, envelopes, effects).
const CHURN_PATCH: &str = "tools/test-host/golden/enth-plugin.patch.json";

struct Options {
    package: String,
    cycles: usize,
    repeat: usize,
    churn_cycles: usize,
    editor: bool,
    churn: bool,
    build: bool,
    bundle_dir: Option<PathBuf>,
    max_kb: f64,
}

fn parse_options(args: &[String], root: &Path) -> Result<Options, String> {
    let (package, rest) = plugin::one_package(plugin::split_packages(args, false)?, root)?;
    let mut options = Options {
        package,
        cycles: 200,
        repeat: 2,
        churn_cycles: 200,
        editor: true,
        churn: true,
        build: true,
        bundle_dir: None,
        max_kb: MAX_EDITOR_KB_PER_CYCLE,
    };
    let mut iter = rest.iter();
    let number = |name: &str, value: Option<&String>| -> Result<usize, String> {
        value
            .and_then(|v| v.parse().ok())
            .filter(|n| *n >= 1)
            .ok_or_else(|| format!("{name} needs a positive whole number; {USAGE}"))
    };
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--cycles" => options.cycles = number(arg, iter.next())?,
            "--repeat" => options.repeat = number(arg, iter.next())?,
            "--churn-cycles" => options.churn_cycles = number(arg, iter.next())?,
            "--only" => match iter.next().map(String::as_str) {
                Some("editor") => options.churn = false,
                Some("churn") => options.editor = false,
                other => return Err(format!("--only takes editor or churn, got {other:?}")),
            },
            "--no-build" => options.build = false,
            "--bundle-dir" => {
                options.bundle_dir = Some(PathBuf::from(
                    iter.next().ok_or("--bundle-dir needs a directory")?,
                ));
                options.build = false;
            }
            "--max-kb-per-cycle" => {
                options.max_kb = iter
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|v: &f64| v.is_finite() && *v > 0.0)
                    .ok_or_else(|| {
                        format!("--max-kb-per-cycle needs a positive number; {USAGE}")
                    })?;
            }
            other => return Err(format!("unknown option {other:?}; {USAGE}")),
        }
    }
    if options.cycles <= WARMUP + 2 {
        return Err(format!(
            "--cycles must be above {}: the first {WARMUP} are the warm-up",
            WARMUP + 2
        ));
    }
    if options.churn_cycles < 3 {
        return Err("--churn-cycles needs at least 3".to_owned());
    }
    Ok(options)
}

/// One gate line.
struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

pub fn leak_test(args: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("leak-test measures /proc and Xvfb: Linux only".to_owned());
    }
    let meta = crate::metadata()?;
    let root = meta.workspace_root.clone();
    let options = parse_options(args, &root)?;
    plugin::require_member(&meta, &options.package)?;
    require_xvfb_run()?;
    let out = meta.target_directory.join("leak-test");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;

    let clap = out.join(format!("{}.clap", options.package));
    let host = meta.target_directory.join("release").join("enth-test-host");
    if options.build {
        // The plugin with its editor counters, copied out of `target/bundled` at once so that a later plain bundle
        // never runs with the feature on (or this gate without it).
        bundle::bundle(&[
            options.package.clone(),
            "--release".to_owned(),
            "--locked".to_owned(),
            "--features".to_owned(),
            "editor-counts".to_owned(),
        ])?;
        let built = bundle::bundled_dir()?.join(format!("{}.clap", options.package));
        bundle::replace_with_copy(&built, &clap)?;
        let status = crate::cargo()
            .args(["build", "--release", "--locked", "-p", "enth-test-host"])
            .status()
            .map_err(|e| format!("cannot run cargo build: {e}"))?;
        if !status.success() {
            return Err(format!("building enth-test-host failed ({status})"));
        }
    } else if let Some(dir) = &options.bundle_dir {
        let given = dir.join(format!("{}.clap", options.package));
        if !given.exists() {
            return Err(format!("{} does not exist", given.display()));
        }
        bundle::replace_with_copy(&given, &clap)?;
    }
    for (path, how) in [
        (&clap, "build it (leave out --no-build)"),
        (&host, "cargo build --release -p enth-test-host"),
    ] {
        if !path.exists() {
            return Err(format!("{} is missing: {how}", path.display()));
        }
    }
    println!("leak-test: plugin {}", clap.display());
    println!("leak-test: host {}", host.display());
    println!("leak-test: reports in {}", out.display());

    let mut checks = Vec::new();
    let mut summary =
        json!({ "schema": "enth.xtask.leak-test/1", "plugin": clap.display().to_string() });
    if options.editor {
        let report = editor_cycles(&host, &clap, &out, &options)?;
        let counts = read_counts(&out.join("editor-counts.jsonl"))?;
        let (editor_checks, facts) = judge_editor(&report, &counts, &options);
        checks.extend(editor_checks);
        summary["editor_cycles"] = facts;
    }
    if options.churn {
        let (ok, facts) = churn(&host, &clap, &out, &root, &options)?;
        checks.push(Check {
            name: "churn: instance lives leave nothing behind",
            ok,
            detail: facts["detail"].as_str().unwrap_or_default().to_owned(),
        });
        summary["churn"] = facts;
    }

    let failed = checks.iter().filter(|c| !c.ok).count();
    summary["checks"] = checks
        .iter()
        .map(|c| json!({ "name": c.name, "ok": c.ok, "detail": c.detail }))
        .collect();
    summary["passed"] = json!(failed == 0);
    let path = out.join("leak-test.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&summary).unwrap_or_default(),
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    for c in &checks {
        println!(
            "  {:<4} {}: {}",
            if c.ok { "ok" } else { "FAIL" },
            c.name,
            c.detail
        );
    }
    if failed == 0 {
        println!("leak-test: ok ({} checks)", checks.len());
        Ok(())
    } else {
        Err(format!(
            "leak-test: {failed} of {} checks failed (reports in {})",
            checks.len(),
            out.display()
        ))
    }
}

fn require_xvfb_run() -> Result<(), String> {
    let found = std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join("xvfb-run").is_file())
    });
    if found {
        Ok(())
    } else {
        Err(
            "`xvfb-run` not found in PATH: the editor must never open on the real display"
                .to_owned(),
        )
    }
}

/// `enth-test-host <args>` under its own Xvfb server, with no display variable from this environment.
fn under_xvfb(host: &Path, args: &[String]) -> Command {
    let mut cmd = Command::new("xvfb-run");
    cmd.args(["-a", "-s", XVFB_SCREEN])
        .arg(host)
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    cmd
}

/// Runs `editor-cycles --mode both` and returns its JSON report.
fn editor_cycles(host: &Path, clap: &Path, out: &Path, options: &Options) -> Result<Value, String> {
    let counts = out.join("editor-counts.jsonl");
    let report = out.join("editor-cycles.json");
    for stale in [&counts, &report] {
        if stale.exists() {
            std::fs::remove_file(stale).map_err(|e| format!("{}: {e}", stale.display()))?;
        }
    }
    println!(
        "leak-test: editor-cycles, {} cycles, {} editor and {} baseline runs",
        options.cycles, options.repeat, options.repeat
    );
    let args = [
        "editor-cycles".to_owned(),
        clap.display().to_string(),
        "-n".to_owned(),
        options.cycles.to_string(),
        "--warmup".to_owned(),
        WARMUP.to_string(),
        "--mode".to_owned(),
        "both".to_owned(),
        "--repeat".to_owned(),
        options.repeat.to_string(),
        "--json-out".to_owned(),
        report.display().to_string(),
    ];
    let log = out.join("editor-cycles.log");
    let output = under_xvfb(host, &args)
        .env("ENTH_EDITOR_COUNTS", &counts)
        .output()
        .map_err(|e| format!("cannot run xvfb-run: {e}"))?;
    let mut text = output.stdout.clone();
    text.extend_from_slice(&output.stderr);
    std::fs::write(&log, &text).map_err(|e| format!("{}: {e}", log.display()))?;
    if !output.status.success() {
        return Err(format!(
            "enth-test-host editor-cycles failed ({}); see {}",
            output.status,
            log.display()
        ));
    }
    let json =
        std::fs::read_to_string(&report).map_err(|e| format!("{}: {e}", report.display()))?;
    serde_json::from_str(&json).map_err(|e| format!("{}: {e}", report.display()))
}

/// The plugin's `enth.plugin-editor-counts/1` lines: (builds, `editor_closed` calls), one per plugin instance dropped.
fn read_counts(path: &Path) -> Result<Vec<(u64, u64)>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: Value =
                serde_json::from_str(line).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((
                v["builds"].as_u64().unwrap_or(0),
                v["editor_closed"].as_u64().unwrap_or(0),
            ))
        })
        .collect()
}

/// Least-squares slope of `ys` against their index, after the first `warmup` values.
fn slope_after(ys: &[f64], warmup: usize) -> Option<f64> {
    let ys = ys.get(warmup..)?;
    let n = ys.len();
    if n < 2 {
        return None;
    }
    let mx = (n - 1) as f64 / 2.0;
    let my = ys.iter().sum::<f64>() / n as f64;
    let (mut num, mut den) = (0.0, 0.0);
    for (x, y) in ys.iter().enumerate() {
        let dx = x as f64 - mx;
        num += dx * (y - my);
        den += dx * dx;
    }
    Some(num / den)
}

fn numbers(v: &Value) -> Vec<f64> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

/// The editor gate's checks from the test host's report and the plugin's counts.
fn judge_editor(report: &Value, counts: &[(u64, u64)], options: &Options) -> (Vec<Check>, Value) {
    let runs = report["runs"].as_array().cloned().unwrap_or_default();
    let of = |mode: &str| -> Vec<&Value> { runs.iter().filter(|r| r["mode"] == mode).collect() };
    let (editor, baseline) = (of("editor"), of("baseline"));
    let anon = |r: &Value| slope_after(&numbers(&r["series"]["pss_anon_kb"]), WARMUP);
    let editor_slopes: Vec<f64> = editor.iter().filter_map(|r| anon(r)).collect();
    let baseline_slopes: Vec<f64> = baseline.iter().filter_map(|r| anon(r)).collect();
    let base_mean = (!baseline_slopes.is_empty())
        .then(|| baseline_slopes.iter().sum::<f64>() / baseline_slopes.len() as f64);
    let worst = editor_slopes.iter().copied().reduce(f64::max);
    let growth = worst.zip(base_mean).map(|(w, b)| w - b);

    let mut threads: Vec<u64> = editor
        .iter()
        .flat_map(|r| {
            r["series"]["threads_closed"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|t| t.as_u64())
        .collect();
    threads.sort_unstable();
    threads.dedup();
    let threads_steady = editor
        .iter()
        .all(|r| r["summary"]["threads_constant_after_warmup"] == true);
    let drawn: Vec<u64> = editor
        .iter()
        .map(|r| r["summary"]["opens_drawn_in_time"].as_u64().unwrap_or(0))
        .collect();
    let cycles = options.cycles as u64;
    let editor_counts: Vec<(u64, u64)> = counts.iter().copied().filter(|(b, _)| *b > 0).collect();
    let baseline_counts = counts.iter().filter(|(b, _)| *b == 0).count();
    let round = |v: &[f64]| {
        v.iter()
            .map(|x| (x * 100.0).round() / 100.0)
            .collect::<Vec<_>>()
    };
    let host_pss = &report["comparison"];

    let checks = vec![
        Check {
            name: "editor: one editor_closed call per build, one build per cycle",
            ok: editor_counts.len() == editor.len()
                && !editor.is_empty()
                && editor_counts
                    .iter()
                    .all(|&(b, c)| b == cycles && c == cycles),
            detail: format!(
                "(builds, editor_closed) per editor run {editor_counts:?} for {cycles} cycles; {baseline_counts} baseline \
                 runs counted nothing{}",
                if counts.is_empty() {
                    " (no counts at all: was the bundle built with --features editor-counts?)"
                } else {
                    ""
                }
            ),
        },
        Check {
            name: "editor: anonymous PSS growth per cycle",
            ok: growth.is_some_and(|g| g <= options.max_kb),
            detail: format!(
                "worst editor run minus baseline {} KB per cycle, limit {} (anonymous slopes: editor {:?}, baseline {:?}; \
                 the test host's pss_kb slopes: editor {}, baseline {})",
                growth.map_or("n/a".to_owned(), |g| format!("{g:.2}")),
                options.max_kb,
                round(&editor_slopes),
                round(&baseline_slopes),
                host_pss["editor_pss_growth_kb_per_cycle"],
                host_pss["baseline_pss_growth_kb_per_cycle"],
            ),
        },
        Check {
            name: "editor: threads constant after close",
            ok: threads.len() == 1 && threads_steady,
            detail: format!("thread counts after a close: {threads:?}"),
        },
        Check {
            name: "editor: every open drew its first frame in time",
            ok: !drawn.is_empty() && drawn.iter().all(|d| *d == cycles),
            detail: format!("{drawn:?} of {cycles} per editor run"),
        },
    ];
    let facts = json!({
        "cycles": cycles,
        "repeat": options.repeat,
        "warmup": WARMUP,
        "anon_kb_per_cycle": { "editor": editor_slopes, "baseline": baseline_slopes, "worst_minus_baseline": growth },
        "pss_kb_per_cycle_test_host": host_pss,
        "threads_after_close": threads,
        "drawn_in_time": drawn,
        "counts": counts,
        "loadavg": report["environment"]["loadavg"],
    });
    (checks, facts)
}

/// Runs `churn` and returns (passed, facts).
fn churn(
    host: &Path,
    clap: &Path,
    out: &Path,
    root: &Path,
    options: &Options,
) -> Result<(bool, Value), String> {
    let report = out.join("churn.json");
    println!("leak-test: churn, {} instance lives", options.churn_cycles);
    let args = [
        "churn".to_owned(),
        clap.display().to_string(),
        "-n".to_owned(),
        options.churn_cycles.to_string(),
        "--state".to_owned(),
        root.join(CHURN_PATCH).display().to_string(),
        "--json-out".to_owned(),
        report.display().to_string(),
    ];
    let log = out.join("churn.log");
    let output = under_xvfb(host, &args)
        .output()
        .map_err(|e| format!("cannot run xvfb-run: {e}"))?;
    let mut text = output.stdout.clone();
    text.extend_from_slice(&output.stderr);
    std::fs::write(&log, &text).map_err(|e| format!("{}: {e}", log.display()))?;
    let code = output.status.code();
    if !matches!(code, Some(0 | 1)) {
        return Err(format!(
            "enth-test-host churn failed to run ({}); see {}",
            output.status,
            log.display()
        ));
    }
    let json: Value = std::fs::read_to_string(&report)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let detail = format!("exit {}; {}", code.unwrap_or(-1), churn_detail(&json));
    Ok((
        code == Some(0),
        json!({ "exit": code, "detail": detail, "report": json }),
    ))
}

/// The churn report's headline numbers (`enth.test-host.churn/1`).
fn churn_detail(report: &Value) -> String {
    let s = &report["summary"];
    if s.is_null() {
        return "no report".to_owned();
    }
    format!(
        "PSS {} KB per cycle after the warm-up ({} in the second half; limit {}), threads +{}, descriptors +{}, \
         failures {}",
        s["pss_kb_per_cycle_after_warmup"],
        s["pss_kb_per_cycle_second_half"],
        report["limits"]["max_kb_per_cycle"],
        s["thread_growth_after_warmup"],
        s["fd_growth_after_warmup"],
        report["failures"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slope_skips_the_warm_up() {
        let ys: Vec<f64> = (0..20)
            .map(|i| if i < 5 { 1000.0 } else { 2.0 * f64::from(i) })
            .collect();
        let s = slope_after(&ys, 5).unwrap();
        assert!((s - 2.0).abs() < 1e-9, "{s}");
        assert_eq!(slope_after(&ys, 19), None);
    }

    fn run(mode: &str, anon: &[f64], threads: &[u64], drawn: u64) -> Value {
        json!({
            "mode": mode,
            "series": { "pss_anon_kb": anon, "threads_closed": threads },
            "summary": { "threads_constant_after_warmup": true, "opens_drawn_in_time": drawn },
        })
    }

    fn options(cycles: usize) -> Options {
        Options {
            package: "enth-plugin".to_owned(),
            cycles,
            repeat: 1,
            churn_cycles: 3,
            editor: true,
            churn: false,
            build: false,
            bundle_dir: None,
            max_kb: 26.0,
        }
    }

    #[test]
    fn the_editor_gate_fails_a_leak_and_missing_close_calls() {
        let n = 30;
        let flat: Vec<f64> = vec![100.0; n];
        let leaky: Vec<f64> = (0..n).map(|i| 100.0 + 40.0 * i as f64).collect();
        let threads = vec![12; n];
        let good = json!({ "runs": [run("baseline", &flat, &threads, 0), run("editor", &flat, &threads, n as u64)] });
        let (checks, _) = judge_editor(&good, &[(0, 0), (n as u64, n as u64)], &options(n));
        assert!(
            checks.iter().all(|c| c.ok),
            "{:?}",
            checks.iter().map(|c| &c.detail).collect::<Vec<_>>()
        );

        let leak = json!({ "runs": [run("baseline", &flat, &threads, 0), run("editor", &leaky, &threads, n as u64)] });
        let (checks, _) = judge_editor(&leak, &[(0, 0), (n as u64, n as u64)], &options(n));
        assert!(!checks[1].ok, "a 40 KB per cycle leak fails");

        // Stock nice-plug-egui never calls editor_closed.
        let (checks, _) = judge_editor(&good, &[(0, 0), (n as u64, 0)], &options(n));
        assert!(!checks[0].ok);
        // A bundle without the feature reports nothing.
        let (checks, _) = judge_editor(&good, &[], &options(n));
        assert!(!checks[0].ok);

        let mut changing = threads.clone();
        changing[n - 1] = 13;
        let thread_leak = json!({ "runs": [run("baseline", &flat, &threads, 0), run("editor", &flat, &changing, n as u64)] });
        let (checks, _) = judge_editor(&thread_leak, &[(0, 0), (n as u64, n as u64)], &options(n));
        assert!(!checks[2].ok);
    }
}
