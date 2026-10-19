//! `enth-listen`: the owner's listening tools (lane T). Two blind tests, one binary (docs/listening.md).
//!
//! - [`abx`]: "does the search result sound the same as the target?" The G0 search gate wants at least 14 of 20
//!   targets judged *same* within 60 s on 8 workers. Stimuli: spike-metric's toy synth and CMA-ES solver used as a
//!   library (the real engine still has placeholder oscillators and filters, and `enth-solver` has no search yet).
//! - [`reverb`]: a blind, level-matched preference test of our Signalsmith reverb port (rendered through
//!   `enth-render`) against fundsp 0.23's `reverb2_stereo` and `reverb4_stereo`, the fallback named in ADR-0006.
//!
//! Shared pieces: [`wav`] (24-bit PCM files), [`player`] (`pw-play --media-role=Production` by default),
//! [`loudness`] (EBU R128 level matching), [`flags`] (clicks and discontinuities are flagged, never filtered),
//! [`session`] (the interactive trial loop and answer files, resumable), [`stats`] (exact binomial tests).
//!
//! Nothing here plays audio unless the owner runs a session: `--dry-run` skips playback and feeds synthetic
//! answers, and every test uses it or a scripted player. This is tooling (exempt from the layer rules): it does
//! I/O, spawns threads through spike-metric's evaluator and runs a player process.
#![allow(
    clippy::needless_range_loop,
    reason = "numeric code indexes several arrays by one counter; iterator zips would obscure it"
)]

pub mod abx;
pub mod files;
pub mod flags;
pub mod loudness;
pub mod player;
pub mod reverb;
pub mod session;
pub mod stats;
pub mod wav;

/// The error type of the tool: a message for the owner.
pub type Res<T> = Result<T, Box<dyn std::error::Error>>;
