//! **Owner: lane EUI** (the Ear view, feature `ear`; lane SKE writes the first seam at F2e). Pre-declared by lane S0e (R4/R5); empty until the lane writes
//! it.
//!
//! The editor's Ear state and the seam to the shell (docs/ear.md, "The session and the Ear view"): `EarIntent` (what
//! the view asks for: open or close the view, start a workout or a game, answer, replay, give up, one more, close),
//! `Editor::set_ear` (the session's ear read model, handed over before each frame, as `set_round` hands over a round)
//! and `Editor::apply_ear` (the intents applied to the session's ear edit trait after the frame). It mirrors
//! `create/state.rs` and `learn/state.rs`.
