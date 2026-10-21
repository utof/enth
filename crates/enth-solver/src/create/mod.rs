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
//! intake (FE)  ──►  Target::from_intake  ──►  portfolio: heuristic · templates · k-NN (nearest) · proposals  ──►  starts
//!                                                                                                      │
//!            caller's Objective  ◄──  CreateJob: screen, then K Searches, one merged batch per round,
//!            (rayon in the shell)       successive halving at first_rung · eta^r renders per branch, up to the cap
//!                                                                                                      │
//!                                                                         CreateResult  ──►  enth.create/1 (record)
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
//! - A result is a pure function of the target, the starts and the settings, for any worker count: each round's
//!   batch is the concatenation of every live branch's `ask()` in branch order, and the distances are split back by
//!   index.
//! - **The schedule is in renders, never in time, and never in the budget.** Each branch's rung milestones sit at
//!   [`CreateCfg::first_rung`] · [`CreateCfg::eta`]^r of its own renders; pruning and the rounds' composition depend
//!   on render counts and distances alone. **The budget is only a hard cap**, and a wall-clock cap (the progress
//!   callback returning `false`) only truncates: a run capped by either is an exact prefix of a longer one, so the
//!   render-equivalent twin of a timed run is the same job with `budget` = its renders, and one long run holds every
//!   shorter one (docs/r3.md, the determinism contract, point 3).
//! - Every result records its starts as patches, the settings it ran under, the decoded audio's hash and the input
//!   features' hash ([`record`]), so a run replays from its starts even where a lossy decoder differs in its last
//!   bits.
//!
//! # The seam with the intake (lane FE)
//!
//! [`Target`] holds what `create` needs from FE's [`Intake`], field for field under FE's names (agreed through the
//! coordinator), and [`Target::from_intake`] maps one onto the other: the mono clip with the note-on at `perf.on_s`
//! (FE keeps a pre-roll of whole engine segments), cut to the scored window ([`SCORE_S`], 3 s from the clip's start;
//! the intake keeps up to 5 s for its analysis, its warnings and audition), the file's rate, the inferred
//! performance, the detected bandwidth and a confirmed noise floor. enth-analysis is L3, so FE's `InferredPerf`
//! cannot hold an `enth_render::Performance`: [`Target::performance`] builds it. Everything else of the intake report
//! (warnings, clipping, hum, the span) is the shell's to show and goes into the record whole
//! ([`record::RecordContext::intake`]). The objective (metric v1 today, OBJ's robust distance later) is the caller's
//! [`Objective`], built from the same clip, performance and seed with [`Target::metric_target`].
//!
//! [`Search`]: crate::search::Search
//! [`Objective`]: crate::search::Objective
//! [`Intake`]: enth_analysis::intake::Intake

pub mod candidate;
pub mod portfolio;
pub mod record;
pub mod schedule;

use core::fmt;

use enth_analysis::descriptors::Want;
use enth_analysis::features::{FeatureCfg, FeatureExtractor, Features};
use enth_analysis::intake::Intake;
use enth_analysis::{Describer, Descriptors};
use enth_challenge::phrases::{self, HELD};
use enth_challenge::random::observable;
use enth_patch::{ModuleKind, ParamAddr, Patch};
use enth_render::{Note, Performance, RenderSpec};
use sha2::{Digest, Sha256};

use crate::eval::note_timing;
use crate::nearest::{self, Bank, Embedding};
use crate::search::{FreeSet, MetricTarget, Objective, SearchError};

pub use candidate::{
    BranchStep, Candidate, CandidateState, CreateResult, CreateStop, Provenance, RoundInfo,
    StartOutcome,
};
pub use portfolio::{Portfolio, Start, Template};
pub use record::{CreateRecord, SCHEMA};
pub use schedule::{BranchView, CreateJob, Progress};

/// The scored window (s): what every candidate renders and the objective compares, from the clip's first sample
/// (the pre-roll included). The intake keeps up to 5 s for its analysis, its warnings and audition; scoring the
/// first 3 s costs about 60 % of scoring 5 (TP's cost table, docs/r3.md), and the attack and the early body carry
/// most of what tells patches apart. A coordinator decision at F1s; the CLI's `--score-seconds` (CLC) passes
/// another to [`Target::from_intake_scored`], and EVK sweeps 2, 3 and 5 s on dev.
pub const SCORE_S: f64 = 3.0;

