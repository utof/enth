//! `cargo xtask reaper-test`: a local gate (not CI) that drives a real host.
//!
//! It bundles the package (`--package`, default: see `plugin::default_package`; or takes prebuilt bundles from
//! `--bundle-dir`), then runs native REAPER headless — always inside `xvfb-run`, with Dummy Audio and a throwaway copy
//! of the REAPER install — through Lua scripts in `xtask/reaper/`. Each script appends `check` and `data` lines to a
//! log; this module runs the scripts, reads the logs, and makes the checks that need the saved state or rendered
//! audio (`checks.rs`).
//!
//! The scenarios, each on CLAP and on VST3 (`--scenario <name>` runs some):
//!
//! | scenario | what it proves |
//! | --- | --- |
//! | `state` (`state-create`, `state-reload`) | 72 parameters with stable ids and names, automation lanes, a patch loaded as state and the host's values saved with it, all back after a reload in a new process |
//! | `patch-roundtrip` | a rich non-default patch comes back byte for byte after a save, a reload, a second save and a second reload |
//! | `slot-automation` | automation on slots 1 and 64 makes the rendered sound brighter and louder, as the patch maps them |
//! | `macro-automation` | automation on macros 1 and 8 makes it louder and brighter |
//! | `undo-state-load` | REAPER's undo and redo across plugin state loads restore the patch and the parameters |
//! | `two-instances` | two instances keep their own patch and values, through a reload and a deleted neighbour |
//! | `editor-smoke` | the editor opens, draws and closes five times, under Xvfb |
//! | `dirty-flag` | reads `IsProjectDirty` after an editor-only patch edit (the open check), with a control |
//! | `cross-format` | CLAP and VST3 agree: parameter ids, the saved patch, the rendered audio |
//!
//! The spike shell (`--package spike-shell`) keeps its older three: `state`, `automation` and `editor-smoke`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::{bundle, plugin};

mod audio;
mod checks;
mod chunk;

const DEFAULT_REAPER: &str = "apps/reaper-7.82/reaper_linux_x86_64/REAPER/reaper";
/// Hard limit for one REAPER launch; a hung script (e.g. a modal error dialog) is killed.
const LAUNCH_TIMEOUT_SECS: u32 = 90;

/// Whether REAPER is expected to mark the project dirty after an editor-only patch edit. nice-plug never calls CLAP
/// `mark_dirty`, so today it does not (`reaper-test` measured it: see `docs/plugin.md`), and the `dirty-flag`
/// scenario reports that as a known gap. A sixth fork patch adds the marking; flip this to `true` then, and a clean
/// project after an edit fails the scenario.
const DIRTY_MARKING_EXPECTED: bool = false;

