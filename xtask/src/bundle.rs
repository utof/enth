//! `cargo xtask bundle` and `cargo xtask install`.
//!
//! Bundling is nice-plug's own bundler (`nice-plug-xtask`): it builds the package, detects which
//! plugin formats the library exports and writes `<pkg>.clap` and `<pkg>.vst3` under
//! `target/bundled`. Install copies those into the host plugin folders.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::plugin;

/// `cargo xtask bundle [<pkg> | --package <pkg>]... [--release] [--universal] [any other cargo build option]`.
/// Without a package it bundles [`plugin::default_package`]. `--universal` (macOS only) builds `x86_64-apple-darwin`
/// and `aarch64-apple-darwin` and merges them with `lipo` (nice-plug-xtask's `bundle-universal`; `release.yml` uses
/// it for the friend build); it cannot be combined with `--target`.
pub fn bundle(args: &[String]) -> Result<(), String> {
    let (packages, rest) = packages_and_options(args)?;
    let (universal, rest) = take_flag(rest, UNIVERSAL);
    if universal && !cfg!(target_os = "macos") {
        return Err(format!(
            "{UNIVERSAL} is macOS only: it builds both Apple targets and merges them with lipo"
        ));
    }
    let nice_plug_args = std::iter::once(bundle_command(universal).to_owned())
        .chain(packages.iter().flat_map(|p| ["-p".to_owned(), p.clone()]))
        .chain(rest);
    nice_plug_xtask::main_with_args("cargo xtask", nice_plug_args).map_err(|e| format!("{e:#}"))
}

/// The option that asks for a macOS universal bundle.
const UNIVERSAL: &str = "--universal";

/// The nice-plug-xtask command behind `bundle`: a plain build for this machine, or the universal macOS one.
fn bundle_command(universal: bool) -> &'static str {
    if universal {
        "bundle-universal"
    } else {
        "bundle"
    }
}

/// Takes every occurrence of `flag` out of `args`; the bool says whether there was one.
fn take_flag(args: Vec<String>, flag: &str) -> (bool, Vec<String>) {
    let before = args.len();
    let rest: Vec<String> = args.into_iter().filter(|a| a != flag).collect();
    (rest.len() != before, rest)
}

/// The packages named in `args` (a leading `<pkg>`, `--package` or `-p`), or the default one, and the remaining
/// options.
fn packages_and_options(args: &[String]) -> Result<(Vec<String>, Vec<String>), String> {
    let meta = crate::metadata()?;
    let plugin::Split { mut packages, rest } = plugin::split_packages(args, true)?;
    if packages.is_empty() {
        packages.push(plugin::default_package(&meta.workspace_root));
    }
    for package in &packages {
        plugin::require_member(&meta, package)?;
    }
    Ok((packages, rest))
}

/// Where `bundle` leaves its output.
pub fn bundled_dir() -> Result<PathBuf, String> {
    Ok(crate::metadata()?.target_directory.join("bundled"))
}

/// `cargo xtask install [<pkg> | --package <pkg>]... [--release] [...]`: bundle, then copy the `.clap` file to
/// `~/.clap` and the `.vst3` bundle to `~/.vst3`. `ENTH_CLAP_DIR` and `ENTH_VST3_DIR` override
/// the destinations (for tests; hosts only look in the defaults).
pub fn install(args: &[String]) -> Result<(), String> {
    let (packages, _) = packages_and_options(args)?;
    bundle(args)?;
    let bundled = bundled_dir()?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let destination = |var: &str, default: &str| -> Result<PathBuf, String> {
        match (std::env::var_os(var), &home) {
            (Some(dir), _) => Ok(PathBuf::from(dir)),
            (None, Some(home)) => Ok(home.join(default)),
            (None, None) => Err(format!("neither {var} nor HOME is set")),
        }
    };
    for package in &packages {
        let mut installed = 0;
        for (extension, dir) in [
            ("clap", destination("ENTH_CLAP_DIR", ".clap")?),
            ("vst3", destination("ENTH_VST3_DIR", ".vst3")?),
        ] {
            let source = bundled.join(format!("{package}.{extension}"));
            if !source.exists() {
                continue;
            }
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let target = dir.join(source.file_name().unwrap_or_default());
            replace_with_copy(&source, &target)?;
            println!("installed {}", target.display());
            installed += 1;
        }
        if installed == 0 {
            return Err(format!(
                "nothing to install: no {package}.clap or {package}.vst3 in {}",
                bundled.display()
            ));
        }
    }
    Ok(())
}

/// Copies a file or directory tree to `target`, replacing whatever is there.
pub fn replace_with_copy(source: &Path, target: &Path) -> Result<(), String> {
    if target.is_dir() {
        std::fs::remove_dir_all(target).map_err(|e| format!("{}: {e}", target.display()))?;
    } else if target.exists() {
        std::fs::remove_file(target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    copy_tree(source, target)
}

/// Recursive copy that keeps permissions and recreates symlinks.
pub fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    let fail =
        |e: std::io::Error| format!("copy {} -> {}: {e}", source.display(), target.display());
    let meta = std::fs::symlink_metadata(source).map_err(fail)?;
    if meta.is_symlink() {
        let link = std::fs::read_link(source).map_err(fail)?;
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(link, target);
        // Plugin bundles are built and installed on Linux; elsewhere a symlink becomes a copy of what it points to.
        #[cfg(not(unix))]
        let made = std::fs::copy(source.parent().unwrap_or(source).join(link), target).map(|_| ());
        made.map_err(fail)
    } else if meta.is_dir() {
        std::fs::create_dir_all(target).map_err(fail)?;
        let mut names: Vec<OsString> = std::fs::read_dir(source)
            .map_err(fail)?
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<Result<_, _>>()
            .map_err(fail)?;
        names.sort();
        for name in names {
            copy_tree(&source.join(&name), &target.join(&name))?;
        }
        Ok(())
    } else {
        std::fs::copy(source, target).map(|_| ()).map_err(fail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn universal_comes_out_of_the_cargo_options_and_picks_the_universal_command() {
        let (universal, rest) = take_flag(
            strings(&["--release", "--universal", "--locked"]),
            UNIVERSAL,
        );
        assert!(universal);
        assert_eq!(rest, strings(&["--release", "--locked"]));
        assert_eq!(bundle_command(universal), "bundle-universal");
    }

    #[test]
    fn without_universal_the_options_are_untouched_and_the_command_is_bundle() {
        let args = strings(&["--release", "--target", "x86_64-pc-windows-msvc"]);
        let (universal, rest) = take_flag(args.clone(), UNIVERSAL);
        assert!(!universal);
        assert_eq!(rest, args);
        assert_eq!(bundle_command(universal), "bundle");
    }
}
