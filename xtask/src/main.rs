//! Project automation: `cargo xtask <command>`. Keep dependencies minimal.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "xtask is a CLI tool"
)]

mod affected;
mod agent_eval;
mod bench;
mod bundle;
mod engine;
mod golden;
mod gr1_report;
mod host_render;
mod leak;
mod plugin;
mod preset;
mod preset_add;
mod reaper;
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
  bundle [<pkg>|--package <pkg>]... [--release] [...]
                                    build plugins and create .clap / .vst3 bundles in target/bundled
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
  host-render [--package <pkg>] [--release] [--no-build] [--update]
                                    plugin-level golden: bundle <pkg>, render it offline through the test host (Linux)
  leak-test [--package <pkg>] [--cycles 200] [--repeat 2] [--churn-cycles 200] [--only editor|churn] [--no-build]
            [--bundle-dir <dir>]
                                    the leak gates under Xvfb (started here): editor-cycles (at most 26 KB per open/close
                                    cycle, threads constant, one editor_closed per open; release bundle with the
                                    editor-counts feature) and instance churn (flat PSS, no thread or descriptor left)
  extract-presets <project.rpp>... [--out DIR] [--name NAME] [--force] [--dry-run]
                                    write each enth instance's saved patch in REAPER projects to
                                    content/presets/<track or FX name>.patch.json, byte for byte (owner build 2)
  starter-project --out FILE.rpp [--tracks bass,pad,pluck,lead] [--patch FILE] [--force]
                                    a REAPER project with one enth (CLAP) per named track, loaded with the starter patch
                                    (every P1 module; the editor cannot add modules yet): where owner build 2 starts
  preset-add FILE --name NAME --tags T1,T2
                                    (not built yet: R1/S1 lane PR) normalise a drafted patch into content/presets/
                                    as exactly Patch::to_json(), with its meta and the engine stamp
  gr1-report <round log>...         (not built yet: R1/S1 lane GK) the GR1 numbers from the friends' round logs
  wasm-hash [--simd128]             spike-engine render and patch hashes: native = wasm32-wasip1 = golden
  golden [--update] [--wasm]        golden renders of tests/golden: compare every case (--wasm: under wasmtime) or
                                    rewrite the expected files (refused under CI; prints the changed cases)
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
        Some("host-render") => host_render::host_render(&args[1..]),
        Some("leak-test") => leak::leak_test(&args[1..]),
        Some("extract-presets") => preset::extract_presets(&args[1..]),
        Some("starter-project") => preset::write_starter_project(&args[1..]),
        Some("preset-add") => preset_add::preset_add(&args[1..]),
        Some("gr1-report") => gr1_report::gr1_report(&args[1..]),
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
