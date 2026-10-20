//! `cargo xtask golden [--challenges [--only <case>]] [--update] [--wasm]`: the golden renders of `tests/golden`
//! (package `enth-golden`).
//!
//! Compares every case in `tests/golden/cases` with its `expected/` file and reports all mismatches in one run.
//! `--update` rewrites the expected files (refused under `CI`) and prints the list of changed cases for the commit
//! message. `--wasm` builds the same binary for `wasm32-wasip1` and runs it under wasmtime (`$WASMTIME` or `wasmtime`
//! on `PATH`); the hashes must equal the native ones. See `tests/golden/src/lib.rs` for the design.
//!
//! `--challenges` runs the golden challenges of `tests/golden/challenges/` instead (R1/S1 lane GC): the same flags,
//! `--only <case>` for one case. `--update` there creates a new case's pins and adds a new scorer set to an old one,
//! and refuses to change anything already pinned (the scorer id must change instead); see `docs/challenge.md`.

use std::path::PathBuf;
use std::process::Command;

const USAGE: &str = "usage: cargo xtask golden [--challenges [--only <case>]] [--update] [--wasm]";

/// What to run: the golden renders, or the golden challenges (optionally one case).
#[derive(Default)]
struct Which {
    challenges: bool,
    only: Option<String>,
}

impl Which {
    /// The arguments after the binary's name.
    fn args(&self, update: bool) -> Vec<String> {
        let mut args = Vec::new();
        if self.challenges {
            args.push("--challenges".to_owned());
        }
        if update {
            args.push("--update".to_owned());
        }
        if let Some(only) = &self.only {
            args.extend(["--only".to_owned(), only.clone()]);
        }
        args
    }
}

pub fn golden(args: &[String]) -> Result<(), String> {
    let (mut update, mut wasm) = (false, false);
    let mut which = Which::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--update" => update = true,
            "--wasm" => wasm = true,
            "--challenges" => which.challenges = true,
            "--only" => which.only = Some(args.next().ok_or_else(|| USAGE.to_owned())?.clone()),
            _ => return Err(USAGE.to_owned()),
        }
    }
    if which.only.is_some() && !which.challenges {
        return Err("--only needs --challenges".to_owned());
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
        run_wasm(&which)?
    } else {
        run_native(update, &which)?
    };
    if status.success() {
        Ok(())
    } else {
        Err("golden: failed (report above)".to_owned())
    }
}

fn run_native(update: bool, which: &Which) -> Result<std::process::ExitStatus, String> {
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
    cmd.args(which.args(update));
    cmd.status()
        .map_err(|e| format!("cannot run cargo run: {e}"))
}

fn run_wasm(which: &Which) -> Result<std::process::ExitStatus, String> {
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
        .args(which.args(false))
        .status()
        .map_err(|e| format!("cannot run wasmtime: {e}"))
}
