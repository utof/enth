//! `cargo xtask reaper-soak --minutes N`: the long REAPER run behind gate GS1 ("no crashes or audio-thread allocations
//! under clap-validator and a one-hour REAPER soak"). A local gate, not CI.
//!
//! It runs the **debug** bundle (nice-plug's `assert_process_allocs` aborts the process on any allocation in
//! `process()`), in native REAPER under `xvfb-run` with Dummy Audio, like `reaper-test`. `xtask/reaper/soak.lua` plays
//! and edits for the whole time (virtual-keyboard notes on four instances, parameter sweeps, state loads, instance
//! churn, the editor opened and closed); this module supervises the REAPER process from outside:
//!
//! - it samples the process every `--sample-secs` (PSS and swapped PSS from `smaps_rollup`, threads, descriptors, the
//!   Xvfb server's PSS), so a leak the script cannot see still shows;
//! - it watches the script's log for a heartbeat (a REAPER that stops answering is a hang: stacks are taken with gdb,
//!   then the process group is killed) and the process itself (a REAPER that dies before the script's `finished` line
//!   is a crash; an `assert_process_allocs` abort is told apart by the message it prints);
//! - after the run it fits trends (Theil-Sen, robust to outliers) to the settled samples with the editor closed and
//!   fails on growth beyond the stated limits.
//!
//! Output: `soak-report.json` (`enth.reaper-soak/1`: config, limits, checks, samples, editor cycles, events) in the
//! output directory and a short text summary on stdout. Exit status 0 only if every check passed.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Format, Sandbox, chunk, prepare_sandbox, reaper_path, require_tool};
use crate::{bundle, plugin};

/// The script ends REAPER itself; this much extra wall time is allowed for its start and its quit.
const OVERRUN_GRACE_SECS: u64 = 300;
/// REAPER scans the bundles, makes a project and adds the instances before the script's first log line.
const STARTUP_SECS: f64 = 240.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SelfTest {
    /// SIGSEGV to REAPER 30 s in: the run must fail as a crash.
    Crash,
    /// SIGSTOP to REAPER 30 s in: the run must fail as a hang.
    Hang,
}

struct Limits {
    /// Growth of the anonymous memory (`Pss_Anon` plus `SwapPss`), MB per hour, over the settled samples with the editor closed.
    pss_slope_mb_h: f64,
    /// Most threads the process may differ by (settled, editor closed, after warm-up).
    thread_spread: i64,
    /// Most descriptors the end may exceed the start by (medians of the first and last settled samples).
    fd_growth: i64,
    /// Seconds without a new line in the script's log before the run counts as hung.
    hang_secs: f64,
}

struct Options {
    package: String,
    plugin_name: String,
    minutes: f64,
    bundle_dir: Option<PathBuf>,
    build: bool,
    format: Format,
    instances: u32,
    /// Seconds between editor opens; 0 never opens it.
    editor_secs: f64,
    editor_hold_secs: f64,
    notes: bool,
    sweeps: bool,
    load_secs: f64,
    churn_secs: f64,
    sample_secs: f64,
    work_dir: Option<PathBuf>,
    report: Option<PathBuf>,
    /// A stock REAPER effect to run instead of enth (the leak investigation's control).
    control_fx: Option<String>,
    warmup_secs: Option<f64>,
    seed: u64,
    limits: Limits,
    self_test: Option<SelfTest>,
    /// Further `ENTH_SOAK_*` settings for the script, `NAME=value`.
    extra_env: Vec<(String, String)>,
}

fn parse_options(args: &[String], root: &Path) -> Result<Options, String> {
    let (package, rest) = plugin::one_package(plugin::split_packages(args, false)?, root)?;
    let mut o = Options {
        plugin_name: plugin::reaper_plugin_name(&package),
        package,
        minutes: 60.0,
        bundle_dir: std::env::var_os("ENTH_BUNDLE_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from),
        build: true,
        format: Format::Clap,
        instances: 4,
        editor_secs: 60.0,
        editor_hold_secs: 5.0,
        notes: true,
        sweeps: true,
        load_secs: 90.0,
        churn_secs: 300.0,
        sample_secs: 15.0,
        work_dir: None,
        report: None,
        control_fx: None,
        warmup_secs: None,
        seed: 20_261_020,
        limits: Limits {
            pss_slope_mb_h: 5.0,
            thread_spread: 2,
            fd_growth: 4,
            hang_secs: 90.0,
        },
        self_test: None,
        extra_env: Vec::new(),
    };
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| -> Result<String, String> {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        let number = |name: &str, text: String| -> Result<f64, String> {
            text.parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or_else(|| format!("{name} takes a number of at least 0, got {text:?}"))
        };
        match arg.as_str() {
            "--minutes" => o.minutes = number("--minutes", value("--minutes")?)?,
            "--bundle-dir" => o.bundle_dir = Some(PathBuf::from(value("--bundle-dir")?)),
            "--no-build" => o.build = false,
            "--format" => {
                o.format = match value("--format")?.as_str() {
                    "clap" => Format::Clap,
                    "vst3" => Format::Vst3,
                    other => return Err(format!("--format takes clap or vst3, got {other:?}")),
                };
            }
            "--instances" => o.instances = number("--instances", value("--instances")?)? as u32,
            "--editor-secs" => o.editor_secs = number("--editor-secs", value("--editor-secs")?)?,
            "--editor-hold-secs" => {
                o.editor_hold_secs = number("--editor-hold-secs", value("--editor-hold-secs")?)?;
            }
            "--no-notes" => o.notes = false,
            "--no-sweeps" => o.sweeps = false,
            "--load-secs" => o.load_secs = number("--load-secs", value("--load-secs")?)?,
            "--churn-secs" => o.churn_secs = number("--churn-secs", value("--churn-secs")?)?,
            "--sample-secs" => o.sample_secs = number("--sample-secs", value("--sample-secs")?)?,
            "--work-dir" => o.work_dir = Some(PathBuf::from(value("--work-dir")?)),
            "--report" => o.report = Some(PathBuf::from(value("--report")?)),
            "--control-fx" => o.control_fx = Some(value("--control-fx")?),
            "--warmup-secs" => {
                o.warmup_secs = Some(number("--warmup-secs", value("--warmup-secs")?)?);
            }
            "--seed" => o.seed = number("--seed", value("--seed")?)? as u64,
            "--max-pss-mb-per-hour" => {
                o.limits.pss_slope_mb_h =
                    number("--max-pss-mb-per-hour", value("--max-pss-mb-per-hour")?)?;
            }
            "--max-thread-spread" => {
                o.limits.thread_spread =
                    number("--max-thread-spread", value("--max-thread-spread")?)? as i64;
            }
            "--max-fd-growth" => {
                o.limits.fd_growth = number("--max-fd-growth", value("--max-fd-growth")?)? as i64;
            }
            "--hang-secs" => o.limits.hang_secs = number("--hang-secs", value("--hang-secs")?)?,
            "--plugin-name" => o.plugin_name = value("--plugin-name")?,
            "--script-env" => {
                let pair = value("--script-env")?;
                let (k, v) = pair.split_once('=').ok_or(
                    "--script-env takes NAME=value (an ENTH_SOAK_* setting for the script)",
                )?;
                o.extra_env.push((k.to_owned(), v.to_owned()));
            }
            "--self-test" => {
                o.self_test = Some(match value("--self-test")?.as_str() {
                    "crash" => SelfTest::Crash,
                    "hang" => SelfTest::Hang,
                    other => return Err(format!("--self-test takes crash or hang, got {other:?}")),
                });
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if o.minutes <= 0.0 {
        return Err("--minutes must be above 0".to_owned());
    }
    if o.instances == 0 || o.instances > 16 {
        return Err("--instances takes 1 to 16 (one MIDI channel each)".to_owned());
    }
    Ok(o)
}

// ---- reading the process ---------------------------------------------------------------------------------------------

/// Memory of one process, in kB, from `smaps_rollup`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Mem {
    rss: u64,
    pss: u64,
    pss_anon: u64,
    pss_file: u64,
    swap_pss: u64,
}

impl Mem {
    /// The anonymous memory the process holds, resident or swapped out: the heap, the thread stacks and every other
    /// private mapping, where a leak shows. File-backed pages come and go with the page cache, and this machine swaps,
    /// so neither PSS alone nor RSS is steady; the anonymous total is.
    fn footprint(self) -> u64 {
        self.pss_anon + self.swap_pss
    }
}

fn parse_rollup(text: &str) -> Mem {
    let field = |name: &str| -> u64 {
        text.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
            .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
            .unwrap_or(0)
    };
    Mem {
        rss: field("Rss"),
        pss: field("Pss"),
        pss_anon: field("Pss_Anon"),
        pss_file: field("Pss_File"),
        swap_pss: field("SwapPss"),
    }
}

fn read_mem(pid: u32) -> Option<Mem> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    Some(parse_rollup(&text))
}

fn count_entries(path: &str) -> Option<usize> {
    Some(std::fs::read_dir(path).ok()?.count())
}

/// The thread names of a process and how many threads carry each.
fn thread_names(pid: u32) -> BTreeMap<String, usize> {
    let mut names = BTreeMap::new();
    if let Ok(dir) = std::fs::read_dir(format!("/proc/{pid}/task")) {
        for entry in dir.flatten() {
            let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
            *names.entry(comm.trim().to_owned()).or_insert(0) += 1;
        }
    }
    names
}

/// The process whose executable is `exe` (the sandbox's copy of REAPER).
fn find_pid_by_exe(exe: &Path) -> Option<u32> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if std::fs::read_link(entry.path().join("exe")).is_ok_and(|p| p == exe) {
            return Some(pid);
        }
    }
    None
}

