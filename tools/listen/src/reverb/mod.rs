//! Blind, level-matched reverb A/B: our Signalsmith port against fundsp 0.23's `reverb2_stereo` and
//! `reverb4_stereo` (ADR-0006's fallback). The owner gate for G1: *which reverb do you prefer?*
//!
//! # Stimuli
//!
//! Three dry sources, rendered by the real engine (`enth-render`): a **pad** (a slow chord of detuned saws), a
//! **pluck** (a four-note phrase of short plucked saws) and a **transient** (two short percussive hits). The same
//! dry signal goes to all three reverbs:
//!
//! - **ours**: the engine's `reverb` module (`enth-render`, `dry` off, `wet` as asked, `decay` calibrated);
//! - **fundsp** `reverb2_stereo` and `reverb4_stereo`, fed the engine's dry signal and mixed in by this tool.
//!
//! # Matching
//!
//! - **Decay:** each reverb's control is calibrated until the RT60 measured from its own impulse response
//!   (Schroeder backward integration, T20 extrapolated) equals the target (`--rt60`, default 2 s). Colour, damping,
//!   early reflections and stereo width are each reverb's own; only broadband decay time is matched.
//! - **Wet level:** the fundsp wet signals are scaled so their BS.1770 stereo loudness equals our wet signal's, so
//!   all three mixes have the same dry level and the same wetness.
//! - **Overall level:** every final clip is brought to -23 LUFS (stereo meter), with one shared factor per source
//!   if a clip would peak above the ceiling.
//!
//! # Trials
//!
//! Every source, every pair of reverbs (ours/reverb2, ours/reverb4, reverb2/reverb4) and `repeats` repeats, each
//! with a fresh random order; one session, resumable. The owner answers `a`, `b`, or `=` for no preference.

mod analyze;
mod design;
pub mod fund;
mod rt60;
mod run;
mod sources;

use serde::{Deserialize, Serialize};

use crate::flags::Flag;

pub use analyze::{Report, analyze};
pub use design::{Designed, design};
pub use rt60::rt60_t20;
pub use run::{ReverbArgs, run, synthetic_answers};
pub use sources::{Source, sources};

/// Manifest schema (the hidden key to every trial).
pub const SCHEMA_MANIFEST: &str = "enth.listen.reverb-manifest/1";
/// Answer file schema.
pub const SCHEMA_ANSWERS: &str = "enth.listen.reverb-answers/1";
/// Report schema.
pub const SCHEMA_REPORT: &str = "enth.listen.reverb-report/1";
/// Seconds of silence between A and B.
pub const GAP_S: f32 = 0.8;

/// A reverb under test, in the order pairs are named (the first of a pair is the lower one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Candidate {
    /// Our port of Signalsmith Basics' reverb, through the engine.
    Ours,
    /// fundsp 0.23 `reverb2_stereo`.
    Reverb2,
    /// fundsp 0.23 `reverb4_stereo`.
    Reverb4,
}

impl Candidate {
    /// All three, in order.
    pub const ALL: [Candidate; 3] = [Candidate::Ours, Candidate::Reverb2, Candidate::Reverb4];

    /// A short name for tables.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Candidate::Ours => "ours",
            Candidate::Reverb2 => "reverb2",
            Candidate::Reverb4 => "reverb4",
        }
    }

    /// The full name for the report.
    #[must_use]
    pub fn long_label(self) -> &'static str {
        match self {
            Candidate::Ours => "our Signalsmith port (enth-render)",
            Candidate::Reverb2 => "fundsp reverb2_stereo",
            Candidate::Reverb4 => "fundsp reverb4_stereo",
        }
    }
}

/// Everything that shapes the design.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Seeds the engine renders and the trial design.
    pub seed: u64,
    /// Sample rate of every render.
    pub sample_rate: u32,
    /// The decay all three reverbs are calibrated to, seconds (RT60).
    pub rt60_s: f64,
    /// Our reverb's wet level, dB (the catalogue default is -6).
    pub wet_db: f64,
    /// Repeats of every (source, pair).
    pub repeats: usize,
    /// Tail length after the dry sound ends, in multiples of the RT60.
    pub tail_rt60s: f64,
}

/// One dry source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// `pad`, `pluck` or `transient`.
    pub name: String,
    /// What it is.
    pub description: String,
    /// Length of the clips made from it, seconds.
    pub seconds: f64,
}

/// How a reverb was calibrated to the target decay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Which reverb.
    pub candidate: Candidate,
    /// The control that was tuned.
    pub parameter: String,
    /// Its final value.
    pub value: f64,
    /// The RT60 measured with that value, seconds.
    pub measured_rt60_s: f64,
    /// Measure-and-adjust rounds used.
    pub rounds: usize,
}

/// One clip file under `clips/`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipInfo {
    /// Random file stem.
    pub id: String,
    /// The source name.
    pub source: String,
    /// The reverb.
    pub candidate: Candidate,
    /// Gain applied to the wet signal to match our wet loudness, dB (0 for ours).
    pub wet_match_db: f64,
    /// Gain applied to the dry+wet mix (the -23 LUFS match times the shared peak factor), dB.
    pub final_gain_db: f64,
    /// Integrated stereo loudness of the final clip, LUFS.
    pub lufs: f64,
    /// Peak of the final clip, dBFS.
    pub peak_dbfs: f64,
    /// RMS of the final clip, dBFS (information).
    pub rms_dbfs: f64,
    /// Click and discontinuity findings (never a reason to drop a clip).
    pub flags: Vec<Flag>,
}

/// One trial.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trial {
    /// 1-based position in the session.
    pub n: usize,
    /// Random id (the answer file refers to it).
    pub id: String,
    /// The source name.
    pub source: String,
    /// Repeat number, 1-based.
    pub repeat: usize,
    /// Clip played as A.
    pub a: String,
    /// Clip played as B.
    pub b: String,
    /// The reverb behind A.
    pub a_candidate: Candidate,
    /// The reverb behind B.
    pub b_candidate: Candidate,
}

/// Everything about a design: the key to the answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`SCHEMA_MANIFEST`].
    pub schema: String,
    /// The design settings.
    pub config: Config,
    /// The sources.
    pub sources: Vec<SourceInfo>,
    /// How the three reverbs were calibrated.
    pub calibration: Vec<Calibration>,
    /// Clip files.
    pub clips: Vec<ClipInfo>,
    /// The session's trials, in play order.
    pub trials: Vec<Trial>,
}
