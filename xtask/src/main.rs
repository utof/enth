//! Project automation: `cargo xtask <command>`. Keep dependencies minimal.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "xtask is a CLI tool"
)]

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
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check-layers") => check_layers(),
        Some("wasm-check") => wasm_check(),
        _ => Err("usage: cargo xtask <check-layers|wasm-check>".to_owned()),
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
