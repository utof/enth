//! **Owner: lane SK3** (`enth create` and its engine, freeze point F2s), then **CLC** (the flags, `--perf`, the schema
//! snapshots and docs/agent-cli.md's section, additive). Pre-declared by lane S0s (R3) as a hidden command that answers
//! `not_built_yet` (exit 2, with a hint), so the lanes that build it never edit `cli.rs` or `cmd/mod.rs` to add it: they
//! fill [`Args`] and [`run`] here, then un-hide the command in `cli.rs`, add its schema and snapshots, and document it in
//! docs/agent-cli.md.
//!
//! `enth create --target x.wav --candidates 8 --seconds 60 [--perf …] [--model m.bin]` (docs/r3.md, "`enth create`"):
//! the agent evals' and GR3's door to "create from sound". Like `enth match` it is stateless, the search runs on the
//! CLI's rayon-backed, index-ordered evaluator (so a result is the same for any worker count), and the wall-clock cap
//! lives here, in the `search_with` callback, because the solver reads no clock. The audio decoders are this crate's
//! (WAV through `hound`, FLAC through `claxon`, Ogg Vorbis through `lewton`, MP3 through `nanomp3`, and an in-house AIFF
//! reader); `symphonia` (MPL-2.0) waits for the owner's OK. The output is JSON with `"schema": "enth.create/1"` once
//! built: the intake report, every candidate (patch, distance, provenance: heuristic, template, k-NN or model; its trace
//! of renders and time), the K starts as patches, the input features' hash and the decoded audio's hash.

use super::Res;
use crate::error::{self, CliError};

/// `enth create`'s arguments. Until its lane defines them, whatever follows the command reaches the stub.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// Everything after the command (the stub ignores it).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub rest: Vec<String>,
}

/// Run `enth create`: not built yet.
pub fn run(_args: &Args) -> Res {
    Err(not_built())
}

/// The answer of an R3 command this build does not have yet (lane S0s's hidden stub): `not_built_yet`, exit 2.
fn not_built() -> CliError {
    error::usage(
        "not_built_yet",
        "`enth create` is not built yet (R3 lane SK3)",
        "`enth --help` lists the commands this build has; `enth guide` shows the workflow",
    )
}
