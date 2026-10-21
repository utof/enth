//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3); empty until PUI writes it.
//!
//! The editor's Create state and the seam to the shell (docs/r3.md, "The Create view and the session"): `CreateIntent`
//! (what the view asks for: open the view, choose or drop a file, run, cancel, audition a candidate, A/B, use this,
//! close), `Editor::set_create` (the session's create read model, handed over before each frame, as `set_round` hands
//! over a round) and `Editor::apply_create` (the intents applied to the session's create edit trait after the frame).
//! It mirrors `recreate/state.rs` and `learn/state.rs`.
