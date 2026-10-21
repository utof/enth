//! **Owner: lane SKL** (`show` and `check`, freeze point F2l), then **EV3** (`list`, `hint` and `closest`, additive).
//! Pre-declared by lane S0l (R2) as a hidden command that answers `not_built_yet` (exit 2, with a hint), so the lanes
//! that build it never edit `cli.rs` or `cmd/mod.rs` to add it: they fill [`Args`] and [`run`] here, then un-hide the
//! command in `cli.rs`, add its schema and snapshots, and document it in docs/agent-cli.md.
//!
//! `enth lesson list|show|check|hint|closest`: the agent evals' door to lessons (docs/learn.md, "Where Learn lives").
//! Stateless like `enth check`: the lesson logic is `enth-learn`'s (the CLI reads packs in the authoring layout through
//! its `toml` feature, or the built `.enthpack`), the solver jobs run on the CLI's rayon-backed evaluator, and the
//! output is JSON with `"schema": "enth.lesson/1"` once built.

use super::Res;
use crate::error::{self, CliError};

/// `enth lesson`'s arguments. Until its lane defines them, whatever follows the command reaches the stub.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// Everything after the command (the stub ignores it).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub rest: Vec<String>,
}

/// Run `enth lesson`: not built yet.
pub fn run(_args: &Args) -> Res {
    Err(not_built())
}

/// The answer of an R2 command this build does not have yet (lane S0l's hidden stub): `not_built_yet`, exit 2.
fn not_built() -> CliError {
    error::usage(
        "not_built_yet",
        "`enth lesson` is not built yet (R2 lane SKL)",
        "`enth --help` lists the commands this build has; `enth guide` shows the workflow",
    )
}
