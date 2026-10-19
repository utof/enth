//! `enth-test-host`: a minimal CLAP host for CI, determinism checks and editor leak measurement.
//! Every subcommand prints one JSON object on stdout; errors are JSON on stderr with exit code 2.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool: JSON on stdout is the interface"
)]

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clack_host::prelude::PluginEntry;
use enth_test_host::editor::{self, Mode};
use enth_test_host::host::{load_entry, resolve_plugin_id};
use enth_test_host::instances;
use enth_test_host::notes;
use enth_test_host::procfs;
use enth_test_host::query;
use enth_test_host::render::{
    ProcessThread, RenderConfig, RenderOutcome, RenderRequest, render_once,
};
use enth_test_host::wav;
use serde_json::{Value, json};

const USAGE: &str = "enth-test-host: a minimal CLAP test host (docs/spikes/test-host.md)

usage:
  enth-test-host info <plugin.clap> [--plugin-id ID]
  enth-test-host render <plugin.clap> --notes <script|file> [--sr 48000] [--block 64] [--dur 2]
        [--state FILE] [--param NAME=VALUE]... [--out out.wav] [--save-state FILE]
        [--process-on thread|caller] [--plugin-id ID]
  enth-test-host instances <plugin.clap> --notes <script|file> [-n 2] [--sr 48000] [--block 64] [--dur 2]
        [--state FILE] [--param NAME=VALUE]... [--modes sequential-fresh,sequential-late,lockstep,concurrent]
  enth-test-host editor-cycles <plugin.clap> [-n 200] [--mode both|editor|baseline] [--repeat 1]
        [--warmup 10] [--hold-ms 100] [--settle-ms 50] [--sr 48000] [--block 256] [--window 360x200]

Notes script: entries separated by , ; or newline, `#` comments. `C4@0+1.5:0.8` is a note (key@start+duration[:velocity],
seconds); `Slot 1=0.2@0.5` sets a parameter (name or numeric id = value @ seconds).

editor-cycles opens windows, so it refuses to run unless DISPLAY is an Xvfb server:
  xvfb-run -a -s \"-screen 0 1920x1080x24\" enth-test-host editor-cycles plugin.clap -n 200

Any command takes `--json-out FILE`: the JSON is also written there. Use it under xvfb-run, which folds the
command's stderr (plugin log lines) into its stdout.

exit codes: 0 ok, 1 a check failed (instances differ), 2 error";

/// Parsed command line: positional arguments and `--name value` options.
struct Args {
    positional: Vec<String>,
    options: BTreeMap<String, Vec<String>>,
}

