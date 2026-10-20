//! **Owner: lane CT** (this file: the `Scorer` trait and `ScoreReport`, freeze point F1r). Pre-declared by lane S0r;
//! the sub-modules belong to lanes SS ([`sound`]) and ST ([`structure`], [`combined`]).
//!
//! Scores (report 02, summary "Sound %" and "Structure %, combined, time"): sound % decides pass; structure % and the
//! combined score (w · sound + (1 − w) · structure, w from 0.5 to 0.8 by difficulty) and time are reported beside it.
//! `ScoreReport` carries the descriptor components (attack, pitch in cents, brightness, envelope, noisiness, movement)
//! the results drawer shows.

pub mod combined;
pub mod sound;
pub mod structure;
