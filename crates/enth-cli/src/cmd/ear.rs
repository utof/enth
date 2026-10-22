//! **Owner: lane SKE** (`enth ear` and its engine, freeze point F2e), then **EV4** (the rest of the command and the agent
//! tracks, additive). Pre-declared by lane S0e (R4) as a hidden command that answers `not_built_yet` (exit 2, with a
//! hint), so the lanes that build it never edit `cli.rs` or `cmd/mod.rs` to add it: they fill [`Args`] and [`run`] here,
//! then un-hide the command in `cli.rs`, add its schema and snapshots, and document it in docs/agent-cli.md.
//!
//! `enth ear` (docs/ear.md, "The session and the Ear view"): play and answer ear games from the command line, so agents
//! can take them too (paid agent runs only with the owner's OK). Like `enth lesson` it is stateless: the questions are
//! `enth-music`'s generator and `enth-challenge`'s `ear`, the scorers judge with no render, and the output is JSON with
//! `"schema": "enth.ear/1"` once built. A game is (game, level, seed, count) and the answers come in as indices or values;
//! `ear-check` (lane ECK) drives the same code with its oracle, random, bias and deaf players. No network code.

use super::Res;
use crate::error::{self, CliError};

/// `enth ear`'s arguments. Until its lane defines them, whatever follows the command reaches the stub.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// Everything after the command (the stub ignores it).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub rest: Vec<String>,
}

/// Run `enth ear`: not built yet.
pub fn run(_args: &Args) -> Res {
    Err(not_built())
}

/// The answer of an R4 command this build does not have yet (lane S0e's hidden stub): `not_built_yet`, exit 2.
fn not_built() -> CliError {
    error::usage(
        "not_built_yet",
        "`enth ear` is not built yet (R4 lane SKE)",
        "`enth --help` lists the commands this build has; `enth guide` shows the workflow",
    )
}