impl Args {
    fn parse(raw: &[String], valued: &[&str], flags: &[&str]) -> Result<Self, String> {
        let mut args = Self {
            positional: Vec::new(),
            options: BTreeMap::new(),
        };
        let mut iter = raw.iter();
        while let Some(arg) = iter.next() {
            let name = if let Some(long) = arg.strip_prefix("--") {
                long
            } else if arg == "-n" {
                "n"
            } else {
                args.positional.push(arg.clone());
                continue;
            };
            if flags.contains(&name) {
                args.options.entry(name.to_owned()).or_default();
            } else if valued.contains(&name) {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("option {arg} needs a value"))?;
                args.options
                    .entry(name.to_owned())
                    .or_default()
                    .push(value.clone());
            } else {
                return Err(format!(
                    "unknown option {arg}; this command takes {}",
                    valued
                        .iter()
                        .chain(flags)
                        .map(|o| format!("--{o}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        }
        Ok(args)
    }

    fn one(&self, name: &str) -> Option<&str> {
        self.options
            .get(name)
            .and_then(|v| v.last())
            .map(String::as_str)
    }

    fn many(&self, name: &str) -> &[String] {
        self.options.get(name).map_or(&[], Vec::as_slice)
    }

    fn number<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.one(name) {
            None => Ok(default),
            Some(text) => text
                .parse()
                .map_err(|_| format!("--{name}: {text:?} is not a valid number")),
        }
    }

    fn plugin_path(&self) -> Result<PathBuf, String> {
        match self.positional.as_slice() {
            [one] => Ok(PathBuf::from(one)),
            [] => Err("missing <plugin.clap>".to_owned()),
            more => Err(format!("expected one <plugin.clap>, got {}", more.len())),
        }
    }
}

/// Pulls `--json-out FILE` out of the arguments, whatever the command. `xvfb-run` folds the command's
/// stderr into its stdout, so plugin log lines can end up next to the JSON; the file is always clean.
fn take_json_out(mut raw: Vec<String>) -> (Vec<String>, Option<String>) {
    let Some(at) = raw.iter().position(|a| a == "--json-out") else {
        return (raw, None);
    };
    let path = raw.get(at + 1).cloned();
    raw.drain(at..(at + 2).min(raw.len()));
    (raw, path)
}

fn print_json(value: &Value) {
    let text = if std::io::stdout().is_terminal() {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    };
    println!(
        "{}",
        text.unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    );
}

fn main() -> ExitCode {
    let (raw, json_out) = take_json_out(std::env::args().skip(1).collect());
    let Some((command, rest)) = raw.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let result = match command.as_str() {
        "info" => cmd_info(rest),
        "render" => cmd_render(rest),
        "instances" => cmd_instances(rest),
        "editor-cycles" => cmd_editor_cycles(rest),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    };
    match result {
        Ok((value, ok)) => {
            print_json(&value);
            if let Some(path) = json_out
                && let Err(e) = std::fs::write(&path, format!("{value}\n"))
            {
                eprintln!("--json-out {path}: {e}");
                return ExitCode::from(2);
            }
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(message) => {
            let error = json!({
                "schema": "enth.test-host.error/1",
                "error": message,
                "hint": "run `enth-test-host help` for the commands and options",
            });
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

type Outcome = Result<(Value, bool), String>;

fn load(args: &Args) -> Result<(PathBuf, PluginEntry, std::ffi::CString), String> {
    let path = args.plugin_path()?;
    let entry = load_entry(&path)?;
    let id = resolve_plugin_id(&entry, args.one("plugin-id"))?;
    Ok((path, entry, id))
}

fn cmd_info(rest: &[String]) -> Outcome {
    let args = Args::parse(rest, &["plugin-id"], &[])?;
    let path = args.plugin_path()?;
    let entry = load_entry(&path)?;
    let ids = enth_test_host::host::plugin_ids(&entry)?;
    let selected: Vec<_> = match args.one("plugin-id") {
        Some(_) => vec![resolve_plugin_id(&entry, args.one("plugin-id"))?],
        None => ids,
    };
    let plugins = selected
        .iter()
        .map(|id| query::plugin_report(&entry, id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((
        json!({
            "schema": "enth.test-host.info/1",
            "path": path.display().to_string(),
            "clap_version": entry.version().to_string(),
            "plugins": plugins,
        }),
        true,
    ))
}

/// The options `render` and `instances` share.
struct RenderArgs {
    request: RenderRequest,
    state_bytes: Option<usize>,
}

fn render_args(args: &Args) -> Result<RenderArgs, String> {
    let sample_rate: f64 = args.number("sr", 48_000.0)?;
    let block: u32 = args.number("block", 64)?;
    let duration: f64 = args.number("dur", 2.0)?;
    let config = RenderConfig::new(sample_rate, block, duration)?;
    let script = notes::load_script(args.one("notes").ok_or("--notes is required")?)?;
    let state = args
        .one("state")
        .map(|p| std::fs::read(p).map_err(|e| format!("--state {p}: {e}")))
        .transpose()?;
    let state_bytes = state.as_ref().map(Vec::len);
    let params = args
        .many("param")
        .iter()
        .map(|p| {
            let (name, value) = p
                .rsplit_once('=')
                .ok_or_else(|| format!("--param {p:?}: expected NAME=VALUE"))?;
            let value = value
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("--param {p:?}: {value:?} is not a number"))?;
            Ok((name.trim().to_owned(), value))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(RenderArgs {
        request: RenderRequest {
            script,
            params,
            state,
            config,
        },
        state_bytes,
    })
}

fn config_json(config: &RenderConfig, channels: usize) -> Value {
    json!({
        "sample_rate": config.sample_rate,
        "block": config.block,
        "blocks": config.blocks(),
        "frames": config.frames,
        "channels": channels,
    })
}

fn cmd_render(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &[
            "notes",
            "sr",
            "block",
            "dur",
            "state",
            "param",
            "out",
            "save-state",
            "process-on",
            "plugin-id",
        ],
        &[],
    )?;
    let (path, entry, id) = load(&args)?;
    let render = render_args(&args)?;
    let thread = match args.one("process-on").unwrap_or("thread") {
        "thread" => ProcessThread::Dedicated,
        "caller" => ProcessThread::Calling,
        other => return Err(format!("--process-on {other:?}: use `thread` or `caller`")),
    };
    let RenderOutcome {
        rendered,
        state,
        params,
    } = render_once(
        &entry,
        &id,
        &render.request,
        thread,
        args.one("save-state").is_some(),
    )?;
    if let Some(out) = args.one("out") {
        wav::write_f32(
            Path::new(out),
            render.request.config.sample_rate.round() as u32,
            rendered.channels as u16,
            &rendered.samples,
        )?;
    }
    if let (Some(path), Some(bytes)) = (args.one("save-state"), &state) {
        std::fs::write(path, bytes).map_err(|e| format!("--save-state {path}: {e}"))?;
    }
    let resolve = |n: &str| query::resolve_param(&params, n);
    let schedule = notes::parse(
        &render.request.script,
        render.request.config.sample_rate,
        render.request.config.frames,
        &resolve,
    )?;
    Ok((
        json!({
            "schema": "enth.test-host.render/1",
            "path": path.display().to_string(),
            "plugin_id": id.to_string_lossy(),
            "config": config_json(&render.request.config, rendered.channels),
            "process_thread": match thread { ProcessThread::Dedicated => "dedicated thread", ProcessThread::Calling => "calling (main) thread" },
            "notes": {
                "note_ons": schedule.note_count(),
                "events": schedule.events.len(),
                "schedule": notes::describe(&schedule),
            },
            "params_set": render.request.params.iter().map(|(n, v)| json!({"name": n, "value": v})).collect::<Vec<_>>(),
            "state": {
                "loaded_bytes": render.state_bytes,
                "saved_bytes": state.as_ref().map(Vec::len),
                "saved_to": args.one("save-state"),
            },
            "output": rendered.to_json(&render.request.config),
            "sha256": rendered.sha256,
            "wav": args.one("out"),
        }),
        true,
    ))
}

fn cmd_instances(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &[
            "notes",
            "sr",
            "block",
            "dur",
            "state",
            "param",
            "n",
            "modes",
            "plugin-id",
        ],
        &[],
    )?;
    let (path, entry, id) = load(&args)?;
    let render = render_args(&args)?;
    let n: usize = args.number("n", 2)?;
    let modes: Vec<&str> = match args.one("modes") {
        Some(list) => list.split(',').map(str::trim).collect(),
        None => instances::MODES.to_vec(),
    };
    let report = instances::run(&entry, &id, &render.request, n, &modes)?;
    let identical = report.identical;
    Ok((
        json!({
            "schema": "enth.test-host.instances/1",
            "path": path.display().to_string(),
            "plugin_id": id.to_string_lossy(),
            "instances": n,
            "config": config_json(&render.request.config, report.reference.channels),
            "report": report.to_json(),
            "identical": identical,
        }),
        identical,
    ))
}

fn parse_window(text: &str) -> Result<(u16, u16), String> {
    let (w, h) = text
        .split_once('x')
        .ok_or_else(|| format!("--window {text:?}: expected WIDTHxHEIGHT"))?;
    let parse = |v: &str| {
        v.parse::<u16>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("--window {text:?}: {v:?} is not a size"))
    };
    Ok((parse(w)?, parse(h)?))
}

fn editor_options(args: &Args, mode: Mode) -> Result<editor::Options, String> {
    let defaults = editor::Options::default();
    Ok(editor::Options {
        cycles: args.number("n", defaults.cycles)?,
        warmup: args.number("warmup", defaults.warmup)?,
        hold_ms: args.number("hold-ms", defaults.hold_ms)?,
        settle_ms: args.number("settle-ms", defaults.settle_ms)?,
        first_frame_timeout_ms: args
            .number("first-frame-timeout-ms", defaults.first_frame_timeout_ms)?,
        sample_rate: args.number("sr", defaults.sample_rate)?,
        block: args.number("block", defaults.block)?,
        window: args
            .one("window")
            .map_or(Ok(defaults.window), parse_window)?,
        mode,
    })
}

fn cmd_editor_cycles(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &[
            "n",
            "mode",
            "repeat",
            "warmup",
            "hold-ms",
            "settle-ms",
            "first-frame-timeout-ms",
            "sr",
            "block",
            "window",
            "plugin-id",
        ],
        &[],
    )?;
    match args.one("mode").unwrap_or("both") {
        "editor" | "baseline" => {
            // One measurement process. Run directly (also the form to use under heaptrack).
            let mode = if args.one("mode") == Some("editor") {
                Mode::Editor
            } else {
                Mode::Baseline
            };
            // The guard comes before the plugin library is even loaded.
            enth_test_host::xvfb::require_xvfb()?;
            let opts = editor_options(&args, mode)?;
            let (_, entry, id) = load(&args)?;
            let mut run = editor::run_cycles(&entry, &id, &opts)?;
            run["schema"] = json!("enth.test-host.editor-run/1");
            Ok((run, true))
        }
        "both" => editor_both(&args),
        other => Err(format!("--mode {other:?}: use both, editor or baseline")),
    }
}

/// Runs baseline and editor measurements in separate child processes (so one run's memory never
/// counts toward the other) and compares them.
fn editor_both(args: &Args) -> Outcome {
    // Refuse before spawning anything.
    let xvfb = enth_test_host::xvfb::require_xvfb()?;
    let path = args.plugin_path()?;
    let repeat: usize = args.number("repeat", 1)?;
    let exe = std::env::current_exe().map_err(|e| format!("cannot find myself: {e}"))?;
    let mut runs = Vec::new();
    let mut close_warnings = 0usize;
    let mut stderr_samples: Vec<String> = Vec::new();
    for _ in 0..repeat {
        for mode in [Mode::Baseline, Mode::Editor] {
            let mut cmd = Command::new(&exe);
            cmd.arg("editor-cycles")
                .arg(&path)
                .args(["--mode", mode.name()]);
            for name in [
                "n",
                "warmup",
                "hold-ms",
                "settle-ms",
                "first-frame-timeout-ms",
                "sr",
                "block",
                "window",
                "plugin-id",
            ] {
                if let Some(v) = args.one(name) {
                    cmd.arg(format!("--{name}")).arg(v);
                }
            }
            let output = cmd
                .output()
                .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
            let stderr = String::from_utf8_lossy(&output.stderr);
            for line in stderr.lines() {
                if line.contains("X11 thread disconnected") {
                    close_warnings += 1;
                }
                if stderr_samples.len() < 8 && !stderr_samples.iter().any(|s| s == line) {
                    stderr_samples.push(line.to_owned());
                }
            }
            if !output.status.success() {
                return Err(format!(
                    "the {} run failed ({}): {}",
                    mode.name(),
                    output.status,
                    stderr.lines().last().unwrap_or("no output")
                ));
            }
            let run: Value = serde_json::from_slice(&output.stdout)
                .map_err(|e| format!("bad JSON from the {} run: {e}", mode.name()))?;
            runs.push(run);
        }
    }
    let pick = |mode: &str, field: &[&str]| -> Vec<f64> {
        runs.iter()
            .filter(|r| r["mode"] == mode)
            .filter_map(|r| {
                let mut v = r;
                for f in field {
                    v = &v[*f];
                }
                v.as_f64()
            })
            .collect()
    };
    let slope_path = ["summary", "pss_growth_kb_per_cycle_after_warmup"];
    let editor_slope = pick("editor", &slope_path);
    let baseline_slope = pick("baseline", &slope_path);
    let mean = |v: &[f64]| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    let comparison = json!({
        "editor_pss_growth_kb_per_cycle": editor_slope,
        "baseline_pss_growth_kb_per_cycle": baseline_slope,
        "editor_minus_baseline_kb_per_cycle": mean(&editor_slope).zip(mean(&baseline_slope)).map(|(e, b)| e - b),
        "note": "least-squares slope of PSS after close, cycles after the warm-up only",
    });
    Ok((
        json!({
            "schema": "enth.test-host.editor-cycles/1",
            "path": path.display().to_string(),
            "environment": {
                "display": xvfb.display,
                "xvfb_pid": xvfb.pid,
                "renderer": "Xvfb with whatever GL the system provides (Mesa software rendering; never the real GPU)",
                "loadavg": procfs::loadavg(),
                "kernel": std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_owned()).ok(),
            },
            "repeat": repeat,
            "comparison": comparison,
            "close_warnings_x11_thread_disconnected": close_warnings,
            "stderr_samples": stderr_samples,
            "runs": runs,
        }),
        true,
    ))
}