/// The Xvfb server REAPER draws on: the process whose command line is `Xvfb :N` for the `DISPLAY` of REAPER.
fn find_xvfb(reaper_pid: u32) -> Option<u32> {
    let environ = std::fs::read(format!("/proc/{reaper_pid}/environ")).ok()?;
    let display = environ
        .split(|b| *b == 0)
        .find_map(|v| v.strip_prefix(b"DISPLAY="))
        .map(|v| String::from_utf8_lossy(v).into_owned())?;
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let mut words = cmdline.split(|b| *b == 0);
        let program = words.next().unwrap_or_default();
        let first_arg = words.next().unwrap_or_default();
        if program.ends_with(b"Xvfb") && first_arg == display.as_bytes() {
            return Some(pid);
        }
    }
    None
}

fn signal(pid_or_group: i64, name: &str) {
    let _ = Command::new("kill")
        .arg(format!("-{name}"))
        .arg("--")
        .arg(pid_or_group.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

// ---- the script's log ---------------------------------------------------------------------------------------------------

/// `a=1 b=x` into a map.
fn parse_kv(text: &str) -> BTreeMap<String, String> {
    text.split_whitespace()
        .filter_map(|w| w.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn num(map: &BTreeMap<String, String>, key: &str) -> Option<f64> {
    map.get(key)?.parse().ok()
}

/// What the script wrote: `check` rows, and `data` lines in order.
#[derive(Default)]
struct ScriptLog {
    checks: Vec<(String, bool, String)>,
    /// (key, value) in file order.
    data: Vec<(String, String)>,
}

impl ScriptLog {
    fn parse(text: &str) -> ScriptLog {
        let mut log = ScriptLog::default();
        for line in text.lines() {
            let fields: Vec<&str> = line.splitn(4, '\t').collect();
            match fields.as_slice() {
                ["check", name, status, detail] => {
                    log.checks
                        .push(((*name).to_owned(), *status == "ok", (*detail).to_owned()));
                }
                ["check", name, status] => {
                    log.checks
                        .push(((*name).to_owned(), *status == "ok", String::new()));
                }
                ["data", key, value] => log.data.push(((*key).to_owned(), (*value).to_owned())),
                _ => {}
            }
        }
        log
    }

    fn all(&self, key: &str) -> Vec<BTreeMap<String, String>> {
        self.data
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| parse_kv(v))
            .collect()
    }

    fn last(&self, key: &str) -> Option<BTreeMap<String, String>> {
        self.data
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| parse_kv(v))
    }

    fn has(&self, key: &str) -> bool {
        self.data.iter().any(|(k, _)| k == key)
    }
}

// ---- samples and statistics -----------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Sample {
    /// Seconds since the script's first log line (negative before it).
    t: f64,
    mem: Mem,
    threads: usize,
    fds: usize,
    xvfb_pss: u64,
    /// The editor was closed for at least a few seconds (the memory is settled), per the script's last heartbeat.
    settled_closed: bool,
    editor_open: bool,
}

/// Theil-Sen slope of `points` (x, y): the median of the slopes between all pairs. Robust to the odd outlier; a leak
/// that grows steadily shows as its rate, and noise averages out. `None` below two distinct x values.
fn theil_sen(points: &[(f64, f64)]) -> Option<f64> {
    // Keep the pairwise work bounded: thin to at most 400 points, evenly.
    let thinned: Vec<(f64, f64)> = if points.len() > 400 {
        (0..400)
            .map(|i| points[i * (points.len() - 1) / 399])
            .collect()
    } else {
        points.to_vec()
    };
    let mut slopes = Vec::new();
    for (i, a) in thinned.iter().enumerate() {
        for b in &thinned[i + 1..] {
            if (b.0 - a.0).abs() > f64::EPSILON {
                slopes.push((b.1 - a.1) / (b.0 - a.0));
            }
        }
    }
    if slopes.is_empty() {
        return None;
    }
    slopes.sort_by(f64::total_cmp);
    let mid = slopes.len() / 2;
    Some(if slopes.len() % 2 == 1 {
        slopes[mid]
    } else {
        f64::midpoint(slopes[mid - 1], slopes[mid])
    })
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    Some(if v.len() % 2 == 1 {
        v[mid]
    } else {
        f64::midpoint(v[mid - 1], v[mid])
    })
}

/// Slope of the (t seconds, kB) points in MB per hour.
fn mb_per_hour(points: &[(f64, f64)]) -> Option<f64> {
    theil_sen(points).map(|kb_per_s| kb_per_s * 3600.0 / 1024.0)
}

// ---- the checks ------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Fail,
    Info,
}

