//! Project automation: `cargo xtask <command>`. Keep dependencies minimal.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "xtask is a CLI tool"
)]

mod affected;
mod agent_eval;
mod bank;
mod bench;
mod bundle;
mod engine;
mod golden;
mod gr1_report;
mod gr2;
mod gr3;
mod host_render;
mod leak;
mod lesson;
mod plugin;
mod preset;
mod preset_add;
mod reaper;
mod rtsan;
mod validate;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::Deserialize;

#[derive(Deserialize)]
struct LayersFile {
    layers: BTreeMap<String, u8>,
    core: Core,
    exempt: Exempt,
}

#[derive(Deserialize)]
struct Core {
    max_layer: u8,
}

#[derive(Deserialize)]
struct Exempt {
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    prefixes: Vec<String>,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    workspace_root: PathBuf,
    target_directory: PathBuf,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: PathBuf,
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    kind: Option<String>,
    path: Option<PathBuf>,
}

enum Class {
    Layer(u8),
    Exempt,
    Unlisted,
}

impl LayersFile {
    fn classify(&self, name: &str) -> Class {
        if let Some(&layer) = self.layers.get(name) {
            Class::Layer(layer)
        } else if self.exempt.packages.iter().any(|p| p == name)
            || self
                .exempt
                .prefixes
                .iter()
                .any(|p| name.starts_with(p.as_str()))
        {
            Class::Exempt
        } else {
            Class::Unlisted
        }
    }
}

