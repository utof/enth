//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3); empty until PUI writes it.
//!
//! The drop zone (docs/r3.md, "The Create view and the session"): it takes egui's `dropped_files` (baseview's drag events
//! mapped by fork patch 11, lane FK3) and, once the shell has decoded the file, shows its name, length, detected pitch
//! and the intake's warnings (a chord, a phrase, a clipped or low-passed upload). A short sound is shown as it is:
//! nothing is rejected by duration. Sizes go in `theme/sizes/create.rs`.