/// Whether CLAP ignoring a slot automation lane that never moves, in REAPER's offline render, is a known gap.
/// `slot-automation` measures it (a lane held at 0.25 on osc1's level) and reports `info` while this is `true`. It was
/// (lane RH found it): REAPER loads the project's state while the plugin is active and sends the lane's value once,
/// in the first block, and the plugin's re-baseline after a load adopted that value with the restored ones. Lane PL
/// fixed it (the plugin adopts only the values the load restored), so the CLAP result is a check.
const CLAP_FLAT_LANE_IS_KNOWN_GAP: bool = false;

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

    /// How REAPER lists the plugin in this format; `TrackFX_AddByName` matches it as a substring.
    fn reaper_name(self, plugin_name: &str) -> String {
        match self {
            Format::Clap => format!("CLAPi: {plugin_name}"),
            Format::Vst3 => format!("VST3i: {plugin_name}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Scenario {
    State,
    PatchRoundtrip,
    SlotAutomation,
    MacroAutomation,
    UndoStateLoad,
    TwoInstances,
    EditorSmoke,
    DirtyFlag,
    /// The spike shell's older automation scenario (macro 1 and slot 1 ramped together).
    Automation,
    CrossFormat,
}

impl Scenario {
    const PRODUCT: [Scenario; 9] = [
        Scenario::State,
        Scenario::PatchRoundtrip,
        Scenario::SlotAutomation,
        Scenario::MacroAutomation,
        Scenario::UndoStateLoad,
        Scenario::TwoInstances,
        Scenario::EditorSmoke,
        Scenario::DirtyFlag,
        Scenario::CrossFormat,
    ];
    const SPIKE: [Scenario; 4] = [
        Scenario::State,
        Scenario::Automation,
        Scenario::EditorSmoke,
        Scenario::CrossFormat,
    ];

    fn name(self) -> &'static str {
        match self {
            Scenario::State => "state",
            Scenario::PatchRoundtrip => "patch-roundtrip",
            Scenario::SlotAutomation => "slot-automation",
            Scenario::MacroAutomation => "macro-automation",
            Scenario::UndoStateLoad => "undo-state-load",
            Scenario::TwoInstances => "two-instances",
            Scenario::EditorSmoke => "editor-smoke",
            Scenario::DirtyFlag => "dirty-flag",
            Scenario::Automation => "automation",
            Scenario::CrossFormat => "cross-format",
        }
    }

    fn parse(text: &str) -> Option<Scenario> {
        let all = Scenario::PRODUCT.iter().chain(&Scenario::SPIKE);
        match text {
            "state-create" | "state-reload" => Some(Scenario::State),
            _ => all.copied().find(|s| s.name() == text),
        }
    }
}

struct Options {
    package: String,
    /// How REAPER lists the plugin (`--plugin-name`, else `plugin::reaper_plugin_name`).
    plugin_name: String,
    release: bool,
    build: bool,
    /// A directory that already holds `<package>.clap` and `<package>.vst3` (`--bundle-dir`, else `ENTH_BUNDLE_DIR`):
    /// the coordinator's release-bundle cache, so a harness lane never rebuilds the plugin. Implies `--no-build`.
    bundle_dir: Option<PathBuf>,
    formats: Vec<Format>,
    /// `--scenario`, any number; empty runs every scenario of the package.
    scenarios: Vec<Scenario>,
}

fn parse_options(args: &[String], root: &Path) -> Result<Options, String> {
    let (package, rest) = plugin::one_package(plugin::split_packages(args, false)?, root)?;
    let mut options = Options {
        plugin_name: plugin::reaper_plugin_name(&package),
        package,
        release: false,
        build: true,
        bundle_dir: std::env::var_os("ENTH_BUNDLE_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from),
        formats: Vec::new(),
        scenarios: Vec::new(),
    };
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--release" => options.release = true,
            "--no-build" => options.build = false,
            "--bundle-dir" => {
                options.bundle_dir = Some(PathBuf::from(
                    args.next()
                        .ok_or("--bundle-dir needs a directory with the prebuilt bundles")?,
                ));
            }
            "--format" => match args.next().map(String::as_str) {
                Some("clap") => options.formats.push(Format::Clap),
                Some("vst3") => options.formats.push(Format::Vst3),
                other => return Err(format!("--format takes clap or vst3, got {other:?}")),
            },
            "--scenario" => {
                let name = args.next().ok_or("--scenario needs a scenario name")?;
                options.scenarios.push(Scenario::parse(name).ok_or_else(|| {
                    let names: Vec<&str> = Scenario::PRODUCT.iter().map(|s| s.name()).collect();
                    format!(
                        "unknown scenario {name:?} (the product's: {}; the spike shell's older `automation`)",
                        names.join(", ")
                    )
                })?);
            }
            "--plugin-name" => {
                options.plugin_name = args
                    .next()
                    .cloned()
                    .ok_or("--plugin-name needs a name (how REAPER lists the plugin)")?;
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if options.formats.is_empty() {
        options.formats = vec![Format::Clap, Format::Vst3];
    }
    options.formats.sort();
    options.formats.dedup();
    options.scenarios.sort();
    options.scenarios.dedup();
    Ok(options)
}

/// How a row of the report counts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Fail,
    /// A measurement or a known gap: printed, never a failure.
    Info,
}

/// One line of the final report.
struct Row {
    scope: String,
    name: String,
    status: Status,
    detail: String,
}

#[derive(Default)]
struct Report {
    rows: Vec<Row>,
}

impl Report {
    fn add(&mut self, scope: &str, name: &str, ok: bool, detail: impl Into<String>) {
        let status = if ok { Status::Ok } else { Status::Fail };
        self.push(scope, name, status, detail);
    }

    fn info(&mut self, scope: &str, name: &str, detail: impl Into<String>) {
        self.push(scope, name, Status::Info, detail);
    }

    fn push(&mut self, scope: &str, name: &str, status: Status, detail: impl Into<String>) {
        self.rows.push(Row {
            scope: scope.to_owned(),
            name: name.to_owned(),
            status,
            detail: detail.into(),
        });
    }

    fn failures(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.status == Status::Fail)
            .count()
    }

    fn checks(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.status != Status::Info)
            .count()
    }

    /// (passed, checked) for the rows of `scope`.
    fn count(&self, scope: &str) -> (usize, usize) {
        let rows = self
            .rows
            .iter()
            .filter(|r| r.scope == scope && r.status != Status::Info);
        let total = rows.clone().count();
        (rows.filter(|r| r.status == Status::Ok).count(), total)
    }
}

