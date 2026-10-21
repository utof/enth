//! **Owner: lane TP** (freeze point F1s). `enth.create/1`: the record of a create job, written by `enth create` (and
//! the GR3 kit's runs) and read back to replay one.
//!
//! The determinism contract (docs/r3.md, point 4) asks that **every result records its starts as patches**, the
//! input features' hash and the decoded audio's hash, so a run replays from its starts even where a lossy decoder
//! differs in its last bits across CPUs. A record holds:
//!
//! | Field | What |
//! | --- | --- |
//! | `schema` | `"enth.create/1"` ([`SCHEMA`]) |
//! | `input` | the decoded clip's SHA-256, the input features' SHA-256, the rate, the frames, the render seed, the performance, the bandwidth and noise floor the intake reported |
//! | `settings` | every [`CreateCfg`] field, the objective's metric id (the caller's), the embedding id, the bank's SHA-256 and size when one was used, the templates' names |
//! | `starts` | **every start as a patch** (patch file schema 1), with its provenance, its screening distance and the branch it became |
//! | `candidates` | the K branches: the refined patch, its distance, the proposal's distance, state, last rung, renders, runs, trace |
//! | `rounds` | per round: renders, rung, live branches, and `seconds` when the shell stamped its clock |
//! | `renders`, `budget`, `stop`, `best`, `notes` | the totals, why it ended, the best candidate's index, notices |
//!
//! **Lenient on input:** a record read back ignores fields it does not know, so a shell may add its own (the CLI's
//! `hint`; FE's intake report, embedded whole as `intake` once FE's type lands, additively). Distances that can be
//! non-finite (a start never evaluated) are `null`. Changing a field's meaning needs `enth.create/2`.
//!
//! [`CreateRecord::starts`] rebuilds the starts and [`CreateRecord::cfg`] the settings, so
//! `CreateJob::new(target, analysis, portfolio, cfg)` with the same target replays the run.

use enth_patch::file::PatchFile;
use enth_patch::{Invalid, Patch};
use serde::{Deserialize, Serialize};

use super::candidate::{BranchStep, CandidateState, CreateResult, CreateStop, Provenance};
use super::portfolio::{Portfolio, Start};
use super::{Analysis, CreateCfg, Target};
use crate::nearest::{Bank, EMBEDDING_ID};

/// The schema id of a create record.
pub const SCHEMA: &str = "enth.create/1";

/// Lowercase hex of a hash.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0xf)]));
    }
    out
}

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

/// The performance a job played, under FE's names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerfRecord {
    /// MIDI note.
    pub key: u8,
    /// Velocity, 0–1.
    pub velocity: f32,
    /// Note-on, clip time.
    pub on_s: f64,
    /// Hold.
    pub hold_s: f64,
    /// The render's length.
    pub duration_s: f64,
    /// YIN's median pitch, when pitched.
    pub f0_hz: Option<f64>,
}

/// What the job was given.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputRecord {
    /// SHA-256 of the decoded mono clip (`f32` little-endian samples).
    pub audio_sha256: String,
    /// SHA-256 of the input features (the k-NN embedding, then the log-mel).
    pub features_sha256: String,
    /// The file's rate.
    pub sample_rate: u32,
    /// Frames in the clip.
    pub frames: usize,
    /// The render seed.
    pub render_seed: u64,
    /// The performance.
    pub perf: PerfRecord,
    /// The intake's detected bandwidth.
    pub bandwidth_hz: Option<f64>,
    /// The intake's noise floor, dBFS.
    pub noise_floor_dbfs: Option<f64>,
}

/// The bank a job retrieved from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankRecord {
    /// SHA-256 of the bank file.
    pub sha256: String,
    /// Its entries.
    pub entries: usize,
    /// Its generator version.
    pub generator: u16,
}

