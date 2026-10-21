//! **Owner: lane FE** (the intake, freeze point F1s). Pre-declared by lane S0s (R3); empty until FE writes it.
//!
//! The effective bandwidth of an upload (docs/r3.md, "The intake"): where its spectrum stops, which is the low-pass of
//! a codec (a 64 kbps MP3 cuts near 11 kHz; the search copied it, with a cutoff of 4.58 kHz), a phone speaker or a low
//! sample rate. The estimate is within 1/3 octave (FE's acceptance). A low-rate file simply has empty top bands: there
//! is no resampler in the product path. Lane OBJ's robust distance compares only the band below the cut-off.
