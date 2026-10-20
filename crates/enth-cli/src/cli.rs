//! The command line, as clap derive types.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::perf::PerfArgs;
use crate::wav::Bits;

/// Output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// One JSON object (the default when stdout is not a terminal).
    Json,
    /// Tables for people (the default on a terminal).
    Table,
}

#[derive(Debug, Parser)]
#[command(
    name = "enth",
    version,
    about = "enth: build, render and measure synth patches from the command line. Start with `enth guide`.",
    long_about = "enth: build, render and measure synth patches from the command line.\n\nOutput is one JSON object when stdout is not a terminal, a table when it is. Values take real units (800Hz, 40ms, -6dB). Start with `enth guide`.",
    color = clap::ColorChoice::Never,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Output format (default: json when stdout is not a terminal, table when it is).
    #[arg(long, global = true, value_enum)]
    pub format: Option<Format>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Edits for `apply` and `step`.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct Edits {
    /// Ops JSON: an array of ops or one op, inline, `@file.json`, or `-` for stdin (`enth guide --topic ops`).
    #[arg(long, value_name = "JSON")]
    pub ops: Option<String>,
    /// address=value pairs, comma-separated (`filter1.cutoff=1.2kHz,env1.attack=40ms`); may be repeated.
    #[arg(long = "set", value_name = "K=V[,K=V]")]
    pub set: Vec<String>,
}

/// Fix options for `check` and `step`.
#[derive(Clone, Debug, PartialEq, Eq, clap::Args)]
pub struct FixArgs {
    /// Do not search for a fix (faster; for scripts).
    #[arg(long)]
    pub no_fix: bool,
    /// The most renders the fix search may use.
    #[arg(long, value_name = "N", default_value_t = crate::fix::DEFAULT_BUDGET)]
    pub fix_budget: u32,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Print the agent workflow guide (markdown, about 6 KB).
    Guide {
        /// A topic page: main, expect, ops or descriptors.
        #[arg(long, value_name = "TOPIC")]
        topic: Option<String>,
    },
    /// List the JSON Schemas, or print one (draft 2020-12).
    Schema {
        /// A schema name (patch, ops, perf, check, ...) or id (enth.check/2).
        name: Option<String>,
    },
    /// List the module kinds.
    Modules,
    /// The parameters of a module kind (or voice, master, or a module id like filter1): units, ranges, defaults.
    Params {
        /// A kind (svf), a built-in (voice, master), or a module id (filter1; with --patch, looked up in it).
        kind: String,
        /// A patch to look the module id up in.
        #[arg(long, value_name = "PATCH")]
        patch: Option<PathBuf>,
    },
    /// Write a new patch from a template.
    New {
        /// The template (`init`, `pluck_1osc`, `pad_2osc`, `bass_sub`).
        #[arg(long, value_name = "NAME")]
        template: Option<String>,
        /// Where to write the patch.
        #[arg(long, value_name = "PATCH")]
        out: Option<PathBuf>,
        /// List the templates instead.
        #[arg(long)]
        list: bool,
    },
    /// Apply ops to a patch (in place unless --out).
    Apply {
        /// The patch file.
        patch: PathBuf,
        #[command(flatten)]
        edits: Edits,
        /// Write the result here instead of over the input.
        #[arg(long, value_name = "PATCH")]
        out: Option<PathBuf>,
        /// Show what would change; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Read values from a patch.
    Get {
        /// The patch file.
        patch: PathBuf,
        /// Addresses (filter1.cutoff), module ids (filter1), voice, master, routes, modules, meta. None: a summary.
        addrs: Vec<String>,
        /// Plain numbers instead of unit strings.
        #[arg(long)]
        plain: bool,
    },
    /// Compare two patches.
    Diff {
        /// The first patch.
        a: PathBuf,
        /// The second patch.
        b: PathBuf,
        /// Also render both and compare their descriptors.
        #[arg(long)]
        describe: bool,
        #[command(flatten)]
        perf: PerfArgs,
    },
    /// Render a patch to a WAV (32-bit float with the performance in an `enth` chunk).
    Render {
        /// The patch file.
        patch: PathBuf,
        #[command(flatten)]
        perf: PerfArgs,
        /// The WAV to write (default: the patch path with .wav).
        #[arg(long, value_name = "WAV")]
        out: Option<PathBuf>,
        /// Sample format: f32 (default), 16 or 24.
        #[arg(long, value_enum, default_value = "f32")]
        bits: Bits,
        /// The render seed (default: the fixed default seed).
        #[arg(long, value_name = "SEED")]
        seed: Option<u64>,
    },
    /// Perceptual descriptors of a patch (rendered first) or a WAV.
    Describe {
        /// A patch file or a WAV.
        input: PathBuf,
        #[command(flatten)]
        perf: PerfArgs,
        /// The render seed, for a patch.
        #[arg(long, value_name = "SEED")]
        seed: Option<u64>,
    },
    /// Judge a patch or WAV against expectations: pass/fail, margin, noise. Exit 1 if any item fails.
    Check {
        /// A patch file or a WAV.
        input: PathBuf,
        /// The expectation list (`enth guide --topic expect`).
        #[arg(long, value_name = "LIST")]
        expect: String,
        #[command(flatten)]
        perf: PerfArgs,
        #[command(flatten)]
        fix: FixArgs,
        /// The judged render seed, for a patch.
        #[arg(long, value_name = "SEED")]
        seed: Option<u64>,
    },
    /// Apply, render, describe and check in one call.
    Step {
        /// The patch file.
        patch: PathBuf,
        #[command(flatten)]
        edits: Edits,
        /// The expectation list.
        #[arg(long, value_name = "LIST")]
        expect: Option<String>,
        #[command(flatten)]
        perf: PerfArgs,
        /// Write the edited patch here instead of over the input.
        #[arg(long, value_name = "PATCH")]
        out: Option<PathBuf>,
        /// The WAV to write (default: the output patch path with .wav).
        #[arg(long, value_name = "WAV")]
        wav: Option<PathBuf>,
        /// Echo every descriptor, not only the headline and the expected ones.
        #[arg(long)]
        full: bool,
        #[command(flatten)]
        fix: FixArgs,
        /// The judged render seed.
        #[arg(long, value_name = "SEED")]
        seed: Option<u64>,
    },
    /// Engine speed (wall-clock, information only).
    Bench {
        /// The patch to render (default: the `pad_2osc` template).
        #[arg(long, value_name = "PATCH")]
        patch: Option<PathBuf>,
        /// Voices held at once (1-16).
        #[arg(long, value_name = "N", default_value_t = 8)]
        voices: u32,
        /// Seconds of audio to render.
        #[arg(long, value_name = "SECONDS", default_value_t = 10.0)]
        seconds: f64,
    },
    // R1 commands, pre-declared by lane S0r: hidden, and each answers `not_built_yet` until its lane builds it in
    // its own file under `cmd/` (which also holds its arguments) and removes `hide` here.
    /// Score a patch against a challenge's target (lanes SKR, then CL).
    #[command(hide = true)]
    Compare(crate::cmd::compare::Args),
    /// The coach's next move towards a challenge's target (lane CL).
    #[command(hide = true)]
    Suggest(crate::cmd::suggest::Args),
    /// Fit a patch to a sound or a challenge (lane CL).
    #[command(hide = true)]
    Match(crate::cmd::matching::Args),
    /// What a parameter or module does (lane CL).
    #[command(hide = true)]
    Explain(crate::cmd::explain::Args),
    /// Challenges and their codes (lanes SKR, then CL).
    #[command(hide = true)]
    Challenge(crate::cmd::challenge::Args),
}
