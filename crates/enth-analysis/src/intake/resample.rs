//! **Owner: lane FE** (the intake). Pre-declared by lane S0s (R3), **an open check**: this file may go.
//!
//! phase-3 lists `resample.rs` among FE's files, but docs/r3.md and report 03 say there is **no resampler in the
//! product path**: render and analysis run at the file's own rate, as `enth match` already does, and the model's
//! features put their bands in Hz. Whether the intake needs one at all (a model trained at one rate, say) is FE's open
//! check at F1s. If not, FE deletes this file and its `mod` line in `intake/mod.rs`, and says so in the report.