struct Row {
    name: String,
    status: Status,
    detail: String,
}

#[derive(Default)]
struct Checks {
    rows: Vec<Row>,
}

impl Checks {
    fn add(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        let status = if ok { Status::Ok } else { Status::Fail };
        self.rows.push(Row {
            name: name.to_owned(),
            status,
            detail: detail.into(),
        });
    }

    fn info(&mut self, name: &str, detail: impl Into<String>) {
        self.rows.push(Row {
            name: name.to_owned(),
            status: Status::Info,
            detail: detail.into(),
        });
    }

    fn failures(&self) -> Vec<&Row> {
        self.rows
            .iter()
            .filter(|r| r.status == Status::Fail)
            .collect()
    }
}

/// How the REAPER process ended.
#[derive(Debug, PartialEq, Eq)]
enum Ending {
    /// The script reached `finished` and REAPER quit.
    Finished,
    /// The process died first (the exit description says how).
    Died(String),
    /// No log line for `hang_secs`; the process was killed.
    Hung,
    /// Past the deadline, still running; killed.
    Overran,
}

/// What a shell exit status of `xvfb-run timeout reaper` means.
fn describe_exit(code: Option<i32>, signal: Option<i32>) -> String {
    match (code, signal) {
        (Some(0), _) => "exit 0".to_owned(),
        (Some(124), _) => "killed by timeout (124)".to_owned(),
        (Some(c), _) if c > 128 => {
            format!("exit {c} (signal {} {})", c - 128, signal_name(c - 128))
        }
        (Some(c), _) => format!("exit {c}"),
        (None, Some(s)) => format!("signal {s} {}", signal_name(s)),
        (None, None) => "unknown".to_owned(),
    }
}

fn signal_name(signal: i32) -> &'static str {
    match signal {
        6 => "SIGABRT",
        7 => "SIGBUS",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        15 => "SIGTERM",
        _ => "",
    }
}

/// The allocation abort of `assert_process_allocs`: std's alloc-error handler prints this, then aborts.
fn alloc_abort_in(output: &str) -> Option<String> {
    output
        .lines()
        .find(|l| l.contains("memory allocation of") && l.contains("failed"))
        .map(|l| l.trim().to_owned())
}

/// Whether the plugin binary carries nice-plug's allocation guard (a debug build with `assert_process_allocs`): the
/// `nice_assert_no_alloc` crate is not linked into a release build.
fn alloc_guard_armed(artefact: &Path) -> bool {
    let needle = b"nice_assert_no_alloc";
    let path = if artefact.is_dir() {
        // A VST3 bundle: Contents/x86_64-linux/<name>.so
        let name = artefact
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        artefact
            .join("Contents/x86_64-linux")
            .join(format!("{name}.so"))
    } else {
        artefact.to_path_buf()
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return false;
    };
    let first = needle[0];
    let mut from = 0;
    while let Some(i) = bytes[from..].iter().position(|b| *b == first) {
        let at = from + i;
        if bytes.get(at..at + needle.len()) == Some(needle.as_slice()) {
            return true;
        }
        from = at + 1;
    }
    false
}

// ---- the run ------------------------------------------------------------------------------------------------------------------

struct Outcome {
    ending: Ending,
    samples: Vec<Sample>,
    threads_first: BTreeMap<String, usize>,
    threads_last: BTreeMap<String, usize>,
    wall_secs: f64,
    exit: String,
    hang_stacks: Option<PathBuf>,
}

pub fn reaper_soak(args: &[String]) -> Result<(), String> {
    let meta = crate::metadata()?;
    let o = parse_options(args, &meta.workspace_root)?;
    plugin::require_member(&meta, &o.package)?;
    if o.package != plugin::PRODUCT_PACKAGE {
        return Err(format!(
            "reaper-soak runs {} only, not {}",
            plugin::PRODUCT_PACKAGE,
            o.package
        ));
    }
    let reaper = reaper_path()?;
    require_tool(
        "xvfb-run",
        "REAPER must never open a window on the real display",
    )?;
    require_tool("timeout", "needed to stop a hung REAPER")?;
    require_tool(
        "python3",
        "the editor cycles send X events with xtask/reaper/xinput.py",
    )?;

    if o.build && o.bundle_dir.is_none() {
        bundle::bundle(&[o.package.clone(), "--locked".to_owned()])?;
    }
    let bundled = match &o.bundle_dir {
        Some(dir) => dir
            .canonicalize()
            .map_err(|e| format!("--bundle-dir {}: {e}", dir.display()))?,
        None => bundle::bundled_dir()?,
    };
    let artefact = bundled.join(format!("{}.{}", o.package, o.format.tag()));
    if !artefact.exists() {
        return Err(format!(
            "{} is missing; run without --no-build or point --bundle-dir at it",
            artefact.display()
        ));
    }
    let guard_armed = alloc_guard_armed(&artefact);

    let work = o
        .work_dir
        .clone()
        .unwrap_or_else(|| meta.target_directory.join("reaper-soak"));
    let sandbox = Sandbox {
        plugin_name: o.plugin_name.clone(),
        reaper,
        home: work.join("home"),
        out: work.join("out"),
        lua: meta.workspace_root.join("xtask/reaper"),
    };
    prepare_sandbox(&sandbox, &bundled)?;
    let chunks = write_chunks(&sandbox.out, o.format)?;
    println!(
        "reaper-soak: {} min, {} x {} {}, bundle {} (allocation guard {})",
        o.minutes,
        o.instances,
        o.control_fx.as_deref().unwrap_or("enth"),
        o.format.tag(),
        bundled.display(),
        if guard_armed {
            "armed"
        } else {
            "NOT armed: not a debug bundle?"
        }
    );
    println!("reaper-soak: results in {}", sandbox.out.display());

    let outcome = supervise(&sandbox, &o, &chunks)?;
    let stdout_text =
        std::fs::read_to_string(sandbox.out.join("soak.stdout.txt")).unwrap_or_default();
    let log = ScriptLog::parse(
        &std::fs::read_to_string(sandbox.out.join("soak.log")).unwrap_or_default(),
    );
    let (checks, fits) = evaluate(&o, &outcome, &log, &stdout_text, guard_armed);

    let report = build_report(
        &meta.workspace_root,
        &o,
        &outcome,
        &log,
        &checks,
        &fits,
        &bundled,
        guard_armed,
    );
    let report_path = sandbox.out.join("soak-report.json");
    let text = serde_json::to_string_pretty(&report).map_err(|e| format!("report: {e}"))?;
    std::fs::write(&report_path, &text).map_err(|e| format!("{}: {e}", report_path.display()))?;
    if let Some(copy) = &o.report {
        if let Some(parent) = copy.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(copy, &text).map_err(|e| format!("{}: {e}", copy.display()))?;
    }
    print_summary(&o, &outcome, &checks, &fits, &report_path);
    let failures = checks.failures();
    if failures.is_empty() {
        println!("reaper-soak: ok");
        Ok(())
    } else {
        Err(format!(
            "reaper-soak: {} check(s) failed: {} (report {})",
            failures.len(),
            failures
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            report_path.display()
        ))
    }
}

