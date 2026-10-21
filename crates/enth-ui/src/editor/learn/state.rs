//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2); empty until LUI writes it.
//!
//! The editor's Learn state and the seam to the shell (docs/learn.md, "The Learn view"): `LearnIntent` (what the view
//! asks for: open the browser, start a lesson, next, back, hint, show me, end), `Editor::set_lesson` (the session's
//! `LearnEdit::lesson()` read model, handed over before each frame, as `set_round` hands over a round) and
//! `Editor::apply_learn` (the intents applied to the session's `LearnEdit` after the frame). It mirrors
//! `recreate/state.rs`.