const USAGE: &str = "usage: cargo xtask <command>
  check-layers                      workspace members only depend on lower layers
  wasm-check                        core crates build for wasm32-unknown-unknown
  affected [--base <rev>]           the workspace crates a change touches, plus their dependants (one per line;
                                    the change is the working tree against the merge base with main; used by
                                    `scripts/gate.sh lane`)
  bundle [<pkg>|--package <pkg>]... [--release] [--universal] [...]
                                    build plugins and create .clap / .vst3 bundles in target/bundled
                                    (--universal, macOS only: both Apple targets merged with lipo; release.yml)
  install [<pkg>|--package <pkg>]... [--release] [...]
                                    bundle, then copy to ~/.clap and ~/.vst3
  validate [--package <pkg>] [--release] [--no-build] [-- <clap-validator options>]
                                    bundle, then run the pinned clap-validator 0.4.1 (downloaded into target/tools/,
                                    sha256-checked) on the .clap file; the debug bundle has assert_process_allocs on
  reaper-test [--package <pkg>] [--plugin-name <name>] [--release] [--no-build] [--bundle-dir <dir>]
              [--format clap|vst3] [--scenario <name>]...
                                    local gate: the REAPER scenarios (state, patch-roundtrip, slot-automation,
                                    macro-automation, mod-wheel, undo-state-load, two-instances, editor-smoke,
                                    dirty-flag, starter-project, cross-format) on CLAP and VST3, headless under Xvfb;
                                    --bundle-dir (or
                                    ENTH_BUNDLE_DIR) runs prebuilt bundles instead of building
  reaper-soak [--minutes N] [--bundle-dir <dir>] [--no-build] [--format clap|vst3] [--instances N] [--report FILE]
              [--editor-secs S] [--load-secs S] [--churn-secs S] [--max-pss-mb-per-hour MB] [...]
                                    local gate: the long REAPER soak on the debug bundle (assert_process_allocs on), headless
                                    under Xvfb: virtual-keyboard notes, parameter sweeps, state loads, instance churn and
                                    editor open/close for N minutes (default 60); samples PSS, threads and descriptors; fails
                                    on a crash, an allocation abort, a hang or growth past the limits; writes soak-report.json
  host-render [--package <pkg>] [--release] [--no-build] [--update]
                                    plugin-level golden: bundle <pkg>, render it offline through the test host (Linux)
  leak-test [--package <pkg>] [--cycles 200] [--repeat 2] [--churn-cycles 200] [--only editor|churn] [--no-build]
            [--bundle-dir <dir>]
                                    the leak gates under Xvfb (started here): editor-cycles (at most 26 KB per open/close
                                    cycle, threads constant, one editor_closed per open; release bundle with the
                                    editor-counts feature) and instance churn (flat PSS, no thread or descriptor left)
  rtsan [--toolchain nightly] [--secs 10] [--no-build] [--only CHECK]...
                                    the plugin's process() under RealtimeSanitizer in the test host (both built with
                                    a nightly and -Zsanitizer=realtime): renders, state-load swaps, floods, A/B,
                                    bursts and note ends must be clean, and a deliberate allocation must be caught
  extract-presets <project.rpp>... [--out DIR] [--name NAME] [--force] [--dry-run]
                                    write each enth instance's saved patch in REAPER projects to
                                    content/presets/<track or FX name>.patch.json, named and typed after the track
                                    instead of Init (only the metadata changes; owner build 2)
  starter-project --out FILE.rpp [--tracks bass,pad,pluck,lead] [--patch FILE] [--force]
                                    a REAPER project with one enth (CLAP) per named track, loaded with the starter patch
                                    (Init: every P1 module, the extras off, gain-staged): where owner build 2 starts
  preset-add FILE [--name NAME] [--tags TYPE,WORD,...] [--out DIR] [--folder SUB] [--force] [--move]
  preset-add --promote NAME[,NAME...] | --picks PICKS.md [--to DIR] [--force] [--move]
                                    normalise a CLI-drafted patch into a preset (exactly Patch::to_json(), name,
                                    tags from the factory vocabulary, engine stamp, unique name) under
                                    content/presets-candidates/<type>/; promote a picked candidate into
                                    content/presets/<type>/ (`preset-add --help`; R1/S1 lane PR)
  gr1-report [options] <round log>...
                                    the GR1 numbers from the friends' round logs (enth.round-log/1): rounds, pass
                                    rate by level, time to pass, assists, agreement, blind rating vs score, disputes,
                                    want more, and the agent baseline beside the no-op and CMA-ES floors; --json,
                                    --validate, --codes; `gr1-report --help`, docs/gr1.md (R1/S1 lane GK)
  lesson-check [--cross] [--solve] [--budget N] [--jobs N] [--lesson ID]... [<pack>...]
                                    every lesson of content/lessons/ honest: the files, each step's check fails at its
                                    start and passes after its witness through a session, --cross, --solve (R2 lane LCK)
  lesson-coverage [--all] [<pack>...]
                                    every P1 module and Basic-tier parameter taught, a non-patch check per module;
                                    fails on a hole (R2 lane LCK)
  lesson new [--in DIR] <pack> <id> scaffold a lesson in the authoring layout (R2 lane LCK)
  lesson-gen deconstruct <preset>   a deconstruct lesson from a preset, by ablation (`lesson-gen studies`: the six studies)
  pack [--out DIR] [--check] [<pack>...]
                                    build content/lessons/<pack> into a reproducible .enthpack and compare it with its
                                    pins (R2 lane LCK)
  gr2-report [options] <dir>...     the GR2 numbers from what each participant sent back (answers, progress file,
                                    round log): pre/mid/post-test items, families better or worse, lessons, votes,
                                    notes, the proposed criteria; --json, --notes, --validate; docs/r2-owner.md (R2 INT2)
  gr3-run --arm A|B|C|D [options]   GR3's four arms over a held-out set of content/gr3/ (wall-clock or render
                                    budget); test runs append to content/gr3/runs.jsonl (R3 lane EVK, docs/gr3.md)
  gr3-report [--run R | --a R --b R] GR3's clause-1 endpoints and decision from content/gr3/runs.jsonl (lane EVK)
  bank [options]                    (not built yet: R3 lane TP) build the k-NN retrieval bank (drives `enth-datagen bank`)
  wasm-hash [--simd128]             spike-engine render and patch hashes: native = wasm32-wasip1 = golden
  golden [--update] [--wasm]        golden renders of tests/golden: compare every case (--wasm: under wasmtime) or
                                    rewrite the expected files (refused under CI; prints the changed cases)
  golden --challenges [--only <c>]  the golden challenges of tests/golden/challenges (same flags; --update only adds
                                    pins, a pinned result changes with its scorer id; R1/S1 lane GC)
  bench [-p <pkg>] [--bench <name>] gungraun instruction budgets (default package enth-engine; alias: budget)
  agent-eval [--dry-run] [...]      the agent-eval harness: agents design sounds with `enth` (--dry-run: scripted fake
                                    agent, no model call; a real run needs --confirm-spend); see `agent-eval --help`

