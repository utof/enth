//! `cargo xtask wasm-hash [--simd128]`: the engine spike's native-vs-wasm determinism check.
//!
//! Builds `spike-engine`'s `engine-hash` for the host and for `wasm32-wasip1`, runs the wasm under wasmtime
//! (`$WASMTIME` or `wasmtime` on `PATH`), and requires: native output == wasm output byte for byte, both
//! equal to the committed golden files, and the SIMD-lane renders equal to the scalar renders on both.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const PKG: &str = "spike-engine";
const BIN: &str = "engine-hash";

pub fn wasm_hash(args: &[String]) -> Result<(), String> {
    let simd = args.iter().any(|a| a == "--simd128");
    let root = super::metadata()?.workspace_root;
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let wasmtime =
        std::env::var_os("WASMTIME").map_or_else(|| PathBuf::from("wasmtime"), PathBuf::from);
    let version = Command::new(&wasmtime).arg("--version").output().map_err(|e| {
        format!(
            "cannot run `{}`: {e}. Install wasmtime 48+ (CI uses taiki-e/install-action) or set WASMTIME=/path/to/wasmtime",
            wasmtime.display()
        )
    })?;
    let version = String::from_utf8_lossy(&version.stdout).trim().to_owned();

    build(&root, None, None, &[])?;
    let (wasm_dir, flags): (PathBuf, &[(&str, &str)]) = if simd {
        (
            target.join("wasm-simd128"),
            &[("RUSTFLAGS", "-C target-feature=+simd128")],
        )
    } else {
        (target.clone(), &[])
    };
    let wasm_target_dir = simd.then_some(wasm_dir.as_path());
    build(&root, Some("wasm32-wasip1"), wasm_target_dir, flags)?;

    let native_bin = target.join("release").join(BIN);
    let wasm_bin = wasm_dir
        .join("wasm32-wasip1/release")
        .join(format!("{BIN}.wasm"));

    let native_all = run(Command::new(&native_bin).arg("all"))?;
    let wasm_all = run(Command::new(&wasmtime).arg("run").arg(&wasm_bin).arg("all"))?;
    let native_lanes = run(Command::new(&native_bin).arg("lanes"))?;
    let wasm_lanes = run(Command::new(&wasmtime)
        .arg("run")
        .arg(&wasm_bin)
        .arg("lanes"))?;

    let mut errors = Vec::new();
    diff(
        "native vs wasm32-wasip1",
        &native_all,
        &wasm_all,
        &mut errors,
    );
    diff(
        "native lanes vs wasm lanes",
        &native_lanes,
        &wasm_lanes,
        &mut errors,
    );
    let golden_dir = root.join("spikes/spike-engine/golden");
    let golden = ["render.txt", "patches.txt"]
        .iter()
        .map(|f| std::fs::read_to_string(golden_dir.join(f)).map_err(|e| format!("{f}: {e}")))
        .collect::<Result<String, String>>()?;
    diff(
        "wasm32-wasip1 vs golden files",
        &golden,
        &wasm_all,
        &mut errors,
    );
    lanes_equal_scalar("native", &native_all, &native_lanes, &mut errors);
    lanes_equal_scalar("wasm", &wasm_all, &wasm_lanes, &mut errors);

    if errors.is_empty() {
        let n = wasm_all.lines().filter(|l| !l.starts_with('#')).count();
        println!(
            "wasm-hash: ok{}: {n} hashes identical (native == wasm32-wasip1 under {version} == golden); \
             x4/x8 lanes equal scalar on both",
            if simd { " (+simd128)" } else { "" }
        );
        Ok(())
    } else {
        Err(format!("wasm-hash failed:\n  {}", errors.join("\n  ")))
    }
}

fn build(
    root: &Path,
    target: Option<&str>,
    target_dir: Option<&Path>,
    env: &[(&str, &str)],
) -> Result<(), String> {
    let mut cmd = super::cargo();
    cmd.current_dir(root)
        .args(["build", "--release", "--locked", "-p", PKG, "--bin", BIN]);
    if let Some(t) = target {
        cmd.args(["--target", t]);
    }
    if let Some(d) = target_dir {
        cmd.arg("--target-dir").arg(d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run cargo build: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "cargo build failed (target {target:?}); is `rustup target add wasm32-wasip1` done?"
        ))
    }
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run {cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("non-UTF-8 output from {cmd:?}: {e}"))
}

fn diff(what: &str, expected: &str, got: &str, errors: &mut Vec<String>) {
    let (e, g): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), got.lines().collect());
    let bad: Vec<String> = (0..e.len().max(g.len()))
        .filter(|&i| e.get(i) != g.get(i))
        .take(5)
        .map(|i| format!("{what}: line {}: {:?} vs {:?}", i + 1, e.get(i), g.get(i)))
        .collect();
    errors.extend(bad);
}

/// `render-x4/<case> <hash>` and `render-x8/<case> <hash>` must equal `render/<case> <hash>`.
fn lanes_equal_scalar(label: &str, scalar: &str, lanes: &str, errors: &mut Vec<String>) {
    let by_case: BTreeMap<&str, &str> = scalar
        .lines()
        .filter_map(|l| l.strip_prefix("render/"))
        .filter_map(|l| l.split_once(' '))
        .collect();
    if by_case.is_empty() {
        errors.push(format!("{label}: no render/ lines found"));
    }
    for line in lanes.lines() {
        let Some((key, hash)) = line.split_once(' ') else {
            continue;
        };
        let Some((_, case)) = key.split_once('/') else {
            continue;
        };
        if by_case.get(case) != Some(&hash) {
            errors.push(format!("{label}: {key} differs from the scalar render"));
        }
    }
}
