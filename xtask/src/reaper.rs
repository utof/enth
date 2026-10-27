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
//! | `macro-automation` | automation on macros 1 and 8 makes it louder and brighter, and a second offline render of the project is the same audio |
//! | `mod-wheel` | the mod wheel (MIDI CC 1 in a MIDI item) reaches the patch: VST3 through the one CC parameter the plugin maps |
//! | `wt` | (lane SKS, F2m) a patch with a wavetable oscillator phase-modulated through an audio route, loaded as state: saved byte for byte, played by short notes whose level and brightness are the engine's own (the route is heard), reloaded in a new process, saved and rendered again |
//! | `undo-state-load` | REAPER's undo and redo across plugin state loads restore the patch and the parameters |
//! | `two-instances` | two instances keep their own patch and values, through a reload and a deleted neighbour |
//! | `editor-smoke` | the editor opens, draws and closes five times, under Xvfb |
//! | `dirty-flag` | an editor-only patch edit marks the project dirty (`IsProjectDirty`; fork patch 6), and no edit leaves it clean |
//! | `preset-load` | a user preset loaded through the editor's browser (real clicks): the patch changes, the bookmark is saved, no host parameter moves, the project is dirty, and one editor Undo and Redo take the whole load back and again; ▶ held with the transport stopped is measured (lane PBU) |
//! | `round` | (lane SKR) a Recreate round through the editor's Recreate bar (real clicks), the transport stopped: Start prepares the target, Target ▶ and Mine ▶ sound on the track's meter, a submit is scored, End removes the round and writes the round log |
//! | `lesson` | (lane SKL; on the default bundle since lane LFU turned the `learn` feature on by default) the tour's first lesson from the bundled `basics` pack through the Learn view (real clicks), the transport stopped: Start, an example sounds on the track's meter, Next to the Try step, Show me passes it (its sound check rendered on the worker) and the progress file records it, the project saved mid-lesson; reopened in a new process, the lesson resumes at its step and moves on |
//! | `duel` | (lane SKD, F2d; on a bundle built with `--features duel`, where it joins the default run) two instances on two tracks, the transport stopped: instance 1 starts from a saved round whose patch is the target, passes it and shares it through the duel card (real clicks); its message goes through the X clipboard (Copy, then the platform's Paste, Ctrl+V, into instance 2's paste box) and opens a duel; its answer code arrives sealed, opens once instance 2 has given up, and is checked on instance 2's worker "as reported" with the commitment matched (the duel log, `ENTH_DUEL_LOG`); lane DUI: both instances' duel history file (`ENTH_DUELS_DIR`) keeps the sharer's salt and codes and the receiver's check |
//! | `starter-project` | (CLAP) the project `cargo xtask starter-project` writes opens with the starter patch on each of its four tracks, and `extract-presets` gets it back from REAPER's save, each sound named and typed after its track (not "Init") |
//! | `cross-format` | CLAP and VST3 agree: parameter ids, the saved patch, the rendered audio |
//!
//! It runs the product plugin only (`enth-plugin`, the default package). The Phase 0 shell spike's three scenarios went
//! with the spike when P1b lane INT retired it from the workspace.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::{bundle, plugin};

mod audio;
mod checks;
mod chunk;
mod soak;

/// The state-chunk code `cargo xtask extract-presets` and `starter-project` share with the scenarios (`crate::preset`).
pub(crate) use chunk::{base64_decode, base64_encode, nice_plug_state};
/// `cargo xtask reaper-soak` (lane SOAK): the long run, `soak.rs`.
pub use soak::reaper_soak;

const DEFAULT_REAPER: &str = "apps/reaper-7.82/reaper_linux_x86_64/REAPER/reaper";
/// Hard limit for one REAPER launch; a hung script (e.g. a modal error dialog) is killed.
const LAUNCH_TIMEOUT_SECS: u32 = 90;