/// Results of one Lua script: its `data` lines, with the `check` lines already in the report.
struct Log {
    data: BTreeMap<String, String>,
}

struct Sandbox {
    /// How REAPER lists the plugin (the scripts add it with `TrackFX_AddByName`).
    plugin_name: String,
    reaper: PathBuf,
    home: PathBuf,
    out: PathBuf,
    lua: PathBuf,
}

/// The patches the product's scenarios load, written once per format as state chunks.
struct Chunks {
    state: Option<String>,
    roundtrip: Option<String>,
    slot: Option<String>,
    macros: Option<String>,
    dirty_control: Option<String>,
}

/// The product plugin's patch for `state-create` (lane SK, F2b): a non-default patch (a label, a route, a named
/// macro, slots 1 and 64) loaded as state, so the saved and the reloaded project can be told from a fresh instance.
const PRODUCT_STATE_PATCH: &str = include_str!("../reaper/enth-state.patch.json");
/// `patch-roundtrip`, `undo-state-load` and `two-instances`: a patch with every module kind, routes, named macros and
/// slots, in the plugin's own serialisation (it saves exactly `Patch::to_json()`, so the saved patch must equal it).
const PRODUCT_ROUNDTRIP_PATCH: &str = include_str!("../reaper/enth-roundtrip.patch.json");
/// `slot-automation`: slot 1 is the cutoff, slot 64 osc1's level.
const PRODUCT_SLOT_PATCH: &str = include_str!("../reaper/enth-slot-automation.patch.json");
/// `macro-automation`: macro 1 raises osc1's level, macro 8 opens the filter.
const PRODUCT_MACRO_PATCH: &str = include_str!("../reaper/enth-macro-automation.patch.json");
/// `dirty-flag` control: every knob of the first card is bound to a host slot.
const PRODUCT_DIRTY_CONTROL_PATCH: &str = include_str!("../reaper/enth-dirty-control.patch.json");

impl Chunks {
    fn write(out: &Path, format: Format, product: bool) -> Result<Chunks, String> {
        let make = |name: &str, patch: &str| -> Result<Option<String>, String> {
            if product {
                write_state_chunk(out, name, format, patch).map(Some)
            } else {
                Ok(None)
            }
        };
        Ok(Chunks {
            state: make("state", PRODUCT_STATE_PATCH)?,
            roundtrip: make("roundtrip", PRODUCT_ROUNDTRIP_PATCH)?,
            slot: make("slot", PRODUCT_SLOT_PATCH)?,
            macros: make("macro", PRODUCT_MACRO_PATCH)?,
            dirty_control: make("dirty-control", PRODUCT_DIRTY_CONTROL_PATCH)?,
        })
    }
}

use chunk::write_state_chunk;

/// Environment for a script that loads a chunk (`ENTH_STATE_CHUNK_FILE`, and `ENTH_STATE_CHUNK_FILE_B` for a second).
fn chunk_env(first: Option<&String>, second: Option<&String>) -> Vec<(&'static str, String)> {
    let mut env = Vec::new();
    if let Some(chunk) = first {
        env.push(("ENTH_STATE_CHUNK_FILE", chunk.clone()));
    }
    if let Some(chunk) = second {
        env.push(("ENTH_STATE_CHUNK_FILE_B", chunk.clone()));
    }
    env
}

/// What the scenarios leave for the checks that compare the two formats.
#[derive(Default)]
struct Shared {
    idents: BTreeMap<Format, String>,
    /// `patch-roundtrip`'s first save of the patch text, per format.
    patch_text: BTreeMap<Format, String>,
}

