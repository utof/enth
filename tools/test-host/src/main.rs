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
use enth_test_host::host::{load_entry, plugin_version, resolve_plugin_id};
use enth_test_host::render::{
    ProcessThread, RenderConfig, RenderOutcome, RenderRequest, render_once,
};
use enth_test_host::{
    churn, cli_equal, events, instances, latency, notes, procfs, query, state, stats, wav,
};
use serde_json::{Value, json};

const USAGE: &str = "enth-test-host: a minimal CLAP test host (docs/spikes/test-host.md)

usage:
  enth-test-host info <plugin.clap> [--plugin-id ID]
  enth-test-host render <plugin.clap> --notes <script|file> [--sr 48000] [--block 64] [--dur 2]
        [--state FILE [--state-format auto|blob|patch|envelope] [--state-field patch]]
        [--param NAME=VALUE]... [--out out.wav] [--save-state FILE] [--process-on thread|caller] [--plugin-id ID]
  enth-test-host instances <plugin.clap> --notes <script|file> [-n 2] [--sr 48000] [--block 64] [--dur 2]
        [--state FILE ...] [--param NAME=VALUE]... [--modes sequential-fresh,sequential-late,lockstep,concurrent]
  enth-test-host cli-equal <plugin.clap> [--cases DIR|FILE]... [--expected DIR | --no-expected] [--only NAME]...
        [--blocks 1,64,100,4096] [--profile FILE] [--state-field patch] [--min-compared 1]
        [--write-expected DIR] [--inject flip-bit]
  enth-test-host churn <plugin.clap> [-n 200] [--warmup 10] [--notes SCRIPT] [--state FILE ...] [--sr 48000]
        [--block 64] [--blocks 64] [--settle-ms 10] [--max-kb-per-cycle 1] [--max-thread-growth 0] [--system-malloc] [--system-thp]
        [--max-fd-growth 0] [--process-on thread|caller] [--inject leak-kb=N|fd|thread]
  enth-test-host bursts <plugin.clap> [--counts 16,64,256,1024,4096] [--kind mixed|notes|params] [--block 64]
        [--capacity N] [--state FILE ...] [--sr 48000] [--warm-blocks 8] [--after-blocks 8] [--tail 1.0]
        [--order both|notes-first|params-first] [--allow-unended] [--inject drop-after=K]
  enth-test-host note-end <plugin.clap> [--notes SCRIPT] [--blocks 1,32,64,100,512] [--tail 2] [--state FILE ...]
        [--sr 48000] [--allow-unended] [--inject drop-end=K,late-end=F]
  enth-test-host process-stats <plugin.clap> [--secs 120] [--warmup 10] [--block 64] [--sr 48000] [--rt 10|none]
        [--require-rt] [--pin CPU,CPU] [--instances 1] [--threads 1] [--editor] [--notes SCRIPT] [--state FILE ...]
        [--gate g1|g1-multi] [--max-p99-us N] [--max-p999-us N] [--max-callback-p99-pct N]
        [--inject busy-us=N[,every=K]] [--swap-state FILE [--swap-ms 250] [--swap-kind instant|crossfade]
        [--max-swap-p99-us N] [--max-crossfade-p99-us N]] [--flood N [--flood-ms 250]]
        [--ab MS (a bundle built with --features ab-toggle: a Recreate round's A/B flipping every MS ms)]
  enth-test-host latency <plugin.clap> --param NAME --to VALUE [--from VALUE] [--notes SCRIPT] [--at 0.5]
        [--offsets 0,17,63] [--after 0.5] [--block 64] [--sr 48000] [--threshold-db -60] [--state FILE ...]
        [--max-onset-ms N] [--max-audible-ms N] [--inject delay-blocks=K]
  enth-test-host editor-cycles <plugin.clap> [-n 200] [--mode both|editor|baseline] [--repeat 1]
        [--warmup 10] [--hold-ms 100] [--settle-ms 50] [--first-frame-timeout-ms 1000] [--sr 48000] [--block 256]
        [--window 360x200] [--plugin-id ID] [--system-malloc] [--system-thp]

Notes script: entries separated by , ; or newline, `#` comments. `C4@0+1.5:0.8` is a note (key@start+duration[:velocity],
seconds); `Slot 1=0.2@0.5` sets a parameter (name or numeric id = value @ seconds).

`--state FILE` loads a state through the CLAP state extension before activation. FILE is a state blob the plugin saved
(`render --save-state`), a patch document (JSON, wrapped into the plugin's `patch` persisted field with the plugin's
version) or the envelope JSON ({\"version\",\"params\",\"fields\"}); `--state-format` forces one, the default tells them apart.

editor-cycles (and process-stats --editor) open windows, so they refuse to run unless DISPLAY is an Xvfb server:
  xvfb-run -a -s \"-screen 0 1920x1080x24\" enth-test-host editor-cycles plugin.clap -n 200
editor-cycles and churn set glibc's allocator to one arena with fixed 128 KiB mmap and trim thresholds before they
load the plugin, so a run's memory never depends on which arena a new thread gets, and churn gives every free page
back (malloc_trim) before each sample; --system-malloc keeps glibc's defaults and skips the trim. Both also turn
transparent huge pages off (prctl PR_SET_THP_DISABLE), so memory grows in 4 KB pages on any kernel setting (THP
`always` read 2 MB per editor cycle on the CI runner); --system-thp keeps the kernel's setting. editor-cycles also
records SwapPss beside Pss_Anon, and lets threads that are still exiting go (up to 1 s) before it counts them.
ENTH_LEAK_DIAG=1 adds a `diag` object per run: thread names when the count changes, and the smaps mappings behind
every jump of more than 2 MB in Pss_Anon.
Timings (process-stats, latency's process_us) belong under `flock -x /tmp/enth-quiet.lock`.

Any command takes `--json-out FILE`: the JSON is also written there. Use it under xvfb-run, which folds the
command's stderr (plugin log lines) into its stdout.

exit codes: 0 ok, 1 a check failed (instances differ, a hash differs, a budget is exceeded), 2 error";

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

    /// A comma-separated list of numbers, or `default`.
    fn list<T: std::str::FromStr + Clone>(
        &self,
        name: &str,
        default: &[T],
    ) -> Result<Vec<T>, String> {
        match self.one(name) {
            None => Ok(default.to_vec()),
            Some(text) => text
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| {
                    p.parse()
                        .map_err(|_| format!("--{name}: {p:?} is not a valid number"))
                })
                .collect(),
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
        "cli-equal" => cmd_cli_equal(rest),
        "churn" => cmd_churn(rest),
        "bursts" => cmd_bursts(rest),
        "note-end" => cmd_note_end(rest),
        "process-stats" => cmd_process_stats(rest),
        "latency" => cmd_latency(rest),
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

/// The `--state` options every command that plays audio takes.
const STATE_OPTIONS: [&str; 3] = ["state", "state-format", "state-field"];

/// The valued options of a command: its own plus the shared ones.
fn valued<'a>(own: &[&'a str], shared: &[&'a str]) -> Vec<&'a str> {
    own.iter().chain(shared).copied().collect()
}

/// A state file read and made ready for the plugin's `state.load`.
struct LoadedState {
    prepared: state::Prepared,
    file: String,
}

impl LoadedState {
    fn to_json(&self) -> Value {
        json!({
            "file": self.file,
            "loaded_bytes": self.prepared.bytes.len(),
            "loaded_as": self.prepared.kind.name(),
            "field": self.prepared.field,
        })
    }
}

/// `--state FILE [--state-format F] [--state-field NAME]`, resolved against the plugin: a patch document is wrapped
/// into the persisted field with the plugin's own version, so the plugin sees what a host's project load would give it.
fn load_state(
    args: &Args,
    entry: &PluginEntry,
    id: &std::ffi::CStr,
) -> Result<Option<LoadedState>, String> {
    let Some(file) = args.one("state") else {
        if args.one("state-format").is_some() || args.one("state-field").is_some() {
            return Err("--state-format and --state-field need --state FILE".to_owned());
        }
        return Ok(None);
    };
    let bytes = std::fs::read(file).map_err(|e| format!("--state {file}: {e}"))?;
    let format = state::Format::parse(args.one("state-format").unwrap_or("auto"))?;
    let field = args.one("state-field").unwrap_or(state::DEFAULT_FIELD);
    let prepared = state::prepare(bytes, format, field, &plugin_version(entry, id))
        .map_err(|e| format!("--state {file}: {e}"))?;
    Ok(Some(LoadedState {
        prepared,
        file: file.to_owned(),
    }))
}

/// The options `render` and `instances` share.
struct RenderArgs {
    request: RenderRequest,
    state: Option<LoadedState>,
}

fn render_args(
    args: &Args,
    entry: &PluginEntry,
    id: &std::ffi::CStr,
) -> Result<RenderArgs, String> {
    let sample_rate: f64 = args.number("sr", 48_000.0)?;
    let block: u32 = args.number("block", 64)?;
    let duration: f64 = args.number("dur", 2.0)?;
    let config = RenderConfig::new(sample_rate, block, duration)?;
    let script = notes::load_script(args.one("notes").ok_or("--notes is required")?)?;
    let loaded = load_state(args, entry, id)?;
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
            state: loaded.as_ref().map(|s| s.prepared.bytes.clone()),
            config,
        },
        state: loaded,
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

/// For a patch document loaded into a persisted field: does the plugin save the same JSON back in that field?
/// `None` when the state was not a patch document or nothing was saved.
fn patch_round_trip(loaded: Option<&LoadedState>, saved: Option<&[u8]>) -> Option<Value> {
    let loaded = loaded?;
    let field = loaded.prepared.field.as_ref()?;
    let sent = state::fields_of(&loaded.prepared.bytes)
        .ok()?
        .get(field)?
        .clone();
    let sent_json: Value = serde_json::from_str(&sent).ok()?;
    let back = state::fields_of(saved?).ok()?.get(field).cloned();
    let back_json = back
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    Some(json!({
        "field": field,
        "saved_field_present": back.is_some(),
        "identical_text": back.as_ref().map(|b| *b == sent),
        "equal_json": back_json.as_ref().map(|b| *b == sent_json),
    }))
}

fn cmd_render(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &valued(
            &[
                "notes",
                "sr",
                "block",
                "dur",
                "param",
                "out",
                "save-state",
                "process-on",
                "plugin-id",
            ],
            &STATE_OPTIONS,
        ),
        &["check-roundtrip"],
    )?;
    let (path, entry, id) = load(&args)?;
    let render = render_args(&args, &entry, &id)?;
    let check_roundtrip = args.options.contains_key("check-roundtrip");
    if check_roundtrip && args.one("save-state").is_none() {
        return Err("--check-roundtrip needs --save-state FILE".to_owned());
    }
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
    let round_trip = patch_round_trip(render.state.as_ref(), state.as_deref());
    let round_trip_ok = round_trip.as_ref().is_some_and(|r| r["equal_json"] == true);
    if check_roundtrip && round_trip.is_none() {
        return Err("--check-roundtrip needs --state with a patch document (not a blob or envelope), and a plugin that saves a state".to_owned());
    }
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
                "loaded_bytes": render.state.as_ref().map(|s| s.prepared.bytes.len()),
                "loaded": render.state.as_ref().map(LoadedState::to_json),
                "saved_bytes": state.as_ref().map(Vec::len),
                "saved_to": args.one("save-state"),
                "patch_round_trip": round_trip,
                "saved_fields": state.as_deref().and_then(|b| state::fields_of(b).ok())
                    .map(|f| f.iter().map(|(k, v)| (k.clone(), json!(v.len()))).collect::<serde_json::Map<_, _>>()),
            },
            "output": rendered.to_json(&render.request.config),
            "sha256": rendered.sha256,
            "wav": args.one("out"),
        }),
        !check_roundtrip || round_trip_ok,
    ))
}

