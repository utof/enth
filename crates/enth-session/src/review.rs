//! **Owner: lane REV** (the session's review source, R2). The reference is docs/learn.md, "Review (lane REV)"; the
//! boxes and the queue are `enth_learn::review`'s, and this file binds them to the live round. Like the rest of the
//! session it does no I/O, spawns no threads and reads no clocks.
//!
//! | The session | How |
//! | --- | --- |
//! | **Starts a Review item** | [`LearnEdit::review_start`](crate::learn::LearnEdit::review_start) plays the item's seeded sibling challenge as a round of origin [`RoundOrigin::Review`] (`RoundSource::Challenge`). It stashes your patch like a game round (End's Restore brings it back) and is refused while a lesson is live, whose stash is the only one |
//! | **No hint on the first try** | A Review round refuses a hint before its first submit ([`GameError::FirstTry`]); after it the coach's levels open as in any round (report 01 §3) |
//! | **Feeds the rings back** | When any played round ends (the game's, a lesson's, a Review item's), its first submit's rings on its target ([`round_rings`]) go to the shell as [`LearnRequest::Rings`], which applies them to the progress file's boxes with today's day. A round a host load ended is skipped: the host may close and reopen the same round (REAPER's undo), which must not move a box twice |
//!
//! **Which rings.** The first submit's structure rows, from the round's saved `first_report` (`enth.score-report/1`
//! text): after it the rings show live, so later submits are feedback, not evidence. **Assisted** (no box moves up):
//! the coach's last level (a hint level of [`HINT_LEVELS`], which gives the move), a Reveal, or "How close am I?"
//! (lane LFU's `closest`, once it lands). The skills are read off the round's target patch, so a round whose target
//! is gone (a restored round that broke) feeds nothing.
//!
//! **After a reload.** A Review item's draw target has no code, so like any challenge round outside a lesson it
//! restores broken, its stash kept: ending it brings your patch back, and the queue offers the item again.

use enth_learn::review::{ReviewItem, RoundRings};
use enth_patch::Patch;
use std::sync::Arc;

use crate::game::{
    EndedBy, GameError, HINT_LEVELS, RoundOrigin, RoundRecord, RoundSource, ScoreReport,
};
use crate::learn::{LearnError, LearnRequest};
use crate::model::Session;

/// The rings a played round gives the review boxes: its first submit's structure rows on `target` (the round's
/// target patch), assisted when the coach's last level, a Reveal or "How close am I?" was used. `None` when it has no
/// readable first report, or no ring on a skill.
pub fn round_rings(record: &RoundRecord, target: &Patch) -> Option<RoundRings> {
    let report: ScoreReport = serde_json::from_str(record.first_report.as_deref()?).ok()?;
    let rings = RoundRings::from_structure(&report.structure, target, assisted(record));
    (!rings.is_empty()).then_some(rings)
}

/// Whether a round counts as assisted for review: a hint at the coach's last level, a Reveal, or "How close am I?".
/// The lower hint levels and the dim assist name an aspect or a module, never the value, so they hold no box back.
pub fn assisted(record: &RoundRecord) -> bool {
    record.hints >= HINT_LEVELS || record.revealed
}

/// Whether the round refuses a hint now: a Review round before its first submit.
pub(crate) fn refuses_hint(record: &RoundRecord) -> bool {
    record.origin == RoundOrigin::Review && record.submits.is_empty()
}

impl Session {
    /// [`LearnEdit::review_start`](crate::learn::LearnEdit::review_start)'s body: the item's sibling challenge as a
    /// Review round. A live game or Review round ends first, logged, and its stash carries over, as a new game round's
    /// does.
    pub(crate) fn review_open(&mut self, item: ReviewItem) -> Result<(), LearnError> {
        if self.learn_live() {
            return Err(LearnError::Round(GameError::LessonLive));
        }
        self.game_open(
            RoundSource::Challenge(Box::new(item.challenge)),
            RoundOrigin::Review,
        )
        .map_err(LearnError::Round)
    }

    /// A played round closed for `why` (called from `game_close`, with the round's target if it was ready): send its
    /// rings to the shell, unless a host load closed it.
    pub(crate) fn review_note_close(
        &mut self,
        record: &RoundRecord,
        why: EndedBy,
        target: Option<Arc<Patch>>,
    ) {
        if why == EndedBy::Host {
            return;
        }
        if let Some(rings) = target.and_then(|t| round_rings(record, &t)) {
            self.learn_request(LearnRequest::Rings(rings));
        }
    }
}
