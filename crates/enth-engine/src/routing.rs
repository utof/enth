//! **Audio routes between generators: lane RTE's seam file** (S2a; created empty by lane S0d at freeze F0m).
//!
//! RTE writes the audio-route machinery here, so that its edits to the shared files (`plan.rs`, `engine.rs`,
//! `module.rs`) shrink to call sites and lane SVP's stereo voice path ([`crate::stereo`]) never meets it in the
//! same lines (docs/phase-s2.md, "Rules for every lane", the hotspot table). Nothing here runs yet.
//!
//! The contract RTE builds to (research report 06 §3.1; the F1m judge checks its six points):
//!
//! 1. **Destinations.** An `audio` parameter ([`enth_params::Flags::audio`]; so far the placeholder `wt.pm` and
//!    `wt.am`) accepts only generator sources, and a generator source targets only `audio` parameters (RTE also
//!    owns `enth-patch`'s `validate.rs` rule and the `canon.rs` check). These routes stay out of the control-rate
//!    matrix ([`crate::modulation`]).
//! 2. **Order.** Generators sort topologically by audio routes, ties in patch order (a stable sort), so a patch
//!    without audio routes runs exactly as today. A cycle between modules is refused (`audio_cycle`). Hard sync
//!    keeps its rule.
//! 3. **Buffers.** Each generator writes its pre-level output into its own segment buffer, allocated in `compile`;
//!    a carrier reads it per frame, scaled by the route depth. The voice sum stays in patch order, which keeps the
//!    73 goldens bit-exact.
//! 4. **Feedback** is a parameter inside a module, never a route between modules.
//! 5. **Carriers.** PM only on phase-read waves (`wt`), so PM never meets a polyBLEP shape.
//! 6. **Masks.** A silent generator still counts when an audio route leaves it (generator v1 draws none, so v1 is
//!    unchanged).
//!
//! The call sites RTE adds: the generator order in `plan.rs`'s `compile`, the per-generator buffers in `engine.rs`'s
//! step 7 (the generator loop, before the voice effects), and `VoiceCx::audio_in` in `module.rs`.