/// The inferred performance of the upload, under the names of FE's `InferredPerf`. All times are clip time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetPerf {
    /// The MIDI note nearest the measured pitch (48 when the clip is unpitched; a chord's lowest tone; the first of
    /// several notes).
    pub key: u8,
    /// Velocity, 0–1 (FE fixes it at 100/127).
    pub velocity: f32,
    /// The note-on: FE keeps a pre-roll of whole 32-frame engine segments before the onset, so `round(on_s · sr)` is
    /// a frame of the render grid.
    pub on_s: f64,
    /// How long the note is held: [`Target::performance`] puts the note-off at `on_s + hold_s` (it may fall after
    /// the end of the render: the note is then held throughout).
    pub hold_s: f64,
    /// The clip's length (the render's length): the scored window's.
    pub duration_s: f64,
    /// FE's pitch reading (YIN's median, or the harmonic peaks'), when the clip is pitched: the k-NN reference's
    /// fallback when the descriptors find no pitch.
    pub f0_hz: Option<f64>,
}

impl TargetPerf {
    /// Whether the clip holds anything after the note-off. When the note-off falls at or after the end, no release
    /// is ever heard, and a job does not search the envelopes' release times.
    #[must_use]
    pub fn release_heard(&self) -> bool {
        self.on_s + self.hold_s < self.duration_s
    }
}

/// What a create job is given: the intake's output on the solver's side (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// The mono clip at `sample_rate`, the note-on at `perf.on_s`, exactly `round(perf.duration_s · sample_rate)`
    /// frames. Short clips are kept, never rejected.
    pub audio: Vec<f32>,
    /// The file's own rate (8,000–191,999 Hz); render and analysis run at it, with no resampling.
    pub sample_rate: u32,
    /// The render seed of every candidate (the shell's; [`enth_render::DEFAULT_SEED`] unless a file says otherwise).
    pub render_seed: u64,
    /// The inferred (or overridden) performance.
    pub perf: TargetPerf,
    /// The highest frequency the upload still carries, when the intake found a steep low-pass (a codec, a phone):
    /// the k-NN embedding masks the bands above it, and OBJ's robust distance above `min(bandwidth, sr / 2)`.
    pub bandwidth_hz: Option<f64>,
    /// The noise floor in dBFS, when the intake measured one and its lead-in confirms it (recorded; OBJ's robust
    /// distance uses it; the search ignores it). An unconfirmed floor stays in the intake report only.
    pub noise_floor_dbfs: Option<f64>,
}

impl Target {
    /// The target of an intake, scored over its first [`SCORE_S`] seconds: see [`Target::from_intake_scored`].
    #[must_use]
    pub fn from_intake(intake: &Intake, render_seed: u64) -> Target {
        Target::from_intake_scored(intake, render_seed, SCORE_S)
    }

    /// The target of an intake, scored over its first `score_s` seconds (from the clip's first sample, the pre-roll
    /// included; a shorter clip is scored whole; a non-finite or non-positive `score_s` scores the whole clip):
    ///
    /// | Field | From |
    /// | --- | --- |
    /// | `audio` | `intake.audio`, cut to the window |
    /// | `sample_rate` | `report.sample_rate` (no resampling) |
    /// | `render_seed` | the argument |
    /// | `perf.key`, `velocity`, `on_s`, `hold_s`, `f0_hz` | `report.perf`'s (`on_s` is clip time on the render grid) |
    /// | `perf.duration_s` | the window: `audio.len() / sample_rate` |
    /// | `bandwidth_hz` | `report.degradation.bandwidth_hz` |
    /// | `noise_floor_dbfs` | `report.degradation.noise_floor_dbfs` when the floor is confirmed, else `None` |
    ///
    /// The rest of the report (the note-off FE found, the onsets, the voicing, the SNR, the hum, the clipping, the
    /// tail, the span, the warnings, the channels and lengths) is not mapped: it goes into `enth.create/1` whole.
    #[must_use]
    pub fn from_intake_scored(intake: &Intake, render_seed: u64, score_s: f64) -> Target {
        let r = &intake.report;
        let sr = f64::from(r.sample_rate);
        let window = if score_s.is_finite() && score_s > 0.0 {
            (libm::round(score_s * sr) as usize).max(1)
        } else {
            usize::MAX
        };
        let audio = intake.audio[..intake.audio.len().min(window)].to_vec();
        let duration_s = audio.len() as f64 / sr;
        debug_assert_eq!(libm::round(duration_s * sr) as usize, audio.len());
        let d = &r.degradation;
        Target {
            audio,
            sample_rate: r.sample_rate,
            render_seed,
            perf: TargetPerf {
                key: r.perf.key,
                velocity: r.perf.velocity,
                on_s: r.perf.on_s,
                hold_s: r.perf.hold_s,
                duration_s,
                f0_hz: r.perf.f0_hz,
            },
            bandwidth_hz: d.bandwidth_hz,
            noise_floor_dbfs: d.noise_floor_dbfs.filter(|_| d.floor_confirmed),
        }
    }

    /// The performance every candidate is rendered with: one note at `perf.on_s`, released at `on_s + hold_s`, in a
    /// render of `perf.duration_s`.
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