--package defaults to enth-plugin, the one plugin in the workspace (src/plugin.rs).";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check-layers") => check_layers(),
        Some("wasm-check") => wasm_check(),
        Some("affected") => affected::affected(&args[1..]),
        Some("bundle") => bundle::bundle(&args[1..]),
        Some("install") => bundle::install(&args[1..]),
        Some("validate") => validate::validate(&args[1..]),
        Some("reaper-test") => reaper::reaper_test(&args[1..]),
        Some("reaper-soak") => reaper::reaper_soak(&args[1..]),
        Some("host-render") => host_render::host_render(&args[1..]),
        Some("leak-test") => leak::leak_test(&args[1..]),
        Some("rtsan") => rtsan::rtsan(&args[1..]),
        Some("extract-presets") => preset::extract_presets(&args[1..]),
        Some("starter-project") => preset::write_starter_project(&args[1..]),
        Some("preset-add") => preset_add::preset_add(&args[1..]),
        Some("gr1-report") => gr1_report::gr1_report(&args[1..]),
        Some("gr2-report") => gr2::gr2_report(&args[1..]),
        Some("gr3-run") => gr3::gr3_run(&args[1..]),
        Some("gr3-report") => gr3::gr3_report(&args[1..]),
        Some("bank") => bank::bank(&args[1..]),
        Some("lesson-check") => lesson::lesson_check(&args[1..]),
        Some("lesson-coverage") => lesson::lesson_coverage(&args[1..]),
        Some("lesson") => lesson::lesson(&args[1..]),
        Some("lesson-gen") => lesson::lesson_gen(&args[1..]),
        Some("pack") => lesson::pack(&args[1..]),
        Some("wasm-hash") => engine::wasm_hash(&args[1..]),
        Some("golden") => golden::golden(&args[1..]),
        Some("bench" | "budget") => bench::bench(&args[1..]),
        Some("agent-eval") => agent_eval::agent_eval(&args[1..]),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("xtask: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn metadata() -> Result<Metadata, String> {
    let out = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("bad cargo metadata JSON: {e}"))
}

fn layers(root: &Path) -> Result<LayersFile, String> {
    let path = root.join("xtask/layers.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn check_layers() -> Result<(), String> {
    let meta = metadata()?;
    let cfg = layers(&meta.workspace_root)?;
    let members: BTreeMap<&str, &Package> = meta
        .packages
        .iter()
        .filter(|p| meta.workspace_members.contains(&p.id))
        .map(|p| (p.name.as_str(), p))
        .collect();
    let mut errors = Vec::new();
    for (&name, pkg) in &members {
        let my_layer = match cfg.classify(name) {
            Class::Layer(l) => l,
            Class::Exempt => continue,
            Class::Unlisted => {
                errors.push(format!(
                    "{name}: workspace member is neither layered nor exempt in xtask/layers.toml"
                ));
                continue;
            }
        };
        for dep in pkg.dependencies.iter().filter(|d| d.path.is_some()) {
            if !members.contains_key(dep.name.as_str()) {
                continue;
            }
            let kind = dep.kind.as_deref().unwrap_or("normal");
            match cfg.classify(&dep.name) {
                Class::Layer(dep_layer) if dep_layer < my_layer => {}
                Class::Layer(dep_layer) => errors.push(format!(
                    "{name} (L{my_layer}) -> {} (L{dep_layer}) [{kind}]: dependencies must point to a strictly lower layer",
                    dep.name
                )),
                Class::Exempt | Class::Unlisted => errors.push(format!(
                    "{name} (L{my_layer}) -> {} [{kind}]: layered crates must not depend on tooling or unlisted crates",
                    dep.name
                )),
            }
        }
    }
    if errors.is_empty() {
        println!("check-layers: ok ({} workspace members)", members.len());
        Ok(())
    } else {
        Err(format!("check-layers failed:\n  {}", errors.join("\n  ")))
    }
}

fn wasm_check() -> Result<(), String> {
    let meta = metadata()?;
    let cfg = layers(&meta.workspace_root)?;
    let core: Vec<&str> = meta
        .packages
        .iter()
        .filter(|p| meta.workspace_members.contains(&p.id))
        .filter(|p| matches!(cfg.classify(&p.name), Class::Layer(l) if l <= cfg.core.max_layer))
        .map(|p| p.name.as_str())
        .collect();
    if core.is_empty() {
        println!("wasm-check: no core crates yet");
        return Ok(());
    }
    let mut cmd = cargo();
    cmd.args(["build", "--lib", "--target", "wasm32-unknown-unknown"]);
    for name in &core {
        cmd.args(["-p", name]);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run cargo build: {e}"))?;
    if status.success() {
        println!("wasm-check: ok ({})", core.join(", "));
        Ok(())
    } else {
        Err("wasm-check failed: a core crate does not build for wasm32-unknown-unknown".to_owned())
    }
}
