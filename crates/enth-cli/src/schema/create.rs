//! **Owner: lane CLC.** The object `enth create` prints, `enth.create/1` (R3, "Sound to patch"): `enth_solver`'s
//! `CreateRecord` (lane TP: what a job was given, how it ran, every start and candidate as a patch) plus the fields
//! the shell adds (`decoded`, `elapsed_s`, `cap_s`, `score_s`, `played`, `wrote`, `hint`).
//!
//! The types here only **describe** that object: `enth create` builds it from the solver's record and prints it
//! **exactly** (no 4-digit rounding: the patches and seeds replay the run), and the tests deserialize the output into
//! [`CreateOut`] (`deny_unknown_fields`) so the code and `enth schema create` cannot drift. The solver's record reads
//! leniently (a newer build's extra field or enum value is ignored there); this one is exact for the build.
//!
//! The intake's report is FE's `IntakeReport` (its own `version`, 1) and is left open here: it is described in
//! docs/r3.md. Changing a field's meaning needs `enth.create/2`; adding a field to the object is allowed within
//! `/1` (readers ignore what they do not know), and this type gains it in the same change.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::{PatchDoc, PerfFile};

/// `enth.create/1`: a sound file in, K candidate patches out, and everything needed to replay the run.
///
/// Numbers are printed exactly. Distances are the objective's (`settings.objective`, lane OBJ's
/// `logmel-pool85-robust/1`: 0 is identical, smaller is closer) and are **not** on `enth compare`'s or
/// `enth match`'s scale (`logmel-l1/1`). The run is a pure function of the decoded clip (`input.audio_sha256`), the
/// settings and the renders: the same for any `--workers`, and `rounds[].seconds`, `elapsed_s` and `cap_s` are the
/// only clock. A run stopped by `--seconds` repeats on any machine as the same job with its `renders` as the budget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateOut {
    /// `enth.create/1`.
    pub schema: String,
    /// What the job was given: the scored clip, the performance **played**, the intake's bandwidth and floor.
    pub input: CreateInputOut,
    /// FE's intake report, whole: its settings, the performance it **inferred** (before any `--perf`, `--note`,
    /// `--vel` or `--hold`), the span kept, the degradation it found (bandwidth, floor, clipping, hum, SNR, tail) and
    /// its warnings. Open here: see docs/r3.md.
    pub intake: Option<Value>,
    /// Every setting the job ran under, as the job reports them.
    pub settings: CreateSettingsOut,
    /// Every start, as a patch: the heuristic start, the templates, the k-NN neighbours.
    pub starts: Vec<CreateStartOut>,
    /// The K candidates (fewer when the portfolio held fewer starts, or the budget ended the screening), in branch
    /// order.
    pub candidates: Vec<CreateCandidateOut>,
    /// Every round, in order.
    pub rounds: Vec<CreateRoundOut>,
    /// The renders the job used (at most `settings.budget`).
    pub renders: usize,
    /// Why it ended. Exit 0 for every cause: a run the cap stopped is a result.
    pub stop: CreateStopOut,
    /// The closest candidate: its index in `candidates` (null when no candidate has a distance).
    pub best: Option<usize>,
    /// Notices, in the order they arose: what the file's decoding found, a performance override, a missing bank, a
    /// window that cut the clip, fewer candidates than K, a stop at the cap. The same lines go to stderr as `note:`.
    pub notes: Vec<String>,
    /// How the file was read: `WAV, 16-bit, 48000 Hz, mono`.
    pub decoded: String,
    /// Wall-clock seconds from the start to the end of the command.
    pub elapsed_s: f64,
    /// The `--seconds` cap.
    pub cap_s: f64,
    /// The `--score-seconds` window asked for (the scored clip is `input.frames` long: shorter when the file is).
    pub score_s: f64,
    /// The performance every candidate was played with, as `enth.perf/1`: pass it as `--perf` to `enth render`,
    /// `enth describe`, `enth compare --target FILE` or `enth match`.
    pub played: PerfFile,
    /// The files written (`--out`, `--record`), in that order.
    pub wrote: Vec<String>,
    /// What to do next.
    pub hint: String,
}

/// What the job was given.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInputOut {
    /// SHA-256 of the scored mono clip (`f32` little-endian samples): a lossy decoder may differ in its last bits
    /// across CPUs, so a replay checks this.
    pub audio_sha256: String,
    /// SHA-256 of the input features (the k-NN embedding, then the log-mel).
    pub features_sha256: String,
    /// The file's own rate (Hz): the clip's, and every candidate's render rate (no resampling).
    pub sample_rate: u32,
    /// Frames in the scored clip.
    pub frames: usize,
    /// The render seed of every candidate.
    pub render_seed: u64,
    /// The performance played: the intake's inference, or the override.
    pub perf: CreatePerfOut,
    /// The highest frequency the upload still carries, when the intake found a steep low-pass.
    pub bandwidth_hz: Option<f64>,
    /// The noise floor in dBFS, when the intake measured one and its lead-in confirms it.
    pub noise_floor_dbfs: Option<f64>,
}

/// The performance a job played, under the intake's names. All times are clip time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePerfOut {
    /// The MIDI key (the note nearest the measured pitch; 48 when unpitched).
    pub key: u8,
    /// Velocity, 0 to 1 (the intake fixes it at 100/127).
    pub velocity: f32,
    /// The note-on, on the engine's 32-frame grid (a pre-roll of about 5 ms precedes the sound).
    pub on_s: f64,
    /// How long the note is held; the note-off may fall after the clip's end (the note is then held throughout).
    pub hold_s: f64,
    /// The scored clip's length, which is the render's.
    pub duration_s: f64,
    /// The measured fundamental, when the clip is pitched.
    pub f0_hz: Option<f64>,
}

