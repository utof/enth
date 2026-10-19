//! `enth-listen`: the owner's listening tools. See docs/listening.md.
#![allow(clippy::print_stderr, reason = "CLI: errors go to stderr")]
#![allow(clippy::exit, reason = "CLI: exit code 1 for a failed command")]

use clap::{Parser, Subcommand};
use enth_listen::session::StdinLines;
use enth_listen::{abx, reverb};

#[derive(Parser)]
#[command(
    name = "enth-listen",
    about = "Blind listening tests for the owner: ABX on search results (G0) and a reverb A/B (G1)",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// ABX: does the search result sound the same as the target? (the G0 search gate)
    Abx(abx::AbxArgs),
    /// Reverb A/B: our Signalsmith port against fundsp's reverb2 and reverb4, blind and level-matched (G1)
    ReverbAb(reverb::ReverbArgs),
}

fn main() {
    let cli = Cli::parse();
    let mut out = std::io::stdout();
    let mut lines = StdinLines;
    let result = match cli.cmd {
        Cmd::Abx(a) => abx::run(&a, &mut out, &mut lines, None),
        Cmd::ReverbAb(a) => reverb::run(&a, &mut out, &mut lines, None),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
