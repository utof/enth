//! `spike-metric`: the Phase 0 metric and search spike (Task 6). See docs/spikes/metric.md.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI spike: the printed tables are the product"
)]
#![allow(clippy::exit, reason = "CLI spike: exit codes for bad arguments")]

mod bench;
mod heurcheck;
mod listen_cmd;
mod solve;
mod util;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "spike-metric",
    about = "Phase 0 metric and search spike: solve, bench, listen"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Recover random targets from random or heuristic starts with CMA-ES; report solve rate, evaluations, time.
    Solve(solve::Args),
    /// Cost per evaluation and scaling with the number of workers.
    Bench(bench::Args),
    /// Accuracy of the heuristic start, per parameter.
    Heurcheck(heurcheck::Args),
    /// The owner's listening test (rate loudness-matched pairs, then analyse) or its dry run.
    Listen(listen_cmd::Args),
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Solve(a) => solve::run(&a),
        Cmd::Bench(a) => bench::run(&a),
        Cmd::Heurcheck(a) => {
            heurcheck::run(&a);
            Ok(())
        }
        Cmd::Listen(a) => listen_cmd::run(&a),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