/// Seconds per scenario and format, for the summary table.
#[derive(Default)]
struct Timing {
    seconds: BTreeMap<(String, Format), f64>,
    order: Vec<String>,
}

impl Timing {
    fn add(&mut self, name: &str, format: Format, seconds: f64) {
        if !self.order.iter().any(|n| n == name) {
            self.order.push(name.to_owned());
        }
        *self.seconds.entry((name.to_owned(), format)).or_default() += seconds;
    }
}

pub fn reaper_test(args: &[String]) -> Result<(), String> {
    let meta = crate::metadata()?;
    let options = parse_options(args, &meta.workspace_root)?;
    plugin::require_member(&meta, &options.package)?;
    let package = options.package.as_str();
    let reaper = reaper_path()?;
    require_tool(
        "xvfb-run",
        "REAPER must never open a window on the real display",
    )?;
    require_tool("timeout", "needed to stop a hung REAPER")?;
    require_tool(
        "python3",
        "the editor scenarios send X events with xtask/reaper/xinput.py",
    )?;

    if options.build && options.bundle_dir.is_none() {
        let mut build_args = vec![package.to_owned(), "--locked".to_owned()];
        if options.release {
            build_args.push("--release".to_owned());
        }
        bundle::bundle(&build_args)?;
    }
    let bundled = match &options.bundle_dir {
        Some(dir) => dir
            .canonicalize()
            .map_err(|e| format!("--bundle-dir {}: {e}", dir.display()))?,
        None => bundle::bundled_dir()?,
    };
    for format in &options.formats {
        let artefact = bundled.join(format!("{package}.{}", format.tag()));
        if !artefact.exists() {
            return Err(format!(
                "{} is missing; run without --no-build (or point --bundle-dir at a directory that holds it)",
                artefact.display()
            ));
        }
    }

    let product = package == plugin::PRODUCT_PACKAGE;
    let mut scenarios: Vec<Scenario> = if options.scenarios.is_empty() {
        if product {
            Scenario::PRODUCT.to_vec()
        } else {
            Scenario::SPIKE.to_vec()
        }
    } else {
        options.scenarios.clone()
    };
    // The comparison between formats needs what these leave behind.
    if scenarios.contains(&Scenario::CrossFormat) {
        let needed: &[Scenario] = if product {
            &[
                Scenario::State,
                Scenario::PatchRoundtrip,
                Scenario::SlotAutomation,
                Scenario::MacroAutomation,
            ]
        } else {
            &[Scenario::State, Scenario::Automation]
        };
        scenarios.extend_from_slice(needed);
        scenarios.sort();
        scenarios.dedup();
    }
    let allowed = if product {
        &Scenario::PRODUCT[..]
    } else {
        &Scenario::SPIKE[..]
    };
    if let Some(bad) = scenarios.iter().find(|s| !allowed.contains(s)) {
        return Err(format!(
            "scenario {} does not run for {package}",
            bad.name()
        ));
    }

    let work = meta.target_directory.join("reaper-test");
    let sandbox = Sandbox {
        plugin_name: options.plugin_name.clone(),
        reaper,
        home: work.join("home"),
        out: work.join("out"),
        lua: meta.workspace_root.join("xtask/reaper"),
    };
    prepare_sandbox(&sandbox, &bundled)?;
    println!(
        "reaper-test: package {package} (REAPER lists it as {:?})",
        sandbox.plugin_name
    );
    println!("reaper-test: REAPER {}", sandbox.reaper.display());
    println!("reaper-test: bundles {}", bundled.display());
    println!("reaper-test: results in {}", sandbox.out.display());

    let mut report = Report::default();
    let mut shared = Shared::default();
    let mut timing = Timing::default();
    for &format in &options.formats {
        let started = Instant::now();
        let chunks = Chunks::write(&sandbox.out, format, product)?;
        for &scenario in scenarios.iter().filter(|s| **s != Scenario::CrossFormat) {
            let t = Instant::now();
            run_scenario(
                &sandbox,
                &mut report,
                &mut shared,
                format,
                &chunks,
                scenario,
            )?;
            timing.add(scenario.name(), format, t.elapsed().as_secs_f64());
        }
        println!(
            "reaper-test: {} finished in {:.0} s",
            format.tag(),
            started.elapsed().as_secs_f64()
        );
    }

    if scenarios.contains(&Scenario::CrossFormat) {
        cross_format(&sandbox, &mut report, &shared, &options.formats, product);
    }

    print_report(&report);
    print_table(&report, &timing, &options.formats);
    if report.failures() == 0 {
        println!("reaper-test: ok ({} checks)", report.checks());
        Ok(())
    } else {
        Err(format!(
            "reaper-test: {} of {} checks failed (logs and stdout in {})",
            report.failures(),
            report.checks(),
            sandbox.out.display()
        ))
    }
}