fn cmd_instances(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &valued(
            &[
                "notes",
                "sr",
                "block",
                "dur",
                "param",
                "n",
                "modes",
                "plugin-id",
            ],
            &STATE_OPTIONS,
        ),
        &[],
    )?;
    let (path, entry, id) = load(&args)?;
    let render = render_args(&args, &entry, &id)?;
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
            "state": render.state.as_ref().map(LoadedState::to_json),
            "config": config_json(&render.request.config, report.reference.channels),
            "report": report.to_json(),
            "identical": identical,
        }),
        identical,
    ))
}

/// The directory `rel` in the working directory or the nearest parent that has it.
fn find_up(rel: &str) -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(rel);
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Fills in what every report of the new commands carries and turns its `ok` into the exit status.
fn finish(
    mut out: Value,
    schema: &str,
    path: &Path,
    id: &std::ffi::CStr,
    loaded: Option<&LoadedState>,
) -> (Value, bool) {
    let ok = out["ok"] == true;
    out["schema"] = json!(schema);
    out["path"] = json!(path.display().to_string());
    out["plugin_id"] = json!(id.to_string_lossy());
    out["state"] = loaded.map_or(Value::Null, LoadedState::to_json);
    (out, ok)
}

fn process_thread(args: &Args) -> Result<ProcessThread, String> {
    match args.one("process-on").unwrap_or("thread") {
        "thread" => Ok(ProcessThread::Dedicated),
        "caller" => Ok(ProcessThread::Calling),
        other => Err(format!("--process-on {other:?}: use `thread` or `caller`")),
    }
}

