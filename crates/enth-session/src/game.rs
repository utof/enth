//! **Owner: lane SG** (the session game API, freeze point F1r). Pre-declared by lane S0r (R1/S1), with the one type
//! the saved state already reserves.
//!
//! The pure Recreate round over an `enth_challenge` Challenge (report 01 §2.6, summary "Code placement"): round
//! states (setup, preparing, playing, submitted), the stash of the player's patch and history, submit, reveal and
//! give up, and the jobs a shell runs for it (the target render, scoring). Like the rest of the session it does no
//! I/O, spawns no threads and reads no clocks: the shell passes the time in (`now_ms`) and runs the jobs. SG writes
//! the threading contract into docs/session.md.

use serde::{Deserialize, Serialize};

/// A round to resume, as the `enth` field saves it ([`InstanceState::game`](crate::InstanceState::game)): reserved by
/// lane S0r, and empty until lane SG gives it fields at F1r.
///
/// The field is optional and absent while there is no round, so a state without one keeps its bytes. SG decides
/// whether its contents stay within `enth.plugin-state/1` (new optional fields with defaults) or need version 2
/// (docs/session.md, "The saved state, version 1").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameState {}
