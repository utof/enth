//! enth-analysis (L3): measurements of rendered audio. The numbers behind `enth describe` and `enth check`, the
//! hint solver's objective and the sound score all come from here (`docs/dsp/analysis.md`).
//!
//! - [`descriptors`]: the descriptor vocabulary ([`Feature`]) and [`Describer`], which measures pitch, a
//!   beat-corrected envelope (attack, decay, sustain, release, duration), brightness, noisiness, beating,
//!   tremolo, tonal pitch under noise, brightness sweeps, and level (loudness, sample and true peak, clipping) on
//!   the float render.
//! - [`expect`]: the expectation parser (`brightness>2kHz`, `pitch=C3±20ct`, `beating in [3Hz, 9Hz]`) and the
//!   checker (pass, value, signed margin, nuisance noise, `thin`); [`margin`] holds the margin rules.
//! - [`features`] and [`distance`]: multi-resolution log-mel and friends, and metric v1 (log-mel L1, provisional);
//!   [`pooled`]: metric v1.1 (the mel magnitudes pooled over ~85 ms before the log, its own id); [`harmonics`]: the
//!   odd share of the harmonic energy (the sound score's waveform row).
//! - [`beat`] and [`envelope`]: the multi-harmonic beating estimator and fitted beat removal; [`sweep`]: filter
//!   envelopes and filter LFOs from the centroid over time.
//! - [`fft`]: the scalar (identical everywhere) and dispatched FFT paths; [`loudness`], [`pitch`] (YIN), [`units`].
//! - [`test_signals`]: synthetic signals with known answers, for tests and sweeps.
//!
//! R3 files, pre-declared empty by lane S0s so the lanes that fill them never edit this one (every file above is
//! untouched):
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`intake`] | "create from sound"'s intake: `trim`, `floor`, `bandwidth`, `clip`, `perf` (and an open `resample`); mono, the noise floor, the bandwidth, clipped spans, the inferred performance | FE |
//! | [`robust`] | the degradation-aware distance, under a new metric id (`sound/1` and the metrics above unchanged) | OBJ |
//!
//! Pure functions of samples: no I/O, no clocks, no threads, no global RNG; builds for wasm32. Transcendentals go
//! through `libm`, and every FFT that feeds a pass/fail decision runs on [`FftPath::Scalar`].

pub mod beat;
pub mod descriptors;
pub mod distance;
pub mod envelope;
pub mod expect;
pub mod features;
pub mod fft;
pub mod harmonics;
pub mod intake;
mod linalg;
pub mod loudness;
pub mod margin;
pub mod pitch;
pub mod pooled;
pub mod robust;
pub mod sweep;
pub mod test_signals;
pub mod units;

pub use descriptors::{Describer, Descriptors, Feature, NoteTiming};
pub use expect::{CheckReport, Expect, ItemResult, check, parse_list};
pub use fft::FftPath;
