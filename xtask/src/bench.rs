//! `cargo xtask bench [-p <package>] [--bench <name>]`: the gungraun instruction budgets (alias `budget`).
//!
//! Instruction counts under callgrind are exact and independent of machine load, so they are the performance gate
//! (wall-clock numbers are information only; docs/benchmarks.md). Each bench file sets absolute `hard_limits`
//! (measured count + 10 %), so no stored baseline is needed. Runs `cargo bench` for `enth-engine` unless told
//! otherwise, after checking that valgrind and a `gungraun-runner` of the same version as the `gungraun` crate pinned
//! in the root `Cargo.toml` are present (`GUNGRAUN_RUNNER` names the runner when it is not on `PATH`).

use std::path::PathBuf;
use std::process::Command;

const USAGE: &str = "usage: cargo xtask bench [-p <package>] [--bench <name>]";

pub fn bench(args: &[String]) -> Result<(), String> {
    let mut package = "enth-engine".to_owned();
    let mut bench: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-p" | "--package" => it.next().ok_or(USAGE)?.clone_into(&mut package),
            "--bench" => bench = Some(it.next().ok_or(USAGE)?.clone()),
            _ => return Err(USAGE.to_owned()),
        }
    }
    let meta = super::metadata()?;
    let want = pinned_gungraun_version(&meta.workspace_root)?;
    preflight(&want)?;
    let mut cmd = super::cargo();
    cmd.current_dir(&meta.workspace_root)
        .args(["bench", "--locked", "--package", &package]);
    if let Some(b) = &bench {
        cmd.args(["--bench", b]);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run cargo bench: {e}"))?;
    if status.success() {
        println!("bench: ok ({package}: every instruction budget held)");
        Ok(())
    } else {
        Err(format!(
            "bench failed for {package}: an instruction budget was exceeded or a bench did not run (report above)"
        ))
    }
}

/// `gungraun = "=0.20.0"` in the root `[workspace.dependencies]` gives `0.20.0`.
fn pinned_gungraun_version(root: &std::path::Path) -> Result<String, String> {
    let path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: toml::Table = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    doc.get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.get("gungraun"))
        .and_then(|g| g.as_str())
        .map(|v| v.trim_start_matches('=').to_owned())
        .ok_or_else(|| {
            format!(
                "{}: no `gungraun = \"=x.y.z\"` in [workspace.dependencies]",
                path.display()
            )
        })
}

fn preflight(want: &str) -> Result<(), String> {
    Command::new("valgrind")
        .arg("--version")
        .output()
        .map_err(|e| {
            format!("cannot run valgrind: {e}. Install it (apt-get install valgrind); gungraun runs callgrind")
        })?;
    let runner = std::env::var_os("GUNGRAUN_RUNNER")
        .map_or_else(|| PathBuf::from("gungraun-runner"), PathBuf::from);
    let out = Command::new(&runner).arg("--version").output().map_err(|e| {
        format!(
            "cannot run `{}`: {e}. Install it: cargo install --locked --version {want} gungraun-runner (or set GUNGRAUN_RUNNER)",
            runner.display()
        )
    })?;
    let have = String::from_utf8_lossy(&out.stdout);
    if have.split_whitespace().last() == Some(want) {
        Ok(())
    } else {
        Err(format!(
            "gungraun-runner is `{}` but the gungraun crate is {want}; install the same version: cargo install --locked --version {want} gungraun-runner",
            have.trim()
        ))
    }
}
