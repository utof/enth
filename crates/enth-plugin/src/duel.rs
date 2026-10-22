//! **Owner: lane SKD** (the Duel skeleton, freeze point F2d; feature `duel`), then **DUI** (the history file under
//! `duel/`, additively). Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until SKD writes it.
//!
//! The plugin's side of duels (docs/duels.md, "The flow" and "Sharing and paste"; docs/plugin.md gets its Duel section
//! from SKD). Shells translate and hold no product rules: the codes, `verify` and paste-anything are `enth-challenge`'s,
//! the state machine is `enth-session`'s `duel`, and this file is the one place that runs the jobs and touches the
//! clipboard and the disk for them:
//!
//! - **Duel jobs run on the game worker** ([`game`](crate::game)): `verify` is one prepare and one scored render, under
//!   about 1 s, and its result is posted back the way a Recreate round's is. Nothing runs on the audio thread.
//! - **The clipboard is written on a click and read only on an explicit Paste**, never on its own (a test pins it).
//! - **No networking code at all**: no sockets, no HTTP, no QR. Live duels get their own research session after GR1, and
//!   the standalone app first.
//! - **The duel history** is a per-user file this module's lane DUI owns (`<Documents>/enth/Duels/`, like the progress
//!   file: the codes, both re-scored results, the date you played and the optional names the receiver types). It leaves
//!   the machine only if you send it.
//!
//! The editor glue calls [`before_frame`] and [`after_frame`] around each frame (`editor.rs`'s `round_frame`), where SKD
//! hands the duel card its read model and applies its intents. Off by default: the plugin's `duel` feature (it forwards
//! `enth-ui/duel`) gates this module; the session's `duel`, the result codes and `verify` always compile.

use enth_session::Session;
use enth_ui::editor::Editor;

/// Before a frame, under the session lock: hand the editor the session's duel read model (lane SKD). Inert until SKD.
pub fn before_frame(_editor: &mut Editor, _session: &mut Session) {}

/// After a frame, under the session lock: apply the duel card's intents to the session (lane SKD). Inert until SKD.
pub fn after_frame(_editor: &mut Editor, _session: &mut Session) {}