/// The patches a state load rotates through, as state chunks: the REAPER scenarios' fixtures and the starter patch.
fn write_chunks(out: &Path, format: Format) -> Result<Vec<String>, String> {
    let patches: [(&str, &str); 7] = [
        ("roundtrip", super::PRODUCT_ROUNDTRIP_PATCH),
        ("starter", crate::preset::STARTER_PATCH),
        ("state", super::PRODUCT_STATE_PATCH),
        ("slot", super::PRODUCT_SLOT_PATCH),
        ("macro", super::PRODUCT_MACRO_PATCH),
        ("wheel", super::PRODUCT_WHEEL_PATCH),
        ("dirty-control", super::PRODUCT_DIRTY_CONTROL_PATCH),
    ];
    patches
        .iter()
        .map(|(name, patch)| chunk::write_state_chunk(out, name, format, patch))
        .collect()
}

fn script_env(sb: &Sandbox, o: &Options, chunks: &[String]) -> Vec<(String, String)> {
    let b = |on: bool| if on { "1" } else { "0" }.to_owned();
    let mut env: Vec<(String, String)> = vec![
        ("ENTH_OUT".into(), sb.out.display().to_string()),
        ("ENTH_LUA_DIR".into(), sb.lua.display().to_string()),
        (
            "ENTH_EMPTY_PROJECT".into(),
            sb.lua.join("empty.rpp").display().to_string(),
        ),
        ("ENTH_LOG".into(), "soak.log".into()),
        ("ENTH_FX".into(), o.format.reaper_name(&sb.plugin_name)),
        ("ENTH_TAG".into(), o.format.tag().into()),
        ("ENTH_PHASE".into(), "soak".into()),
        (
            "ENTH_SOAK_SECS".into(),
            format!("{}", (o.minutes * 60.0).round()),
        ),
        ("ENTH_SOAK_INSTANCES".into(), o.instances.to_string()),
        ("ENTH_SOAK_EDITOR_SECS".into(), o.editor_secs.to_string()),
        (
            "ENTH_SOAK_EDITOR_HOLD".into(),
            o.editor_hold_secs.to_string(),
        ),
        ("ENTH_SOAK_NOTES".into(), b(o.notes)),
        ("ENTH_SOAK_SWEEPS".into(), b(o.sweeps)),
        ("ENTH_SOAK_LOAD_SECS".into(), o.load_secs.to_string()),
        ("ENTH_SOAK_CHURN_SECS".into(), o.churn_secs.to_string()),
        ("ENTH_SOAK_SEED".into(), o.seed.to_string()),
        ("ENTH_SOAK_CHUNKS".into(), chunks.join(",")),
    ];
    if let Some(fx) = &o.control_fx {
        env.push(("ENTH_SOAK_CONTROL_FX".into(), fx.clone()));
    }
    env.extend(o.extra_env.iter().cloned());
    env
}

