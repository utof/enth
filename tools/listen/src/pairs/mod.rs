//! The real-engine pairs session: **how similar does a listener find two sounds at a given `sound/1` distance?**
//! The ratings fit the sound score's curve on real-engine sounds (`sound/2`), where until now it is bridged from the
//! toy synth of the Phase 0 metric spike (ADR-0007's 2026-10-20 notes; docs/challenge.md, "Sound score").
//!
//! # Pairs
//!
//! About 40 test pairs plus four checks, all rendered by the real engine through the generator's phrase, exactly as a
//! round's target is scored ([`plan`] and docs/listening.md, "4. Real-engine pairs", for the reasoning):
//!
//! - **Targets** from the frozen generator v1 (`enth-challenge`'s `RandomSpec`), levels 1 to 5 in turn, 40-bit seeds
//!   drawn from the design seed (so each target has a short code the game can load).
//! - **Answers** are the target with its parameters moved by `m` of `sound/1`'s own calibration steps (the steps that
//!   define its JND scale): every observable continuous parameter and route (`all`, the scale's own definition, so
//!   `m` ≈ JND), one parameter the generator randomized (`one`), or osc 1's pitch (`pitch`, which the pitch guard
//!   weighs). Each test pair has a planned bin of `sound/1` JND distance, from well below the pass mark (0.99 JND) to
//!   well above the 50 % point (5.11); the design scores a fixed ladder of `m` and keeps the step count whose
//!   distance lands in the bin, closest to its centre.
//! - **Checks:** two identical pairs (the same clip twice) and two obviously different pairs (targets of different
//!   levels), as in `spike-metric listen` and `enth-listen abx`.
//!
//! The plan (targets, moves, planned bins, trial orders, file names) is drawn from the seed alone, without audio, and
//! its hash is pinned by a test; only the chosen step counts and the measured distances come from renders, which are
//! bit-identical on every platform.
//!
//! # Trials
//!
//! A, B, then X (an exact copy of A or B), as in the ABX test: the owner says which one X is, then rates how similar
//! A and B sound from 1 (very different) to 5 (indistinguishable), as in metric session 2. Both clips of a pair are
//! level-matched the way the game's A/B is (`enth_challenge::loudness::ab_gains_db` on the 48 kHz mid channel), and
//! every pair sits at −23 LUFS, so the volume is set once. Two sessions hold the same pairs in other orders; the
//! second (a retest on another day) is optional.
//!
//! # Output
//!
//! `pairs-session<N>.json` per session (each answer with its pair's metric distance and `sound/1` score),
//! `pairs-report.json` and `pairs-fit.json` (one row per pair with everything a `sound/2` fit needs, and a proposed
//! curve fitted to the ratings; [`report`]).

mod design;
mod fit;
mod perturb;
pub mod plan;
pub mod report;
mod run;

use serde::{Deserialize, Serialize};

use crate::flags::Flag;

pub use design::{Designed, choose, design, measure_pair, pair_audio, sounds, write_clips};
pub use fit::{CurveFit, Interval, RatingFit, fit_ratings, rating_model};
pub use perturb::{answer_patch, moved_params};
pub use plan::{Plan, PlannedPair, PlannedTrial, make_plan};
pub use report::{FitData, FitRow, Report, analyze, fit_rows};
pub use run::{
    PairsAction, PairsArgs, SessionEnd, clips_dir, load_results, manifest_path, results_path, run,
    synthetic_results,
};

/// Manifest schema (the hidden key: every pair's distance and score).
pub const SCHEMA_MANIFEST: &str = "enth.listen.pairs-manifest/1";
/// Results schema: one file per session, saved after every answer.
pub const SCHEMA_RESULTS: &str = "enth.listen.pairs-results/1";
/// Report schema.
pub const SCHEMA_REPORT: &str = "enth.listen.pairs-report/1";
/// The fit data schema: what a `sound/2` fit reads.
pub const SCHEMA_FIT: &str = "enth.listen.pairs-fit/1";

