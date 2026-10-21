//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3); empty until PUI writes it.
//!
//! One progress bar for the whole job (renders against the budget, time left) and Cancel (docs/r3.md, "The Create view
//! and the session"). A cancel lands within one batch of the shell's pool (about one evaluation, ~10 ms). The bar
//! repaints on a worker post only: no timers while the view is hidden or closed. Sizes go in `theme/sizes/create.rs`.