/// Whether REAPER is expected to mark the project dirty after an editor-only patch edit. Stock nice-plug never calls
/// CLAP `mark_dirty` or VST3 `setDirty`, so REAPER did not (lane RH measured it); the vendored fork's patch 6 (lane
/// FK2) adds the request and the plugin makes it after every editor edit, undo and redo (`docs/plugin.md`). So a clean
/// project after an edit fails the `dirty-flag` scenario; `false` would report it as a known gap instead.
const DIRTY_MARKING_EXPECTED: bool = true;

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
    ModWheel,
    Wt,
    UndoStateLoad,
    TwoInstances,
    EditorSmoke,
    DirtyFlag,
    StarterProject,
    CrossFormat,
    PresetLoad,
    Round,
    Lesson,
    Duel,
}

impl Scenario {
    const PRODUCT: [Scenario; 15] = [
        Scenario::State,
        Scenario::PatchRoundtrip,
        Scenario::SlotAutomation,
        Scenario::MacroAutomation,
        Scenario::ModWheel,
        Scenario::Wt,
        Scenario::UndoStateLoad,
        Scenario::TwoInstances,
        Scenario::EditorSmoke,
        Scenario::DirtyFlag,
        Scenario::StarterProject,
        Scenario::CrossFormat,
        Scenario::PresetLoad,
        Scenario::Round,
        Scenario::Lesson,
    ];
    fn name(self) -> &'static str {
        match self {
            Scenario::State => "state",
            Scenario::PatchRoundtrip => "patch-roundtrip",
            Scenario::SlotAutomation => "slot-automation",
            Scenario::MacroAutomation => "macro-automation",
            Scenario::ModWheel => "mod-wheel",
            Scenario::Wt => "wt",
            Scenario::UndoStateLoad => "undo-state-load",
            Scenario::TwoInstances => "two-instances",
            Scenario::EditorSmoke => "editor-smoke",
            Scenario::DirtyFlag => "dirty-flag",
            Scenario::StarterProject => "starter-project",
            Scenario::CrossFormat => "cross-format",
            Scenario::PresetLoad => "preset-load",
            Scenario::Round => "round",
            Scenario::Lesson => "lesson",
            Scenario::Duel => "duel",
        }
    }

    fn parse(text: &str) -> Option<Scenario> {
        match text {
            "state-create" | "state-reload" => Some(Scenario::State),
            "duel" => Some(Scenario::Duel),
            _ => Scenario::PRODUCT.iter().copied().find(|s| s.name() == text),
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
    /// `--features`: cargo features for the bundle build (lane SKD: `duel` adds the `duel` scenario to the default run).
    features: Vec<String>,
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
        features: Vec::new(),
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
                        "unknown scenario {name:?} (the scenarios: {})",
                        names.join(", ")
                    )
                })?);
            }
            "--features" => {
                let list = args.next().ok_or("--features needs a list (e.g. duel)")?;
                options.features.extend(
                    list.split(',')
                        .map(str::trim)
                        .filter(|f| !f.is_empty())
                        .map(str::to_owned),
                );
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
    wheel: Option<String>,
    wt: Option<String>,
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
/// `mod-wheel`: the mod wheel turns osc1's level down by 100 %.
const PRODUCT_WHEEL_PATCH: &str = include_str!("../reaper/enth-mod-wheel.patch.json");
/// `wt` (lane SKS, F2m): a wavetable oscillator phase-modulated by a sine operator through an audio route, with two
/// slots on its parameters, in the plugin's own serialisation.
const PRODUCT_WT_PATCH: &str = include_str!("../reaper/enth-wt.patch.json");
/// `dirty-flag` control: every knob of the first card is bound to a host slot.
const PRODUCT_DIRTY_CONTROL_PATCH: &str = include_str!("../reaper/enth-dirty-control.patch.json");
/// `duel` (lane SKD): instance 1's state, the nice-plug fields `patch` and `enth` of a round whose patch is its target.
const DUEL_ROUND_STATE: &str = include_str!("../reaper/enth-duel-round.state.json");

impl Chunks {
    fn write(out: &Path, format: Format) -> Result<Chunks, String> {
        let make = |name: &str, patch: &str| -> Result<Option<String>, String> {
            write_state_chunk(out, name, format, patch).map(Some)
        };
        Ok(Chunks {
            state: make("state", PRODUCT_STATE_PATCH)?,
            roundtrip: make("roundtrip", PRODUCT_ROUNDTRIP_PATCH)?,
            slot: make("slot", PRODUCT_SLOT_PATCH)?,
            macros: make("macro", PRODUCT_MACRO_PATCH)?,
            wheel: make("wheel", PRODUCT_WHEEL_PATCH)?,
            wt: make("wt", PRODUCT_WT_PATCH)?,
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
    if package != plugin::PRODUCT_PACKAGE {
        return Err(format!(
            "reaper-test runs the scenarios of {} only, not {package}",
            plugin::PRODUCT_PACKAGE
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
        "the editor scenarios send X events with xtask/reaper/xinput.py",
    )?;

    let work = meta.target_directory.join("reaper-test");
    // Every scenario, `lesson` included, runs on the default bundle: the `learn` feature is on by default (lane LFU),
    // so the separate `learn` build lane SKL needed is gone.
    if options.build && options.bundle_dir.is_none() {
        let mut build_args = vec![package.to_owned(), "--locked".to_owned()];
        if options.release {
            build_args.push("--release".to_owned());
        }
        if !options.features.is_empty() {
            build_args.push("--features".to_owned());
            build_args.push(options.features.join(","));
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
    let mut scenarios: Vec<Scenario> = if options.scenarios.is_empty() {
        let mut all = Scenario::PRODUCT.to_vec();
        // The duel card exists only in a bundle built with the `duel` feature (off by default, lane SKD).
        if options
            .features
            .iter()
            .any(|f| f == "duel" || f == "enth-plugin/duel")
        {
            all.push(Scenario::Duel);
        }
        all
    } else {
        options.scenarios.clone()
    };
    // The comparison between formats needs what these leave behind.
    if scenarios.contains(&Scenario::CrossFormat) {
        scenarios.extend_from_slice(&[
            Scenario::State,
            Scenario::PatchRoundtrip,
            Scenario::SlotAutomation,
            Scenario::MacroAutomation,
            Scenario::Wt,
        ]);
        scenarios.sort();
        scenarios.dedup();
    }

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
        let chunks = Chunks::write(&sandbox.out, format)?;
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
        cross_format(&sandbox, &mut report, &shared, &options.formats);
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
            checks::state(sb, report, format);
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
            preset_extracted(&sb.out, report, format);
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
                // The same project rendered again must give the same audio: REAPER restarted a CLAP plugin in the
                // middle of every offline render, at a point that depended on the render's speed, until the fork's
                // patch 9 (`CLAP_RESTART_ON_RENDER_MODE_CHANGE`); that cut a note somewhere different each time.
                let rendered = sb.out.join(format!("render-macro-{tag}.wav"));
                let first = sb.out.join(format!("render-macro-{tag}-first.wav"));
                std::fs::rename(&rendered, &first)
                    .map_err(|e| format!("{}: {e}", rendered.display()))?;
                render_project(
                    sb,
                    report,
                    format,
                    name,
                    &format!("macro-automation-{tag}.rpp"),
                )?;
                checks::render_repeatable(report, format, &first, &rendered);
            }
        }
        Scenario::ModWheel => {
            let mut extra = chunk_env(chunks.wheel.as_ref(), None);
            extra.push(("ENTH_SCENARIO", "wheel".to_owned()));
            run_script(sb, report, format, "mod-wheel", "render", "build", &extra);
            render_project(
                sb,
                report,
                format,
                "mod-wheel",
                &format!("wheel-automation-{tag}.rpp"),
            )?;
            checks::mod_wheel(&sb.out, report, format);
        }
        Scenario::Wt => {
            // Build and render the project, then reload it in a new process, save it again and render that too.
            for (phase, project) in [("create", "a"), ("reload", "b")] {
                let extra = if phase == "create" {
                    chunk_env(chunks.wt.as_ref(), None)
                } else {
                    Vec::new()
                };
                run_script(sb, report, format, "wt", "wt", phase, &extra);
                render_project(sb, report, format, "wt", &format!("wt-{tag}-{project}.rpp"))?;
            }
            checks::wt(&sb.out, report, format, PRODUCT_WT_PATCH);
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
        Scenario::StarterProject => starter_project(sb, report, format)?,
        Scenario::CrossFormat => {}
        Scenario::PresetLoad => preset_load(sb, report, format, chunks)?,
        Scenario::Round => round(sb, report, format)?,
        Scenario::Lesson => lesson(sb, report, format)?,
        Scenario::Duel => duel(sb, report, format)?,
    }
    Ok(())
}

/// `duel` (lane SKD, F2d): instance 1's saved round, a round on level 2's seed 77 whose patch is the target rebuilt by
/// edits (`xtask/reaper/enth-duel-round.state.json`, written and checked by `enth-plugin`'s `tests_duel.rs`), so its
/// first Submit passes. The duel log (`ENTH_DUEL_LOG`), the round log, the crash folder and an empty `HOME` are per
/// format in the results: with the real home, REAPER loaded an enth installed in `~/.vst3` instead of the VST3 bundle
/// under test (lane SKD found it: the installed build had no `duel` feature). `xtask/reaper/duel.lua` clicks both instances' duel cards and makes the checks.
fn duel(sb: &Sandbox, report: &mut Report, format: Format) -> Result<(), String> {
    let tag = format.tag();
    let dirs: Vec<PathBuf> = ["rounds", "crashes", "home", "duels"]
        .iter()
        .map(|d| sb.out.join(format!("duel-{d}-{tag}")))
        .collect();
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let log = sb.out.join(format!("duel-log-{tag}.jsonl"));
    let _ = std::fs::remove_file(&log);
    let fields: serde_json::Value = serde_json::from_str(DUEL_ROUND_STATE)
        .map_err(|e| format!("enth-duel-round.state.json: {e}"))?;
    let chunk = chunk::write_state_fields(&sb.out, "duel-round", format, &fields)?;
    let mut extra = chunk_env(Some(&chunk), None);
    extra.push(("ENTH_ROUNDS_DIR", dirs[0].display().to_string()));
    extra.push(("ENTH_CRASH_DIR", dirs[1].display().to_string()));
    extra.push(("ENTH_DUEL_LOG", log.display().to_string()));
    // An empty home: REAPER also scans the standard `~/.vst3` (and `~/.clap`), where an installed enth (`cargo xtask
    // install`, an owner build) would win over the bundle under test, without the `duel` feature.
    extra.push(("HOME", dirs[2].display().to_string()));
    // The duel history (lane DUI): both instances keep theirs in one file, in the results.
    extra.push(("ENTH_DUELS_DIR", dirs[3].display().to_string()));
    let played = run_script(sb, report, format, "duel", "duel", "play", &extra);
    duel_history(report, &format!("{tag}/duel"), &dirs[3], &log);
    let get = |k: &str| played.data.get(k).cloned().unwrap_or_default();
    report.info(
        &format!("{tag}/duel"),
        "duel-sizes",
        format!(
            "scorecard {} symbols, answer {} symbols (a passing answer: the target rebuilt by edits); codes in {} s, check in {} s",
            get("scorecard-symbols"),
            get("answer-symbols"),
            get("share-s"),
            get("verify-s")
        ),
    );
    Ok(())
}

/// `duel`'s history checks (lane DUI): the two instances' duel history file (`ENTH_DUELS_DIR/duels.json`,
/// `enth.duel-history/1`) holds the sharer's entry (the salt, 16 hex digits, and the codes the duel log saw shared) and
/// the receiver's (the sharer's scorecard and answer code, and its check: "as reported", the commitment matched).
fn duel_history(report: &mut Report, scope: &str, dir: &Path, log: &Path) {
    let shared: Option<serde_json::Value> = std::fs::read_to_string(log).ok().and_then(|t| {
        t.lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|l| l["event"] == "share")
    });
    let text = std::fs::read_to_string(dir.join("duels.json")).unwrap_or_default();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let entries = doc["entries"].as_array().cloned().unwrap_or_default();
    report.add(
        scope,
        "history-file",
        doc["schema"] == "enth.duel-history/1" && entries.len() == 2,
        format!(
            "{} entries in {}",
            entries.len(),
            dir.join("duels.json").display()
        ),
    );
    let Some(shared) = shared else {
        report.add(scope, "history-salt", false, "no share in the duel log");
        return;
    };
    let sharer = entries
        .iter()
        .find(|e| e["own"]["answer"] == shared["answer"]);
    report.add(
        scope,
        "history-salt",
        sharer.is_some_and(|e| {
            e["salt"].as_str().is_some_and(|s| s.len() == 16)
                && e["own"]["scorecard"] == shared["scorecard"]
        }),
        "the sharer's entry: its salt and the codes it shared",
    );
    let receiver = entries
        .iter()
        .find(|e| e["answer"] == shared["answer"] && e["card"] == shared["scorecard"]);
    report.add(
        scope,
        "history-check",
        receiver.is_some_and(|e| {
            e["checked"]["as_reported"] == true && e["checked"]["commitment_ok"] == true
        }),
        "the receiver's entry: the sharer's codes and the check",
    );
}

/// `lesson` (lane SKL, F2l; on the default bundles since lane LFU): two launches (`play`, then `reload` in a new
/// process), with the Learn and Packs folders (`ENTH_LEARN_DIR`, `ENTH_PACKS_DIR`) and the crash folder in the
/// results, so the owner's Documents are never touched. `xtask/reaper/lesson.lua` clicks the Learn view and makes the
/// checks.
fn lesson(sb: &Sandbox, report: &mut Report, format: Format) -> Result<(), String> {
    let tag = format.tag();
    let scope = format!("{tag}/lesson");
    let dirs: Vec<PathBuf> = ["learn", "packs", "crashes"]
        .iter()
        .map(|d| sb.out.join(format!("lesson-{d}-{tag}")))
        .collect();
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let extra = vec![
        ("ENTH_LEARN_DIR", dirs[0].display().to_string()),
        ("ENTH_PACKS_DIR", dirs[1].display().to_string()),
        ("ENTH_CRASH_DIR", dirs[2].display().to_string()),
    ];
    let play = run_script(sb, report, format, "lesson", "lesson", "play", &extra);
    let _ = run_script(sb, report, format, "lesson", "lesson", "reload", &extra);
    report.info(
        &scope,
        "lesson-levels",
        format!(
            "▶ Saw, A peaked at {} (transport stopped); progress file: {}",
            play.data.get("example-peak").cloned().unwrap_or_default(),
            play.data.get("progress").cloned().unwrap_or_default()
        ),
    );
    Ok(())
}

/// `preset-load` (lane PBU): the user folder (`ENTH_PRESETS_DIR`) holds one preset, the roundtrip patch as
/// `roundtrip.patch.json`, and the crash folder (`ENTH_CRASH_DIR`) is empty, so no crash notice covers the editor; the
/// project starts from the `state` patch. `xtask/reaper/preset.lua` clicks the browser and makes the checks.
fn preset_load(
    sb: &Sandbox,
    report: &mut Report,
    format: Format,
    chunks: &Chunks,
) -> Result<(), String> {
    let tag = format.tag();
    let presets = sb.out.join(format!("presets-{tag}"));
    let crashes = sb.out.join(format!("crashes-{tag}"));
    for dir in [&presets, &crashes] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = presets.join("roundtrip.patch.json");
    std::fs::write(&file, PRODUCT_ROUNDTRIP_PATCH)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    let mut extra = chunk_env(chunks.state.as_ref(), None);
    extra.push(("ENTH_PRESETS_DIR", presets.display().to_string()));
    extra.push(("ENTH_CRASH_DIR", crashes.display().to_string()));
    let log = run_script(sb, report, format, "preset-load", "preset", "load", &extra);
    let scope = format!("{tag}/preset-load");
    let sounds = log
        .data
        .get("audition-sounds-while-stopped")
        .map(String::as_str);
    let peak = log.data.get("audition-peak").cloned().unwrap_or_default();
    report.info(
        &scope,
        "audition-while-stopped",
        match sounds {
            Some("1") => format!("REAPER kept calling process() with the transport stopped: ▶ held peaked at {peak}"),
            Some(_) => format!("silent: REAPER did not process while stopped (peak {peak}); the open check stays open"),
            None => "not measured".to_owned(),
        },
    );
    Ok(())
}

/// `round` (lane SKR, F2r): a Recreate round through the editor's Recreate bar (real clicks), the transport stopped:
/// Start prepares the target on the worker, Target ▶ and Mine ▶ sound (the track's peak meter), a submit is scored, End
/// removes the round and writes the round log (`ENTH_ROUNDS_DIR`, per format). The crash folder is empty, so no crash
/// notice covers the editor. `xtask/reaper/round.lua` clicks and makes the checks.
fn round(sb: &Sandbox, report: &mut Report, format: Format) -> Result<(), String> {
    let tag = format.tag();
    let rounds = sb.out.join(format!("rounds-{tag}"));
    let crashes = sb.out.join(format!("round-crashes-{tag}"));
    for dir in [&rounds, &crashes] {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let extra = vec![
        ("ENTH_ROUNDS_DIR", rounds.display().to_string()),
        ("ENTH_CRASH_DIR", crashes.display().to_string()),
    ];
    let log = run_script(sb, report, format, "round", "round", "play", &extra);
    let scope = format!("{tag}/round");
    let get = |k: &str| log.data.get(k).cloned().unwrap_or_default();
    report.info(
        &scope,
        "round-levels",
        format!(
            "prepared in {} s; Target ▶ peak {}, Mine ▶ peak {} (transport stopped)",
            get("prepare-s"),
            get("target-peak"),
            get("mine-peak")
        ),
    );
    Ok(())
}

/// `starter-project` (lane INT, CLAP only: the project holds the CLAP): the project `cargo xtask starter-project` writes
/// opens in REAPER with the starter patch loaded on each of its four named tracks, and `extract-presets` on REAPER's
/// save of it gives the four tracks' patches back by name, each named and typed after its track rather than "Init"
/// (R1/S1 lane INT), so owner build 2 can start there.
fn starter_project(sb: &Sandbox, report: &mut Report, format: Format) -> Result<(), String> {
    let scope = format!("{}/starter-project", format.tag());
    if format != Format::Clap {
        report.info(
            &scope,
            "clap-only",
            "the starter project holds enth's CLAP; the VST3 is inserted by hand",
        );
        return Ok(());
    }
    let project = sb.out.join(format!("starter-{}.rpp", format.tag()));
    let text = crate::preset::starter_project(
        &crate::preset::STARTER_TRACKS,
        crate::preset::STARTER_PATCH,
    );
    std::fs::write(&project, text).map_err(|e| format!("{}: {e}", project.display()))?;
    let log = run_script(
        sb,
        report,
        format,
        "starter-project",
        "starter",
        "open",
        &[("ENTH_STARTER_PROJECT", project.display().to_string())],
    );
    let names = crate::preset::STARTER_TRACKS.join(",");
    let tracks = log.data.get("starter-tracks").cloned().unwrap_or_default();
    report.add(
        &scope,
        "tracks-named",
        tracks == names,
        format!("tracks {tracks:?}"),
    );
    let loaded = (1..=crate::preset::STARTER_TRACKS.len())
        .filter(|i| {
            chunk::Saved::read(&sb.out, format, &format!("starter-{i}"))
                .is_ok_and(|s| s.patch_text() == Some(crate::preset::STARTER_PATCH))
        })
        .count();
    report.add(
        &scope,
        "starter-patch-loaded",
        loaded == crate::preset::STARTER_TRACKS.len(),
        format!(
            "{loaded} of {} plugins hold the starter patch, byte for byte",
            crate::preset::STARTER_TRACKS.len()
        ),
    );
    let saved = sb.out.join(format!("starter-{}-saved.rpp", format.tag()));
    let (ok, detail) = match std::fs::read_to_string(&saved)
        .map_err(|e| format!("{}: {e}", saved.display()))
        .and_then(|text| crate::preset::instances(&text))
    {
        Ok(found) => {
            let got: Vec<String> = found
                .iter()
                .enumerate()
                .map(|(i, f)| crate::preset::preset_name(f, "starter", i))
                .collect();
            let same = found
                .iter()
                .all(|f| f.patch == crate::preset::STARTER_PATCH);
            // What `extract-presets` writes (lane INT, R1/S1): each sound named after its track, typed by it, without
            // Init's name, tag and description, and otherwise the starter patch.
            let presets: Vec<Result<(String, Vec<String>), String>> = found
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    crate::preset::preset_text(
                        &f.patch,
                        &crate::preset::preset_title(f, "starter", i),
                    )
                })
                .collect();
            let titles: Vec<String> = presets
                .iter()
                .map(|p| {
                    p.as_ref()
                        .ok()
                        .and_then(|(text, _)| enth_patch::file::load(text).ok())
                        .map_or_else(String::new, |l| {
                            format!("{} {:?}", l.patch.meta.name, l.patch.meta.tags)
                        })
                })
                .collect();
            let want: Vec<String> = crate::preset::STARTER_TRACKS
                .iter()
                .map(|t| format!("{} [{t:?}]", crate::preset::title(t)))
                .collect();
            let factory_ready = presets.iter().all(|p| {
                p.as_ref().is_ok_and(|(text, problems)| {
                    problems.is_empty() && same_apart_from_meta(text, crate::preset::STARTER_PATCH)
                })
            });
            (
                got.join(",") == names && same && titles == want && factory_ready,
                format!(
                    "extract-presets on REAPER's save: files {got:?}, every saved patch the starter's: {same}; presets \
                     {titles:?}, factory-ready and otherwise the starter: {factory_ready}"
                ),
            )
        }
        Err(e) => (false, e),
    };
    report.add(&scope, "presets-extracted-by-name", ok, detail);
    Ok(())
}

/// Two patch documents that differ at most in their `meta` (name, author, description, tags).
fn same_apart_from_meta(a: &str, b: &str) -> bool {
    let strip = |text: &str| {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .map(|mut v| {
                if let Some(o) = v.as_object_mut() {
                    o.remove("meta");
                }
                v
            })
    };
    matches!((strip(a), strip(b)), (Some(x), Some(y)) if x == y)
}

/// `cargo xtask extract-presets` on the project `patch-roundtrip` saved first (lane INT): the one enth instance in it
/// gives back the patch the scenario loaded, byte for byte, so a sound the owner saves in REAPER becomes a preset file
/// that differs from it only in its metadata (the name and tags, R1/S1 lane INT).
fn preset_extracted(out: &Path, report: &mut Report, format: Format) {
    let project = out.join(format!("roundtrip-{}-a.rpp", format.tag()));
    let want = PRODUCT_ROUNDTRIP_PATCH.trim_end();
    let (ok, detail) = match std::fs::read_to_string(&project)
        .map_err(|e| format!("{}: {e}", project.display()))
        .and_then(|text| crate::preset::instances(&text))
    {
        Ok(found) => {
            // The preset file `extract-presets` writes: the saved patch with only its metadata changed.
            let preset_same = found.first().is_some_and(|f| {
                crate::preset::preset_text(&f.patch, "roundtrip")
                    .is_ok_and(|(text, _)| same_apart_from_meta(&text, want))
            });
            (
                found.len() == 1 && found[0].patch == want && preset_same,
                format!(
                    "{} enth instance(s) in the saved project; the saved patch is {} bytes, {}; the preset file {}",
                    found.len(),
                    found.first().map_or(0, |f| f.patch.len()),
                    if found.first().is_some_and(|f| f.patch == want) {
                        "identical to the patch loaded"
                    } else {
                        "NOT the patch loaded"
                    },
                    if preset_same {
                        "differs only in its metadata"
                    } else {
                        "DIFFERS beyond its metadata"
                    }
                ),
            )
        }
        Err(e) => (false, e),
    };
    report.add(
        &format!("{}/patch-roundtrip", format.tag()),
        "preset-extracted",
        ok,
        detail,
    );
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
    // No false positive: with the editor open and nothing edited, the project stays clean (a plugin that marked it
    // dirty on every frame, or at the editor's open, would pass the check above and fail this one).
    let idle_now = get(idle, "dirty-after-edit");
    report.add(
        &scope,
        "clean-without-an-edit",
        idle_now == "0",
        format!("IsProjectDirty = {idle_now} 0.5 s after saving, with the editor open and no edit"),
    );
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
fn cross_format(sb: &Sandbox, report: &mut Report, shared: &Shared, formats: &[Format]) {
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
    report.add(
        scope,
        "saved-patch-same-in-both-formats",
        shared.patch_text.get(first).is_some_and(|t| !t.is_empty())
            && shared.patch_text.get(first) == shared.patch_text.get(second),
        "the same patch loaded as state is saved as the same text by CLAP and VST3",
    );
    for kind in ["slot", "macro", "wt"] {
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
                .filter(|s| s == name || (name == "state" && s.starts_with("state-")))
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
    write_reaper_ini(sb, bundled)?;
    if sb.out.exists() {
        std::fs::remove_dir_all(&sb.out).map_err(|e| format!("{}: {e}", sb.out.display()))?;
    }
    std::fs::create_dir_all(&sb.out).map_err(|e| format!("{}: {e}", sb.out.display()))
}

/// REAPER's settings for the sandbox, with its plugin path at `bundled` (the default bundles, or the `learn` ones for
/// the `lesson` scenario). The plugin caches go with it, so REAPER rescans exactly those bundles.
fn write_reaper_ini(sb: &Sandbox, bundled: &Path) -> Result<(), String> {
    for stale in [
        "reaper-vstplugins64.ini",
        "reaper-clap-linux-x86_64.ini",
        "reaper-recentfx.ini",
    ] {
        let _ = std::fs::remove_file(sb.home.join(stale));
    }
    std::fs::write(sb.home.join("reaper.ini"), reaper_ini_text(bundled))
        .map_err(|e| format!("reaper.ini: {e}"))
}

/// The sandbox's `reaper.ini`: the plugin paths, Dummy Audio at 48 kHz, and no update check.
///
/// `verchk=0` is the "check for new versions on startup" preference (lane RHX, 2026-10-27). The ini is rewritten once
/// per run, so the first REAPER launch of a run has no `[verchk] lastt` and checks; REAPER 7.83 had just come out, and
/// about 7 s into that launch a modal "REAPER New Version Notification" (725 x 380 at 598, 209 on the Xvfb screen, with
/// no window manager to keep it behind) opened over the plugin window. It covered the `round` scenario's End menu item,
/// so a single-scenario run never ended its round, and it blocked File > Quit until `timeout` killed REAPER after 90 s.
/// A full run only escaped because its first launch (`state`) is shorter than 7 s and writes `lastt` on the way out,
/// which spares every later launch. The test must not phone home and a new REAPER release must not fail it.
fn reaper_ini_text(bundled: &Path) -> String {
    format!(
        "[REAPER]\n\
         clap_path_linux-x86_64={dir}\n\
         vstpath={dir}\n\
         linux_audio_mode=2\n\
         linux_audio_srate=48000\n\
         renderclosewhendone=4\n\
         verchk=0\n",
        dir = bundled.display()
    )
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
    let empty_home = sb.out.join(format!("{label}.home"));
    std::fs::create_dir_all(&empty_home).map_err(|e| format!("{label}: {e}"))?;
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
        // An empty home for every launch: REAPER also scans the standard `~/.vst3` and `~/.clap`, so with the real
        // home it loads an installed enth (`cargo xtask install`) instead of the bundle under test (lane SKD found it
        // on the VST3 runs, 2026-10-22). A scenario's own `HOME` below still wins.
        .env("HOME", &empty_home)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sandbox_ini_points_at_the_bundles_and_never_checks_for_a_new_reaper() {
        let ini = reaper_ini_text(Path::new("/b/bundled"));
        let lines: Vec<&str> = ini.lines().collect();
        assert_eq!(lines[0], "[REAPER]");
        for want in [
            "clap_path_linux-x86_64=/b/bundled",
            "vstpath=/b/bundled",
            "linux_audio_mode=2",
            // The update check opens a modal dialog over the plugin window about 7 s into the first launch of a run.
            "verchk=0",
        ] {
            assert!(lines.contains(&want), "{want} is missing from {ini:?}");
        }
        assert_eq!(lines.iter().filter(|l| l.starts_with('[')).count(), 1);
    }
}
