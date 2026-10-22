//! **Owner: lane EUI** (the Ear view, feature `ear`). Pre-declared by lane S0e (R4/R5); empty until the lane writes
//! it.
//!
//! Progress as the player sees it (docs/ear.md, "Workouts"): thresholds per skill over time in semitones, ms, cents or
//! dB, a month calendar of practised days that is cumulative and cannot be "lost", and the warm-up one level below the
//! last level reached after a break, with no "you missed N days". Read from the per-user progress file's `drills` field
//! (`enth-learn`, lane DRL).
