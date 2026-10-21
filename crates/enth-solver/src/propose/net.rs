//! **Owner: lane ONX** (freeze point F3s). Pre-declared by lane S0s (R3); empty until ONX writes it.
//!
//! `Net::run`: the layers a [`ModelPack`](super::pack) lists, run over [`ops`](super::ops) on one thread (about 160 ms
//! for a 0.8 M-parameter CNN with naive scalar code, far under the 60 s budget; the encoder runs once). Its outputs are
//! K hypothesis head sets (continuous values, categoricals, structure flags) and a confidence head that ranks them
//! before anything is rendered.
