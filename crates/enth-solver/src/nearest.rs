//! **Owner: lane MT** (match). A documented stub: **cut to R2** (`docs/phase-2.md`, the cut list: "nearest-solution
//! (MT's `nearest`)").
//!
//! The nearest reachable patch to a sound: the start of a match drawn from the starter superset and the factory
//! presets, scored by the same metric as a match. `enth match` does not need it to work: [`search`](crate::search)
//! starts from the patch it is given (the template's init patch for a challenge) and picks the waveform and filter
//! mode itself with a probe, which is what `nearest` would otherwise have supplied. It would help most when the
//! template's structure is wrong for the target (a filter missing, a second oscillator absent), which `search` does
//! not change either: it moves the parameters of the modules that are there (`docs/dsp/match.md`, "Limits").
//!
//! The design when it is built, so the stub is not a blank:
//!
//! 1. A *candidate set*: the factory presets (`content/presets`) plus the starter superset's template patches.
//! 2. Render each at the target's performance and score it with [`MetricTarget`](crate::search::MetricTarget):
//!    one render per candidate, batched through an [`Objective`](crate::search::Objective), so it parallelises like
//!    a search and is identical for any worker count.
//! 3. Return the best few as start patches for [`search`](crate::search::search), with their distances.
//!
//! Nothing here is public yet. Adding it is additive: a `pub fn nearest(…)` and no change to the search.
