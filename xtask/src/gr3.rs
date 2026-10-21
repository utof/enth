//! **Owner: lane EVK** (the GR3 kit, wave 2 after TP and DS; **`EVb`** adds the runs). Pre-declared by lane S0s (R3, freeze
//! point F0s); built by EVK (docs/gr3.md is the protocol and the reader's guide).
//!
//! GR3's clause 1 (docs/r3.md, "GR3 and its hygiene"): on about 120 held-out synth targets, model + search beats the best
//! search alone at equal time. `cargo xtask gr3-run` runs the arms over a set and appends every run to
//! `content/gr3/runs.jsonl` (build, date, load; **none is dropped**); `cargo xtask gr3-report` reads the runs and
//! prints the endpoints fixed in advance. **xtask stays light** (S0r's rule: no engine, render or solver crate here):
//! the runs drive a tool binary with the engine crates in-process, `enth-datagen gr3 run` (`tools/datagen/src/gr3.rs`),
//! the way `lesson.rs` drives `enth-lesson-tool`; the statistics are pure Rust over JSON. Local files only: no network
//! code, no telemetry, no paid run.
//!
//! | xtask command | File | Does | Lane |
//! | --- | --- | --- | --- |
//! | `gr3-run --arm <A to D> [--split] [--set] [--mode <wall or renders>] [--seconds N] [--budget N] [--twin-of RUN]` | `arms.rs`, `runs.rs` | the four arms over a set of `content/gr3/` (A: R3a's best tuned search; B: model + search; C: the model's proposals unrefined; D: A given the true template), in wall-clock and render-budget modes (the "render-equivalent twin"); refuses an unfilled amendable item in a test run; appends each target to `runs.jsonl` | EVK; `EVb` |
//! | `gr3-report [--run RUN \| --a RUN --b RUN] [--at 60] [--long-a RUN --long-b RUN]` | `stats.rs`, `report.rs` | the pass rate at 60 s by `McNemar` on discordant targets, the per-target difference in JND units (Wilcoxon, Hodges-Lehmann interval), anytime curves at 10, 30, 60 and 300 s, time to pass (Kaplan-Meier, censored at the cap), results per stratum and level, the decision | EVK |
//!
//! Timed runs take `flock -x /tmp/enth-quiet.lock` per target (the tool does), at the plugin's worker count (4) or the
//! CLI's (8), and record the CPU other processes used meanwhile. **The statistics are checked against hand values**
//! (EVK's acceptance; `stats.rs` and `report.rs` tests). `docs/gr3.md` (EVK, then `EVb`) is the protocol's reader's
//! guide.

mod arms;
mod report;
mod runs;
mod stats;

/// `cargo xtask gr3-run`: the arms over a set; see [`arms`].
pub fn gr3_run(args: &[String]) -> Result<(), String> {
    arms::gr3_run(args)
}

/// `cargo xtask gr3-report`: the endpoints from the runs; see [`report`].
pub fn gr3_report(args: &[String]) -> Result<(), String> {
    report::gr3_report(args)
}
