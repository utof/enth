//! **Owner: lane CV3** (the items; lane G4 tunes their ladder in [`super::ladder`]). Pre-declared by lane S0e (R4/R5,
//! freeze point F0e); empty until CV3 writes it. The reference is docs/ear.md, "The ear generator" (the parameter games).
//!
//! The parameter games need the catalogue, which `enth-music` cannot read, so their items are v2 draws built here:
//! **Spot the change** (pick the knob that moved), **Dial it in** (turn it to match: a constrained Recreate round) and
//! **Ear check** (a 3-interval odd one out on one parameter, giving a threshold in st, ms, ct or dB). The game picks the
//! option controls and the one that moves, at the level's distance in parameter steps (design.md §9's steps: cutoff 2
//! semitones, times x 1.5, levels 3 dB; generous steps for reverb), in parameter space only, as every generator rule
//! is. They stay on one-note phrases, where `sound/1` can measure them, and hold the pitch fixed while training
//! brightness; only observable controls are offered, with the editor's own knob names.
