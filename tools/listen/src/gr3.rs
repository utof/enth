//! **Owner: lane LT** (GR3 listening, wave 1 after F0s). Pre-declared by lane S0s (R3, freeze point F0s) as a hidden
//! subcommand that refuses to run, so LT never edits `main.rs` to add it: it fills [`Gr3Args`] and [`run`] here, un-hides
//! the command in `main.rs`, and writes its section of docs/listening.md.
//!
//! `enth-listen gr3` (docs/r3.md, "GR3 and its hygiene", clause 2): the blind listening test of model + search against
//! search alone on real and degraded sounds, reusing listening.md's random names, hidden key, resume, replay and
//! `--dry-run`. The trial: the reference, then A, then B, "which is closer to the reference?", forced choice, on each
//! arm's **top-ranked** candidate by its own objective, never a human pick. Every clip, the reference included, at
//! -23 LUFS (the in-house BS.1770), a trial with a clip over -1 dBTP lowered as a whole, the A/B order counterbalanced;
//! six catch trials (two identical pairs, four against the Init patch); a listener under 80 % on attention is excluded.
//! **Degraded references are judged against the clean original.** The sets are `content/gr3/`'s (lane DS).
//!
//! - **`--export-kit`** writes the friends' kit: random-named, loudness-matched WAVs and an answers file, for any OS and
//!   any player (about 30 references, 15 minutes).
//! - **`--analyze`** reads the answers and applies the decision rule fixed in advance: B is preferred when the 95 %
//!   cluster-bootstrap lower bound (resampling references, then listeners) is above 50 % and the degraded subset's point
//!   estimate is at least 50 %. LT checks the bootstrap against a Python reference.
//! - **The listening key stays closed until every listener is done**, and whoever prepares stimuli does not audition
//!   them. **Never plays audio in a test**: the tool is tested with `--dry-run` and a recording mock player.
//!
//! The manifests are read with `toml` and committed hashes with `sha2`, both declared in this package's manifest.

use std::io::Write;

use crate::Res;
use crate::player::Player;
use crate::session::LineSource;

/// `enth-listen gr3`'s arguments. Until lane LT defines them, whatever follows the command reaches the stub.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct Gr3Args {
    /// Everything after the command (the stub ignores it).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub rest: Vec<String>,
}

/// Run `enth-listen gr3`: not built yet.
///
/// # Errors
/// Always: the command is lane LT's and is not built yet.
pub fn run(
    _args: &Gr3Args,
    _out: &mut dyn Write,
    _lines: &mut dyn LineSource,
    _player: Option<&mut dyn Player>,
) -> Res<()> {
    Err("`enth-listen gr3` is not built yet (R3 lane LT)".into())
}
