//! Which plugin package the plugin-level commands (`bundle`, `install`, `host-render`, `reaper-test`,
//! `validate`) act on, and what they need to know about it.
//!
//! Without `--package` they use [`default_package`]: the Phase 0 spike `spike-shell` until the product plugin
//! `enth-plugin` (P1b lane SK) exports a CLAP entry, and `enth-plugin` from then on. So the day SK's
//! `nice_export_clap!` line lands, the gates (`scripts/gate.sh full`, CI) start covering the product with
//! no further edit; the plugin's golden file `tools/test-host/golden/enth-plugin.toml` must exist by then.

use std::path::{Path, PathBuf};

/// The default until the product plugin exists.
pub const SPIKE_PACKAGE: &str = "spike-shell";
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

/// `enth-plugin` once it exports a CLAP entry, else `spike-shell`.
pub fn default_package(root: &Path) -> String {
    if product_exports_clap(root) {
        PRODUCT_PACKAGE.to_owned()
    } else {
        SPIKE_PACKAGE.to_owned()
    }
}

/// Whether `crates/enth-plugin/src` has a line that starts with the `nice_export_clap!(` macro call. A mention
/// inside a string or a comment does not count.
fn product_exports_clap(root: &Path) -> bool {
    let mut dirs = vec![root.join("crates").join(PRODUCT_PACKAGE).join("src")];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && std::fs::read_to_string(&path).is_ok_and(|text| {
                    text.lines()
                        .any(|l| l.trim_start().starts_with("nice_export_clap!("))
                })
            {
                return true;
            }
        }
    }
    false
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
            "unknown package {package:?}: not a workspace member (plugins: {SPIKE_PACKAGE}, spike-ui-plugin, {PRODUCT_PACKAGE})"
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
/// keeps "enth" from also matching `enth spike-shell (enth)` when both are bundled (lane SK).
pub fn reaper_plugin_name(package: &str) -> String {
    match package {
        "spike-shell" => "enth spike-shell".to_owned(),
        "spike-ui-plugin" => "enth spike-ui".to_owned(),
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
        let args = strings(&["spike-shell", "--locked"]);
        let with = split_packages(&args, true).unwrap();
        assert_eq!(with.packages, strings(&["spike-shell"]));
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
        assert_eq!(p, SPIKE_PACKAGE);
        assert_eq!(rest, strings(&["--release"]));
        let two = split_packages(&strings(&["-p", "a", "-p", "b"]), false).unwrap();
        assert!(one_package(two, root).is_err());
    }

    #[test]
    fn the_export_macro_is_found_only_as_a_call() {
        let dir =
            std::env::temp_dir().join(format!("enth-xtask-plugin-test-{}", std::process::id()));
        let src = dir.join("crates").join(PRODUCT_PACKAGE).join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "#![allow(unsafe_code, reason = \"nice_export_clap!(X) expands to entry points\")]\n// nice_export_clap!(X);\n",
        )
        .unwrap();
        assert_eq!(default_package(&dir), SPIKE_PACKAGE);
        std::fs::write(src.join("lib.rs"), "nice_export_clap!(Enth);\n").unwrap();
        assert_eq!(default_package(&dir), PRODUCT_PACKAGE);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
