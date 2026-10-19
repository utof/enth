//! `cargo xtask reaper-test`: a local gate (not CI) that drives a real host.
//!
//! It bundles `spike-shell`, then runs native REAPER headless — always inside `xvfb-run`, with
//! Dummy Audio and a throwaway copy of the REAPER install — through Lua scripts in
//! `xtask/reaper/`. Each script appends `check` and `data` lines to a log; this module runs the
//! scripts, reads the logs, and makes the checks that need the saved state or rendered audio.
//!
//! Covers must-haves (a) 72 params with stable ids and automation, (c) the patch blob surviving
//! a project save and reload in a new process, and (e) VST3 loading, for both formats.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::bundle;

const PACKAGE: &str = "spike-shell";
/// How REAPER lists the plugin; `TrackFX_AddByName` matches this as a substring.
const PLUGIN_NAME: &str = "enth spike-shell";
const DEFAULT_REAPER: &str = "apps/reaper-7.82/reaper_linux_x86_64/REAPER/reaper";
/// Hard limit for one REAPER launch; a hung script (e.g. a modal error dialog) is killed.
const LAUNCH_TIMEOUT_SECS: u32 = 90;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Format {
    Clap,
    Vst3,
}

impl Format {
    fn tag(self) -> &'static str {
        match self {
            Format::Clap => "clap",
            Format::Vst3 => "vst3",
        }
    }

    fn reaper_name(self) -> String {
        match self {
            Format::Clap => format!("CLAPi: {PLUGIN_NAME}"),
            Format::Vst3 => format!("VST3i: {PLUGIN_NAME}"),
        }
    }
}

struct Options {
    release: bool,
    build: bool,
    formats: Vec<Format>,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        release: false,
        build: true,
        formats: Vec::new(),
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--release" => options.release = true,
            "--no-build" => options.build = false,
            "--format" => match args.next().map(String::as_str) {
                Some("clap") => options.formats.push(Format::Clap),
                Some("vst3") => options.formats.push(Format::Vst3),
                other => return Err(format!("--format takes clap or vst3, got {other:?}")),
            },
            other => return Err(format!("unknown option {other}")),
        }
    }
    if options.formats.is_empty() {
        options.formats = vec![Format::Clap, Format::Vst3];
    }
    options.formats.sort();
    options.formats.dedup();
    Ok(options)
}

/// One line of the final report.
struct Row {
    scope: String,
    name: String,
    ok: bool,
    detail: String,
}

#[derive(Default)]
struct Report {
    rows: Vec<Row>,
}

impl Report {
    fn add(&mut self, scope: &str, name: &str, ok: bool, detail: impl Into<String>) {
        self.rows.push(Row {
            scope: scope.to_owned(),
            name: name.to_owned(),
            ok,
            detail: detail.into(),
        });
    }

    fn failures(&self) -> usize {
        self.rows.iter().filter(|r| !r.ok).count()
    }
}

/// Results of one Lua script: its `data` lines, with the `check` lines already in the report.
struct Log {
    data: BTreeMap<String, String>,
}

struct Sandbox {
    reaper: PathBuf,
    home: PathBuf,
    out: PathBuf,
    lua: PathBuf,
}

