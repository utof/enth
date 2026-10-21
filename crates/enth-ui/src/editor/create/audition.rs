//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3); empty until PUI writes it.
//!
//! ▶ audition and A/B against the upload (docs/r3.md, "The Create view and the session"): the candidate and the upload
//! play level-matched through Recreate's `Mailbox` and `Player` (the same A/B machinery as the Target ▶ and Mine ▶
//! buttons), so a candidate is judged by ear against the reference, not against a number. Nothing plays unless the
//! user presses a button.