/// Starts REAPER under `xvfb-run` and watches it until it ends, hangs or overruns, sampling it on the way.
fn supervise(sb: &Sandbox, o: &Options, chunks: &[String]) -> Result<Outcome, String> {
    let secs = (o.minutes * 60.0).ceil() as u64;
    let deadline_secs = secs + OVERRUN_GRACE_SECS;
    let stdout = std::fs::File::create(sb.out.join("soak.stdout.txt"))
        .map_err(|e| format!("soak.stdout.txt: {e}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|e| format!("soak.stdout.txt: {e}"))?;
    let reaper_copy = sb.home.join(sb.reaper.file_name().unwrap_or_default());
    let mut command = Command::new("xvfb-run");
    command
        .args(["-a", "-s", "-screen 0 1920x1080x24"])
        .args(["timeout", "-k", "10", &deadline_secs.to_string()])
        .arg(&reaper_copy)
        .args(["-nosplash", "-newinst"])
        .arg(sb.lua.join("soak.lua"))
        .current_dir(&sb.home)
        // xvfb-run sets DISPLAY for its child; make sure nothing can fall back to the real one.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("GDK_BACKEND", "x11")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    {
        use std::os::unix::process::CommandExt as _;
        // Its own process group, so a hung run can be killed whole (REAPER, timeout, Xvfb).
        command.process_group(0);
    }
    for (key, value) in script_env(sb, o, chunks) {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot run xvfb-run: {e}"))?;
    let group = i64::from(child.id());
    let exe = reaper_copy.canonicalize().unwrap_or(reaper_copy);
    let log_path = sb.out.join("soak.log");

    let started = Instant::now();
    let mut t0: Option<Instant> = None; // the script's `begin` line
    let mut samples: Vec<Sample> = Vec::new();
    let mut pid: Option<u32> = None;
    let mut xvfb: Option<u32> = None;
    let mut last_len = 0u64;
    let mut last_progress = Instant::now();
    let mut next_sample = Instant::now();
    let mut injected = false;
    let mut threads_first = BTreeMap::new();
    let mut threads_last = BTreeMap::new();
    let mut hang_stacks = None;
    let mut last_mark = (0.0, false, false); // (t, editor open, busy) at the script's last change
    let mut beat_len_read = 0u64;
    let ending;
    let exit;

    loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("wait: {e}"))? {
            use std::os::unix::process::ExitStatusExt as _;
            exit = describe_exit(status.code(), status.signal());
            let finished =
                std::fs::read_to_string(&log_path).is_ok_and(|t| t.contains("\tfinished\t"));
            ending = if finished {
                Ending::Finished
            } else {
                Ending::Died(exit.clone())
            };
            break;
        }
        let now = Instant::now();
        // Progress: the script's log grew.
        let len = std::fs::metadata(&log_path).map_or(0, |m| m.len());
        if len != last_len {
            last_len = len;
            last_progress = now;
            if t0.is_none()
                && std::fs::read_to_string(&log_path).is_ok_and(|t| t.contains("\tbegin\t"))
            {
                t0 = Some(now); // the script's clock starts with its `begin` line
            }
        }
        let silent = now.duration_since(last_progress).as_secs_f64();
        let limit = if t0.is_some() {
            o.limits.hang_secs
        } else {
            STARTUP_SECS
        };
        if silent > limit {
            if let Some(p) = pid {
                hang_stacks = capture_stacks(p, &sb.out);
            }
            signal(-group, "KILL");
            let _ = child.wait();
            exit = format!("killed by the supervisor: no log line for {silent:.0} s");
            ending = Ending::Hung;
            break;
        }
        if now.duration_since(started).as_secs() > deadline_secs + 30 {
            signal(-group, "KILL");
            let _ = child.wait();
            exit = "killed by the supervisor: past the deadline".to_owned();
            ending = Ending::Overran;
            break;
        }
        if pid.is_none() {
            pid = find_pid_by_exe(&exe);
        }
        if let (Some(p), Some(t0)) = (pid, t0) {
            let t = now.duration_since(t0).as_secs_f64();
            if let Some(test) = o.self_test
                && !injected
                && t > 30.0
            {
                injected = true;
                signal(
                    i64::from(p),
                    if test == SelfTest::Crash {
                        "SEGV"
                    } else {
                        "STOP"
                    },
                );
            }
            if now >= next_sample {
                next_sample = now + Duration::from_secs_f64(o.sample_secs.max(1.0));
                // The editor's state from the script's last `mark` line (written at every change).
                if len != beat_len_read {
                    beat_len_read = len;
                    if let Ok(text) = std::fs::read_to_string(&log_path)
                        && let Some(mark) = ScriptLog::parse(&text).last("mark")
                    {
                        last_mark = (
                            num(&mark, "t").unwrap_or(0.0),
                            num(&mark, "editor") == Some(1.0),
                            num(&mark, "busy") == Some(1.0),
                        );
                    }
                }
                if xvfb.is_none() {
                    xvfb = find_xvfb(p);
                }
                if let Some(mem) = read_mem(p) {
                    // A cycle's own mark (busy 0) comes after its 3 s settle, so 2 s more is 5 s after the close.
                    let (mark_t, editor_open, busy) = last_mark;
                    let settled_closed = !editor_open && !busy && t - mark_t >= 2.0 && t >= 5.0;
                    let sample = Sample {
                        t,
                        mem,
                        threads: count_entries(&format!("/proc/{p}/task")).unwrap_or(0),
                        fds: count_entries(&format!("/proc/{p}/fd")).unwrap_or(0),
                        xvfb_pss: xvfb.and_then(read_mem).map_or(0, |m| m.pss),
                        settled_closed,
                        editor_open,
                    };
                    if samples.is_empty() {
                        threads_first = thread_names(p);
                    }
                    threads_last = thread_names(p);
                    samples.push(sample);
                } else {
                    pid = None; // gone (or about to be); the exit status says which
                }
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // Whatever is left of the group (an Xvfb that outlived a kill) goes too.
    signal(-group, "KILL");
    Ok(Outcome {
        ending,
        samples,
        threads_first,
        threads_last,
        wall_secs: started.elapsed().as_secs_f64(),
        exit,
        hang_stacks,
    })
}

/// A hung REAPER's thread stacks, best effort: gdb if it is there, else what `/proc` says about each thread.
fn capture_stacks(pid: u32, out: &Path) -> Option<PathBuf> {
    let path = out.join("hang-stacks.txt");
    let mut text = String::new();
    let gdb = Command::new("timeout")
        .args([
            "60",
            "gdb",
            "-batch",
            "-p",
            &pid.to_string(),
            "-ex",
            "thread apply all bt 25",
        ])
        .stdin(Stdio::null())
        .output();
    if let Ok(g) = gdb {
        text.push_str(&String::from_utf8_lossy(&g.stdout));
        text.push_str(&String::from_utf8_lossy(&g.stderr));
    }
    text.push_str("\n--- /proc threads ---\n");
    if let Ok(dir) = std::fs::read_dir(format!("/proc/{pid}/task")) {
        for entry in dir.flatten() {
            let read = |f: &str| {
                std::fs::read_to_string(entry.path().join(f))
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            };
            // utime and stime (clock ticks) are fields 14 and 15 after the "(comm)" of /proc/.../stat: a thread
            // that spins shows a large and growing figure.
            let stat = read("stat");
            let ticks: Vec<&str> = stat
                .rsplit_once(')')
                .map_or("", |(_, rest)| rest)
                .split_whitespace()
                .collect();
            let _ = writeln!(
                text,
                "{} {} state={} utime={} stime={} wchan={} syscall={}",
                entry.file_name().to_string_lossy(),
                read("comm"),
                ticks.first().copied().unwrap_or("?"),
                ticks.get(11).copied().unwrap_or("?"),
                ticks.get(12).copied().unwrap_or("?"),
                read("wchan"),
                read("syscall")
            );
        }
    }
    std::fs::write(&path, text).ok()?;
    Some(path)
}

// ---- judging the run ---------------------------------------------------------------------------------------------------------------

/// Trends over the run, for the report.
#[derive(Default)]
struct Fits {
    warmup_secs: f64,
    settled_samples: usize,
    footprint_mb_h: Option<f64>,
    pss_total_mb_h: Option<f64>,
    pss_file_mb_h: Option<f64>,
    xvfb_mb_h: Option<f64>,
    footprint_start_mb: Option<f64>,
    footprint_end_mb: Option<f64>,
    threads_min: Option<usize>,
    threads_max: Option<usize>,
    fds_start: Option<f64>,
    fds_end: Option<f64>,
    cycles: usize,
    cycle_growth_kb: Option<f64>,
    cycle_footprint_mb_h: Option<f64>,
    /// Median of (memory after a close) - (memory before the open), kB.
    cycle_net_kb: Option<f64>,
}

/// One editor cycle's memory, from the script.
struct Cycle {
    n: u64,
    drew: bool,
    gone: bool,
    t: f64,
    before: u64,
    after: u64,
}

fn cycles_of(log: &ScriptLog) -> Vec<Cycle> {
    log.all("cycle")
        .iter()
        .map(|c| {
            let get = |k: &str| num(c, k).unwrap_or(0.0);
            Cycle {
                n: get("n") as u64,
                drew: get("drew") == 1.0,
                gone: get("gone") == 1.0,
                t: get("t"),
                before: (get("before_anon") + get("before_swap")) as u64,
                after: (get("after_anon") + get("after_swap")) as u64,
            }
        })
        .collect()
}

fn evaluate(
    o: &Options,
    outcome: &Outcome,
    log: &ScriptLog,
    stdout_text: &str,
    guard_armed: bool,
) -> (Checks, Fits) {
    let mut checks = Checks::default();
    let mut fits = Fits::default();
    let wanted_secs = o.minutes * 60.0;
    let beat_last = log.last("beat");
    let summary = log.last("summary");
    let elapsed = summary
        .as_ref()
        .and_then(|s| num(s, "elapsed"))
        .or_else(|| beat_last.as_ref().and_then(|b| num(b, "t")));

    // The process.
    let alloc = alloc_abort_in(stdout_text);
    checks.add(
        "no-allocation-abort",
        alloc.is_none(),
        alloc.as_deref().map_or_else(
            || "no 'memory allocation of N bytes failed' in REAPER's output".to_owned(),
            |l| format!("AUDIO-THREAD ALLOCATION: {l}"),
        ),
    );
    if o.self_test.is_none() || o.self_test == Some(SelfTest::Crash) {
        let (ok, detail) = match &outcome.ending {
            Ending::Finished => (
                true,
                format!(
                    "REAPER ran the script to its end and quit ({})",
                    outcome.exit
                ),
            ),
            Ending::Died(how) => (
                false,
                format!("REAPER died before the script finished: {how}"),
            ),
            _ => (true, "REAPER did not die".to_owned()),
        };
        checks.add("no-crash", ok, detail);
    } else {
        checks.add(
            "no-crash",
            !matches!(outcome.ending, Ending::Died(_)),
            format!("{:?}", outcome.ending),
        );
    }
    let hang_detail = match (&outcome.ending, &outcome.hang_stacks) {
        (Ending::Hung, Some(path)) => {
            format!("{}; thread stacks in {}", outcome.exit, path.display())
        }
        (Ending::Hung | Ending::Overran, None) => outcome.exit.clone(),
        _ => format!("a log line at least every {:.0} s", o.limits.hang_secs),
    };
    checks.add(
        "no-hang",
        !matches!(outcome.ending, Ending::Hung | Ending::Overran),
        hang_detail,
    );
    checks.add(
        "ran-to-the-end",
        outcome.ending == Ending::Finished
            && log.has("finished")
            && elapsed.is_some_and(|e| e >= wanted_secs * 0.98),
        format!(
            "{} s of {wanted_secs:.0} s requested",
            elapsed.map_or_else(|| "?".to_owned(), |e| format!("{e:.0}"))
        ),
    );
    checks.add(
        "allocation-guard-armed",
        guard_armed,
        if guard_armed {
            "the bundle carries nice-plug's allocation guard (debug build)"
        } else {
            "the plugin binary has no nice_assert_no_alloc: not a debug bundle with assert_process_allocs, so an audio-thread allocation would go unnoticed"
        },
    );

    // The script's own checks (instances loaded, script errors).
    for (name, ok, detail) in &log.checks {
        checks.add(name, *ok, detail.clone());
    }

    // The work done. A run that did not finish has its cause above; what the script had not yet done is no news.
    let completed = outcome.ending == Ending::Finished;
    if !completed {
        checks.info(
            "script-results-skipped",
            "the run did not finish, so the checks on the script's work (calls, editor cycles, audio) are not judged",
        );
    } else if let Some(s) = &summary {
        let g = |k: &str| num(s, k).unwrap_or(0.0);
        let fails = g("param_fail") + g("load_fail") + g("churn_fail");
        checks.add(
            "host-calls-ok",
            fails == 0.0,
            format!(
                "{} notes, {} controller moves, {} parameter sets ({} refused), {} state loads ({} failed), {} instance churns ({} failed)",
                g("note_on"), g("cc"), g("sets"), g("param_fail"), g("loads"), g("load_fail"), g("churns"), g("churn_fail")
            ),
        );
        if o.editor_secs > 0.0 {
            let cycles = g("cycles");
            checks.add(
                "editor-cycles-drew-and-closed",
                cycles > 0.0 && g("cycles_drew") == cycles && g("cycles_closed") == cycles,
                format!(
                    "{} cycles: {} drew an editor, {} removed the window; {} of {} XTest edits changed the patch",
                    cycles, g("cycles_drew"), g("cycles_closed"), g("edits_applied"), g("edits")
                ),
            );
        }
        checks.info(
            "underruns",
            format!(
                "GetUnderrunTime {} / {} ({} changes seen); Dummy Audio has no deadline",
                g("underrun_audio"),
                g("underrun_media"),
                g("xrun_events")
            ),
        );
    } else {
        checks.add("script-summary", false, "the script wrote no summary line");
    }
    if completed && o.notes && o.control_fx.is_none() {
        let peaks = log.all("peaks");
        let silent_minutes = summary
            .as_ref()
            .and_then(|s| num(s, "silent_minutes"))
            .unwrap_or(0.0);
        let mut per_instance_max = vec![0.0f64; o.instances as usize];
        for row in &peaks {
            for (i, slot) in per_instance_max.iter_mut().enumerate() {
                *slot = slot.max(num(row, &format!("p{}", i + 1)).unwrap_or(0.0));
            }
        }
        let dead: Vec<usize> = per_instance_max
            .iter()
            .enumerate()
            .filter(|(_, p)| **p <= 1e-6)
            .map(|(i, _)| i + 1)
            .collect();
        checks.add(
            "audio-flows",
            !peaks.is_empty() && dead.is_empty(),
            format!(
                "peak per instance over the run: {}; instances that never made a sound: {dead:?}; minutes with no sound at all: {silent_minutes}",
                per_instance_max.iter().map(|p| format!("{p:.2}")).collect::<Vec<_>>().join(" ")
            ),
        );
    }

    // Memory, threads, descriptors.
    let warmup = o
        .warmup_secs
        .unwrap_or_else(|| (wanted_secs * 0.25).clamp(60.0, 600.0));
    fits.warmup_secs = warmup;
    let settled: Vec<&Sample> = outcome
        .samples
        .iter()
        .filter(|s| s.t >= warmup && s.settled_closed)
        .collect();
    let settled_all_editor_closed: Vec<&Sample> = if o.editor_secs > 0.0 {
        settled.clone()
    } else {
        outcome.samples.iter().filter(|s| s.t >= warmup).collect()
    };
    fits.settled_samples = settled_all_editor_closed.len();
    let series = |f: &dyn Fn(&Sample) -> f64| -> Vec<(f64, f64)> {
        settled_all_editor_closed
            .iter()
            .map(|s| (s.t, f(s)))
            .collect()
    };
    let kb = |v: u64| v as f64;
    fits.footprint_mb_h = mb_per_hour(&series(&|s| kb(s.mem.footprint())));
    fits.pss_total_mb_h = mb_per_hour(&series(&|s| kb(s.mem.pss + s.mem.swap_pss)));
    fits.pss_file_mb_h = mb_per_hour(&series(&|s| kb(s.mem.pss_file)));
    fits.xvfb_mb_h = mb_per_hour(&series(&|s| kb(s.xvfb_pss)));
    if let (Some(first), Some(last)) = (outcome.samples.first(), outcome.samples.last()) {
        fits.footprint_start_mb = Some(first.mem.footprint() as f64 / 1024.0);
        fits.footprint_end_mb = Some(last.mem.footprint() as f64 / 1024.0);
    }
    let threads: Vec<usize> = settled_all_editor_closed
        .iter()
        .map(|s| s.threads)
        .collect();
    fits.threads_min = threads.iter().copied().min();
    fits.threads_max = threads.iter().copied().max();
    let fd_values: Vec<f64> = settled_all_editor_closed
        .iter()
        .map(|s| s.fds as f64)
        .collect();
    let edge = (fd_values.len() / 5).clamp(1, 10);
    if fd_values.len() >= 2 {
        fits.fds_start = median(&fd_values[..edge]);
        fits.fds_end = median(&fd_values[fd_values.len() - edge..]);
    }
    let enough = settled_all_editor_closed.len() >= 8;
    let judged_long = wanted_secs >= 1800.0;
    if enough {
        if let Some(slope) = fits.footprint_mb_h {
            checks.add(
                "memory-growth",
                slope <= o.limits.pss_slope_mb_h,
                format!(
                    "anonymous memory (Pss_Anon + SwapPss) {slope:+.1} MB/h (Theil-Sen over {} settled samples with the editor closed after {warmup:.0} s; limit {:.1} MB/h); PSS in all {:+.1}, file-backed {:+.1}, Xvfb {:+.1} MB/h; {:.0} -> {:.0} MB anonymous over the run",
                    settled_all_editor_closed.len(),
                    o.limits.pss_slope_mb_h,
                    fits.pss_total_mb_h.unwrap_or(0.0),
                    fits.pss_file_mb_h.unwrap_or(0.0),
                    fits.xvfb_mb_h.unwrap_or(0.0),
                    fits.footprint_start_mb.unwrap_or(0.0),
                    fits.footprint_end_mb.unwrap_or(0.0),
                ),
            );
        }
        let (tmin, tmax) = (fits.threads_min.unwrap_or(0), fits.threads_max.unwrap_or(0));
        checks.add(
            "threads-steady",
            (tmax as i64 - tmin as i64) <= o.limits.thread_spread,
            format!("{tmin} to {tmax} threads when settled with the editor closed (limit: a spread of {})", o.limits.thread_spread),
        );
        let (f0, f1) = (fits.fds_start.unwrap_or(0.0), fits.fds_end.unwrap_or(0.0));
        checks.add(
            "fds-steady",
            f1 - f0 <= o.limits.fd_growth as f64,
            format!("{f0:.0} descriptors at the start, {f1:.0} at the end (medians of the first and last settled samples; limit +{})", o.limits.fd_growth),
        );
    } else {
        let detail = format!(
            "{} settled samples with the editor closed after the {warmup:.0} s warm-up: too few to fit a trend",
            settled_all_editor_closed.len()
        );
        if judged_long {
            checks.add("memory-trend", false, detail);
        } else {
            checks.info("memory-trend", detail);
        }
    }

    // Per editor cycle: the memory the script read just before each open and after each close.
    let cycles = cycles_of(log);
    fits.cycles = cycles.len();
    let late: Vec<&Cycle> = cycles
        .iter()
        .filter(|c| c.t >= warmup && c.drew && c.gone)
        .collect();
    if late.len() >= 4 {
        let after: Vec<(f64, f64)> = late.iter().map(|c| (c.n as f64, c.after as f64)).collect();
        fits.cycle_growth_kb = theil_sen(&after);
        let by_time: Vec<(f64, f64)> = late.iter().map(|c| (c.t, c.after as f64)).collect();
        fits.cycle_footprint_mb_h = mb_per_hour(&by_time);
        let net: Vec<f64> = late
            .iter()
            .map(|c| c.after as f64 - c.before as f64)
            .collect();
        fits.cycle_net_kb = median(&net);
    }
    if let Some(per_cycle) = fits.cycle_growth_kb {
        checks.info(
            "editor-cycle-growth",
            format!(
                "{per_cycle:+.0} kB per open/close cycle (Theil-Sen over {} cycles after the warm-up, memory read 3 s after each close); the median cycle ends {:+.0} kB from where it began; the test host's leak gate allows 26 kB",
                late.len(),
                fits.cycle_net_kb.unwrap_or(0.0)
            ),
        );
    }
    (checks, fits)
}

#[allow(
    clippy::too_many_arguments,
    reason = "one call site; the report gathers everything the run produced"
)]
fn build_report(
    root: &Path,
    o: &Options,
    outcome: &Outcome,
    log: &ScriptLog,
    checks: &Checks,
    fits: &Fits,
    bundled: &Path,
    guard_armed: bool,
) -> Value {
    let commit = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    let samples: Vec<Value> = outcome
        .samples
        .iter()
        .map(|s| {
            json!({
                "t": (s.t * 10.0).round() / 10.0,
                "pss_kb": s.mem.pss, "pss_anon_kb": s.mem.pss_anon, "pss_file_kb": s.mem.pss_file,
                "swap_pss_kb": s.mem.swap_pss, "rss_kb": s.mem.rss,
                "threads": s.threads, "fds": s.fds, "xvfb_pss_kb": s.xvfb_pss,
                "editor_open": s.editor_open, "settled_closed": s.settled_closed,
            })
        })
        .collect();
    let cycles: Vec<Value> = log.all("cycle").into_iter().map(|c| json!(c)).collect();
    let events: Vec<Value> = log
        .data
        .iter()
        .filter(|(k, _)| k == "event")
        .take(500)
        .map(|(_, v)| json!(v))
        .collect();
    let rows: Vec<Value> = checks
        .rows
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "status": match r.status { Status::Ok => "ok", Status::Fail => "fail", Status::Info => "info" },
                "detail": r.detail,
            })
        })
        .collect();
    json!({
        "schema": "enth.reaper-soak/1",
        "commit": commit,
        "verdict": if checks.failures().is_empty() { "pass" } else { "fail" },
        "failed_checks": checks.failures().iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
        "config": {
            "minutes": o.minutes, "format": o.format.tag(), "instances": o.instances,
            "control_fx": o.control_fx, "editor_secs": o.editor_secs, "editor_hold_secs": o.editor_hold_secs,
            "notes": o.notes, "sweeps": o.sweeps, "load_secs": o.load_secs, "churn_secs": o.churn_secs,
            "sample_secs": o.sample_secs, "seed": o.seed, "bundle_dir": bundled.display().to_string(),
            "allocation_guard_armed": guard_armed, "self_test": o.self_test.map(|t| format!("{t:?}")),
            "script_env": o.extra_env,
        },
        "limits": {
            "memory_growth_mb_per_hour": o.limits.pss_slope_mb_h, "thread_spread": o.limits.thread_spread,
            "fd_growth": o.limits.fd_growth, "hang_secs": o.limits.hang_secs,
            "warmup_secs": fits.warmup_secs,
        },
        "ending": format!("{:?}", outcome.ending),
        "exit": outcome.exit,
        "wall_secs": outcome.wall_secs,
        "script": log.last("summary"),
        "fits": {
            "settled_samples": fits.settled_samples,
            "footprint_mb_per_hour": fits.footprint_mb_h, "pss_total_mb_per_hour": fits.pss_total_mb_h,
            "pss_file_mb_per_hour": fits.pss_file_mb_h, "xvfb_pss_mb_per_hour": fits.xvfb_mb_h,
            "footprint_start_mb": fits.footprint_start_mb, "footprint_end_mb": fits.footprint_end_mb,
            "threads_min": fits.threads_min, "threads_max": fits.threads_max,
            "fds_start": fits.fds_start, "fds_end": fits.fds_end,
            "editor_cycles": fits.cycles, "editor_cycle_growth_kb": fits.cycle_growth_kb,
            "editor_cycle_footprint_mb_per_hour": fits.cycle_footprint_mb_h,
            "editor_cycle_net_kb_median": fits.cycle_net_kb,
        },
        "threads_first": outcome.threads_first,
        "threads_last": outcome.threads_last,
        "checks": rows,
        "samples": samples,
        "editor_cycles": cycles,
        "peaks": log.all("peaks"),
        "events": events,
    })
}