/// The settings a job ran with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsRecord {
    /// K.
    pub k: usize,
    /// The render budget.
    pub budget: usize,
    /// The seed.
    pub seed: u64,
    /// k-NN neighbours proposed.
    pub neighbours: usize,
    /// Their minimum spread, nepers.
    pub min_spread: f64,
    /// λ (0 = Hansen's default).
    pub lambda: usize,
    /// The goal distance, if any.
    pub stop_at: Option<f64>,
    /// The rungs' shares.
    pub rungs: [f64; 3],
    /// The finalists.
    pub finalists: usize,
    /// The objective's metric id (the caller's: `logmel-l1/1`, or OBJ's robust id).
    pub objective: String,
    /// The k-NN embedding's id.
    pub embedding: String,
    /// The bank, when one was used.
    pub bank: Option<BankRecord>,
    /// The templates proposed, by name.
    pub templates: Vec<String>,
}

/// One start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StartRecord {
    /// Its index.
    pub index: usize,
    /// Where it came from.
    pub provenance: Provenance,
    /// The start patch.
    pub patch: PatchFile,
    /// Its screening distance, when screened.
    pub screen_distance: Option<f64>,
    /// The branch it became, or `null` if screened out.
    pub branch: Option<usize>,
}

/// One candidate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateRecord {
    /// The branch.
    pub branch: usize,
    /// Its start's index.
    pub start: usize,
    /// Where the start came from.
    pub provenance: Provenance,
    /// The refined patch.
    pub patch: PatchFile,
    /// Its distance.
    pub distance: Option<f64>,
    /// The proposal's distance.
    pub proposal_distance: Option<f64>,
    /// Where it stands.
    pub state: CandidateState,
    /// The last rung it took part in.
    pub rung: usize,
    /// Renders its search used.
    pub renders: usize,
    /// CMA-ES runs begun.
    pub runs: usize,
    /// One step per round it took part in.
    pub trace: Vec<StepRecord>,
}

/// One branch step.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    /// The round.
    pub round: usize,
    /// The branch's renders.
    pub renders: usize,
    /// The job's renders.
    pub job_renders: usize,
    /// The branch's best distance.
    pub best: Option<f64>,
}

/// One round.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoundRecord {
    /// Its number.
    pub round: usize,
    /// The job's renders after it.
    pub renders: usize,
    /// Its rung (`null` for the screening round).
    pub rung: Option<usize>,
    /// The branches (or starts) that evaluated a batch in it.
    pub live: usize,
    /// Seconds since the job started when the round ended, stamped by the shell (the solver reads no clock).
    pub seconds: Option<f64>,
}

/// `enth.create/1` (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateRecord {
    /// `"enth.create/1"`.
    pub schema: String,
    /// What the job was given.
    pub input: InputRecord,
    /// How it ran.
    pub settings: SettingsRecord,
    /// Every start, as a patch.
    pub starts: Vec<StartRecord>,
    /// The K candidates, in branch order.
    pub candidates: Vec<CandidateRecord>,
    /// Every round.
    pub rounds: Vec<RoundRecord>,
    /// Renders used.
    pub renders: usize,
    /// The budget.
    pub budget: usize,
    /// Why it ended.
    pub stop: CreateStop,
    /// The best candidate's index in `candidates`.
    pub best: Option<usize>,
    /// Notices.
    pub notes: Vec<String>,
}

/// What a record needs besides the result: the job's inputs.
#[derive(Clone, Copy, Debug)]
pub struct RecordContext<'a> {
    /// The target.
    pub target: &'a Target,
    /// Its analysis.
    pub analysis: &'a Analysis,
    /// The settings.
    pub cfg: &'a CreateCfg,
    /// The objective's metric id.
    pub objective: &'a str,
    /// The bank, when one was used.
    pub bank: Option<&'a Bank>,
    /// The templates' names.
    pub templates: &'a [String],
}

