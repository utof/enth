//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3); empty until PUI writes it.
//!
//! The K candidate tiles (docs/r3.md, "The Create view and the session"): each shows its distance and its state
//! (proposed, searching, done or pruned), its provenance (heuristic, template, k-NN or, in R3b, model), the patch it
//! holds (the proposal and the refined one, Synplant-style) and a **"Use this"** that the session turns into one undo
//! step. Sizes go in `theme/sizes/create.rs`.
