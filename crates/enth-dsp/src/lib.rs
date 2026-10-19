//! enth-dsp (see docs/architecture.md).
//!
//! Every module below is pre-declared by P1a lane S0 (freeze point F0) as an empty file naming the lane that fills
//! it, so no lane edits this file. To split a module into submodules, turn `x.rs` into `x/mod.rs`: `mod x;` still
//! resolves.

pub mod compressor;
pub mod envelope;
pub mod lfo;
pub mod noise;
pub mod osc;
pub mod reverb;
pub mod smoothing;
pub mod svf;
pub mod util;
pub mod wavetable;
