//! **Owner: lane TP** (the portfolio, freeze point F1s, with FE's intake types); **MS** edits only `schedule.rs`
//! (R3b's model proposals). Pre-declared by lane S0s (R3, freeze point F0s).
//!
//! "Create from sound", R3a's search side (docs/r3.md, "The start portfolio and k-NN retrieval" and "K branches and
//! successive halving"): a create job starts from several places at once, refines each as its own [`Search`] (the
//! ask/tell CMA-ES that takes one start), and answers K [`Candidate`]s. Like the rest of the solver it spawns no
//! threads, reads no clocks and does no I/O: parallelism comes from the caller's [`Objective`], which returns
//! distances by index, and **the wall-clock cap lives in the caller's progress callback**, so the outcome is the same
//! for any worker count and the schedule never depends on wall time (TP's acceptance).
//!
//! ```text
//! intake (FE)  ──►  Target  ──►  portfolio: heuristic · templates · k-NN (nearest)  ──►  starts
//!                                                                                         │
//!            caller's Objective  ◄──  CreateJob: screen, then K Searches, one merged batch per round,
//!            (rayon in the shell)       successive halving 10 % / 30 % / rest of the render budget
//!                                                                                         │
//!                                                            CreateResult  ──►  enth.create/1 (record)
//! ```
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | this file | [`Target`] (the intake's output, solver-side), [`CreateCfg`], [`Inputs`], [`create`] | TP |
//! | [`candidate`] | [`Candidate`] (patch, distance, provenance, trace), [`Provenance`], [`CreateResult`] | TP |
//! | [`portfolio`] | the start set: the heuristic start (always kept), the templates, k-NN retrieval over the bank (R3a runs without it), later the model's hypotheses | TP |
//! | [`schedule`] | [`CreateJob`]: the screening round, K branches, merged ask/tell batches, deterministic successive halving; MS adds the model's proposals here | TP; MS |
//! | [`record`] | `enth.create/1`: everything a run needs to replay, the K starts recorded as patches | TP |
//!
//! # The determinism contract (docs/r3.md, points 2–4)
//!
//! - A result is a pure function of the target, the starts, the settings and the render budget, for any worker
//!   count: each round's batch is the concatenation of every live branch's `ask()` in branch order, and the
//!   distances are split back by index.
//! - **The schedule is in renders, never in time.** Rung boundaries, pruning and the final cut are functions of
//!   evaluation counts and distances; the shell maps seconds to a budget, and a wall-clock cap (the progress callback
//!   returning `false`) only truncates: a cancelled run is a prefix of the full one.
//! - Every result records its starts as patches, the decoded audio's hash and the input features' hash
//!   ([`record`]), so a run replays from its starts even where a lossy decoder differs in its last bits.
//!
//! # The seam with the intake (lane FE)
//!
//! [`Target`] mirrors what `create` needs from FE's `enth_analysis::intake::Intake`, field for field under FE's names
//! (agreed through the coordinator): the mono clip with the note-on at `perf.on_s` (FE keeps a few ms of pre-roll),
//! the file's rate, the inferred performance, the detected bandwidth and the noise floor. `Target::from_intake` is
//! added when FE's types land (enth-analysis is L3, so FE's `InferredPerf` cannot hold an `enth_render::Performance`;
//! [`Target::performance`] builds it). Everything else of the intake report (warnings, clipping, hum) is the shell's
//! to show; the objective (metric v1 today, OBJ's robust distance later) is the caller's [`Objective`].
//!
//! [`Search`]: crate::search::Search
//! [`Objective`]: crate::search::Objective

pub mod candidate;
pub mod portfolio;
pub mod record;
pub mod schedule;

use core::fmt;

use enth_analysis::descriptors::Want;
use enth_analysis::features::{FeatureCfg, FeatureExtractor, Features};
use enth_analysis::{Describer, Descriptors};
use enth_patch::Patch;
use enth_render::{Note, Performance, RenderSpec};
use sha2::{Digest, Sha256};

use crate::eval::note_timing;
use crate::nearest::{self, Bank, Embedding};
use crate::search::{Objective, SearchError};

pub use candidate::{
    BranchStep, Candidate, CandidateState, CreateResult, CreateStop, Provenance, RoundInfo,
    StartOutcome,
};
pub use portfolio::{Portfolio, Start, Template};
pub use record::{CreateRecord, SCHEMA};
pub use schedule::{BranchView, CreateJob, Progress};

/// The inferred performance of the upload, under the names of FE's `InferredPerf`. All times are clip time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetPerf {
    /// The MIDI note nearest the measured pitch (48 when the clip is unpitched).
    pub key: u8,
    /// Velocity, 0–1 (FE fixes it at 100/127).
    pub velocity: f32,
    /// The note-on: FE keeps a few ms of pre-roll before the onset, rounded to the engine's 32-frame segments.
    pub on_s: f64,
    /// How long the note is held (note-off minus note-on).
    pub hold_s: f64,
    /// The clip's length (the render's length).
    pub duration_s: f64,
    /// YIN's median pitch, when the clip is pitched (recorded; the k-NN reference uses the descriptors' reading).
    pub f0_hz: Option<f64>,
}