/// Runs one scenario for one format: its REAPER launches (and renders), then the checks on what they left.
fn run_scenario(
    sb: &Sandbox,
    report: &mut Report,
    shared: &mut Shared,
    format: Format,
    chunks: &Chunks,
    scenario: Scenario,
) -> Result<(), String> {
    let tag = format.tag();
    let product = chunks.state.is_some();
    match scenario {
        Scenario::State => {
            // (a) and (c): parameters, ids, automation lane, state; then reload in a new process.
            for phase in ["create", "reload"] {
                let extra = if phase == "create" {
                    chunk_env(chunks.state.as_ref(), None)
                } else {
                    Vec::new()
                };
                let scope = format!("state-{phase}");
                let log = run_script(sb, report, format, &scope, "state", phase, &extra);
                if phase == "create" {
                    if let Some(ids) = log.data.get("idents-create") {
                        shared.idents.insert(format, ids.clone());
                    }
                } else {
                    let created = shared.idents.get(&format).cloned();
                    let reloaded = log.data.get("idents-reload").cloned();
                    report.add(
                        &format!("{tag}/state-reload"),
                        "param-ids-restored",
                        created.is_some() && created == reloaded,
                        "the 72 ids are identical after reload in a new process",
                    );
                }
            }
            checks::state(sb, report, format, product);
        }
        Scenario::PatchRoundtrip => {
            for phase in ["create", "reload1", "reload2"] {
                let extra = if phase == "create" {
                    chunk_env(chunks.roundtrip.as_ref(), None)
                } else {
                    Vec::new()
                };
                run_script(
                    sb,
                    report,
                    format,
                    "patch-roundtrip",
                    "roundtrip",
                    phase,
                    &extra,
                );
            }
            checks::roundtrip(&sb.out, report, format, PRODUCT_ROUNDTRIP_PATCH);
            if let Ok(saved) = chunk::Saved::read(&sb.out, format, "roundtrip-create")
                && let Some(text) = saved.patch_text()
            {
                shared.patch_text.insert(format, text.to_owned());
            }
        }
        Scenario::SlotAutomation | Scenario::MacroAutomation => {
            let (name, kind, chunk) = if scenario == Scenario::SlotAutomation {
                ("slot-automation", "slot", &chunks.slot)
            } else {
                ("macro-automation", "macro", &chunks.macros)
            };
            let mut extra = chunk_env(chunk.as_ref(), None);
            extra.push(("ENTH_SCENARIO", kind.to_owned()));
            run_script(sb, report, format, name, "render", "build", &extra);
            render_project(
                sb,
                report,
                format,
                name,
                &format!("{kind}-automation-{tag}.rpp"),
            )?;
            if scenario == Scenario::SlotAutomation {
                checks::slot_automation(&sb.out, report, format);
                // A lane that never moves, built and rendered on its own.
                let mut extra = chunk_env(chunks.slot.as_ref(), None);
                extra.push(("ENTH_SCENARIO", "flat".to_owned()));
                run_script(sb, report, format, name, "render", "build-flat", &extra);
                render_project(
                    sb,
                    report,
                    format,
                    name,
                    &format!("flat-automation-{tag}.rpp"),
                )?;
                checks::flat_lane(&sb.out, report, format, CLAP_FLAT_LANE_IS_KNOWN_GAP);
            } else {
                checks::macro_automation(&sb.out, report, format);
            }
        }
        Scenario::Automation => {
            // The spike shell: macro 1 and slot 1 ramped together, heard in an offline render.
            run_script(sb, report, format, "automation", "automation", "build", &[]);
            render_project(sb, report, format, "automation", &format!("auto-{tag}.rpp"))?;
            checks::automation_legacy(&sb.out, report, format);
        }
        Scenario::UndoStateLoad => {
            let extra = chunk_env(chunks.state.as_ref(), chunks.roundtrip.as_ref());
            run_script(sb, report, format, "undo-state-load", "undo", "run", &extra);
        }
        Scenario::TwoInstances => {
            for phase in ["create", "reload"] {
                let extra = if phase == "create" {
                    chunk_env(chunks.state.as_ref(), chunks.roundtrip.as_ref())
                } else {
                    Vec::new()
                };
                run_script(sb, report, format, "two-instances", "two", phase, &extra);
            }
            checks::two_instances(&sb.out, report, format);
        }
        Scenario::EditorSmoke => {
            // The richest patch, so a bigger editor has every section to draw.
            let extra = chunk_env(chunks.roundtrip.as_ref(), None);
            run_script(
                sb,
                report,
                format,
                "editor-smoke",
                "editor",
                "smoke",
                &extra,
            );
        }
        Scenario::DirtyFlag => {
            let edit = run_script(
                sb,
                report,
                format,
                "dirty-flag",
                "dirty",
                "edit",
                &chunk_env(chunks.state.as_ref(), None),
            );
            let control = run_script(
                sb,
                report,
                format,
                "dirty-flag",
                "dirty",
                "control",
                &chunk_env(chunks.dirty_control.as_ref(), None),
            );
            let idle = run_script(
                sb,
                report,
                format,
                "dirty-flag",
                "dirty",
                "idle",
                &chunk_env(chunks.state.as_ref(), None),
            );
            dirty_flag(report, format, &edit, &control, &idle);
        }
        Scenario::CrossFormat => {}
    }
    Ok(())
}

