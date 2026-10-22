//! **Owner: lane EUI** (the Ear view, feature `ear`). Pre-declared by lane S0e (R4/R5); empty until the lane writes
//! it.
//!
//! Every round type (docs/ear.md, "Round types"): label and clip choice buttons, A/B and odd-one-out loops, the slider
//! estimate, **match it** (a constrained Recreate round, so the Recreate UI is reused unchanged) and, after lane TAP, the
//! tap pad. Each is driven by label in kittest. Nothing plays unless a button is pressed. Sizes go in
//! `theme/sizes/ear.rs`.
