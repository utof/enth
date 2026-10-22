//! **Owner: lane BQ** (the biquad). Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until BQ writes it. Notes:
//! `docs/dsp/biquad.md` (BQ's), the design in `docs/ear.md` ("Rendering and playback").
//!
//! An **offline RBJ peaking and shelf biquad** (Robert Bristow-Johnson's *Audio EQ Cookbook*), the one primitive for
//! R4's EQ games now (Frequency finder's EQ form) and for S2's EQ module later, which wraps it. Coefficients go through
//! [`util::math`](crate::util::math) (the only door to `libm`), so they are bit-exact on every platform. The band a
//! question carries is integers in `enth-music`; this file renders it. Not a voice module and not on the audio thread,
//! so the engine never calls it.
//!
//! This is a new file: no existing output changes, and no existing golden moves. BQ adds its own golden (the same bits
//! on the six golden targets, within 0.1 dB of the cookbook's response). It is distinct from the reverb's private
//! `reverb::biquad`, which stays as it is.
