//! **Owner: lane ST** (structure, combined and time). Pre-declared empty by lane S0r.
//!
//! The combined score and time (report 02): combined = w · sound + (1 − w) · structure, with w from 0.5 to 0.8 by
//! difficulty; only sound decides pass. Time is the first passing submit, from timestamps the shell injects.
//!
//! **The dispatch hook** ([`combiner`], F1r review S5): `score::round` asks this file for the combiner a challenge
//! names, after the stub (`combined-stub/1`). Lane ST adds a match arm for `combined/1` and switches
//! `model::RECREATE_SCORERS` in the same commit. Time (`time/1`) is the session's clock, not a scorer.

use super::{Combiner, ScorerId};

/// The combiner `id`; `None` when this file has none by that id (the caller refuses it, `unknown_scorer`).
pub(crate) fn combiner(id: &ScorerId) -> Option<Box<dyn Combiner>> {
    let _ = id;
    None
}