/// The default design seed. A directory keeps the seed it was designed with.
pub const DEFAULT_SEED: u64 = 20_261_021;
/// The default number of test pairs (plus [`N_SAME`] + [`N_DIFFERENT`] checks).
pub const DEFAULT_PAIRS: usize = 40;
/// Identical pairs per session (the same clip as A and B).
pub const N_SAME: usize = 2;
/// Obviously different pairs per session.
pub const N_DIFFERENT: usize = 2;
/// Sessions: the same pairs, reshuffled; the second is an optional retest.
pub const SESSIONS: u32 = 2;
/// Seconds of silence between A, B and X.
pub const GAP_S: f32 = 0.5;
/// Fade-out at the end of each clip, so a render cut inside a reverb tail does not end with a click (both clips of a
/// pair alike; the scorer's render has no fade, and 10 ms of a 1.5 s tail does not move its loudness).
pub const FADE_S: f32 = 0.01;
/// Peak ceiling after level matching: both clips of a pair are scaled down together if either would pass it.
pub const PEAK_CEILING: f32 = 0.98;

/// The distance bins, in `sound/1` JND units (`SoundScore::jnd`, the guard included): lower edges, the last bin open.
/// The pass mark is at 0.989 JND (rating 4.5 in the bridged curve) and 50 % at 5.11 (rating 3), so two bins sit on
/// either side of the pass mark and one around the 50 % point.
pub const BIN_LOWER: [f64; 6] = [0.0, 0.5, 1.0, 2.0, 4.0, 8.0];
/// The distance each bin aims at when the design picks a step count (about the geometric centre of the bin).
pub const BIN_CENTRE: [f64; 6] = [0.3, 0.7, 1.4, 2.8, 5.6, 11.0];
/// Test pairs per bin, out of 40: denser on either side of the pass mark, where the curve matters most.
pub const BIN_WEIGHTS: [usize; 6] = [5, 8, 8, 7, 7, 5];
/// The step counts the design scores for each answer (`m` calibration steps; about √2 apart, from a tenth of a step,
/// which the closest bin needs on targets where a quarter step of every parameter is already half a JND).
pub const LADDER: [f64; 17] = [
    0.1, 0.15, 0.25, 0.35, 0.5, 0.7, 1.0, 1.4, 2.0, 2.8, 4.0, 5.6, 8.0, 11.0, 16.0, 23.0, 32.0,
];

/// The bin of a `sound/1` JND distance.
#[must_use]
pub fn bin_of(jnd: f64) -> usize {
    if jnd.is_nan() {
        return BIN_LOWER.len() - 1;
    }
    BIN_LOWER.iter().rposition(|lo| jnd >= *lo).unwrap_or(0)
}

/// A bin's range as text (`0.5-1`, `8+`).
#[must_use]
pub fn bin_label(bin: usize) -> String {
    match BIN_LOWER.get(bin + 1) {
        Some(hi) => format!("{}-{}", BIN_LOWER[bin], hi),
        None => format!("{}+", BIN_LOWER[bin]),
    }
}

/// Which of A and B.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The first sound.
    A,
    /// The second sound.
    B,
}

impl Side {
    /// `"a"` or `"b"`, as typed and stored.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Side::A => "a",
            Side::B => "b",
        }
    }
}

/// What a pair is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairRole {
    /// A target and a moved copy: the data.
    Test,
    /// The same clip twice: should be rated 5; X is a guess.
    Same,
    /// Two targets of different levels: should be rated 1.
    Different,
}

/// How an answer is made from its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveKind {
    /// Every observable continuous parameter and route, each in a keyed direction: the JND scale's own move.
    All,
    /// One continuous parameter the generator randomized (not a pitch).
    One,
    /// Osc 1's pitch (`osc1.semi`), in 10 ct steps.
    Pitch,
}

impl MoveKind {
    /// The kind's name, as stored.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            MoveKind::All => "all",
            MoveKind::One => "one",
            MoveKind::Pitch => "pitch",
        }
    }
}

