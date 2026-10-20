//! Which plugin package the plugin-level commands (`bundle`, `install`, `host-render`, `reaper-test`,
//! `validate`) act on, and what they need to know about it.
//!
//! Without `--package` they use [`default_package`], the product plugin `enth-plugin`. Until P1b lane SK exported its
//! CLAP entry the default was the Phase 0 spike `spike-shell`; lane INT retired the shell and UI spikes from the
//! workspace, so `enth-plugin` is the only plugin package left.

use std::path::{Path, PathBuf};

/// The product plugin (`crates/enth-plugin`).
pub const PRODUCT_PACKAGE: &str = "enth-plugin";

/// The packages left after taking `--package` options out of an argument list.
#[derive(Debug, PartialEq, Eq)]
pub struct Split {
    pub packages: Vec<String>,
    pub rest: Vec<String>,
}

/// Takes `--package <p>`, `--package=<p>` and `-p <p>` (any number, any position) out of `args`. With
/// `leading_positional`, a first argument that does not start with `-` is a package too: the older
/// `cargo xtask bundle <package> [cargo options]` form.
pub fn split_packages(args: &[String], leading_positional: bool) -> Result<Split, String> {
    let mut packages = Vec::new();
    let mut rest = Vec::new();
    let mut iter = args.iter().enumerate();
    while let Some((i, arg)) = iter.next() {
        if arg == "--package" || arg == "-p" {
            let value = iter
                .next()
                .map(|(_, v)| v)
                .filter(|v| !v.starts_with('-'))
                .ok_or_else(|| format!("{arg} needs a package name"))?;
            packages.push(value.clone());
        } else if let Some(value) = arg.strip_prefix("--package=") {
            packages.push(value.to_owned());
        } else if i == 0 && leading_positional && !arg.starts_with('-') {
            packages.push(arg.clone());
        } else {
            rest.push(arg.clone());
        }
    }
    Ok(Split { packages, rest })
}

/// The package a single-package command acts on, and its remaining options.
pub fn one_package(split: Split, root: &Path) -> Result<(String, Vec<String>), String> {
    let Split { mut packages, rest } = split;
    match packages.len() {
        0 => Ok((default_package(root), rest)),
        1 => Ok((packages.remove(0), rest)),
        _ => Err(format!(
            "this command takes one --package, got {}",
            packages.join(", ")
        )),
    }
}

/// The plugin a command acts on without `--package`: `enth-plugin`.
pub fn default_package(_root: &Path) -> String {
    PRODUCT_PACKAGE.to_owned()
}

/// Every command that takes a package takes a workspace member.
pub fn require_member(meta: &crate::Metadata, package: &str) -> Result<(), String> {
    let known = meta
        .packages
        .iter()
        .filter(|p| meta.workspace_members.contains(&p.id))
        .any(|p| p.name == package);
    if known {
        Ok(())
    } else {
        Err(format!(
            "unknown package {package:?}: not a workspace member (the plugin: {PRODUCT_PACKAGE})"
        ))
    }
}

/// The `.clap` file `bundle` leaves for `package`, or the reason there is none.
pub fn clap_bundle(target: &Path, package: &str) -> Result<PathBuf, String> {
    let path = target.join("bundled").join(format!("{package}.clap"));
    if path.exists() {
        Ok(path)
    } else {
        Err(format!(
            "{} does not exist: bundle it first (leave out --no-build) and check that {package} exports a plugin (nice_export_clap!)",
            path.display()
        ))
    }
}

/// The plugin-level golden file `host-render` compares `package` against.
pub fn host_golden(package: &str) -> String {
    format!("tools/test-host/golden/{package}.toml")
}

/// How REAPER lists the plugin: `TrackFX_AddByName` matches this as a substring of the plugin's display name.
/// `--plugin-name` overrides it. REAPER lists the product as `CLAPi: enth (enth)` (name, then vendor); the vendor
/// keeps "enth" from also matching another plugin whose name starts with it, such as a Phase 0 spike still installed
/// in `~/.clap` (`enth spike-shell (enth)`).
pub fn reaper_plugin_name(package: &str) -> String {
    match package {
        PRODUCT_PACKAGE => "enth (enth)".to_owned(),
        _ => "enth".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn packages_come_out_of_any_position_and_any_spelling() {
        let split = split_packages(
            &strings(&[
                "--release",
                "-p",
                "a",
                "--package",
                "b",
                "--package=c",
                "--locked",
            ]),
            false,
        )
        .unwrap();
        assert_eq!(split.packages, strings(&["a", "b", "c"]));
        assert_eq!(split.rest, strings(&["--release", "--locked"]));
    }

    #[test]
    fn a_leading_positional_is_a_package_only_when_asked() {
        let args = strings(&["enth-plugin", "--locked"]);
        let with = split_packages(&args, true).unwrap();
        assert_eq!(with.packages, strings(&["enth-plugin"]));
        assert_eq!(with.rest, strings(&["--locked"]));
        let without = split_packages(&args, false).unwrap();
        assert_eq!(without.packages, Vec::<String>::new());
        assert_eq!(without.rest, args);
    }

    #[test]
    fn a_package_flag_without_a_name_is_an_error() {
        assert!(split_packages(&strings(&["--package"]), false).is_err());
        assert!(split_packages(&strings(&["-p", "--release"]), false).is_err());
    }

    #[test]
    fn one_package_defaults_and_refuses_two() {
        let root = Path::new("/nonexistent-enth-root");
        let (p, rest) = one_package(
            split_packages(&strings(&["--release"]), false).unwrap(),
            root,
        )
        .unwrap();
        assert_eq!(p, PRODUCT_PACKAGE);
        assert_eq!(rest, strings(&["--release"]));
        let two = split_packages(&strings(&["-p", "a", "-p", "b"]), false).unwrap();
        assert!(one_package(two, root).is_err());
    }
}
