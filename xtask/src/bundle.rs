//! `cargo xtask bundle` and `cargo xtask install`.
//!
//! Bundling is nice-plug's own bundler (`nice-plug-xtask`): it builds the package, detects which
//! plugin formats the library exports and writes `<pkg>.clap` and `<pkg>.vst3` under
//! `target/bundled`. Install copies those into the host plugin folders.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `cargo xtask bundle <pkg> [--release] [any other cargo build option]`.
pub fn bundle(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: cargo xtask bundle <package> [--release] [cargo build options]".into());
    }
    let nice_plug_args = std::iter::once("bundle".to_owned()).chain(args.iter().cloned());
    nice_plug_xtask::main_with_args("cargo xtask", nice_plug_args).map_err(|e| format!("{e:#}"))
}

/// Where `bundle` leaves its output.
pub fn bundled_dir() -> Result<PathBuf, String> {
    Ok(crate::metadata()?.target_directory.join("bundled"))
}

/// `cargo xtask install <pkg> [--release] [...]`: bundle, then copy the `.clap` file to
/// `~/.clap` and the `.vst3` bundle to `~/.vst3`. `ENTH_CLAP_DIR` and `ENTH_VST3_DIR` override
/// the destinations (for tests; hosts only look in the defaults).
pub fn install(args: &[String]) -> Result<(), String> {
    let package = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or("usage: cargo xtask install <package> [--release] [cargo build options]")?;
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
        std::os::unix::fs::symlink(link, target).map_err(fail)
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
