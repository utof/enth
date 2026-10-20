//! **Owner: lane ST** (structure, combined and time). Pre-declared empty by lane S0r.
//!
//! Structure % (report 02): canonicalise both patches, match modules per kind, and compare only active, observable
//! parameters (the generator's mask) with sensitivity-based tolerances; equivalent patches score as equal.
//!
//! **The dispatch hook** ([`prepare`], F1r review S5): `score::round` asks this file for the structure scorer a
//! challenge names, after the stub (`structure-stub/1`). Lane ST adds a match arm for `structure/1` and switches
//! `model::RECREATE_SCORERS` in the same commit. The module pairing lives in [`super::matching`].

use super::round::PreparedScorer;
use super::{ScoreError, ScorerId, StructureScore, TargetContext};

/// The structure scorer `id`, prepared on `ctx`; `None` when this file has no scorer by that id (the caller refuses
/// it, `unknown_scorer`).
pub(crate) fn prepare(
    id: &ScorerId,
    ctx: &TargetContext<'_>,
) -> Option<Result<Box<dyn PreparedScorer<StructureScore>>, ScoreError>> {
    let _ = (id, ctx);
    None
}