fn state_bytes(loaded: Option<&LoadedState>) -> Option<Vec<u8>> {
    loaded.map(|s| s.prepared.bytes.clone())
}

fn cmd_cli_equal(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &[
            "cases",
            "expected",
            "only",
            "blocks",
            "profile",
            "state-field",
            "min-compared",
            "write-expected",
            "inject",
            "plugin-id",
        ],
        &["no-expected"],
    )?;
    let (path, entry, id) = load(&args)?;
    let cases: Vec<PathBuf> = if args.many("cases").is_empty() {
        vec![find_up("tests/golden/cases").ok_or(
            "--cases is needed: tests/golden/cases was not found above the working directory",
        )?]
    } else {
        args.many("cases").iter().map(PathBuf::from).collect()
    };
    let expected_dir = if args.options.contains_key("no-expected") {
        None
    } else if let Some(dir) = args.one("expected") {
        Some(PathBuf::from(dir))
    } else {
        // `expected` next to the first cases directory (or next to the directory of a case file).
        let sibling = cases
            .first()
            .and_then(|c| {
                if c.is_dir() {
                    c.parent()
                } else {
                    c.parent().and_then(Path::parent)
                }
            })
            .map(|dir| dir.join("expected"))
            .filter(|dir| dir.is_dir());
        Some(
            sibling
                .or_else(|| find_up("tests/golden/expected"))
                .ok_or("no expected hashes found: pass --expected DIR, or --no-expected to compare block sizes with each other only")?,
        )
    };
    let profile = match args.one("profile") {
        Some(file) => cli_equal::parse_profile(
            &std::fs::read_to_string(file).map_err(|e| format!("--profile {file}: {e}"))?,
        )
        .map_err(|e| format!("--profile {file}: {e}"))?,
        None => cli_equal::Profile::default(),
    };
    let blocks = match args.one("blocks") {
        Some(_) => args.list::<u32>("blocks", &[])?,
        None => profile
            .blocks
            .clone()
            .unwrap_or_else(|| cli_equal::DEFAULT_BLOCKS.to_vec()),
    };
    if blocks.is_empty() || blocks.contains(&0) {
        return Err("--blocks needs block sizes of at least 1".to_owned());
    }
    let state_field = args
        .one("state-field")
        .map(str::to_owned)
        .or_else(|| profile.state_field.clone())
        .unwrap_or_else(|| state::DEFAULT_FIELD.to_owned());
    let inject = args
        .one("inject")
        .map(cli_equal::Inject::parse)
        .transpose()?
        .unwrap_or_default();
    let opts = cli_equal::Options {
        cases,
        expected_dir: expected_dir.clone(),
        only: args.many("only").to_vec(),
        blocks: blocks.clone(),
        profile,
        state_field: state_field.clone(),
        min_compared: args.number("min-compared", 1)?,
        write_expected: args.one("write-expected").map(PathBuf::from),
        inject,
    };
    let report = cli_equal::run(&entry, &id, &opts)?;
    let mut out = report.to_json();
    out["blocks"] = json!(blocks);
    out["state_field"] = json!(state_field);
    out["expected_dir"] = json!(expected_dir.map(|d| d.display().to_string()));
    out["inject"] = json!(format!("{inject:?}"));
    out["plugin_version"] = json!(plugin_version(&entry, &id));
    Ok(finish(out, "enth.test-host.cli-equal/1", &path, &id, None))
}