/// The settings a job ran under.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSettingsOut {
    /// K: the branches refined at once (`--candidates`).
    pub k: usize,
    /// The render cap (`--budget`; default 4000): only a cap, the schedule never depends on it.
    pub budget: usize,
    /// The search's seed (`--seed`).
    pub seed: u64,
    /// How many k-NN neighbours were proposed (16).
    pub neighbours: usize,
    /// Neighbours closer than this to one already proposed are skipped (nepers).
    pub min_spread: f64,
    /// The population of each branch's first CMA-ES run (0 is Hansen's default).
    pub lambda: usize,
    /// The goal distance (`--stop-at`), when one was given.
    pub stop_at: Option<f64>,
    /// The first rung, in a branch's own renders (50): rung `r` ends at `first_rung * eta^r`.
    pub first_rung: usize,
    /// The halving factor (2).
    pub eta: usize,
    /// How many branches pruning keeps at least (2).
    pub finalists: usize,
    /// The objective's metric id: `logmel-pool85-robust/1`.
    pub objective: String,
    /// The k-NN embedding's id: `knn-logmel-rel/1`.
    pub embedding: String,
    /// The bank, when `--bank` gave one.
    pub bank: Option<CreateBankOut>,
    /// The templates proposed as starts, by name.
    pub templates: Vec<String>,
}

/// The k-NN bank a job retrieved from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBankOut {
    /// SHA-256 of the bank file.
    pub sha256: String,
    /// Its entries.
    pub entries: usize,
    /// Its generator version.
    pub generator: u16,
}

/// Where a start came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CreateProvenanceOut {
    /// The init template with the upload's descriptors applied (pitch to tuning, envelope to ADSR, centroid to
    /// cutoff). Always kept.
    Heuristic,
    /// A template, its structure kept and its parameters searched.
    Template {
        /// The template's name (`pluck_1osc`).
        name: String,
    },
    /// A neighbour from the k-NN bank, regenerated from its generator spec and transposed to the upload's note.
    Knn {
        /// Its rank among the neighbours proposed (0 is the nearest).
        rank: usize,
        /// Its index in the bank.
        bank_index: usize,
        /// Its generator spec: the seed...
        seed: u64,
        /// ...the difficulty...
        difficulty: u8,
        /// ...and `musical`.
        musical: u8,
        /// Its embedding distance to the upload, in nepers.
        embedding_distance: f64,
    },
    /// One of a model's hypotheses (R3b; not produced yet).
    Model {
        /// The model file's id.
        model: String,
        /// Which of its hypotheses.
        hypothesis: usize,
        /// The model's confidence in it, when it has one.
        confidence: Option<f64>,
    },
}

/// One start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateStartOut {
    /// Its index in `starts`.
    pub index: usize,
    /// Where it came from.
    pub provenance: CreateProvenanceOut,
    /// The start, as a patch file (`enth schema patch`).
    pub patch: PatchDoc,
    /// Its screening distance, when it was screened (the budget may end the screening round first).
    pub screen_distance: Option<f64>,
    /// The branch it became, or null if screening dropped it.
    pub branch: Option<usize>,
}

/// Where a branch stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CreateStateOut {
    /// Kept as a branch, not evaluated yet.
    Proposed,
    /// Its search is running.
    Searching,
    /// Its search ended (the job's end, its own exhaustion, or the budget's final cut).
    Done,
    /// Successive halving dropped it after `rung`.
    Pruned,
}

/// Why a job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CreateStopOut {
    /// The render budget was reached (the normal end of a render-capped run).
    Budget,
    /// The `--seconds` cap stopped it (a result: the rounds that finished).
    Cancelled,
    /// Every branch's search ended on its own before the budget.
    Exhausted,
    /// A candidate reached `--stop-at`.
    Goal,
}

/// One candidate: a start, refined.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCandidateOut {
    /// Its branch (equal to its index in `candidates`).
    pub branch: usize,
    /// The index in `starts` of the start it began from.
    pub start: usize,
    /// Where that start came from.
    pub provenance: CreateProvenanceOut,
    /// The refined patch, as a patch file.
    pub patch: PatchDoc,
    /// Its distance to the upload (smaller is closer); null when it never rendered.
    pub distance: Option<f64>,
    /// The start's own distance (what the proposal gave before the search).
    pub proposal_distance: Option<f64>,
    /// Where it stands.
    pub state: CreateStateOut,
    /// The last rung it took part in.
    pub rung: usize,
    /// Renders its search used.
    pub renders: usize,
    /// CMA-ES runs begun.
    pub runs: usize,
    /// One step per round it took part in.
    pub trace: Vec<CreateStepOut>,
}

/// One round a branch took part in.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateStepOut {
    /// The job's round (0 is the screening round).
    pub round: usize,
    /// The branch's renders so far.
    pub renders: usize,
    /// The job's renders so far.
    pub job_renders: usize,
    /// The branch's best distance so far.
    pub best: Option<f64>,
}

/// One round of the job.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRoundOut {
    /// Its number (0 is the screening round).
    pub round: usize,
    /// The job's renders after it.
    pub renders: usize,
    /// Its rung (null for the screening round).
    pub rung: Option<usize>,
    /// The branches (or starts) that evaluated a batch in it.
    pub live: usize,
    /// Seconds since the job started when the round ended.
    pub seconds: Option<f64>,
}
