//! Phase 0 lane B: the engine speed and determinism spike (`docs/spikes/engine.md`).
//!
//! A small subtractive engine (osc -> filter -> amp, up to 16 voices, plus a P1-like template with
//! 3 oscillators, 2 envelopes, an LFO and an FDN reverb) built to answer two questions:
//! how fast is it per core, and is `render(patch, performance, seed)` a pure function whose SHA-256
//! is the same on every platform, in wasm, in debug and release, with and without FTZ?
//!
//! Not product code: `enth-engine` will be written against the lessons recorded in the spike doc.

pub mod engine;
pub mod fdn;
pub mod math;
pub mod patch;
pub mod prng;
pub mod render;

pub use engine::{Engine, Flush, Note, Path};
pub use patch::Patch;
pub use render::{hash_samples, render};

/// The canonical scoring rate (design.md §5).
pub const SAMPLE_RATE: f32 = 48_000.0;
/// Fixed block size for offline renders.
pub const BLOCK: usize = 64;
/// Control rate: envelopes, LFO and filter coefficients update every this many samples.
pub const CTRL: usize = 32;
/// Voice cap of the spike.
pub const MAX_VOICES: usize = 16;
/// Bumped whenever the DSP changes output; goldens are keyed to it.
pub const ENGINE_VERSION: u32 = 1;