/// What a create job is given: the intake's output on the solver's side (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// The mono clip at `sample_rate`: the mean of the channels, capped (FE: 5 s by default), the note-on at
    /// `perf.on_s`. Short clips are kept, never rejected.
    pub audio: Vec<f32>,
    /// The file's own rate (8,000–191,999 Hz); render and analysis run at it, with no resampling.
    pub sample_rate: u32,
    /// The render seed of every candidate (the shell's; the default seed unless a file says otherwise).
    pub render_seed: u64,
    /// The inferred (or overridden) performance.
    pub perf: TargetPerf,
    /// The highest frequency the upload still carries, when the intake found a steep low-pass (a codec, a phone):
    /// the k-NN embedding masks the bands above it.
    pub bandwidth_hz: Option<f64>,
    /// The noise floor in dBFS, when measured (recorded; OBJ's robust distance uses it).
    pub noise_floor_dbfs: Option<f64>,
}

impl Target {
    /// The performance every candidate is rendered with: one note at `perf.on_s`, held `perf.hold_s`, in a render
    /// of `perf.duration_s`.
    #[must_use]
    pub fn performance(&self) -> Performance {
        let p = &self.perf;
        Performance {
            notes: vec![Note::new(p.key, p.velocity, p.on_s, p.on_s + p.hold_s)],
            duration_s: p.duration_s,
            ..Performance::default()
        }
    }

    /// The render spec of every candidate: the file's rate and the render seed.
    #[must_use]
    pub fn render_spec(&self) -> RenderSpec {
        RenderSpec::new(self.sample_rate, self.render_seed)
    }

    /// The feature settings at the clip's rate (metric v1's, loudness-normalised, scalar FFT path).
    #[must_use]
    pub fn feature_cfg(&self) -> FeatureCfg {
        FeatureCfg {
            sample_rate: f64::from(self.sample_rate),
            ..FeatureCfg::default()
        }
    }

    fn check(&self) -> Result<(), CreateError> {
        if self.audio.is_empty() {
            return Err(CreateError::BadTarget("the clip is empty"));
        }
        if !(8_000..192_000).contains(&self.sample_rate) {
            return Err(CreateError::BadTarget(
                "the sample rate must be 8000 to 191999 Hz",
            ));
        }
        if self.audio.iter().any(|s| !s.is_finite()) {
            return Err(CreateError::BadTarget("the clip has a non-finite sample"));
        }
        let p = &self.perf;
        let times_ok = p.on_s.is_finite()
            && p.on_s >= 0.0
            && p.hold_s.is_finite()
            && p.hold_s >= 0.0
            && p.duration_s.is_finite()
            && p.duration_s > 0.0
            && p.key <= 127
            && p.velocity.is_finite()
            && (0.0..=1.0).contains(&p.velocity);
        if !times_ok {
            return Err(CreateError::BadTarget(
                "the performance needs a key 0-127, a velocity 0-1 and finite times (note-on and hold at least 0, a positive length)",
            ));
        }
        Ok(())
    }

