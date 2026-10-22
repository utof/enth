//! **Owner: lane DUI** (the duel card, feature `duel`; lane SKD writes the first seam at F2d). Pre-declared by lane S0e (R4/R5); empty until the lane writes
//! it.
//!
//! The editor's duel state and the seam to the shell (docs/duels.md, "The flow"): `DuelIntent` (what the card asks for:
//! copy the share line, paste, verify, open a friend's answer once you have played, audition it, load it as one undo
//! step, clear a history entry), `Editor::set_duel` (the session's duel read model, handed over before each frame) and
//! `Editor::apply_duel` (the intents applied to the session's duel edit trait after the frame). It mirrors
//! `create/state.rs` and `learn/state.rs`.
