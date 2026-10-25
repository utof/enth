//! **The stereo voice path: lane SVP's seam file** (S2a; created empty by lane S0d at freeze F0m).
//!
//! SVP writes the stereo voice bus here, so that its edits to the shared files (`plan.rs`, `engine.rs`) shrink to
//! call sites and lane RTE's audio routes ([`crate::routing`]) never meet it in the same lines (docs/phase-s2.md,
//! "Rules for every lane", the hotspot table). Nothing here runs yet.
//!
//! The contract SVP builds to (research report 06 §3.2; checked at F1m):
//!
//! - **A per-plan `stereo` flag**, set only when a module needs a stereo voice (unison spread, lane UNI). While it is
//!   clear, today's mono path runs untouched and bit-exact; switching it is a recompile.
//! - **When it is set**, the voice bus is left and right, and each `voice_fx` module is instantiated twice (one
//!   parameter span, one module key), so the module API does not change. Two instances of one kind in a chain keep
//!   separate state, seeds and swap carry (S2 rule 3).
//! - **`voice.pan` becomes a balance law** on the stereo voice.
//! - **Tests:** stereo with spread 0 equals mono on both channels; mono patches through the stereo-capable loop are
//!   bit-exact (the 73 goldens); no allocation and swap carry over a flag change.
//!
//! The call sites SVP adds: the flag and the doubled `voice_fx` slots in `plan.rs`'s `compile`, and the left/right
//! voice bus and the balance law in `engine.rs`'s step 7 (after the generator loop, which is RTE's).