fn cmd_churn(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &valued(
            &[
                "n",
                "warmup",
                "notes",
                "sr",
                "block",
                "blocks",
                "settle-ms",
                "max-kb-per-cycle",
                "max-thread-growth",
                "max-fd-growth",
                "process-on",
                "inject",
                "plugin-id",
            ],
            &STATE_OPTIONS,
        ),
        &["system-malloc", "system-thp"],
    )?;
    // Before the plugin is loaded and before any thread starts: each cycle's audio thread is a new thread, which with
    // glibc's defaults can bring its own arena (lane LK; procfs::deterministic_malloc).
    let malloc = if args.options.contains_key("system-malloc") {
        json!("system (--system-malloc)")
    } else {
        procfs::deterministic_malloc()
    };
    let thp = thp_setting(&args);
    let (path, entry, id) = load(&args)?;
    let loaded = load_state(&args, &entry, &id)?;
    let d = churn::Options::default();
    let opts = churn::Options {
        cycles: args.number("n", d.cycles)?,
        warmup: args.number("warmup", d.warmup)?,
        sample_rate: args.number("sr", d.sample_rate)?,
        block: args.number("block", d.block)?,
        blocks: args.number("blocks", d.blocks)?,
        notes: match args.one("notes") {
            Some(n) => notes::load_script(n)?,
            None => d.notes,
        },
        state: state_bytes(loaded.as_ref()),
        settle_ms: args.number("settle-ms", d.settle_ms)?,
        max_kb_per_cycle: args.number("max-kb-per-cycle", d.max_kb_per_cycle)?,
        max_thread_growth: args.number("max-thread-growth", d.max_thread_growth)?,
        max_fd_growth: args.number("max-fd-growth", d.max_fd_growth)?,
        inject: args.one("inject").map(churn::Inject::parse).transpose()?,
        process_on: process_thread(&args)?,
        trim: !args.options.contains_key("system-malloc"),
    };
    if opts.cycles < 3 {
        return Err("-n needs at least 3 cycles".to_owned());
    }
    let mut out = churn::run(&entry, &id, &opts)?;
    out["malloc"] = malloc;
    out["thp"] = thp;
    Ok(finish(
        out,
        "enth.test-host.churn/1",
        &path,
        &id,
        loaded.as_ref(),
    ))
}

