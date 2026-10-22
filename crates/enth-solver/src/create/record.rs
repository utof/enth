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
//! | `input` | the scored clip's SHA-256, the input features' SHA-256, the rate, the frames, the render seed, the performance **played** (an override included), the bandwidth and the confirmed noise floor |
//! | `intake` | FE's [`IntakeReport`] whole (its settings, the performance it **inferred**, every degradation and warning), when the shell ran the intake; absent for a target built another way |
//! | `settings` | every [`CreateCfg`] field **the job ran under** (from the result, never the shell's copy), the objective's metric id (the caller's), the embedding id, the bank's SHA-256 and size when one was used, the templates' names; **`model`** when a model proposed starts (R3b, lane MS, additive): its id, the file's SHA-256, the model input's and outputs' hashes, Kₘ and the hypotheses dropped ([`ModelRecord`]) |
//! | `starts` | **every start as a patch** (patch file schema 1), with its provenance, its screening distance and the branch it became |
//! | `candidates` | the K branches: the refined patch, its distance, the proposal's distance, state, last rung, renders, runs, trace |
//! | `rounds` | per round: renders, rung, live branches, and `seconds` when the shell stamped its clock |
//! | `renders`, `stop`, `best`, `notes` | the total, why it ended, the best candidate's index, notices |
//!
//! **Lenient on input:** a record read back ignores fields it does not know, so a shell may add its own (the CLI's
//! `hint`), an enum value it does not know (a newer provenance, state or stop) reads as `unknown`, and an intake report
//! it cannot read reads as absent. Distances that can be non-finite (a start never evaluated) are `null`. Changing a
//! field's meaning needs `enth.create/2`.
//!
//! [`CreateRecord::starts`] rebuilds the starts and [`CreateRecord::replay_cfg`] the settings with the budget set to
//! the renders the run used, so `CreateJob::new(target, analysis, portfolio, cfg)` with the same target replays the
//! run, whatever stopped it (the budget, a wall-clock cap, a cancel): rungs never depend on the budget, so a run is
//! a prefix of any longer one. A replay needs the same clip bit for bit (the recorded `audio_sha256`); the recorded
//! starts make retrieval and inference unnecessary, not decoding.

use enth_analysis::intake::IntakeReport;
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

/// The performance a job played, under FE's names (the inferred one, or the shell's override).
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
    /// SHA-256 of the scored mono clip (`f32` little-endian samples).
    pub audio_sha256: String,
    /// SHA-256 of the input features (the k-NN embedding, then the log-mel).
    pub features_sha256: String,
    /// The file's rate.
    pub sample_rate: u32,
    /// Frames in the scored clip.
    pub frames: usize,
    /// The render seed.
    pub render_seed: u64,
    /// The performance.
    pub perf: PerfRecord,
    /// The intake's detected bandwidth.
    pub bandwidth_hz: Option<f64>,
    /// The intake's noise floor, dBFS, when confirmed.
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

/// The settings a job ran under.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsRecord {
    /// K.
    pub k: usize,
    /// The render budget (a hard cap).
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
    /// The first milestone, in a branch's renders.
    pub first_rung: usize,
    /// The halving factor.
    pub eta: usize,
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
    /// The model whose hypotheses joined the starts (R3b, lane MS; additive within `/1`): absent from the JSON when
    /// no model was used, so a record of a run without one is byte-for-byte what it was, and an older record reads
    /// back as `None`. The shell sets it from [`ModelProposals::record`](super::ModelProposals::record).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRecord>,
}

/// The model a job drew starts from (`settings.model`, lane MS). The starts themselves are recorded as patches with
/// [`Provenance::Model`], so a replay needs neither the model file nor inference; these say which file and which
/// input made them, and let anyone check that inference gave the same bits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRecord {
    /// The model's id (the header's; what each start's provenance names).
    pub id: String,
    /// SHA-256 of the model file.
    pub sha256: String,
    /// `ModelInput::sha256`: the model input's hash (feature set 1, computed from the intake before any override);
    /// `null` when the input could not be computed (a silent window), and the model proposed nothing.
    pub input_sha256: Option<String>,
    /// `Outputs::sha256`: the hash of everything the net output (the golden a model file pins); `null` when it did
    /// not run.
    pub outputs_sha256: Option<String>,
    /// Kₘ: the hypotheses the model gives per inference.
    pub hypotheses: usize,
    /// The hypotheses dropped, in hypothesis order, with why (a non-finite value or confidence, a decode error).
    #[serde(default)]
    pub dropped: Vec<DroppedRecord>,
}

