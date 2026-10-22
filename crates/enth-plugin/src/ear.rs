//! **Owner: lane SKE** (the Ear skeleton, freeze point F2e; feature `ear`), then **TAP** (the tap path, additively).
//! Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until SKE writes it.
//!
//! The plugin's side of the ear games (docs/ear.md, "Rendering and playback" and "Rhythm input"; docs/plugin.md gets its
//! Ear section from SKE). Shells translate and hold no product rules: the questions are `enth-music`'s generator and
//! `enth-challenge`'s `ear`, the state machine is `enth-session`'s `ear`, the drill player and the workout composer are
//! `enth-learn`'s, and this file is the one place that runs the jobs, plays the examples and touches the disk for them:
//!
//! - **Ear jobs run on the game worker** ([`game`](crate::game)), never on a thread of their own and never on the audio
//!   thread: a set of about 10 questions renders in about 0.3 s (one render is 15 to 30 ms), and the results are posted
//!   back the way a Recreate round's are. No engine change.
//! - **Examples play through the Target slot** (at most 8 position-locked loops, an `AtomicPtr` mailbox, never freed on
//!   the audio thread), rendered **at the host's sample rate**, with one additive flag in `game.rs`'s `TargetLoop`:
//!   **"play from the top"**, so a stimulus restarts from its beginning. No host transport: stimuli carry their own tempo
//!   and count-in. SKE checks that every REAPER setup runs effects with the transport stopped (Target ▶ already relies
//!   on it).
//! - **Progress** (thresholds per skill and the practised days) goes to the per-user progress file's `drills` field,
//!   written the way the Review boxes are (re-read, merge, write a temporary file, rename).
//! - **The tap path** is lane TAP's: editor taps stamped on arrival (fork patch 12, lane FK4) first, and a host-MIDI ring
//!   on the audio thread only with the owner's OK. **Nothing here touches the audio thread** until then.
//!
//! The editor glue calls [`before_frame`] and [`after_frame`] around each frame (`editor.rs`'s `round_frame`), where SKE
//! hands the Ear view its read model and applies its intents, as Create does. Off by default: the plugin's `ear` feature
//! (it forwards `enth-ui/ear`) gates this module; the session's `ear`, Challenge v3 and the codes always compile, and an
//! ear session is not saved in the project.

use enth_session::Session;
use enth_ui::editor::Editor;

/// Before a frame, under the session lock: hand the editor the session's ear read model (lane SKE). Inert until SKE.
pub fn before_frame(_editor: &mut Editor, _session: &mut Session) {}

/// After a frame, under the session lock: apply the Ear view's intents to the session (lane SKE). Inert until SKE.
pub fn after_frame(_editor: &mut Editor, _session: &mut Session) {}