pub fn reaper_test(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    let meta = crate::metadata()?;
    let reaper = reaper_path()?;
    require_tool(
        "xvfb-run",
        "REAPER must never open a window on the real display",
    )?;
    require_tool("timeout", "needed to stop a hung REAPER")?;

    if options.build {
        let mut build_args = vec![PACKAGE.to_owned(), "--locked".to_owned()];
        if options.release {
            build_args.push("--release".to_owned());
        }
        bundle::bundle(&build_args)?;
    }
    let bundled = bundle::bundled_dir()?;
    for format in &options.formats {
        let artefact = bundled.join(format!("{PACKAGE}.{}", format.tag()));
        if !artefact.exists() {
            return Err(format!(
                "{} is missing; run without --no-build",
                artefact.display()
            ));
        }
    }

    let work = meta.target_directory.join("reaper-test");
    let sandbox = Sandbox {
        reaper,
        home: work.join("home"),
        out: work.join("out"),
        lua: meta.workspace_root.join("xtask/reaper"),
    };
    prepare_sandbox(&sandbox, &bundled)?;
    println!("reaper-test: REAPER {}", sandbox.reaper.display());
    println!("reaper-test: results in {}", sandbox.out.display());

    let mut report = Report::default();
    let mut idents: BTreeMap<Format, String> = BTreeMap::new();
    for &format in &options.formats {
        let tag = format.tag();
        let started = Instant::now();

        // (a) and (c): parameters, ids, automation lane, state; then reload in a new process.
        for phase in ["create", "reload"] {
            let log = run_script(&sandbox, &mut report, format, "state", phase, &[]);
            if phase == "create" {
                if let Some(ids) = log.data.get("idents-create") {
                    idents.insert(format, ids.clone());
                }
            } else {
                let created = idents.get(&format).cloned();
                let reloaded = log.data.get("idents-reload").cloned();
                report.add(
                    &format!("{tag}/state-reload"),
                    "param-ids-restored",
                    created.is_some() && created == reloaded,
                    "the 72 ids are identical after reload in a new process",
                );
            }
        }
        check_saved_state(&sandbox, &mut report, format);

        // (a): automation reaches the plugin, heard in an offline render.
        run_script(&sandbox, &mut report, format, "automation", "build", &[]);
        let project = sandbox.out.join(format!("auto-{tag}.rpp"));
        if project.exists() {
            run_reaper(
                &sandbox,
                &format!("{tag}-render"),
                &[
                    "-nosplash",
                    "-newinst",
                    "-renderproject",
                    &project.display().to_string(),
                ],
                &[],
            )
            .map_err(|e| format!("{tag} render: {e}"))?;
            check_render(&sandbox, &mut report, format);
        }

        // The placeholder editor opens and closes under Xvfb.
        run_script(&sandbox, &mut report, format, "editor", "smoke", &[]);
        println!(
            "reaper-test: {tag} finished in {:.0} s",
            started.elapsed().as_secs_f64()
        );
    }

    if let [first, second, ..] = options.formats.as_slice() {
        report.add(
            "clap+vst3",
            "param-ids-same-in-both-formats",
            idents.contains_key(first) && idents.get(first) == idents.get(second),
            "CLAP and VST3 report the same 72 ids",
        );
    }

    print_report(&report);
    if report.failures() == 0 {
        println!("reaper-test: ok ({} checks)", report.rows.len());
        Ok(())
    } else {
        Err(format!(
            "reaper-test: {} of {} checks failed (logs and stdout in {})",
            report.failures(),
            report.rows.len(),
            sandbox.out.display()
        ))
    }
}

fn print_report(report: &Report) {
    for row in &report.rows {
        println!(
            "  {:<4} {:<22} {:<28} {}",
            if row.ok { "ok" } else { "FAIL" },
            row.scope,
            row.name,
            row.detail
        );
    }
}

fn reaper_path() -> Result<PathBuf, String> {
    let path = if let Some(path) = std::env::var_os("ENTH_REAPER") {
        PathBuf::from(path)
    } else {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        PathBuf::from(home).join(DEFAULT_REAPER)
    };
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "REAPER not found at {}: install native REAPER (portable layout) or set ENTH_REAPER to its `reaper` binary",
            path.display()
        ))
    }
}

fn require_tool(name: &str, why: &str) -> Result<(), String> {
    let found = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()));
    if found {
        Ok(())
    } else {
        Err(format!("`{name}` not found in PATH ({why})"))
    }
}

