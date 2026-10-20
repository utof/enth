//! **Owner: lane SS** (the sound scorer). Pre-declared empty by lane S0r.
//!
//! Sound % (report 02): metric v1 with its fixes: map the player's modules onto the target's ids before the scoring
//! render, subtract the target's nuisance floor, metric v1.1's pooled mel power (after metric session 2), descriptor
//! components beside the headline. The loudness gain comes from [`crate::loudness`], not `ebur128`.
//!
//! **The dispatch hook** ([`prepare`], F1r review S5): `score::round` asks this file for the sound scorer a challenge
//! names, after the stub (`sound-stub/1`). Lane SS adds a match arm per scorer it lands (`sound/1`, `sound/2`) and
//! switches `model::RECREATE_SCORERS` to `sound/1` in the same commit; nothing else outside this file changes.

use super::round::PreparedScorer;
use super::{ScoreError, ScorerId, SoundScore, TargetContext};

/// The sound scorer `id`, prepared on `ctx`; `None` when this file has no scorer by that id (the caller refuses it,
/// `unknown_scorer`). Lane SS: `if *id == ScorerId::SOUND_V1 { return Some(Ready::prepare(SoundV1, ctx)); }`.
pub(crate) fn prepare(
    id: &ScorerId,
    ctx: &TargetContext<'_>,
) -> Option<Result<Box<dyn PreparedScorer<SoundScore>>, ScoreError>> {
    let _ = (id, ctx);
    None
}
