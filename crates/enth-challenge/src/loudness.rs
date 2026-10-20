//! **Owner: lane SS** (the sound scorer). Pre-declared empty by lane S0r.
//!
//! An in-house ITU-R BS.1770 loudness for the scored path, on `libm` only (summary "Scored-path loudness", report 05):
//! metric v1 takes its gain from `ebur128`, which uses `std` maths, so sound % is not yet bit-exact across platforms.
//! This one is: K-weighting biquads, gated mean square, `libm::log10f`, a fixed summation order. The shared A/B level
//! match uses it too.
