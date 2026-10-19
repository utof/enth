//! `spike-agent`: the agent-loop probe CLI. See docs/spikes/agent-probe.md and run `spike-agent guide`.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI: one JSON object on stdout is the product"
)]
#![allow(clippy::exit, reason = "CLI: stable exit codes (docs/agent-cli.md)")]

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use serde_json::json;
use spike_agent::commands::{self, Out};
use spike_agent::error::{CliError, EXIT_USAGE, Res};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "spike-agent",
    version,
    about = "Agent-loop probe: a CLI over a toy synth. Start with `spike-agent guide`. Output is one JSON object per call"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// One-page workflow guide for an agent.
    Guide,
    /// Parameter names, units, ranges and what each does to the sound.
    Params,
    /// Write a neutral starting patch.
    New {
        /// Where to write the patch.
        #[arg(long)]
        out: PathBuf,
        /// Performance, e.g. note=C3,dur=3s,hold=2s.
        #[arg(long)]
        perf: Option<String>,
    },
    /// Edit parameters: --set key=value[,key=value...] (units allowed: 800Hz, 40ms, -6dB).
    Apply {
        patch: PathBuf,
        #[arg(long = "set", value_delimiter = ',', required = true)]
        set: Vec<String>,
        /// Write here instead of editing the patch in place.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Show the diff, write nothing.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        perf: Option<String>,
    },
    /// Render a patch to a 16-bit mono 48 kHz WAV.
    Render {
        patch: PathBuf,
        /// WAV path (default: the patch path with .wav).
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        perf: Option<String>,
    },
    /// Perceptual descriptors of a patch (rendered first) or a WAV.
    Describe {
        input: PathBuf,
        /// Add a one-line meaning for each descriptor.
        #[arg(long)]
        explain: bool,
        /// Override the performance for this call (a WAV needs hold=... for sustain and release).
        #[arg(long)]
        perf: Option<String>,
    },
    /// Pass/fail per expectation, with measured hints for failures on a patch. Exit 1 if any failed.
    Check {
        input: PathBuf,
        /// e.g. 'brightness>2kHz, attack<10ms, sustain<-20dB, pitch=C3±20ct'
        #[arg(long)]
        expect: String,
        #[arg(long)]
        perf: Option<String>,
    },
    /// Apply + render + describe + check in one call.
    Step {
        patch: PathBuf,
        #[arg(long = "set", value_delimiter = ',')]
        set: Vec<String>,
        #[arg(long)]
        expect: Option<String>,
        /// Write the edited patch here (default: edit in place).
        #[arg(long)]
        out: Option<PathBuf>,
        /// WAV path (default: the output patch path with .wav).
        #[arg(long)]
        wav: Option<PathBuf>,
        #[arg(long)]
        perf: Option<String>,
    },
}

fn run(cmd: Cmd) -> Res<Out> {
    match cmd {
        Cmd::Guide => Ok(commands::cmd_guide()),
        Cmd::Params => Ok(commands::cmd_params()),
        Cmd::New { out, perf } => commands::cmd_new(&out, perf.as_deref()),
        Cmd::Apply {
            patch,
            set,
            out,
            dry_run,
            perf,
        } => commands::cmd_apply(&patch, &set, out.as_deref(), dry_run, perf.as_deref()),
        Cmd::Render { patch, out, perf } => {
            commands::cmd_render(&patch, out.as_deref(), perf.as_deref())
        }
        Cmd::Describe {
            input,
            explain,
            perf,
        } => commands::cmd_describe(&input, explain, perf.as_deref()),
        Cmd::Check {
            input,
            expect,
            perf,
        } => commands::cmd_check(&input, &expect, perf.as_deref()),
        Cmd::Step {
            patch,
            set,
            expect,
            out,
            wav,
            perf,
        } => commands::cmd_step(
            &patch,
            &set,
            expect.as_deref(),
            out.as_deref(),
            wav.as_deref(),
            perf.as_deref(),
        ),
    }
}

fn print_json(v: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string(v).expect("JSON values serialise")
    );
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                e.exit();
            }
            let text = e.render().to_string();
            let first = text
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("bad command line")
                .trim_start_matches("error: ")
                .to_string();
            let err = CliError::new(EXIT_USAGE, "usage", first)
                .with("detail", json!(text.trim()))
                .hint("spike-agent guide shows the workflow; spike-agent <command> --help shows its flags");
            print_json(&err.to_json());
            std::process::exit(EXIT_USAGE);
        }
    };
    match run(cli.cmd) {
        Ok(out) => {
            print_json(&out.json);
            std::process::exit(out.exit);
        }
        Err(e) => {
            print_json(&e.to_json());
            std::process::exit(e.exit);
        }
    }
}