/// Renders a project the script saved, offline (`reaper -renderproject`), into the output directory.
fn render_project(
    sb: &Sandbox,
    report: &mut Report,
    format: Format,
    scenario: &str,
    project_name: &str,
) -> Result<(), String> {
    let tag = format.tag();
    let project = sb.out.join(project_name);
    if !project.exists() {
        report.add(
            &format!("{tag}/{scenario}"),
            "project-built",
            false,
            format!("{} is missing", project.display()),
        );
        return Ok(());
    }
    run_reaper(
        sb,
        &format!("{tag}-{scenario}-render"),
        &[
            "-nosplash",
            "-newinst",
            "-renderproject",
            &project.display().to_string(),
        ],
        &[],
    )
    .map_err(|e| format!("{tag} {scenario} render: {e}"))
}

/// The `dirty-flag` scenario's rows. The edit must have been a patch-only edit (it moved no host parameter), and the
/// answer to the open check is `dirty-after-editor-edit`: the flag read at once and 3 s after the edit, with the editor
/// still open (the case where a user quits REAPER with the editor open). The control (a knob bound to a host slot) and
/// the editor window's closing are measured, not judged: closing a floating FX window marks the project dirty by
/// itself (the idle phase, with no edit, reads 1 too), so it says nothing about the edit.
fn dirty_flag(report: &mut Report, format: Format, edit: &Log, control: &Log, idle: &Log) {
    let scope = format!("{}/dirty-flag", format.tag());
    let get = |log: &Log, key: &str| log.data.get(key).cloned().unwrap_or_else(|| "?".to_owned());
    let moved = get(edit, "host-params-moved");
    report.add(
        &scope,
        "edit-is-patch-only",
        moved == "0",
        format!(
            "the drag at {} moved {moved} host parameters",
            get(edit, "edit-point")
        ),
    );
    let (now, late) = (get(edit, "dirty-after-edit"), get(edit, "dirty-late"));
    let detail = format!(
        "IsProjectDirty = {now} at once and {late} 3 s after an editor-only patch edit with the editor open (clean before: {}; project state changes: {})",
        get(edit, "dirty-after-save"),
        get(edit, "state-changes-during-edit")
    );
    match (now == "1" || late == "1", DIRTY_MARKING_EXPECTED) {
        (true, _) => report.add(&scope, "dirty-after-editor-edit", true, detail),
        (false, false) if now == "0" && late == "0" => report.info(
            &scope,
            "dirty-after-editor-edit",
            format!("{detail}: KNOWN GAP, REAPER does not mark the project dirty; a sixth fork patch adds dirty-marking"),
        ),
        (false, _) => report.add(&scope, "dirty-after-editor-edit", false, detail),
    }
    report.info(
        &scope,
        "dirty-after-closing-editor",
        format!(
            "IsProjectDirty = {} after the editor window closed following an edit, {} after it closed with no edit (closing the window marks the project dirty by itself)",
            get(edit, "dirty-after-close"),
            get(idle, "dirty-after-close")
        ),
    );
    report.info(
        &scope,
        "control-dirty-after-slot-edit",
        format!(
            "IsProjectDirty = {} after an edit of a knob bound to a host slot ({} host parameters moved; project state changes: {})",
            get(control, "dirty-after-edit"),
            get(control, "host-params-moved"),
            get(control, "state-changes-during-edit")
        ),
    );
}

