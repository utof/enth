//! Prints the golden lines: `engine-hash [render|patches|all]`, or `lanes` for the SIMD-lane variants of the
//! render cases. The same program runs natively and as `wasm32-wasip1` under wasmtime; the outputs must be
//! byte-identical and equal to `golden/render.txt` + `golden/patches.txt`.
//!
//! `engine-hash dump <case>` writes the raw little-endian f32 samples of one render case to stdout, for
//! offline feature comparisons (RMS and spectral centroid per frame) when a hash ever breaks.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this binary prints the hashes"
)]

use std::io::Write as _;
use std::process::ExitCode;

use spike_engine::render::{
    GOLDEN_SEED, frames, lanes_text, patch_golden_text, render, render_cases, render_golden_text,
};
use spike_engine::{Flush, Path};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map_or("all", String::as_str) {
        "render" => print!("{}", render_golden_text()),
        "patches" => print!("{}", patch_golden_text()),
        "lanes" => print!("{}", lanes_text()),
        "all" => print!("{}{}", render_golden_text(), patch_golden_text()),
        "dump" => {
            if let Err(msg) = dump(args.get(1).map_or("", String::as_str)) {
                eprintln!("{msg}");
                return ExitCode::from(2);
            }
        }
        other => {
            eprintln!("usage: engine-hash [render|patches|lanes|all|dump <case>] (got {other:?})");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

fn dump(name: &str) -> Result<(), String> {
    let c = render_cases()
        .into_iter()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("unknown case {name:?}; see golden/render.txt"))?;
    let out = render(
        &c.patch,
        &c.notes,
        frames(c.seconds),
        GOLDEN_SEED,
        Path::Scalar,
        Flush::On,
    );
    let bytes: Vec<u8> = out.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::io::stdout()
        .write_all(&bytes)
        .map_err(|e| format!("cannot write samples: {e}"))
}