/// A hypothesis that did not become a start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedRecord {
    /// Which of the Kₘ.
    pub hypothesis: usize,
    /// Why: `confidence_non_finite`, or a decode error's code (`hypothesis_non_finite`, `hypothesis_rekey`, …).
    pub code: String,
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
    /// FE's intake report, whole (absent when the target did not come from the intake). Read leniently: a report
    /// this build cannot read (a newer intake's) reads as absent rather than refusing the record.
    #[serde(default, deserialize_with = "lenient_intake")]
    pub intake: Option<IntakeReport>,
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
    /// Why it ended.
    pub stop: CreateStop,
    /// The best candidate's index in `candidates`.
    pub best: Option<usize>,
    /// Notices.
    pub notes: Vec<String>,
}

/// What a record needs besides the result: the job's inputs. The settings come from the result (what the job ran
/// under), never from here.
#[derive(Clone, Copy, Debug)]
pub struct RecordContext<'a> {
    /// The target.
    pub target: &'a Target,
    /// Its analysis.
    pub analysis: &'a Analysis,
    /// The intake's report, when the target came from the intake ([`Target::from_intake`]).
    pub intake: Option<&'a IntakeReport>,
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
        let cfg = &result.cfg;
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
            intake: ctx.intake.cloned(),
            settings: SettingsRecord {
                k: cfg.k,
                budget: cfg.budget,
                seed: cfg.seed,
                neighbours: cfg.neighbours,
                min_spread: cfg.min_spread,
                lambda: cfg.lambda,
                stop_at: cfg.stop_at,
                first_rung: cfg.first_rung,
                eta: cfg.eta,
                finalists: cfg.finalists,
                objective: ctx.objective.to_owned(),
                embedding: EMBEDDING_ID.to_owned(),
                bank: ctx.bank.map(|b| BankRecord {
                    sha256: hex(&b.sha256()),
                    entries: b.len(),
                    generator: b.header().generator,
                }),
                templates: ctx.templates.to_vec(),
                model: None,
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
            stop: result.stop,
            best: result.best().map(|c| c.branch),
            notes: result.notes.clone(),
        }
    }

    /// The settings the job ran under, its budget (the cap) included.
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
            first_rung: s.first_rung,
            eta: s.eta,
            finalists: s.finalists,
        }
    }

    /// The settings that replay the run whatever stopped it: [`cfg`](Self::cfg) with the budget set to the renders it
    /// used (the render-equivalent twin). The renders, rounds and candidates' patches and distances come back the
    /// same; the stop reads `budget` for a run a cap or a cancel ended, and a branch an exhausted run pruned after its
    /// last round can read `done`.
    #[must_use]
    pub fn replay_cfg(&self) -> CreateCfg {
        CreateCfg {
            budget: self.renders.max(1),
            ..self.cfg()
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

/// An intake report, or nothing when it is absent, `null` or not one this build reads.
fn lenient_intake<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<IntakeReport>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Maybe {
        Report(Box<IntakeReport>),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Maybe::deserialize(d)? {
        Maybe::Report(r) => Some(*r),
        Maybe::Other(_) => None,
    })
}

fn step_record(s: &BranchStep) -> StepRecord {
    StepRecord {
        round: s.round,
        renders: s.renders,
        job_renders: s.job_renders,
        best: finite(s.best),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire names of `enth.create/1`'s enums and small records: a change here is a schema change.
    #[test]
    fn the_wire_names_are_pinned() {
        let provenance = vec![
            Provenance::Heuristic,
            Provenance::Template {
                name: "pluck_1osc".to_owned(),
            },
            Provenance::Knn {
                rank: 0,
                bank_index: 1234,
                seed: 987_654_321,
                difficulty: 100,
                musical: 50,
                embedding_distance: 0.75,
            },
            Provenance::Model {
                model: "propose-v0".to_owned(),
                hypothesis: 3,
                confidence: Some(0.5),
            },
            Provenance::Unknown,
        ];
        insta::assert_json_snapshot!("provenance", provenance);
        let states = (
            [
                CandidateState::Proposed,
                CandidateState::Searching,
                CandidateState::Done,
                CandidateState::Pruned,
                CandidateState::Unknown,
            ],
            [
                CreateStop::Budget,
                CreateStop::Cancelled,
                CreateStop::Exhausted,
                CreateStop::Goal,
                CreateStop::Unknown,
            ],
        );
        insta::assert_json_snapshot!("states", states);
        let round = RoundRecord {
            round: 2,
            renders: 300,
            rung: Some(0),
            live: 8,
            seconds: Some(4.5),
        };
        let step = StepRecord {
            round: 2,
            renders: 40,
            job_renders: 300,
            best: Some(0.4),
        };
        insta::assert_json_snapshot!("round_and_step", (round, step));
        assert_eq!(SCHEMA, "enth.create/1");
    }
}
