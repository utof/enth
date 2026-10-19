//! `cargo xtask golden [--update] [--wasm]`: the golden renders of `tests/golden` (package `enth-golden`).
//!
//! Compares every case in `tests/golden/cases` with its `expected/` file and reports all mismatches in one run.
//! `--update` rewrites the expected files (refused under `CI`) and prints the list of changed cases for the commit
//! message. `--wasm` builds the same binary for `wasm32-wasip1` and runs it under wasmtime (`$WASMTIME` or `wasmtime`
//! on `PATH`); the hashes must equal the native ones. See `tests/golden/src/lib.rs` for the design.

use std::path::PathBuf;
use std::process::Command;

const USAGE: &str = "usage: cargo xtask golden [--update] [--wasm]";

pub fn golden(args: &[String]) -> Result<(), String> {
    let (mut update, mut wasm) = (false, false);
    for arg in args {
        match arg.as_str() {
            "--update" => update = true,
            "--wasm" => wasm = true,
            _ => return Err(USAGE.to_owned()),
        }
    }
    if update && std::env::var_os("CI").is_some() {
        return Err(
            "refusing to update goldens under CI; run `cargo xtask golden --update` locally and commit the diff with the reason"
                .to_owned(),
        );
    }
    if update && wasm {
        return Err("--update runs natively; --wasm only compares".to_owned());
    }
    let status = if wasm {
        run_wasm()?
    } else {
        run_native(update)?
    };
    if status.success() {
        Ok(())
    } else {
        Err("golden: failed (report above)".to_owned())
    }
}

fn run_native(update: bool) -> Result<std::process::ExitStatus, String> {
    let mut cmd = super::cargo();
    cmd.args([
        "run",
        "--quiet",
        "--package",
        "enth-golden",
        "--bin",
        "enth-golden",
        "--",
    ]);
    if update {
        cmd.arg("--update");
    }
    cmd.status()
        .map_err(|e| format!("cannot run cargo run: {e}"))
}

fn run_wasm() -> Result<std::process::ExitStatus, String> {
    let meta = super::metadata()?;
    let wasmtime =
        std::env::var_os("WASMTIME").map_or_else(|| PathBuf::from("wasmtime"), PathBuf::from);
    Command::new(&wasmtime).arg("--version").output().map_err(|e| {
        format!(
            "cannot run `{}`: {e}. Install wasmtime 48+ (CI uses taiki-e/install-action) or set WASMTIME=/path/to/wasmtime",
            wasmtime.display()
        )
    })?;
    let status = super::cargo()
        .current_dir(&meta.workspace_root)
        .args([
            "build",
            "--release",
            "--locked",
            "--package",
            "enth-golden",
            "--bin",
            "enth-golden",
            "--target",
            "wasm32-wasip1",
        ])
        .status()
        .map_err(|e| format!("cannot run cargo build: {e}"))?;
    if !status.success() {
        return Err(
            "cargo build failed for wasm32-wasip1; is `rustup target add wasm32-wasip1` done?"
                .to_owned(),
        );
    }
    let wasm = meta
        .target_directory
        .join("wasm32-wasip1/release/enth-golden.wasm");
    // The binary reads cases/ and expected/ at the absolute path baked in at build time, so preopen exactly that.
    let golden_dir = meta.workspace_root.join("tests/golden");
    Command::new(&wasmtime)
        .arg("run")
        .arg("--dir")
        .arg(&golden_dir)
        .arg(&wasm)
        .status()
        .map_err(|e| format!("cannot run wasmtime: {e}"))
}