/// What CLAP and VST3 must agree on.
fn cross_format(
    sb: &Sandbox,
    report: &mut Report,
    shared: &Shared,
    formats: &[Format],
    product: bool,
) {
    let [first, second, ..] = formats else {
        return;
    };
    let scope = "clap+vst3";
    report.add(
        scope,
        "param-ids-same-in-both-formats",
        shared.idents.contains_key(first) && shared.idents.get(first) == shared.idents.get(second),
        "CLAP and VST3 report the same 72 ids",
    );
    if !product {
        return;
    }
    report.add(
        scope,
        "saved-patch-same-in-both-formats",
        shared.patch_text.get(first).is_some_and(|t| !t.is_empty())
            && shared.patch_text.get(first) == shared.patch_text.get(second),
        "the same patch loaded as state is saved as the same text by CLAP and VST3",
    );
    for kind in ["slot", "macro"] {
        checks::renders_agree(&sb.out, report, kind, *first, *second);
    }
}

fn print_report(report: &Report) {
    for row in &report.rows {
        println!(
            "  {:<4} {:<26} {:<30} {}",
            match row.status {
                Status::Ok => "ok",
                Status::Fail => "FAIL",
                Status::Info => "info",
            },
            row.scope,
            row.name,
            row.detail
        );
    }
}

/// Scenario by format: checks passed of checked, and seconds.
fn print_table(report: &Report, timing: &Timing, formats: &[Format]) {
    let mut header = format!("  {:<18}", "scenario");
    for format in formats {
        let _ = write!(header, " {:>18}", format.tag());
    }
    println!("\n{header}");
    for name in &timing.order {
        // A scenario's rows sit in one or more scopes: `state` is `state-create` and `state-reload`.
        let mut line = format!("  {name:<18}");
        for &format in formats {
            let tag = format.tag();
            let (mut ok, mut total) = (0, 0);
            for scope in report
                .rows
                .iter()
                .filter_map(|r| r.scope.strip_prefix(&format!("{tag}/")))
                .filter(|s| {
                    s == name
                        || (name == "state" && s.starts_with("state-"))
                        || (name == "automation" && s.starts_with("automation"))
                })
                .collect::<std::collections::BTreeSet<_>>()
            {
                let (o, t) = report.count(&format!("{tag}/{scope}"));
                ok += o;
                total += t;
            }
            let seconds = timing
                .seconds
                .get(&(name.clone(), format))
                .copied()
                .unwrap_or(0.0);
            let _ = write!(line, " {:>18}", format!("{ok}/{total} in {seconds:.1} s"));
        }
        println!("{line}");
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

/// Launches REAPER under Xvfb with a Lua script, then reads the script's log into the report. `scenario` names the
/// report scope (`<format>/<scenario>`) and, with the script and phase, the log file.
fn run_script(
    sb: &Sandbox,
    report: &mut Report,
    format: Format,
    scenario: &str,
    script: &str,
    phase: &str,
    extra_env: &[(&'static str, String)],
) -> Log {
    let tag = format.tag();
    // One log per launch: a scenario name and a phase never repeat for a format, a script name can (slot and macro).
    let label = format!("{tag}-{scenario}-{phase}");
    let log_name = format!("{label}.log");
    let lua = sb.lua.join(format!("{script}.lua"));
    let fx = format.reaper_name(&sb.plugin_name);
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
    env.extend(extra_env.iter().map(|(k, v)| (*k, v.clone())));
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let scope = format!("{tag}/{scenario}");
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