    /// Measure the clip once: its descriptors (the heuristic start), its features, its k-NN embedding and the two
    /// hashes a result records.
    ///
    /// # Errors
    /// [`CreateError::BadTarget`] for an empty clip, a rate the analysis cannot handle, a non-finite sample or a bad
    /// performance. A clip too short or too quiet to describe is not an error: it has no descriptors, and the
    /// search runs without the heuristic.
    pub fn analyse(&self) -> Result<Analysis, CreateError> {
        self.check()?;
        let cfg = self.feature_cfg();
        let features = FeatureExtractor::new(cfg).extract(&self.audio);
        let descriptors = Describer::new(f64::from(self.sample_rate))
            .describe_some(&self.audio, note_timing(&self.performance()), Want::ALL)
            .ok()
            .filter(|d| !d.silent);
        let nyquist = 0.5 * f64::from(self.sample_rate);
        let top = self
            .bandwidth_hz
            .filter(|b| b.is_finite() && *b > 0.0)
            .map_or(nyquist, |b| b.min(nyquist));
        let embedding = nearest::embed(
            &features,
            &cfg,
            self.perf.on_s,
            nearest::reference_hz(descriptors.as_ref()),
            top,
        );
        let mut audio_hash = Sha256::new();
        for s in &self.audio {
            audio_hash.update(s.to_le_bytes());
        }
        let mut feature_hash = Sha256::new();
        feature_hash.update(embedding.codes);
        for res in &features.logmel {
            for v in res {
                feature_hash.update(v.to_le_bytes());
            }
        }
        Ok(Analysis {
            descriptors,
            features,
            embedding,
            audio_sha256: audio_hash.finalize().into(),
            features_sha256: feature_hash.finalize().into(),
        })
    }
}

/// What [`Target::analyse`] measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    /// The clip's descriptors (`None` when it is too short or too quiet to describe): the heuristic start's input.
    pub descriptors: Option<Descriptors>,
    /// Metric v1's features of the clip.
    pub features: Features,
    /// The k-NN query ([`nearest::embed`]).
    pub embedding: Embedding,
    /// SHA-256 of the clip's samples (`f32`, little-endian, in order): the decoded audio's hash.
    pub audio_sha256: [u8; 32],
    /// SHA-256 of the embedding's codes, then every log-mel value (`f32`, little-endian, resolution by resolution):
    /// the input features' hash.
    pub features_sha256: [u8; 32],
}

/// The starts a job may draw on besides the heuristic: the caller's templates and bank (the solver does no I/O).
#[derive(Clone, Copy, Debug)]
pub struct Inputs<'a> {
    /// The heuristic start's base: the patch the heuristic fills in (`content/templates/init`).
    pub init: &'a Patch,
    /// The templates, in the order they are proposed (`pluck_1osc`, `pad_2osc`, `bass_sub`; one equal to `init` is
    /// skipped as a duplicate).
    pub templates: &'a [Template],
    /// The k-NN bank, if the shell found one; R3a runs without it.
    pub bank: Option<&'a Bank>,
}

/// Settings of a create job. [`Default`] is the CLI's and the plugin's starting point. The default budget, 4,000
/// renders, is about 60 s on 8 workers for a 2–3 s clip of a patch with effects on the development laptop (TP's
/// cost table, docs/r3.md); a shell should instead set it from the screening round's measured rate
/// ([`CreateJob::set_budget`]), and never reads a clock inside the job.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateCfg {
    /// K: the branches refined at once (the starts are screened down to K when there are more).
    pub k: usize,
    /// The most renders of the whole job, the screening round included. Hard: the last round drops whole batches
    /// that would overshoot it.
    pub budget: usize,
    /// The seed of every branch's search (each branch derives its own from it and its start's index).
    pub seed: u64,
    /// How many k-NN neighbours are proposed as starts.
    pub neighbours: usize,
    /// Neighbours closer than this to an already proposed one (embedding distance, nepers) are skipped, so the
    /// starts differ.
    pub min_spread: f64,
    /// The population of each branch's first CMA-ES run; `0` is Hansen's default. Never derived from a worker count.
    pub lambda: usize,
    /// Stop the whole job once a branch's distance is at or below this.
    pub stop_at: Option<f64>,
    /// The shares of the budget left after screening that the three rungs of successive halving spend: all
    /// branches, then the best half, then the finalists. Normalised, so only the ratios matter.
    pub rungs: [f64; 3],
    /// How many branches the last rung keeps.
    pub finalists: usize,
}

impl Default for CreateCfg {
    fn default() -> Self {
        CreateCfg {
            k: 8,
            budget: 4_000,
            seed: 1,
            neighbours: 8,
            min_spread: 0.25,
            lambda: 0,
            stop_at: None,
            rungs: [0.1, 0.3, 0.6],
            finalists: 2,
        }
    }
}

