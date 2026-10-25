//! enth-dsp (see docs/architecture.md).
//!
//! Every module below is pre-declared by P1a lane S0 (freeze point F0) as an empty file naming the lane that fills
//! it, so no lane edits this file. To split a module into submodules, turn `x.rs` into `x/mod.rs`: `mod x;` still
//! resolves. S2a's lane S0d (freeze F0m) did the same for S2a's cores (`dist`, `wt`, `ott`, `comb`, `formant`,
//! `ladder`, `mseg`, `chorus`, `phaser`, `eq`, `delay`, `resample`) and added [`alias`], the spectral aliasing
//! harness the module lanes share.

pub mod alias;
pub mod biquad;
pub mod chorus;
pub mod comb;
pub mod compressor;
pub mod delay;
pub mod dist;
pub mod envelope;
pub mod eq;
pub mod formant;
pub mod ladder;
pub mod lfo;
pub mod mseg;
pub mod noise;
pub mod osc;
pub mod ott;
pub mod phaser;
pub mod resample;
pub mod reverb;
pub mod smoothing;
pub mod svf;
pub mod util;
pub mod wavetable;
pub mod wt;
