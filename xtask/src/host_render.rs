//! `cargo xtask host-render [--package <pkg>] [--release] [--no-build] [--update]`: the plugin-level golden render.
//!
//! Bundles the package (default: see `plugin::default_package`), builds `enth-test-host`, renders each case in
//! `tools/test-host/golden/<package>.toml` through the plugin's real CLAP entry point (offline, fixed
//! block size, `process()` on one thread, no display), and compares the SHA-256 of the samples with the
//! committed hash. A mismatch means the plugin's sound, its parameter handling, its state loading or the
//! test host changed: update the golden deliberately with `--update` and say why in the commit message.
//!
//! Then, for a package with a `crates/<package>/tests/cli-equal.profile.toml` (enth-plugin), **CLI = plugin**: the
//! engine's own golden cases (`tests/golden`) rendered through the plugin at host blocks 1, 64, 100 and 4096 must give
//! the engine's hashes (`enth-test-host cli-equal --min-compared 60`; P1b lane INT wired it here, so `gate.sh full`
//! and CI's `host-render` job check it on every plugin change instead of only by hand).
//!
//! Linux only: the bundle layout (a bare `.clap` file) and the CI job are Linux's.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::{bundle, plugin};

const TEST_HOST_PACKAGE: &str = "enth-test-host";

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

const USAGE: &str =
    "usage: cargo xtask host-render [--package <pkg>] [--release] [--no-build] [--update]";

struct Options {
    package: String,
    release: bool,
    build: bool,
    update: bool,
}

fn parse_options(args: &[String], root: &Path) -> Result<Options, String> {
    let (package, rest) = plugin::one_package(plugin::split_packages(args, false)?, root)?;
    let mut options = Options {
        package,
        release: false,
        build: true,
        update: false,
    };
    for arg in &rest {
        match arg.as_str() {
            "--release" => options.release = true,
            "--no-build" => options.build = false,
            "--update" => options.update = true,
            other => {
                return Err(format!("unknown option {other:?}; {USAGE}"));
            }
        }
    }
    Ok(options)
}

pub fn host_render(args: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("host-render is Linux-only".to_owned());
    }
    let meta = crate::metadata()?;
    let options = parse_options(args, &meta.workspace_root)?;
    plugin::require_member(&meta, &options.package)?;
    let package = options.package.as_str();
    let root = meta.workspace_root;
    let target = meta.target_directory;
    let profile = if options.release { "release" } else { "debug" };
    println!("host-render: package {package}");

    if options.build {
        let mut bundle_args = vec![package.to_owned(), "--locked".to_owned()];
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

    let clap = plugin::clap_bundle(&target, package)?;
    let host = target.join(profile).join(TEST_HOST_PACKAGE);
    if !host.exists() {
        return Err(format!(
            "{} does not exist (run without --no-build)",
            host.display()
        ));
    }

    let golden_file = plugin::host_golden(package);
    let golden_path = root.join(&golden_file);
    let golden_text = std::fs::read_to_string(&golden_path).map_err(|e| {
        format!(
            "{}: {e} (host-render compares {package} with the committed hashes in {golden_file})",
            golden_path.display()
        )
    })?;
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
            "host-render: updated {} hashes in {golden_file}",
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
        let profile_file = root
            .join("crates")
            .join(package)
            .join("tests")
            .join("cli-equal.profile.toml");
        if profile_file.exists() {
            cli_equal(&host, &clap, &root, &profile_file)?;
        }
        Ok(())
    } else {
        Err(format!(
            "host-render failed:\n  {}\nIf the change is deliberate, run `cargo xtask host-render --update` and explain it in the commit message.",
            failures.join("\n  ")
        ))
    }
}

/// The engine's golden cases through the plugin must give the engine's hashes, at every host block size, with at least
/// 60 cases compared (the test host skips the ones a plugin cannot play; 63 are compared today).
const CLI_EQUAL_MIN_COMPARED: &str = "60";

fn cli_equal(host: &Path, clap: &Path, root: &Path, profile: &Path) -> Result<(), String> {
    let output = Command::new(host)
        .current_dir(root)
        .arg("cli-equal")
        .arg(clap)
        .arg("--profile")
        .arg(profile)
        .args(["--min-compared", CLI_EQUAL_MIN_COMPARED])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .map_err(|e| format!("cannot run {}: {e}", host.display()))?;
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let count = |key: &str| {
        report[key]
            .as_u64()
            .map_or("?".to_owned(), |n| n.to_string())
    };
    let summary = format!(
        "{} compared, {} equal, {} differ, {} skipped",
        count("compared"),
        count("equal"),
        count("differs"),
        count("skipped")
    );
    if output.status.success() {
        println!("host-render: cli-equal ok ({summary}; the engine's goldens through the plugin)");
        Ok(())
    } else {
        Err(format!(
            "host-render: cli-equal failed ({}; {summary}): the plugin no longer renders the engine's golden cases like \
             the CLI. Rerun: enth-test-host cli-equal {} --profile {}\n{}",
            output.status,
            clap.display(),
            profile.display(),
            String::from_utf8_lossy(&output.stderr)
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