/// Makes `home` a private copy of the REAPER install (so the real one and its config are never
/// touched), writes a fresh `reaper.ini` and empties `out`.
fn prepare_sandbox(sb: &Sandbox, bundled: &Path) -> Result<(), String> {
    let install = sb.reaper.parent().ok_or("REAPER path has no parent")?;
    let exe_meta =
        std::fs::metadata(&sb.reaper).map_err(|e| format!("{}: {e}", sb.reaper.display()))?;
    let stamp = format!(
        "{}|{}|{:?}",
        install.display(),
        exe_meta.len(),
        exe_meta.modified().ok()
    );
    let stamp_file = sb.home.join(".enth-source");
    if std::fs::read_to_string(&stamp_file).ok().as_deref() != Some(stamp.as_str()) {
        println!(
            "reaper-test: copying {} to {}",
            install.display(),
            sb.home.display()
        );
        if sb.home.exists() {
            std::fs::remove_dir_all(&sb.home).map_err(|e| format!("{}: {e}", sb.home.display()))?;
        }
        bundle::copy_tree(install, &sb.home)?;
        std::fs::write(&stamp_file, stamp).map_err(|e| format!("{}: {e}", stamp_file.display()))?;
    }
    // Fresh config and plugin caches on every run, so REAPER rescans exactly our bundles.
    for stale in [
        "reaper.ini",
        "reaper-vstplugins64.ini",
        "reaper-clap-linux-x86_64.ini",
        "reaper-recentfx.ini",
    ] {
        let _ = std::fs::remove_file(sb.home.join(stale));
    }
    let ini = format!(
        "[REAPER]\n\
         clap_path_linux-x86_64={dir}\n\
         vstpath={dir}\n\
         linux_audio_mode=2\n\
         linux_audio_srate=48000\n\
         renderclosewhendone=4\n",
        dir = bundled.display()
    );
    std::fs::write(sb.home.join("reaper.ini"), ini).map_err(|e| format!("reaper.ini: {e}"))?;
    if sb.out.exists() {
        std::fs::remove_dir_all(&sb.out).map_err(|e| format!("{}: {e}", sb.out.display()))?;
    }
    std::fs::create_dir_all(&sb.out).map_err(|e| format!("{}: {e}", sb.out.display()))
}

/// Launches REAPER under Xvfb with a Lua script, then reads the script's log into the report.
fn run_script(
    sb: &Sandbox,
    report: &mut Report,
    format: Format,
    script: &str,
    phase: &str,
    extra_env: &[(&str, &str)],
) -> Log {
    let tag = format.tag();
    let label = format!("{tag}-{script}-{phase}");
    let log_name = format!("{label}.log");
    let lua = sb.lua.join(format!("{script}.lua"));
    let fx = format.reaper_name();
    let empty = sb.lua.join("empty.rpp");
    let mut env: Vec<(&str, String)> = vec![
        ("ENTH_OUT", sb.out.display().to_string()),
        ("ENTH_LUA_DIR", sb.lua.display().to_string()),
        ("ENTH_EMPTY_PROJECT", empty.display().to_string()),
        ("ENTH_LOG", log_name.clone()),
        ("ENTH_FX", fx),
        ("ENTH_TAG", tag.to_owned()),
        ("ENTH_PHASE", phase.to_owned()),
    ];
    env.extend(extra_env.iter().map(|&(k, v)| (k, v.to_owned())));
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let scope = format!("{tag}/{script}-{phase}");
    let launched = run_reaper(
        sb,
        &label,
        &["-nosplash", "-newinst", &lua.display().to_string()],
        &env_refs,
    );
    if let Err(e) = &launched {
        report.add(&scope, "reaper-run", false, e.clone());
    }
    let mut data = BTreeMap::new();
    let text = std::fs::read_to_string(sb.out.join(&log_name)).unwrap_or_default();
    for line in text.lines() {
        let fields: Vec<&str> = line.splitn(4, '\t').collect();
        match fields.as_slice() {
            ["check", name, status, detail] => report.add(&scope, name, *status == "ok", *detail),
            ["check", name, status] => report.add(&scope, name, *status == "ok", ""),
            ["data", key, value] => {
                data.insert((*key).to_owned(), (*value).to_owned());
            }
            _ => {}
        }
    }
    if launched.is_ok() && !data.contains_key("finished") {
        report.add(
            &scope,
            "script-finished",
            false,
            format!("no result from the script; see {label}.stdout.txt"),
        );
    }
    Log { data }
}

