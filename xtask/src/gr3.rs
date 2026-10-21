//! **Owner: lane EVK** (the GR3 kit, wave 2 after TP and DS; **`EVb`** adds the runs). Pre-declared by lane S0s (R3, freeze
//! point F0s); both commands refuse to run until EVK builds them.
//!
//! GR3's clause 1 (docs/r3.md, "GR3 and its hygiene"): on about 120 held-out synth targets, model + search beats the best
//! search alone at equal time. `cargo xtask gr3-run` runs the arms over a set and appends every run to
//! `content/gr3/runs.jsonl` (build, date, load; **none is dropped**); `cargo xtask gr3-report` reads the runs and
//! prints the endpoints fixed in advance. **xtask stays light** (S0r's rule: no engine, render or solver crate here):
//! the runs drive the `enth` binary (`enth create` and its flags, `cargo run --release -p enth-cli --`, the way
//! `agent-eval` drives it) or, for anything that needs the engine crates in-process, a tool binary as `lesson.rs` drives
//! `enth-lesson-tool`; the statistics are pure Rust over JSON. Local files only: no network code, no telemetry, no paid
//! run.
//!
//! | xtask command | File | Does | Lane |
//! | --- | --- | --- | --- |
//! | `gr3-run [--arm <A to D>] [--seconds N] [--mode <wall or renders>] <set>` | `arms.rs`, `runs.rs` | the four arms over a set of `content/gr3/` (A: R3a's best tuned search; B: model + search; C: the model's proposals unrefined; D: A given the true template), in wall-clock and render-budget modes (the "render-equivalent twin"); appends each run to `runs.jsonl` | EVK; `EVb` |
//! | `gr3-report [options] <runs.jsonl>` | `stats.rs`, `report.rs` | the pass rate at 60 s by `McNemar` on discordant targets, the per-target difference in JND units (Wilcoxon, Hodges-Lehmann interval), anytime curves at 10, 30, 60 and 300 s, time to pass (Kaplan-Meier, censored at 300 s), results per level and per degradation | EVK |
//!
//! Timed runs hold `flock -x /tmp/enth-quiet.lock` with no lane building, at the plugin's worker count (4) and the CLI's
//! (8), and the arm-A baseline is recorded overnight (wave 4). **The statistics are checked against hand values**
//! (EVK's acceptance). `docs/gr3.md` (planned; EVK, then `EVb`) is the protocol's reader's guide.

mod arms;
mod report;
mod runs;
mod stats;

/// `cargo xtask gr3-run`: not built yet.
pub fn gr3_run(_args: &[String]) -> Result<(), String> {
    Err("`gr3-run` is not built yet (R3 lane EVK)".to_owned())
}

/// `cargo xtask gr3-report`: not built yet.
pub fn gr3_report(_args: &[String]) -> Result<(), String> {
    Err("`gr3-report` is not built yet (R3 lane EVK)".to_owned())
}
