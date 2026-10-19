//! `cargo xtask validate [--package <pkg>] [--release] [--no-build] [-- <clap-validator options>]`: the
//! pinned clap-validator on a plugin bundle.
//!
//! The validator is the release binary of clap-validator 0.4.1, downloaded once into `target/tools/` and checked
//! against a pinned SHA-256 before it is unpacked or run (the same pin as the CI job). The bundle is the debug one
//! unless `--release` is given: debug builds have nice-plug's `assert_process_allocs` on, so any allocation in
//! `process()` aborts the validator's process tests. clap-validator 0.4.1 has no GUI tests; the child still gets no
//! display variables, because nothing here may open a window on the real display.
//!
//! Linux x86-64 only (the release asset is Ubuntu's); the code compiles everywhere.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::{bundle, plugin};

const VERSION: &str = "0.4.1";
/// The zip's inner tarball is misnamed (free-audio/clap-validator#39); the binary inside is 0.4.1.
const ZIP: &str = "clap-validator-0.4.1-127-g152b982-ubuntu-22.04.zip";
const ZIP_SHA256: &str = "49edadcfb407ea0dd946ce418300e853fbd2660fa4b0d00e4f19ff8eef24ad90";
const URL_BASE: &str = "https://github.com/free-audio/clap-validator/releases/download";
/// A generic User-Agent: nothing about the machine or its owner is sent.
const USER_AGENT: &str = "enth-research/0.1";

const USAGE: &str = "usage: cargo xtask validate [--package <pkg>] [--release] [--no-build] [-- <clap-validator options>]";

struct Options {
    package: String,
    release: bool,
    build: bool,
    /// Passed through to `clap-validator validate`, after `--`.
    validator_args: Vec<String>,
}

fn parse_options(args: &[String], root: &Path) -> Result<Options, String> {
    let (own, passthrough) = match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], args[i + 1..].to_vec()),
        None => (args, Vec::new()),
    };
    let (package, rest) = plugin::one_package(plugin::split_packages(own, false)?, root)?;
    let mut options = Options {
        package,
        release: false,
        build: true,
        validator_args: passthrough,
    };
    for arg in &rest {
        match arg.as_str() {
            "--release" => options.release = true,
            "--no-build" => options.build = false,
            other => return Err(format!("unknown option {other:?}; {USAGE}")),
        }
    }
    Ok(options)
}

pub fn validate(args: &[String]) -> Result<(), String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err(
            "validate is Linux x86-64 only: the pinned clap-validator release asset is Ubuntu's"
                .to_owned(),
        );
    }
    let meta = crate::metadata()?;
    let options = parse_options(args, &meta.workspace_root)?;
    plugin::require_member(&meta, &options.package)?;
    let package = options.package.as_str();
    let profile = if options.release { "release" } else { "debug" };
    println!("validate: package {package}, {profile} bundle");

    let validator = ensure_validator(&meta.target_directory.join("tools"))?;

    if options.build {
        let mut bundle_args = vec![package.to_owned(), "--locked".to_owned()];
        if options.release {
            bundle_args.push("--release".to_owned());
        }
        bundle::bundle(&bundle_args)?;
    }
    let clap = plugin::clap_bundle(&meta.target_directory, package)?;

    let status = Command::new(&validator)
        .arg("validate")
        .arg(&clap)
        .arg("--only-failed")
        .args(&options.validator_args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .status()
        .map_err(|e| format!("cannot run {}: {e}", validator.display()))?;
    if status.success() {
        println!("validate: ok ({package}, {profile} bundle, clap-validator {VERSION})");
        Ok(())
    } else {
        Err(format!(
            "validate failed: clap-validator {VERSION} reported failures for {package} ({status})"
        ))
    }
}