/// Runs the sandbox copy of REAPER under `xvfb-run` and waits for it. Never uses the real display.
fn run_reaper(
    sb: &Sandbox,
    label: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<(), String> {
    let stdout = std::fs::File::create(sb.out.join(format!("{label}.stdout.txt")))
        .map_err(|e| format!("{label}: {e}"))?;
    let stderr = stdout.try_clone().map_err(|e| format!("{label}: {e}"))?;
    let mut command = Command::new("xvfb-run");
    command
        .args(["-a", "-s", "-screen 0 1920x1080x24"])
        .args(["timeout", "-k", "5", &LAUNCH_TIMEOUT_SECS.to_string()])
        .arg(sb.home.join(sb.reaper.file_name().unwrap_or_default()))
        .args(args)
        .current_dir(&sb.home)
        // xvfb-run sets DISPLAY for its child; make sure nothing can fall back to the real one.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("GDK_BACKEND", "x11")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    for &(key, value) in env {
        command.env(key, value);
    }
    let status = command
        .status()
        .map_err(|e| format!("cannot run xvfb-run: {e}"))?;
    match status.code() {
        Some(124 | 137) => Err(format!(
            "REAPER was killed after {LAUNCH_TIMEOUT_SECS} s (a script error or dialog may be blocking it); see {label}.stdout.txt"
        )),
        // REAPER can exit non-zero after a scripted quit; the script's log decides.
        _ => Ok(()),
    }
}

// ---- saved state ---------------------------------------------------------------------------

/// Decodes the plugin state REAPER saved before and after the reload and compares the values and
/// the patch blob (which carries a nonce stamped by the plugin on first activation).
fn check_saved_state(sb: &Sandbox, report: &mut Report, format: Format) {
    let scope = format!("{}/state", format.tag());
    let load = |phase: &str| -> Result<serde_json::Value, String> {
        let path = sb.out.join(format!("chunk-{}-{phase}.b64", format.tag()));
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        nice_plug_state(&base64_decode(&text))
    };
    let (created, reloaded) = match (load("create"), load("reload")) {
        (Ok(a), Ok(b)) => (a, b),
        (a, b) => {
            let why = a.err().or(b.err()).unwrap_or_default();
            report.add(&scope, "state-decoded", false, why);
            return;
        }
    };
    let param = |state: &serde_json::Value, id: &str| state["params"][id]["f32"].as_f64();
    for (state, name) in [(&created, "saved"), (&reloaded, "reloaded")] {
        let (m, s) = (param(state, "macro_1"), param(state, "slot_1"));
        report.add(
            &scope,
            &format!("{name}-state-values"),
            m.is_some_and(|m| (m - 0.7).abs() < 0.002)
                && s.is_some_and(|s| (s - 0.3).abs() < 0.002),
            format!("macro_1 = {m:?}, slot_1 = {s:?}"),
        );
    }
    // The blob is stored as a string holding JSON; the nonce proves it came from the saved state.
    let nonce = |state: &serde_json::Value| -> Option<String> {
        let blob: serde_json::Value =
            serde_json::from_str(state["fields"]["patch"].as_str()?).ok()?;
        blob["nonce"].as_str().map(str::to_owned)
    };
    let (before, after) = (nonce(&created), nonce(&reloaded));
    report.add(
        &scope,
        "patch-blob-restored",
        before.is_some() && before == after,
        format!(
            "nonce {} before and after reload",
            before.as_deref().unwrap_or("missing")
        ),
    );
}

