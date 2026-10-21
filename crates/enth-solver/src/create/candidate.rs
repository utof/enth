//! **Owner: lane TP** (freeze point F1s, together with FE's intake types). Pre-declared by lane S0s (R3).
//!
//! What a create job answers: K [`Candidate`]s, each a patch with its distance, its **provenance** (heuristic,
//! template, k-NN or model), the start it grew from (Synplant-style branches: you hear what the start gave and what
//! the search added) and its trace of renders. This is the API the CLI (`enth.create/1`, [`record`](super::record)),
//! the plugin, the session's create state, the GR3 kit and R3b build on, so a judge reviews it at F1s; after F2s it
//! changes only additively (docs/phase-3.md, "Freezes"). The trace holds renders and rounds, never time: the shell
//! stamps each round with its clock ([`RoundInfo`], `seconds` in the record), because the solver reads none.

use enth_patch::Patch;
use serde::{Deserialize, Serialize};

/// Where a start came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Provenance {
    /// The heuristic start: the init template with the target's descriptors applied (pitch → tuning, envelope →
    /// ADSR, centroid → cutoff), by the search's probe. Always kept.
    Heuristic,
    /// A template (`content/templates/<name>`), its structure kept and its parameters searched.
    Template {
        /// The template's name.
        name: String,
    },
    /// A neighbour retrieved from the k-NN bank ([`nearest`](crate::nearest)): a generator-v1 patch regenerated from
    /// its spec and transposed to the query's note.
    Knn {
        /// Its rank among the neighbours proposed (0 is the nearest).
        rank: usize,
        /// Its index in the bank.
        bank_index: usize,
        /// Its generator spec: the seed...
        seed: u64,
        /// ...the difficulty...
        difficulty: u8,
        /// ...and `musical` (the generator and template are the bank's).
        musical: u8,
        /// Its embedding distance to the query, in nepers.
        embedding_distance: f64,
    },
    /// One of a model's hypotheses (R3b, lane MS; reserved so the type does not change at F3s).
    Model {
        /// The model file's id.
        model: String,
        /// Which of its K hypotheses.
        hypothesis: usize,
        /// The model's confidence in it, when it has a confidence head.
        confidence: Option<f64>,
    },
}

impl Provenance {
    /// The kind as a short word: `heuristic`, `template`, `knn` or `model` (the serde tag).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Provenance::Heuristic => "heuristic",
            Provenance::Template { .. } => "template",
            Provenance::Knn { .. } => "knn",
            Provenance::Model { .. } => "model",
        }
    }
}

/// Where a branch stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    /// Kept as a branch, not evaluated yet.
    Proposed,
    /// Its search is running.
    Searching,
    /// Its search ended (the job's end, its own exhaustion, or the budget's final cut).
    Done,
    /// Successive halving dropped it after [`Candidate::rung`].
    Pruned,
}

/// Why a create job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateStop {
    /// The render budget was spent (the normal end).
    Budget,
    /// The caller's progress callback said stop (a wall-clock cap or a cancel).
    Cancelled,
    /// Every branch's search ended on its own before the budget.
    Exhausted,
    /// A branch reached [`CreateCfg::stop_at`](super::CreateCfg::stop_at).
    Goal,
}

/// One round a branch took part in.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BranchStep {
    /// The job's round (0 is the screening round when there is one).
    pub round: usize,
    /// The branch's renders so far, this round included.
    pub renders: usize,
    /// The job's renders so far, this round included (the x axis of an anytime curve).
    pub job_renders: usize,
    /// The branch's best distance so far.
    pub best: f64,
}

/// One round of the job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundInfo {
    /// Its number, from 0.
    pub round: usize,
    /// The job's renders after it.
    pub renders: usize,
    /// The rung it belonged to (0, 1, 2), or `None` for the screening round.
    pub rung: Option<usize>,
    /// The branches that evaluated a batch in it (the starts, for the screening round).
    pub live: usize,
}

/// One start of the portfolio, kept as a branch or screened out.
#[derive(Clone, Debug, PartialEq)]
pub struct StartOutcome {
    /// Where it came from.
    pub provenance: Provenance,
    /// The start patch (the proposal).
    pub patch: Patch,
    /// Its distance in the screening round, when there was one (more starts than K).
    pub screen_distance: Option<f64>,
    /// The branch it became, or `None` if screening dropped it.
    pub branch: Option<usize>,
}

/// One branch's answer: a refined patch and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// The branch's index (0..K, in start order: stable tiles in a UI).
    pub branch: usize,
    /// The start it grew from (an index into [`CreateResult::starts`]).
    pub start: usize,
    /// Where that start came from.
    pub provenance: Provenance,
    /// The start patch, as proposed.
    pub proposal: Patch,
    /// The proposal's own distance, once evaluated (the screening round's, else the search's probe).
    pub proposal_distance: Option<f64>,
    /// The best patch the branch found (the proposal until its search improves on it).
    pub patch: Patch,
    /// Its distance (the caller's objective; lower is better).
    pub distance: f64,
    /// Where the branch stands.
    pub state: CandidateState,
    /// The last rung of successive halving it took part in (0, 1 or 2).
    pub rung: usize,
    /// Renders its search used (the screening render is the job's).
    pub renders: usize,
    /// CMA-ES runs its search began.
    pub runs: usize,
    /// One step per round it took part in.
    pub trace: Vec<BranchStep>,
}

/// What a create job found.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateResult {
    /// Every start of the portfolio, in portfolio order (the heuristic first), each recorded as a patch.
    pub starts: Vec<StartOutcome>,
    /// The K branches, in branch order.
    pub candidates: Vec<Candidate>,
    /// Renders used, the screening round included (at most the budget).
    pub renders: usize,
    /// The budget it ran under.
    pub budget: usize,
    /// Every round, in order.
    pub rounds: Vec<RoundInfo>,
    /// Why it ended.
    pub stop: CreateStop,
    /// What the user should know (a bank entry that no longer regenerates, a clip too short to describe).
    pub notes: Vec<String>,
}

impl CreateResult {
    /// The candidate with the lowest distance (ties to the lower branch).
    #[must_use]
    pub fn best(&self) -> Option<&Candidate> {
        self.candidates.iter().min_by(|a, b| {
            a.distance
                .total_cmp(&b.distance)
                .then(a.branch.cmp(&b.branch))
        })
    }

    /// The candidates from best to worst (ties to the lower branch): the order a "Use this" list shows.
    #[must_use]
    pub fn ranked(&self) -> Vec<&Candidate> {
        let mut out: Vec<&Candidate> = self.candidates.iter().collect();
        out.sort_by(|a, b| {
            a.distance
                .total_cmp(&b.distance)
                .then(a.branch.cmp(&b.branch))
        });
        out
    }
}