fn cmd_bursts(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &valued(
            &[
                "counts",
                "kind",
                "block",
                "capacity",
                "sr",
                "warm-blocks",
                "after-blocks",
                "tail",
                "order",
                "inject",
                "plugin-id",
            ],
            &STATE_OPTIONS,
        ),
        &["allow-unended"],
    )?;
    let (path, entry, id) = load(&args)?;
    let loaded = load_state(&args, &entry, &id)?;
    let opts = events::BurstOptions {
        orders: events::SameFrame::parse_list(args.one("order").unwrap_or("both"))?,
        counts: args.list("counts", &[16usize, 64, 256, 1024, 4096])?,
        kind: events::BurstKind::parse(args.one("kind").unwrap_or("mixed"))?,
        block: args.number("block", 64)?,
        sample_rate: args.number("sr", 48_000.0)?,
        capacity: args
            .one("capacity")
            .map(|c| {
                c.parse()
                    .map_err(|_| format!("--capacity {c:?} is not a number"))
            })
            .transpose()?,
        state: state_bytes(loaded.as_ref()),
        warm_blocks: args.number("warm-blocks", 8)?,
        after_blocks: args.number("after-blocks", 8)?,
        tail_s: args.number("tail", 1.0)?,
        allow_unended: args.options.contains_key("allow-unended"),
        inject: args
            .one("inject")
            .map(events::Inject::parse)
            .transpose()?
            .unwrap_or_default(),
    };
    if opts.counts.is_empty() || opts.block == 0 {
        return Err("--counts needs at least one count and --block at least 1".to_owned());
    }
    let out = events::run_bursts(&entry, &id, &opts)?;
    Ok(finish(
        out,
        "enth.test-host.bursts/1",
        &path,
        &id,
        loaded.as_ref(),
    ))
}

