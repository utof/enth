//! **Owner: lane ONX** (freeze point F3s). Pre-declared by lane S0s (R3); empty until ONX writes it.
//!
//! The model's input (docs/r3.md, "The model: propose v0"): about 2 s from the onset at -23 LUFS, `enth-analysis`'s
//! log-mel (64 bands, 30 Hz to 16 kHz, v1.1 pooling, 2 to 3 resolutions) at the file's own rate, plus f0 and loudness
//! tracks, on the scalar FFT path. **Features are computed in Rust, never re-implemented in Python** (the ML rules in
//! docs/phase-3.md): training reads what this module computes, through `tools/datagen`'s files or `enth-py`.
