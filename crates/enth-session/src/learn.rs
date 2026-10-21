//! **Owner: lane SL** (the session's Learn API, freeze point F1l). Pre-declared by lane S0l (R2).
//!
//! The pure Learn state (docs/learn.md, "The lesson model" and "Progress and the saved state"). Like `game`, it does
//! no I/O, spawns no threads and reads no clocks: **time comes from timestamps the shell passes in**, and background
//! work runs as jobs the shell executes on its own worker and hands back. The audio thread never touches any of it.
//! Lane SL binds `enth-learn`'s player to the live patch here: it stashes your patch, loads each step's start, **refuses
//! an op on a locked address in the edit path** (`SessionError::Locked`, with a hint, so every shell and test gets the
//! rule and the overlay only draws it), sends the Learn jobs out and keeps the read model the Learn view draws. A
//! lesson's Recreate step is a game round through the additive `RoundSource::Challenge`, which never stashes: this
//! module holds the only stash.
//!
//! # What is saved
//!
//! The `enth` field's optional `learn` object ([`LearnState`]) holds only the in-flight `LessonRef`, its step and the
//! stash, so a reload resumes the lesson and brings your own patch back. Per-user progress is not here: it is a file
//! the shell owns (`<Documents>/enth/Learn/progress.json`, `Progress::merge`). The object is absent while no lesson
//! is live, so a state without a lesson keeps its bytes, and it stays within `enth.plugin-state/1` (an optional
//! field with a default; docs/session.md, "The saved state"). The field is 1 MiB and a round already reserves 768 KiB,
//! so the F1l judge checks that the largest lesson plus its round fit.
//!
//! Lane S0l reserved the field and nothing else: [`LearnState`] is empty, and SL gives it fields at F1l.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The Learn state saved in the `enth` field: the lesson in flight, if any. Empty until lane SL adds the lesson
/// reference, the step and the stash (all optional, with defaults, so state version 1 stays). `LearnState::default()`
/// serialises as `{}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnState {}

impl LearnState {
    /// Nothing to save: every field is at its default.
    pub fn is_empty(&self) -> bool {
        *self == LearnState::default()
    }
}

/// The `enth` field's `learn` reader: never fails, so the rest of the field always loads (the rule `game` follows,
/// F1r review M3). A `learn` object this build cannot decode reads as absent; lane SL makes it keep the stash, as
/// [`GameState::lenient`](crate::game::GameState::lenient) does for a round.
pub(crate) fn lenient_learn<'de, D>(deserializer: D) -> Result<Option<LearnState>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|v| LearnState::deserialize(v).ok()))
}
