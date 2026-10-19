//! Phase 0 metric and search spike (Task 6).
//!
//! Layout: everything except [`par`] is pure maths (no I/O, no clocks, no global RNG, no threads) and is written to
//! move into `enth-analysis` (features, distances) and `enth-solver` (CMA-ES, search) later:
//!
//! | module | future home | what |
//! | --- | --- | --- |
//! | [`render`] | (spike only) | the throwaway 10-parameter subtractive voice |
//! | [`features`], [`yin`], [`distance`] | `enth-analysis` | loudness normalisation, log-mel, envelope, centroid, MFCC, f0; distances A/B/C |
//! | [`cmaes`], [`search`], [`heuristic`] | `enth-solver` | ask-tell CMA-ES, restarts with a categorical knob, heuristic start |
//! | [`stats`], [`listen`], [`wav`] | (spike only) | the listening test: stimuli, WAV encoding, Kendall/Spearman with bootstrap CIs |
//! | [`par`] | the caller (CLI) | rayon work-queue evaluator (the only module with threads) |
#![allow(
    clippy::needless_range_loop,
    reason = "numeric code indexes several arrays by one counter; iterator zips would obscure it"
)]

pub mod cmaes;
pub mod distance;
pub mod features;
pub mod heuristic;
pub mod listen;
pub mod par;
pub mod render;
pub mod rng;
pub mod search;
pub mod stats;
pub mod wav;
pub mod yin;