/// What `sound/1` says about a pair (target first, answer second).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measured {
    /// Metric v1.1 distance (`SoundScore::distance`).
    pub distance: f64,
    /// The target's nuisance floor.
    pub floor: f64,
    /// The target's JND scale.
    pub scale: f64,
    /// `max(0, distance − floor) / scale`: the metric's part of the JND distance.
    pub z_metric: f64,
    /// The pitch guard's part, in JND.
    pub pitch_term: f64,
    /// The waveform guard's part, in JND.
    pub wave_term: f64,
    /// `sound/1`'s JND distance: `z_metric + pitch_term + wave_term`.
    pub jnd: f64,
    /// `sound/1`'s sound %.
    pub percent: f64,
    /// Whether `sound/1` passes it (percent ≥ the pass mark).
    pub passes: bool,
    /// |Δ pitch| between the clips' first-note readings, cents (when both have one).
    pub pitch_delta_ct: Option<f64>,
    /// The target's own pitch floor, cents.
    pub pitch_floor_ct: f64,
    /// |Δ odd share| of the harmonic energy (when both have one).
    pub odd_delta: Option<f64>,
    /// The target's own odd-share floor.
    pub odd_floor: f64,
}

/// One pair as designed: its plan, the chosen move size and what `sound/1` measured.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    /// The plan of this pair.
    pub plan: PlannedPair,
    /// The target's short challenge code (paste it into the game to hear the target).
    pub code: Option<String>,
    /// The second sound's code (obviously different pairs).
    pub other_code: Option<String>,
    /// The move actually used (a test pair falls back to `all` when its planned move cannot reach the bin).
    pub kind: Option<MoveKind>,
    /// The chosen step count `m` (test pairs).
    pub steps: Option<f64>,
    /// `sound/1`'s JND distance at each [`LADDER`] step count of the move tried (and of `all` after a fallback).
    pub ladder_jnd: Vec<f64>,
    /// The measurement of the chosen pair.
    pub measured: Measured,
    /// The bin the measured distance falls in.
    pub bin: usize,
}

impl Pair {
    /// The parameter the answer's move changed (`one` and `pitch`; `None` for `all`, also after a fallback).
    #[must_use]
    pub fn moved_param(&self) -> Option<&str> {
        if self.kind == self.plan.kind {
            self.plan.param.as_deref()
        } else {
            None
        }
    }
}

/// One clip file under `pairs-clips/`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipInfo {
    /// Random file stem.
    pub id: String,
    /// The pair it belongs to.
    pub pair: usize,
    /// True for the target (the first sound of an obviously different pair).
    pub target: bool,
    /// Integrated loudness of the mid channel of the raw render, LUFS (the game's measure); `None` when silent.
    pub raw_lufs: Option<f64>,
    /// The gain applied, dB: the game's A/B match, the common offset to −23 LUFS and the shared peak factor.
    pub gain_db: f64,
    /// Peak of the final clip, dBFS.
    pub peak_dbfs: f64,
    /// Length, seconds.
    pub seconds: f64,
    /// Click and discontinuity findings (never a reason to drop a clip).
    pub flags: Vec<Flag>,
}

/// The design settings (stored in the manifest).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Seeds the plan.
    pub seed: u64,
    /// Test pairs.
    pub pairs: usize,
    /// Threads used to score the ladders.
    pub workers: usize,
}

/// Everything about a design: the key to the answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`SCHEMA_MANIFEST`].
    pub schema: String,
    /// The settings.
    pub config: Config,
    /// [`Plan::hash`] of the plan the pairs were made from.
    pub plan_hash: String,
    /// The engine that rendered the clips (`enth_patch::file::ENGINE_VERSION`): a DSP change can move distances.
    pub engine: String,
    /// The scorer measured (`sound/1`).
    pub scorer: String,
    /// The scorer's curve: `percent = 100 / (1 + (jnd / z50)^k)`.
    pub curve_z50: f64,
    /// The curve's steepness.
    pub curve_k: f64,
    /// The pass mark (sound %).
    pub pass_mark: f64,
    /// Logical threads of the machine that designed it.
    pub host_threads: usize,
    /// Seconds the design took.
    pub design_wall_s: f64,
    /// Every pair.
    pub pairs: Vec<Pair>,
    /// Every clip file.
    pub clips: Vec<ClipInfo>,
    /// Every trial of both sessions.
    pub trials: Vec<PlannedTrial>,
}