/// Finds nice-plug's JSON state (`{"version":…`) inside the host's chunk, which wraps it in a
/// format-specific header, and parses the first JSON value there.
fn nice_plug_state(chunk: &[u8]) -> Result<serde_json::Value, String> {
    let start = chunk
        .windows(10)
        .position(|w| w == br#"{"version""#)
        .ok_or("no nice-plug state JSON in the chunk")?;
    serde_json::Deserializer::from_slice(&chunk[start..])
        .into_iter::<serde_json::Value>()
        .next()
        .ok_or("empty state")?
        .map_err(|e| format!("bad state JSON: {e}"))
}

fn base64_decode(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut bits, mut count) = (0u32, 0);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

// ---- rendered audio ------------------------------------------------------------------------

/// Reads the offline render and checks that automating Macro 1 and Slot 1 from 0 to 1 made the
/// sound louder and brighter, with no NaNs or silence. The notes are short and repeated because
/// REAPER restarts the plugin shortly after an offline render starts (see `automation.lua`), so
/// the windows below may each catch part of a restart gap; the measures tolerate that.
fn check_render(sb: &Sandbox, report: &mut Report, format: Format) {
    let scope = format!("{}/automation", format.tag());
    let path = sb.out.join(format!("render-{}.wav", format.tag()));
    let wav = match std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|b| read_wav(&b))
    {
        Ok(wav) => wav,
        Err(e) => {
            report.add(
                &scope,
                "render-readable",
                false,
                format!("{}: {e}", path.display()),
            );
            return;
        }
    };
    let window = |from: f64, to: f64| {
        let rate = f64::from(wav.rate);
        &wav.left[(from * rate) as usize..((to * rate) as usize).min(wav.left.len())]
    };
    if wav.left.len() < (wav.rate as usize * 2) {
        report.add(
            &scope,
            "render-length",
            false,
            format!("{} frames", wav.left.len()),
        );
        return;
    }
    let (early, late) = (window(0.25, 0.55), window(1.7, 2.0));
    let (rms_early, rms_late) = (rms(early), rms(late));
    let (hf_early, hf_late) = (roughness(early), roughness(late));
    report.add(
        &scope,
        "render-finite",
        wav.left.iter().all(|s| s.is_finite()),
        format!("{} frames at {} Hz", wav.left.len(), wav.rate),
    );
    report.add(
        &scope,
        "macro-1-gets-louder",
        rms_early > 0.0 && rms_late > 3.0 * rms_early,
        format!("rms {rms_early:.4} at 0.25-0.55 s -> {rms_late:.4} at 1.7-2.0 s"),
    );
    report.add(
        &scope,
        "slot-1-gets-brighter",
        hf_early > 0.0 && hf_late > 2.0 * hf_early,
        format!("brightness {hf_early:.4} at 0.25-0.55 s -> {hf_late:.4} at 1.7-2.0 s"),
    );
}

struct Wav {
    rate: u32,
    left: Vec<f32>,
}

fn read_wav(bytes: &[u8]) -> Result<Wav, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let le16 = |b: &[u8]| usize::from(u16::from_le_bytes([b[0], b[1]]));
    let le32 = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let (mut format, mut channels, mut rate, mut bits) = (0, 0, 0, 0);
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = le32(&bytes[at + 4..]) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match &bytes[at..at + 4] {
            b"fmt " if body.len() >= 16 => {
                (format, channels, rate, bits) = (
                    le16(body),
                    le16(&body[2..]),
                    le32(&body[4..]),
                    le16(&body[14..]),
                );
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    let data = data.ok_or("no data chunk")?;
    let step = bits / 8 * channels;
    if channels == 0 || step == 0 {
        return Err("no fmt chunk".into());
    }
    let sample = |b: &[u8]| -> Result<f32, String> {
        Ok(match (format, bits) {
            (1, 16) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2.0_f32.powi(31),
            (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            _ => return Err(format!("unsupported WAV format {format} with {bits} bits")),
        })
    };
    let left = data
        .chunks_exact(step)
        .map(sample)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Wav { rate, left })
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// RMS of the first difference relative to the RMS: grows with high-frequency content.
fn roughness(x: &[f32]) -> f64 {
    let diff: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let level = rms(x);
    if level == 0.0 {
        0.0
    } else {
        rms(&diff) / level
    }
}