impl CreateCfg {
    fn validate(&self) -> Result<(), CreateError> {
        if self.k == 0 {
            return Err(CreateError::BadCfg("k must be at least 1"));
        }
        if self.budget == 0 {
            return Err(CreateError::BadCfg("budget must be at least 1 render"));
        }
        if self.lambda != 0 && self.lambda < 4 {
            return Err(CreateError::BadCfg(
                "lambda must be 0 (default) or at least 4",
            ));
        }
        if !self.rungs.iter().all(|f| f.is_finite() && *f >= 0.0)
            || self.rungs.iter().sum::<f64>() <= 0.0
        {
            return Err(CreateError::BadCfg(
                "rungs must be non-negative shares with a positive sum",
            ));
        }
        if self.finalists == 0 {
            return Err(CreateError::BadCfg("finalists must be at least 1"));
        }
        if !(self.min_spread.is_finite() && self.min_spread >= 0.0) {
            return Err(CreateError::BadCfg(
                "min_spread must be a finite distance, at least 0",
            ));
        }
        Ok(())
    }
}

/// Why a create job could not start or continue.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CreateError {
    /// The target is unusable (an empty clip, a rate the analysis cannot handle, a bad performance).
    BadTarget(&'static str),
    /// A setting is out of range.
    BadCfg(&'static str),
    /// No start could be made (no heuristic base, template or neighbour).
    NoStarts,
    /// The objective returned a different number of distances than candidates.
    ObjectiveCount {
        /// Candidates asked.
        expected: usize,
        /// Distances returned.
        got: usize,
    },
    /// A branch's search refused its setup (a bug, reported rather than panicked).
    Search(SearchError),
}

impl CreateError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            CreateError::BadTarget(_) => "bad_target",
            CreateError::BadCfg(_) => "bad_create_cfg",
            CreateError::NoStarts => "no_starts",
            CreateError::ObjectiveCount { .. } => "objective_count",
            CreateError::Search(e) => e.code(),
        }
    }

    /// What to do about it.
    #[must_use]
    pub fn hint(&self) -> String {
        match self {
            CreateError::BadTarget(what) => format!("check the decoded clip: {what}"),
            CreateError::BadCfg(what) => format!("fix the setting: {what}"),
            CreateError::NoStarts => {
                "pass the init template as the heuristic start's base".to_owned()
            }
            CreateError::ObjectiveCount { .. } => {
                "an objective returns exactly one distance per candidate, in order".to_owned()
            }
            CreateError::Search(e) => e.hint(),
        }
    }
}

impl fmt::Display for CreateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CreateError::BadTarget(what) => write!(f, "bad target: {what}"),
            CreateError::BadCfg(what) => write!(f, "bad create setting: {what}"),
            CreateError::NoStarts => f.write_str("no start could be made"),
            CreateError::ObjectiveCount { expected, got } => write!(
                f,
                "the objective returned {got} distances for {expected} candidates"
            ),
            CreateError::Search(e) => write!(f, "a branch's search failed: {e}"),
        }
    }
}

impl std::error::Error for CreateError {}

impl From<SearchError> for CreateError {
    fn from(e: SearchError) -> Self {
        CreateError::Search(e)
    }
}

/// Create patches that sound like `target`: measure it, build the start portfolio from `inputs`, then run a
/// [`CreateJob`] to the end of its budget, evaluating each round's merged batch with `objective`.
///
/// `progress` is called after every round, the last included, with the job (its [`CreateJob::progress`] and
/// [`CreateJob::candidates`] for posting); return `false` to stop, which is how a shell applies a wall-clock cap
/// or a cancel ([`CreateStop::Cancelled`]). The solver reads no clock.
///
/// # Errors
/// As [`Target::analyse`], [`CreateJob::new`] and [`CreateJob::tell`].
pub fn create(
    target: &Target,
    inputs: &Inputs<'_>,
    objective: &mut dyn Objective,
    cfg: &CreateCfg,
    progress: &mut dyn FnMut(&CreateJob) -> bool,
) -> Result<CreateResult, CreateError> {
    let analysis = target.analyse()?;
    let portfolio = Portfolio::build(&analysis, inputs, cfg);
    let mut job = CreateJob::new(target, &analysis, portfolio, cfg)?;
    run(&mut job, objective, progress)?;
    Ok(job.result())
}

/// Drive `job` to its end with `objective`, calling `progress` after every round.
///
/// # Errors
/// As [`CreateJob::tell`].
pub fn run(
    job: &mut CreateJob,
    objective: &mut dyn Objective,
    progress: &mut dyn FnMut(&CreateJob) -> bool,
) -> Result<(), CreateError> {
    loop {
        let batch = job.ask();
        if batch.is_empty() {
            return Ok(());
        }
        let distances = objective.distances(&batch);
        job.tell(&distances)?;
        let go_on = progress(job);
        if !go_on && !job.finished() {
            job.cancel();
        }
    }
}