fn print_summary(o: &Options, outcome: &Outcome, checks: &Checks, fits: &Fits, report: &Path) {
    println!();
    for row in &checks.rows {
        println!(
            "  {:<4} {:<30} {}",
            match row.status {
                Status::Ok => "ok",
                Status::Fail => "FAIL",
                Status::Info => "info",
            },
            row.name,
            row.detail
        );
    }
    println!();
    println!(
        "reaper-soak: {} min requested, {:.1} min of wall clock, {} samples, ended {:?}",
        o.minutes,
        outcome.wall_secs / 60.0,
        outcome.samples.len(),
        outcome.ending
    );
    if let (Some(a), Some(b)) = (fits.footprint_start_mb, fits.footprint_end_mb) {
        println!(
            "reaper-soak: anonymous memory {a:.0} -> {b:.0} MB, trend {:+.1} MB/h (limit {:.1}), threads {}-{}, descriptors {}-{}",
            fits.footprint_mb_h.unwrap_or(0.0),
            o.limits.pss_slope_mb_h,
            fits.threads_min.unwrap_or(0),
            fits.threads_max.unwrap_or(0),
            fits.fds_start.unwrap_or(0.0),
            fits.fds_end.unwrap_or(0.0),
        );
    }
    println!("reaper-soak: report {}", report.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn theil_sen_finds_the_slope_of_a_line_and_ignores_an_outlier() {
        let mut points: Vec<(f64, f64)> = (0..20)
            .map(|i| (f64::from(i), 100.0 + 3.0 * f64::from(i)))
            .collect();
        assert!((theil_sen(&points).unwrap() - 3.0).abs() < 1e-9);
        points[7].1 += 5000.0;
        assert!(
            (theil_sen(&points).unwrap() - 3.0).abs() < 1e-9,
            "one outlier moves a robust slope not at all"
        );
        assert_eq!(theil_sen(&[(1.0, 1.0)]), None);
        assert_eq!(theil_sen(&[(1.0, 1.0), (1.0, 5.0)]), None, "no spread in x");
    }

    #[test]
    fn a_flat_series_has_no_growth_and_a_rising_one_is_in_mb_per_hour() {
        let flat: Vec<(f64, f64)> = (0..30).map(|i| (f64::from(i) * 15.0, 200_000.0)).collect();
        assert_eq!(mb_per_hour(&flat), Some(0.0));
        // 1 kB per second is 3600 kB an hour: 3.515625 MB/h.
        let rising: Vec<(f64, f64)> = (0..30)
            .map(|i| (f64::from(i) * 15.0, 200_000.0 + f64::from(i) * 15.0))
            .collect();
        assert!((mb_per_hour(&rising).unwrap() - 3.515_625).abs() < 1e-9);
    }

    #[test]
    fn smaps_rollup_fields_are_read_by_name() {
        let text = "55d0-7ffd [rollup]\nRss:  1000 kB\nPss:   800 kB\nPss_Dirty:  100 kB\nPss_Anon:   300 kB\nPss_File:   450 kB\nSwapPss:    50 kB\n";
        let mem = parse_rollup(text);
        assert_eq!(
            mem,
            Mem {
                rss: 1000,
                pss: 800,
                pss_anon: 300,
                pss_file: 450,
                swap_pss: 50
            }
        );
        assert_eq!(mem.footprint(), 350);
    }

    #[test]
    fn the_script_log_is_read_into_checks_and_data() {
        let text = "check\tload-1\tok\tCLAPi: enth (enth)\ncheck\tscript-error\tFAIL\tboom\ndata\tbeat\tt=2.0 editor=0 since=7.5\ndata\tbeat\tt=4.0 editor=1\ndata\tfinished\t1\n";
        let log = ScriptLog::parse(text);
        assert_eq!(log.checks.len(), 2);
        assert!(log.checks[0].1 && !log.checks[1].1);
        assert_eq!(log.all("beat").len(), 2);
        let last = log.last("beat").unwrap();
        assert_eq!(num(&last, "t"), Some(4.0));
        assert_eq!(num(&last, "editor"), Some(1.0));
        assert!(log.has("finished"));
        assert!(!log.has("summary"));
    }

    #[test]
    fn an_allocation_abort_is_told_from_other_output() {
        assert_eq!(
            alloc_abort_in("noise\nmemory allocation of 32 bytes failed\nmore").as_deref(),
            Some("memory allocation of 32 bytes failed")
        );
        assert_eq!(
            alloc_abort_in("memory allocation of the buffer succeeded"),
            None
        );
        assert_eq!(alloc_abort_in(""), None);
    }

    #[test]
    fn exit_statuses_are_described() {
        assert_eq!(describe_exit(Some(0), None), "exit 0");
        assert_eq!(
            describe_exit(Some(134), None),
            "exit 134 (signal 6 SIGABRT)"
        );
        assert_eq!(
            describe_exit(Some(139), None),
            "exit 139 (signal 11 SIGSEGV)"
        );
        assert_eq!(describe_exit(Some(124), None), "killed by timeout (124)");
        assert_eq!(describe_exit(None, Some(9)), "signal 9 SIGKILL");
    }

    #[test]
    fn options_parse_and_refuse_nonsense() {
        let root = Path::new("/nonexistent-enth-root");
        let o = parse_options(
            &strings(&[
                "--minutes",
                "2.5",
                "--instances",
                "2",
                "--no-notes",
                "--format",
                "vst3",
            ]),
            root,
        )
        .unwrap();
        assert!(
            (o.minutes - 2.5).abs() < 1e-9
                && o.instances == 2
                && !o.notes
                && o.format == Format::Vst3
        );
        assert!(parse_options(&strings(&["--minutes", "0"]), root).is_err());
        assert!(parse_options(&strings(&["--instances", "17"]), root).is_err());
        assert!(parse_options(&strings(&["--minutes"]), root).is_err());
        assert!(parse_options(&strings(&["--bogus"]), root).is_err());
        assert!(parse_options(&strings(&["--self-test", "alloc"]), root).is_err());
        let o = parse_options(&strings(&["--script-env", "ENTH_SOAK_BEAT_SECS=1"]), root).unwrap();
        assert_eq!(
            o.extra_env,
            vec![("ENTH_SOAK_BEAT_SECS".to_owned(), "1".to_owned())]
        );
    }
}