/// Notes for `note-end`: ten notes for eight voices (steals), a zero-length note, notes that end inside a block
/// and the same key twice at once.
const NOTE_END_SCRIPT: &str = "C3@0+0.4, E3@0+0.4, G3@0+0.4, C4@0+0.4, E4@0+0.4, G4@0+0.4, C5@0+0.4, E5@0+0.4, G5@0.05+0.4, C6@0.1+0.4, D4@0.25+0.00001, A4@0.3+0.0003, F4@0.35+0.0007, B3@0.5+0.2, B3@0.5+0.2";

fn cmd_note_end(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &valued(
            &["notes", "blocks", "tail", "sr", "inject", "plugin-id"],
            &STATE_OPTIONS,
        ),
        &["allow-unended"],
    )?;
    let (path, entry, id) = load(&args)?;
    let loaded = load_state(&args, &entry, &id)?;
    let opts = events::NoteEndOptions {
        script: match args.one("notes") {
            Some(n) => notes::load_script(n)?,
            None => NOTE_END_SCRIPT.to_owned(),
        },
        blocks: args.list("blocks", &[1u32, 32, 64, 100, 512])?,
        sample_rate: args.number("sr", 48_000.0)?,
        tail_s: args.number("tail", 2.0)?,
        state: state_bytes(loaded.as_ref()),
        allow_unended: args.options.contains_key("allow-unended"),
        inject: args
            .one("inject")
            .map(events::Inject::parse)
            .transpose()?
            .unwrap_or_default(),
    };
    if opts.blocks.is_empty() || opts.blocks.contains(&0) {
        return Err("--blocks needs block sizes of at least 1".to_owned());
    }
    let out = events::run_note_end(&entry, &id, &opts)?;
    Ok(finish(
        out,
        "enth.test-host.note-end/1",
        &path,
        &id,
        loaded.as_ref(),
    ))
}

