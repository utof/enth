//! **Owner: lane CL.** `enth match`: fit a patch to a sound or a challenge with CMA-ES (`enth_solver`'s `cmaes`, lane
//! MT). The file is `matching.rs` because `match` is a keyword. Pre-declared by lane S0r (R1/S1) as a hidden command
//! that answers `not_built_yet` (exit 2, with a hint), so the lanes that build it never edit `cli.rs` or `cmd/mod.rs`
//! to add it: they fill [`Args`] and [`run`] here, then un-hide the command in `cli.rs`, add its schema and snapshots,
//! and document it in docs/agent-cli.md.

use super::Res;

/// `enth match`'s arguments. Until its lane defines them, whatever follows the command reaches the stub.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// Everything after the command (the stub ignores it).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub rest: Vec<String>,
}

/// Run `enth match`: not built yet.
pub fn run(_args: &Args) -> Res {
    Err(super::not_built("match", "CL"))
}