impl Manifest {
    /// The trials of one session, in play order.
    #[must_use]
    pub fn session_trials(&self, session: u32) -> Vec<&PlannedTrial> {
        self.trials
            .iter()
            .filter(|t| t.session == session)
            .collect()
    }

    /// The clip with this id.
    #[must_use]
    pub fn clip(&self, id: &str) -> Option<&ClipInfo> {
        self.clips.iter().find(|c| c.id == id)
    }
}

/// One answered trial, with its pair's distance and score (the results file is next to spike-metric's
/// `answers-session<N>.json`, with these fields added).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairAnswer {
    /// 1-based position in the session.
    pub n: usize,
    /// The trial's random id.
    pub id: String,
    /// 1 = very different .. 5 = indistinguishable.
    pub rating: u8,
    /// What the owner said X is: `a` or `b`.
    pub x: String,
    /// Whether that was right; `None` for an identical pair (any answer is a guess).
    pub x_correct: Option<bool>,
    /// Sounds replayed, both prompts together.
    pub replays: u32,
    /// Milliseconds from the first playback to the rating.
    pub ms: u64,
    /// The pair (an index into the manifest's pairs).
    pub pair: usize,
    /// Test, same or different.
    pub role: PairRole,
    /// The target's level (1–5).
    pub level: u8,
    /// The move (test pairs).
    pub kind: Option<MoveKind>,
    /// Metric v1.1 distance between the two sounds.
    pub distance: f64,
    /// `sound/1`'s JND distance.
    pub jnd: f64,
    /// `sound/1`'s sound % (the current score of this answer).
    pub percent: f64,
}

/// The results of one session, saved after every answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResultsFile {
    /// [`SCHEMA_RESULTS`].
    pub schema: String,
    /// Session number (1-based).
    pub session: u32,
    /// True for `--dry-run` answers (generated, never the owner's).
    pub synthetic: bool,
    /// The plan the session was played from.
    pub plan_hash: String,
    /// The engine that rendered the clips.
    pub engine: String,
    /// The answers, in the order given.
    pub answers: Vec<PairAnswer>,
}

impl ResultsFile {
    /// An empty file for `session` of manifest `m`.
    #[must_use]
    pub fn new(m: &Manifest, session: u32, synthetic: bool) -> Self {
        Self {
            schema: SCHEMA_RESULTS.to_string(),
            session,
            synthetic,
            plan_hash: m.plan_hash.clone(),
            engine: m.engine.clone(),
            answers: Vec::new(),
        }
    }

    /// Whether trial `id` has an answer.
    #[must_use]
    pub fn has(&self, id: &str) -> bool {
        self.answers.iter().any(|a| a.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bins_cover_every_distance() {
        assert_eq!(bin_of(0.0), 0);
        assert_eq!(bin_of(0.49), 0);
        assert_eq!(bin_of(0.5), 1);
        assert_eq!(bin_of(0.989), 1);
        assert_eq!(bin_of(1.0), 2);
        assert_eq!(bin_of(5.11), 4);
        assert_eq!(bin_of(1e9), 5);
        assert_eq!(bin_of(f64::INFINITY), 5);
        assert_eq!(bin_of(f64::NAN), 5);
        for (b, c) in BIN_CENTRE.iter().enumerate() {
            assert_eq!(bin_of(*c), b, "the centre of bin {b} is inside it");
        }
        assert_eq!(BIN_WEIGHTS.iter().sum::<usize>(), DEFAULT_PAIRS);
        assert_eq!(bin_label(0), "0-0.5");
        assert_eq!(bin_label(5), "8+");
    }
}