fn cmd_process_stats(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &valued(
            &[
                "secs",
                "warmup",
                "block",
                "sr",
                "rt",
                "pin",
                "instances",
                "threads",
                "notes",
                "gate",
                "max-p99-us",
                "max-p999-us",
                "max-callback-p99-pct",
                "inject",
                "plugin-id",
                "swap-state",
                "swap-ms",
                "swap-kind",
                "max-swap-p99-us",
                "max-crossfade-p99-us",
                "flood",
                "flood-ms",
                "ab",
            ],
            &STATE_OPTIONS,
        ),
        &["require-rt", "editor"],
    )?;
    // Before the plugin loads: the A/B harness reads the environment at activation (lane SKR).
    let ab = args
        .one("ab")
        .map(|ms| {
            ms.parse::<u64>()
                .map_err(|_| format!("--ab {ms:?}: milliseconds between A/B flips"))
                .and_then(stats::AbHarness::arm)
        })
        .transpose()?;
    let (path, entry, id) = load(&args)?;
    let loaded = load_state(&args, &entry, &id)?;
    let mut opts = stats::Options::default();
    // A gate row sets the conditions and the budgets; any option given besides overrides it.
    match args.one("gate") {
        None => {}
        Some("g1") => {
            opts.max_p99_us = Some(333.0);
            opts.max_p999_us = Some(333.0);
            opts.require_rt = true;
        }
        Some("g1-multi") => {
            opts.instances = 16;
            opts.threads = 4;
            opts.max_callback_p99_pct = Some(50.0);
            opts.require_rt = true;
        }
        Some(other) => return Err(format!("--gate {other:?}: use g1 or g1-multi")),
    }
    opts.secs = args.number("secs", opts.secs)?;
    opts.warmup_secs = args.number("warmup", opts.warmup_secs)?;
    opts.sample_rate = args.number("sr", opts.sample_rate)?;
    opts.block = args.number("block", opts.block)?;
    if let Some(rt) = args.one("rt") {
        opts.rt = if rt == "none" {
            None
        } else {
            Some(
                rt.parse()
                    .map_err(|_| format!("--rt {rt:?}: a priority number or `none`"))?,
            )
        };
    }
    opts.require_rt |= args.options.contains_key("require-rt");
    opts.pin = args.list("pin", &[])?;
    opts.instances = args.number("instances", opts.instances)?;
    opts.threads = args.number("threads", opts.threads)?;
    opts.editor = args.options.contains_key("editor");
    if let Some(n) = args.one("notes") {
        opts.notes = notes::load_script(n)?;
    }
    opts.state = state_bytes(loaded.as_ref());
    opts.inject = args
        .one("inject")
        .map(enth_test_host::render::BusyFault::parse)
        .transpose()?;
    // Spiky blocks (lane RT): structural edits and event floods.
    if let Some(file) = args.one("swap-state") {
        let base = opts
            .state
            .clone()
            .ok_or("--swap-state needs --state FILE: the run alternates between the two")?;
        let bytes = std::fs::read(file).map_err(|e| format!("--swap-state {file}: {e}"))?;
        let format = state::Format::parse(args.one("state-format").unwrap_or("auto"))?;
        let field = args.one("state-field").unwrap_or(state::DEFAULT_FIELD);
        let alt = state::prepare(bytes, format, field, &plugin_version(&entry, &id))
            .map_err(|e| format!("--swap-state {file}: {e}"))?;
        opts.swap = Some(enth_test_host::spiky::Swap {
            base_state: base,
            alt_state: alt.bytes,
            every_ms: args.number("swap-ms", 250u64)?,
            kind: enth_test_host::spiky::SwapKind::parse(
                args.one("swap-kind").unwrap_or("instant"),
            )?,
        });
    } else if [
        "swap-ms",
        "swap-kind",
        "max-swap-p99-us",
        "max-crossfade-p99-us",
    ]
    .iter()
    .any(|o| args.one(o).is_some())
    {
        return Err(
            "--swap-ms, --swap-kind and the swap budgets need --swap-state FILE".to_owned(),
        );
    }
    if let Some(events) = args.one("flood") {
        let events: usize = events
            .parse()
            .map_err(|_| format!("--flood {events:?}: a number of note events"))?;
        if events < 2 {
            return Err(
                "--flood needs at least 2 note events (one note-on and its note-off)".to_owned(),
            );
        }
        opts.flood = Some(enth_test_host::spiky::Flood {
            events,
            every_ms: args.number("flood-ms", 250u64)?,
        });
    } else if args.one("flood-ms").is_some() {
        return Err("--flood-ms needs --flood N".to_owned());
    }
    for (name, slot) in [
        ("max-p99-us", &mut opts.max_p99_us),
        ("max-p999-us", &mut opts.max_p999_us),
        ("max-callback-p99-pct", &mut opts.max_callback_p99_pct),
        ("max-swap-p99-us", &mut opts.max_swap_p99_us),
        ("max-crossfade-p99-us", &mut opts.max_crossfade_p99_us),
    ] {
        if let Some(text) = args.one(name) {
            *slot = Some(
                text.parse()
                    .map_err(|_| format!("--{name}: {text:?} is not a number"))?,
            );
        }
    }
    if opts.block == 0 {
        return Err("--block must be at least 1".to_owned());
    }
    opts.ab = ab;
    let (mut report, _) = stats::run(&entry, &id, &opts)?;
    report["gate"] = json!(args.one("gate"));
    Ok(finish(
        report,
        "enth.test-host.process-stats/1",
        &path,
        &id,
        loaded.as_ref(),
    ))
}

