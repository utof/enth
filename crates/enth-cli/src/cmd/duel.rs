//! **Owner: lane SKD** (`enth duel result|verify`, freeze point F2d), then **EV4** (the rest of the command and the agent
//! tracks, additive). Pre-declared by lane S0e (R5) as a hidden command that answers `not_built_yet` (exit 2, with a
//! hint), so the lanes that build it never edit `cli.rs` or `cmd/mod.rs` to add it: they fill [`Args`] and [`run`] here,
//! then un-hide the command in `cli.rs`, add its schema and snapshots, and document it in docs/agent-cli.md.
//!
//! `enth duel` (docs/duels.md, "The flow" and "Verification"): `result` writes a round's scorecard and answer codes,
//! `verify` decodes a pasted code (paste-anything, so a whole chat message works), regenerates the challenge, re-scores
//! the answers on this build and compares them with the claim, with the CLI's rayon-backed, index-ordered evaluator. The
//! receiver's own re-score decides; the wording is "checked on your computer" for scores and "reported" for times. The
//! output is JSON with `"schema": "enth.duel-*/1"` once built. **No network code**: a code is text the user moves.

use super::Res;
use crate::error::{self, CliError};

/// `enth duel`'s arguments. Until its lane defines them, whatever follows the command reaches the stub.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// Everything after the command (the stub ignores it).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub rest: Vec<String>,
}

/// Run `enth duel`: not built yet.
pub fn run(_args: &Args) -> Res {
    Err(not_built())
}

/// The answer of an R5 command this build does not have yet (lane S0e's hidden stub): `not_built_yet`, exit 2.
fn not_built() -> CliError {
    error::usage(
        "not_built_yet",
        "`enth duel` is not built yet (R5 lane SKD)",
        "`enth --help` lists the commands this build has; `enth guide` shows the workflow",
    )
}