impl CreateRecord {
    /// The record of `result` (rounds unstamped: the shell fills `rounds[i].seconds`).
    #[must_use]
    pub fn new(result: &CreateResult, ctx: &RecordContext<'_>) -> CreateRecord {
        let t = ctx.target;
        let cfg = ctx.cfg;
        CreateRecord {
            schema: SCHEMA.to_owned(),
            input: InputRecord {
                audio_sha256: hex(&ctx.analysis.audio_sha256),
                features_sha256: hex(&ctx.analysis.features_sha256),
                sample_rate: t.sample_rate,
                frames: t.audio.len(),
                render_seed: t.render_seed,
                perf: PerfRecord {
                    key: t.perf.key,
                    velocity: t.perf.velocity,
                    on_s: t.perf.on_s,
                    hold_s: t.perf.hold_s,
                    duration_s: t.perf.duration_s,
                    f0_hz: t.perf.f0_hz,
                },
                bandwidth_hz: t.bandwidth_hz,
                noise_floor_dbfs: t.noise_floor_dbfs,
            },
            settings: SettingsRecord {
                k: cfg.k,
                budget: cfg.budget,
                seed: cfg.seed,
                neighbours: cfg.neighbours,
                min_spread: cfg.min_spread,
                lambda: cfg.lambda,
                stop_at: cfg.stop_at,
                rungs: cfg.rungs,
                finalists: cfg.finalists,
                objective: ctx.objective.to_owned(),
                embedding: EMBEDDING_ID.to_owned(),
                bank: ctx.bank.map(|b| BankRecord {
                    sha256: hex(&b.sha256()),
                    entries: b.len(),
                    generator: b.header().generator,
                }),
                templates: ctx.templates.to_vec(),
            },
            starts: result
                .starts
                .iter()
                .enumerate()
                .map(|(index, s)| StartRecord {
                    index,
                    provenance: s.provenance.clone(),
                    patch: s.patch.to_file(),
                    screen_distance: s.screen_distance,
                    branch: s.branch,
                })
                .collect(),
            candidates: result
                .candidates
                .iter()
                .map(|c| CandidateRecord {
                    branch: c.branch,
                    start: c.start,
                    provenance: c.provenance.clone(),
                    patch: c.patch.to_file(),
                    distance: finite(c.distance),
                    proposal_distance: c.proposal_distance.and_then(finite),
                    state: c.state,
                    rung: c.rung,
                    renders: c.renders,
                    runs: c.runs,
                    trace: c.trace.iter().map(step_record).collect(),
                })
                .collect(),
            rounds: result
                .rounds
                .iter()
                .map(|r| RoundRecord {
                    round: r.round,
                    renders: r.renders,
                    rung: r.rung,
                    live: r.live,
                    seconds: None,
                })
                .collect(),
            renders: result.renders,
            budget: result.budget,
            stop: result.stop,
            best: result.best().map(|c| c.branch),
            notes: result.notes.clone(),
        }
    }

    /// The settings the job ran with.
    #[must_use]
    pub fn cfg(&self) -> CreateCfg {
        let s = &self.settings;
        CreateCfg {
            k: s.k,
            budget: s.budget,
            seed: s.seed,
            neighbours: s.neighbours,
            min_spread: s.min_spread,
            lambda: s.lambda,
            stop_at: s.stop_at,
            rungs: s.rungs,
            finalists: s.finalists,
        }
    }

    /// The recorded starts, as a portfolio to replay (in their recorded order, with their provenance).
    ///
    /// # Errors
    /// The first start whose patch file does not load in this build.
    pub fn starts(&self) -> Result<Portfolio, Invalid> {
        let mut starts = Vec::with_capacity(self.starts.len());
        for s in &self.starts {
            let patch: Patch = s.patch.to_patch()?;
            starts.push(Start {
                patch,
                provenance: s.provenance.clone(),
            });
        }
        Ok(Portfolio {
            starts,
            notes: Vec::new(),
        })
    }
}

fn step_record(s: &BranchStep) -> StepRecord {
    StepRecord {
        round: s.round,
        renders: s.renders,
        job_renders: s.job_renders,
        best: finite(s.best),
    }
}