fn cmd_latency(rest: &[String]) -> Outcome {
    let args = Args::parse(
        rest,
        &valued(
            &[
                "param",
                "to",
                "from",
                "notes",
                "at",
                "offsets",
                "after",
                "block",
                "sr",
                "threshold-db",
                "max-onset-ms",
                "max-audible-ms",
                "inject",
                "plugin-id",
            ],
            &STATE_OPTIONS,
        ),
        &[],
    )?;
    let (path, entry, id) = load(&args)?;
    let loaded = load_state(&args, &entry, &id)?;
    let optional = |name: &str| -> Result<Option<f64>, String> {
        args.one(name)
            .map(|t| {
                t.parse()
                    .map_err(|_| format!("--{name}: {t:?} is not a number"))
            })
            .transpose()
    };
    let inject_delay_blocks = match args.one("inject") {
        None => 0,
        Some(text) => text
            .strip_prefix("delay-blocks=")
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("--inject {text:?}: latency knows `delay-blocks=K`"))?,
    };
    let opts = latency::Options {
        param: args.one("param").ok_or("--param is required")?.to_owned(),
        value: optional("to")?.ok_or("--to VALUE is required")?,
        from: optional("from")?,
        notes: match args.one("notes") {
            Some(n) => notes::load_script(n)?,
            None => "C4@0+3600:0.8, G4@0+3600:0.8".to_owned(),
        },
        sample_rate: args.number("sr", 48_000.0)?,
        block: args.number("block", 64)?,
        at_s: args.number("at", 0.5)?,
        offsets: args.list("offsets", &[0u32, 17, 63])?,
        after_s: args.number("after", 0.5)?,
        threshold_db: args.number("threshold-db", -60.0)?,
        state: state_bytes(loaded.as_ref()),
        inject_delay_blocks,
        max_onset_ms: optional("max-onset-ms")?,
        max_audible_ms: optional("max-audible-ms")?,
    };
    if opts.block == 0 || opts.offsets.is_empty() {
        return Err("--block must be at least 1 and --offsets needs a position".to_owned());
    }
    let out = latency::run(&entry, &id, &opts)?;
    Ok(finish(
        out,
        "enth.test-host.latency/1",
        &path,
        &id,
        loaded.as_ref(),
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
        &["system-malloc", "system-thp"],
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
            // Before the plugin is loaded and before any thread starts (lane LK; editor.rs, "repeatable").
            let malloc = if args.options.contains_key("system-malloc") {
                json!("system (--system-malloc)")
            } else {
                procfs::deterministic_malloc()
            };
            let thp = thp_setting(&args);
            let (_, entry, id) = load(&args)?;
            let mut run = editor::run_cycles(&entry, &id, &opts)?;
            run["schema"] = json!("enth.test-host.editor-run/1");
            run["malloc"] = malloc;
            run["thp"] = thp;
            Ok((run, true))
        }
        "both" => editor_both(&args),
        other => Err(format!("--mode {other:?}: use both, editor or baseline")),
    }
}

/// Transparent huge pages off in a measurement process unless `--system-thp` (lane NTF;
/// `procfs::no_transparent_huge_pages`).
fn thp_setting(args: &Args) -> Value {
    if args.options.contains_key("system-thp") {
        json!("system (--system-thp)")
    } else {
        procfs::no_transparent_huge_pages()
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
            for flag in ["system-malloc", "system-thp"] {
                if args.options.contains_key(flag) {
                    cmd.arg(format!("--{flag}"));
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