    /// Metric v1's target for this clip, performance and seed: what a shell's objective compares every candidate
    /// with (`MetricWorker`, or a pool of them). Built here so that no shell scores against another clip, window or
    /// seed than the job renders.
    #[must_use]
    pub fn metric_target(&self) -> MetricTarget {
        MetricTarget::from_audio(&self.audio, &self.performance(), &self.render_spec())
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
            nearest::reference_hz_or(descriptors.as_ref(), self.perf.f0_hz),
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

/// The parameters a create job searches on `patch`: those the held phrase reveals (`observable`, as `enth match`'s
/// default), less every envelope's release when the target's note-off falls at or after the clip's end
/// ([`TargetPerf::release_heard`]).
pub(crate) fn free_set(patch: &Patch, perf: &TargetPerf) -> Result<FreeSet, SearchError> {
    let phrase =
        phrases::phrase(HELD).unwrap_or_else(|| unreachable!("the held phrase is in the table"));
    let mask = observable(patch, phrase);
    let unheard: Vec<ParamAddr> = if perf.release_heard() {
        Vec::new()
    } else {
        patch
            .modulators
            .iter()
            .filter(|m| m.kind == ModuleKind::Adsr)
            .filter_map(|m| ParamAddr::module(m.id.clone(), "release").ok())
            .collect()
    };
    let addrs: Vec<ParamAddr> = mask
        .observable_params()
        .filter(|a| !unheard.contains(a))
        .cloned()
        .collect();
    FreeSet::new(patch, &addrs)
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

/// The starts a job may draw on besides the heuristic: the caller's templates, bank and proposals (the solver does
/// no I/O).
#[derive(Clone, Copy, Debug)]
pub struct Inputs<'a> {
    /// The heuristic start's base: the patch the heuristic fills in (`content/templates/init`).
    pub init: &'a Patch,
    /// The templates, in the order they are proposed (`pluck_1osc`, `pad_2osc`, `bass_sub`; one equal to `init` is
    /// skipped as a duplicate).
    pub templates: &'a [Template],
    /// The k-NN bank, if the shell found one; R3a runs without it.
    pub bank: Option<&'a Bank>,
    /// Proposals from elsewhere, proposed after the k-NN neighbours, in this order: R3b's model hypotheses
    /// ([`Provenance::Model`], lane MS), ranked by the model's confidence. Empty in R3a.
    pub proposals: &'a [Start],
}

/// Settings of a create job. [`Default`] is the CLI's and the plugin's starting point.
///
/// **The budget is only a hard cap.** Successive halving runs on milestones in each branch's own renders
/// ([`first_rung`](Self::first_rung) · [`eta`](Self::eta)^r), so a job capped by its budget, or by the shell's clock
/// in the progress callback, is an exact prefix of the same job with a larger budget. A shell with a wall-clock cap
/// sets a budget it will not reach (or the most renders it allows) and stops the job from its progress callback; the
/// default, 4,000 renders, is about 60 s on 8 workers for a 2–3 s clip of a patch with effects on the development
/// laptop (TP's cost table, docs/r3.md).
#[derive(Clone, Debug, PartialEq)]
pub struct CreateCfg {
    /// K: the branches refined at once (the starts are screened down to K when there are more).
    pub k: usize,
    /// The most renders of the whole job, the screening round included. Hard: the last round keeps the whole
    /// batches that fit, in branch order, and nothing else depends on it.
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
    /// m0: each branch's first milestone, in its own renders (50; tunable on dev). Milestone `r` is
    /// `first_rung · eta^r` ([`CreateCfg::milestone`]).
    pub first_rung: usize,
    /// The halving factor (2): at each milestone the best `1 / eta` of the live branches survive (at least
    /// [`finalists`](Self::finalists)), and the milestones grow by it.
    pub eta: usize,
    /// How many branches pruning keeps at least: they continue in growing rungs until the cap.
    pub finalists: usize,
}

impl Default for CreateCfg {
    fn default() -> Self {
        CreateCfg {
            k: 8,
            budget: 4_000,
            seed: 1,
            neighbours: 16,
            min_spread: 0.25,
            lambda: 0,
            stop_at: None,
            first_rung: 50,
            eta: 2,
            finalists: 2,
        }
    }
}

impl CreateCfg {
    /// Rung `r`'s milestone, in a branch's own renders: `first_rung · eta^r` (saturating).
    #[must_use]
    pub fn milestone(&self, rung: usize) -> usize {
        let exp = u32::try_from(rung).unwrap_or(u32::MAX);
        self.first_rung.saturating_mul(self.eta.saturating_pow(exp))
    }

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
        if self.first_rung == 0 {
            return Err(CreateError::BadCfg("first_rung must be at least 1 render"));
        }
        if self.eta < 2 {
            return Err(CreateError::BadCfg("eta must be at least 2"));
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
    let portfolio = Portfolio::build(target, &analysis, inputs, cfg);
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
