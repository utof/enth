//! **Owner: lane SKR, then CL.** `enth compare`: score a patch against a challenge's target with the game's scorer (SKR
//! writes a first version for the round skeleton, CL the full one). Pre-declared by lane S0r (R1/S1) as a hidden
//! command that answers `not_built_yet` (exit 2, with a hint), so the lanes that build it never edit `cli.rs` or
//! `cmd/mod.rs` to add it: they fill [`Args`] and [`run`] here, then un-hide the command in `cli.rs`, add its schema
//! and snapshots, and document it in docs/agent-cli.md.

use super::Res;

/// `enth compare`'s arguments. Until its lane defines them, whatever follows the command reaches the stub.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// Everything after the command (the stub ignores it).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub rest: Vec<String>,
}

/// Run `enth compare`: not built yet.
pub fn run(_args: &Args) -> Res {
    Err(super::not_built("compare", "SKR, then CL"))
}