/// The path of the pinned clap-validator under `tools`, downloaded and unpacked first if it is not there yet.
fn ensure_validator(tools: &Path) -> Result<PathBuf, String> {
    let dir = tools.join(format!("clap-validator-{VERSION}"));
    let binary = dir.join("clap-validator");
    // Written after a successful unpack; names the zip the binary came from.
    let stamp = dir.join(".unpacked-from-sha256");
    let unpacked = binary.is_file()
        && std::fs::read_to_string(&stamp).is_ok_and(|text| text.trim() == ZIP_SHA256);
    if unpacked {
        return Ok(binary);
    }

    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let zip = dir.join(ZIP);
    if !file_has_sha256(&zip, ZIP_SHA256) {
        download(&format!("{URL_BASE}/{VERSION}/{ZIP}"), &zip)?;
    }
    // Checked again after the download (and on a zip left by an earlier run): nothing is unpacked or run unless
    // the bytes match the pin.
    let actual = file_sha256(&zip)?;
    if actual != ZIP_SHA256 {
        let _ = std::fs::remove_file(&zip);
        return Err(format!(
            "{ZIP}: SHA-256 is {actual}, expected {ZIP_SHA256}; the file was removed. Not unpacking."
        ));
    }

    run_tool(
        Command::new("unzip")
            .arg("-q")
            .arg("-o")
            .arg(&zip)
            .arg("-d")
            .arg(&dir),
        "unzip",
    )?;
    let tarball = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.to_string_lossy().ends_with(".tar.gz"))
        .ok_or_else(|| format!("no .tar.gz inside {ZIP}"))?;
    run_tool(
        Command::new("tar")
            .arg("-xzf")
            .arg(&tarball)
            .arg("-C")
            .arg(&dir),
        "tar",
    )?;
    if !binary.is_file() {
        return Err(format!(
            "{} is missing after unpacking {ZIP}",
            binary.display()
        ));
    }
    let output = Command::new(&binary)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    let version = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !version.contains(VERSION) {
        return Err(format!(
            "{} --version printed {:?}, expected a line with {VERSION}",
            binary.display(),
            version.trim()
        ));
    }
    std::fs::write(&stamp, format!("{ZIP_SHA256}\n"))
        .map_err(|e| format!("{}: {e}", stamp.display()))?;
    println!("validate: unpacked {}", version.trim());
    Ok(binary)
}

/// Downloads `url` to `destination` through `curl` (HTTPS only, redirects followed, generic User-Agent); the
/// bytes land in a temporary file first, so a failed download never leaves a half-written zip.
fn download(url: &str, destination: &Path) -> Result<(), String> {
    println!("validate: downloading {url}");
    let partial = destination.with_extension("zip.part");
    run_tool(
        Command::new("curl")
            .args(["--fail", "--location", "--silent", "--show-error"])
            .args(["--proto", "=https", "--proto-redir", "=https"])
            .args(["--user-agent", USER_AGENT])
            .arg("--output")
            .arg(&partial)
            .arg(url),
        "curl",
    )?;
    std::fs::rename(&partial, destination)
        .map_err(|e| format!("{} -> {}: {e}", partial.display(), destination.display()))
}

fn run_tool(command: &mut Command, name: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|e| format!("cannot run `{name}` (is it installed?): {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{name}` failed ({status})"))
    }
}

fn file_has_sha256(path: &Path, expected: &str) -> bool {
    file_sha256(path).is_ok_and(|actual| actual == expected)
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(hex(&Sha256::digest(&bytes)))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn sha256_matches_the_standard_test_vector() {
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_pin_is_a_sha256_and_names_the_pinned_version() {
        assert_eq!(ZIP_SHA256.len(), 64);
        assert!(ZIP_SHA256.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(ZIP.starts_with(&format!("clap-validator-{VERSION}")));
    }

    #[test]
    fn a_zip_with_the_wrong_bytes_does_not_match() {
        let path =
            std::env::temp_dir().join(format!("enth-xtask-validate-test-{}", std::process::id()));
        std::fs::write(&path, b"not the validator").unwrap();
        assert!(!file_has_sha256(&path, ZIP_SHA256));
        std::fs::remove_file(&path).unwrap();
        assert!(
            !file_has_sha256(&path, ZIP_SHA256),
            "a missing file never matches"
        );
    }

    #[test]
    fn options_split_at_the_double_dash() {
        let root = Path::new("/nonexistent-enth-root");
        let o = parse_options(
            &strings(&["-p", "x", "--release", "--", "--test-filter", "state"]),
            root,
        )
        .unwrap();
        assert_eq!(o.package, "x");
        assert!(o.release && o.build);
        assert_eq!(o.validator_args, strings(&["--test-filter", "state"]));
        assert!(parse_options(&strings(&["--bogus"]), root).is_err());
    }
}
