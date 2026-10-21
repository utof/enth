//! **Owner: lane TP** (the k-NN bank builder, wave 2). Pre-declared by lane S0s (R3); the command is built in
//! `tools/datagen/src/bank.rs`.
//!
//! `cargo xtask bank [options]`: a driver for `enth-datagen bank`, which renders generator patches, pools their log-mel
//! and writes the retrieval bank (seeds plus f16 vectors, a versioned file beside the plugin; TP's open check at F1s,
//! docs/r3.md). **xtask stays light** (no engine crate here), so the work lives in the tooling package `enth-datagen`
//! (`tools/datagen/`), which holds the engine-bound dependencies, and this is a thin driver the way `lesson.rs` is for
//! `enth-lesson-tool`: it runs `cargo run --release -p enth-datagen -- bank <args>` and forwards the exit status. R3a runs
//! without the bank, so nothing depends on this command before TP builds it.

/// `cargo xtask bank`: runs `enth-datagen bank`.
pub fn bank(args: &[String]) -> Result<(), String> {
    let status = super::cargo()
        .args([
            "run",
            "--quiet",
            "--release",
            "--package",
            "enth-datagen",
            "--bin",
            "enth-datagen",
            "--",
            "bank",
        ])
        .args(args)
        .status()
        .map_err(|e| format!("cannot run cargo run: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("enth-datagen bank: failed (report above)".to_owned())
    }
}
