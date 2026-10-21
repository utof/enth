//! **Owner: lane TP** (`bank`, wave 2) and **DG** (`ds1`, `shards`, `exclude`; after F3s). Pre-declared by lane S0s (R3,
//! freeze point F0s): every command refuses to run (exit 1) and names its lane.
//!
//! `enth-datagen`: R3's data tooling, as a tooling package so that `cargo xtask` stays light (no engine crate in xtask,
//! S0r's rule) and so that the engine-bound work has a home. Like `enth-lesson-tool` it is exempt from the layer rules
//! (xtask/layers.toml), and it does I/O, spawns threads (rayon) and reads clocks, which the core crates must not. One
//! file per command, so the lanes never edit each other's:
//!
//! | Command | File | Does | Lane |
//! | --- | --- | --- | --- |
//! | `bank` | `bank.rs` | renders generator patches at a standard note, pools their log-mel and writes the k-NN bank (seeds plus f16 vectors, a versioned file beside the plugin; `cargo xtask bank` drives it) | TP |
//! | `ds1` | `ds1.rs` | the versioned sampler `ds1`: the mixture, the phrase sampler, the labels, the filters and the splits by stream key (docs/r3.md, "`ds1`") | DG |
//! | `shards` | `shards.rs` | deterministic shards of clean renders and their labels, for `ml/` to read as files | DG |
//! | `exclude` | `exclude.rs` | asserts that no training item is on `content/gr3/`'s exclusion lists | DG |
//!
//! Renders are bit-exact, so a dataset is a manifest (a sampler version, seeds, a phrase sampler, an augmentation config)
//! and clean audio never needs shipping or backing up. Features are computed by `enth-analysis`, never re-implemented in
//! Python (the ML rules, docs/phase-3.md). `ml/` is in no cargo or CI build, and this package never imports it.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool: reports go to stdout and stderr"
)]

mod bank;
mod ds1;
mod exclude;
mod shards;

use std::process::ExitCode;

const USAGE: &str = "usage: enth-datagen <command> [args...]
  bank                (not built yet: R3 lane TP) the k-NN retrieval bank
  ds1                 (not built yet: R3 lane DG) the training sampler's manifest
  shards              (not built yet: R3 lane DG) deterministic shards of clean renders and labels
  exclude             (not built yet: R3 lane DG) assert that training excludes the GR3 sets
Run `bank` through `cargo xtask bank`.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("bank") => bank::run(&args[1..]),
        Some("ds1") => ds1::run(&args[1..]),
        Some("shards") => shards::run(&args[1..]),
        Some("exclude") => exclude::run(&args[1..]),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("enth-datagen: {msg}");
            ExitCode::FAILURE
        }
    }
}
