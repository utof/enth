//! `cargo xtask host-render [--release] [--no-build] [--update]`: the plugin-level golden render.
//!
//! Bundles `spike-shell`, builds `enth-test-host`, renders each case in
//! `tools/test-host/golden/spike-shell.toml` through the plugin's real CLAP entry point (offline, fixed
//! block size, `process()` on one thread, no display), and compares the SHA-256 of the samples with the
//! committed hash. A mismatch means the plugin's sound, its parameter handling, its state loading or the
//! test host changed: update the golden deliberately with `--update` and say why in the commit message.
//!
//! Linux only: the bundle layout (a bare `.clap` file) and the CI job are Linux's.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::bundle;

const PACKAGE: &str = "spike-shell";
const TEST_HOST_PACKAGE: &str = "enth-test-host";
const GOLDEN: &str = "tools/test-host/golden/spike-shell.toml";

#[derive(Deserialize)]
struct Golden {
    sample_rate: f64,
    block: u32,
    duration: f64,
    #[serde(rename = "case")]
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    notes: String,
    /// A state blob in the golden directory, loaded before the render.
    state: Option<String>,
    /// Repeated `--param NAME=VALUE` options.
    #[serde(default)]
    params: Vec<String>,
    sha256: String,
}

struct Options {
    release: bool,
    build: bool,
    update: bool,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        release: false,
        build: true,
        update: false,
    };
    for arg in args {
        match arg.as_str() {
            "--release" => options.release = true,
            "--no-build" => options.build = false,
            "--update" => options.update = true,
            other => {
                return Err(format!(
                    "unknown option {other:?}; usage: cargo xtask host-render [--release] [--no-build] [--update]"
                ));
            }
        }
    }
    Ok(options)
}

pub fn host_render(args: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("host-render is Linux-only".to_owned());
    }
    let options = parse_options(args)?;
    let meta = crate::metadata()?;
    let root = meta.workspace_root;
    let target = meta.target_directory;
    let profile = if options.release { "release" } else { "debug" };

    if options.build {
        let mut bundle_args = vec![PACKAGE.to_owned(), "--locked".to_owned()];
        let mut build = crate::cargo();
        build
            .current_dir(&root)
            .args(["build", "--locked", "-p", TEST_HOST_PACKAGE]);
        if options.release {
            bundle_args.push("--release".to_owned());
            build.arg("--release");
        }
        bundle::bundle(&bundle_args)?;
        let status = build
            .status()
            .map_err(|e| format!("cannot run cargo build: {e}"))?;
        if !status.success() {
            return Err(format!("cargo build -p {TEST_HOST_PACKAGE} failed"));
        }
    }

    let clap = target.join("bundled").join(format!("{PACKAGE}.clap"));
    let host = target.join(profile).join(TEST_HOST_PACKAGE);
    for path in [&clap, &host] {
        if !path.exists() {
            return Err(format!(
                "{} does not exist (run without --no-build)",
                path.display()
            ));
        }
    }

    let golden_path = root.join(GOLDEN);
    let golden_text = std::fs::read_to_string(&golden_path)
        .map_err(|e| format!("{}: {e}", golden_path.display()))?;
    let golden: Golden =
        toml::from_str(&golden_text).map_err(|e| format!("{}: {e}", golden_path.display()))?;
    let golden_dir = golden_path.parent().unwrap_or(Path::new("."));
    let out_dir = target.join("host-render");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;

    let mut new_hashes = Vec::new();
    let mut failures = Vec::new();
    for case in &golden.cases {
        let hash = render_case(&host, &clap, &golden, case, golden_dir, &out_dir)?;
        let verdict = if hash == case.sha256 { "ok" } else { "DIFFERS" };
        println!("host-render: {:<14} {verdict}  {hash}", case.name);
        if hash != case.sha256 {
            failures.push(format!(
                "{}: expected {}, got {hash}",
                case.name, case.sha256
            ));
        }
        new_hashes.push((case.name.clone(), hash));
    }

    if options.update {
        let updated = rewrite_hashes(&golden_text, &new_hashes);
        std::fs::write(&golden_path, updated)
            .map_err(|e| format!("{}: {e}", golden_path.display()))?;
        println!(
            "host-render: updated {} hashes in {GOLDEN}",
            new_hashes.len()
        );
        return Ok(());
    }
    if failures.is_empty() {
        println!(
            "host-render: ok ({} cases, {profile} bundle, {} Hz, block {})",
            golden.cases.len(),
            golden.sample_rate,
            golden.block
        );
        Ok(())
    } else {
        Err(format!(
            "host-render failed:\n  {}\nIf the change is deliberate, run `cargo xtask host-render --update` and explain it in the commit message.",
            failures.join("\n  ")
        ))
    }
}

fn render_case(
    host: &Path,
    clap: &Path,
    golden: &Golden,
    case: &Case,
    golden_dir: &Path,
    out_dir: &Path,
) -> Result<String, String> {
    let wav: PathBuf = out_dir.join(format!("{}.wav", case.name));
    let mut cmd = Command::new(host);
    cmd.arg("render")
        .arg(clap)
        .args(["--notes", &case.notes])
        .args(["--sr", &golden.sample_rate.to_string()])
        .args(["--block", &golden.block.to_string()])
        .args(["--dur", &golden.duration.to_string()])
        .arg("--out")
        .arg(&wav)
        // Rendering needs no display; make sure nothing in the plugin can reach one.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    if let Some(state) = &case.state {
        cmd.arg("--state").arg(golden_dir.join(state));
    }
    for param in &case.params {
        cmd.args(["--param", param]);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("cannot run {}: {e}", host.display()))?;
    if !output.status.success() {
        return Err(format!(
            "case {}: render failed ({}):\n{}",
            case.name,
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("case {}: bad JSON from the test host: {e}", case.name))?;
    let out = &report["output"];
    if out["non_finite_samples"] != 0 {
        return Err(format!(
            "case {}: the render has NaN or infinite samples",
            case.name
        ));
    }
    if out["peak"].as_f64().unwrap_or(0.0) <= 0.0 {
        return Err(format!(
            "case {}: the render is silent; a golden of silence proves nothing",
            case.name
        ));
    }
    report["sha256"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("case {}: no sha256 in the report", case.name))
}

/// Replaces each case's `sha256 = "…"` line in the golden file text, leaving comments and layout alone.
fn rewrite_hashes(text: &str, hashes: &[(String, String)]) -> String {
    let mut current: Option<&str> = None;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim_start();
        if line.trim() == "[[case]]" {
            current = None;
        } else if let Some(name) = trimmed
            .strip_prefix("name = \"")
            .and_then(|r| r.strip_suffix('"'))
        {
            current = hashes
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, h)| h.as_str());
        }
        if let (Some(_), Some(hash)) = (trimmed.strip_prefix("sha256 = "), current) {
            out.push_str("sha256 = \"");
            out.push_str(hash);
            out.push_str("\"\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_replaces_only_the_named_cases_hash() {
        let text = "# header\nsample_rate = 48000\n\n[[case]]\nname = \"a\"\nnotes = \"C4@0+1\"\nsha256 = \"old-a\"\n\n[[case]]\nname = \"b\"\nsha256 = \"old-b\"\n";
        let got = rewrite_hashes(
            text,
            &[("a".into(), "new-a".into()), ("b".into(), "new-b".into())],
        );
        assert!(got.contains("sha256 = \"new-a\""));
        assert!(got.contains("sha256 = \"new-b\""));
        assert!(!got.contains("old-"));
        assert!(got.starts_with("# header\n"));
        assert!(got.contains("notes = \"C4@0+1\""));
    }
}
