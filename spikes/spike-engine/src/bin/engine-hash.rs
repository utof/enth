//! Prints the golden lines: `engine-hash [render|patches|all]`.
//! The same program runs natively and as `wasm32-wasip1` under wasmtime; the outputs must be byte-identical
//! and equal to `golden/render.txt` + `golden/patches.txt`.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this binary prints the hashes"
)]

use spike_engine::render::{patch_golden_text, render_golden_text};

fn main() {
    let what = std::env::args().nth(1).unwrap_or_else(|| "all".to_owned());
    match what.as_str() {
        "render" => print!("{}", render_golden_text()),
        "patches" => print!("{}", patch_golden_text()),
        "all" => print!("{}{}", render_golden_text(), patch_golden_text()),
        other => {
            eprintln!("usage: engine-hash [render|patches|all] (got {other:?})");
            std::process::exit(2);
        }
    }
}
