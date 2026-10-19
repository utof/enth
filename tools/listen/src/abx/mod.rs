//! ABX on the search results: **does the solver's patch sound the same as the target?** (the G0 search gate:
//! at least 14 of 20 targets judged *same*, within 60 s on 8 workers).
//!
//! # Stimuli
//!
//! spike-metric's toy synth and CMA-ES search, used as a library: 20 targets from its seeded list, each solved with
//! the Phase 0 settings of ADR-0007 (distance A, heuristic start, IPOP, population 16 for 8 workers, a cap of 5,000
//! evaluations, a wall-clock cap of 60 s enforced by this tool, stop at the strict proxy threshold T1). The real
//! engine cannot make 20 distinct targets yet (its oscillator is a reference saw, its filter a pass-through) and
//! `enth-solver` has no search; when both exist, only [`design`] changes.
//!
//! # Trials
//!
//! `trials_per_target` sessions (default 8). Session *s* has one trial for every target plus two *null* trials
//! (A and B are the same sound, so nobody can be right) and two *obviously different* trials (two different
//! targets), in random order. In every trial A and B are two clips, X is a copy of one of them, and the owner says
//! which. For the real trials A and B are the target and the solution in random order, X is either, and across a
//! target's sessions both are balanced. All clips are loudness-matched to -23 LUFS with `enth-analysis`.
//!
//! # Verdicts
//!
//! A target is judged *different* when the owner's correct answers beat guessing (one-sided exact binomial, default
//! level 0.05: with 8 trials, 7 or 8 correct) and *same* otherwise. The null trials estimate false alarms, the
//! obviously different trials check attention, and the report says what the test could not see (its power).

mod analyze;
mod design;
mod run;

use serde::{Deserialize, Serialize};
use spike_metric::render::RenderCfg;

use crate::flags::Flag;

pub use analyze::{Report, analyze};
pub use design::{Designed, design};
pub use run::{AbxArgs, SessionEnd, SessionSel, clip_path, ensure_clips, run, synthetic_answers};

/// Manifest schema (the hidden key to every trial).
pub const SCHEMA_MANIFEST: &str = "enth.listen.abx-manifest/1";
/// Answer file schema.
pub const SCHEMA_ANSWERS: &str = "enth.listen.abx-answers/1";
/// Report schema.
pub const SCHEMA_REPORT: &str = "enth.listen.abx-report/1";

/// G0's bar: at least this many of 20 targets judged *same*.
pub const G0_SAME_OF_20: usize = 14;
/// Seconds of silence between A, B and X.
pub const GAP_S: f32 = 0.5;

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
    /// `"a"` or `"b"`, as stored in the answer files.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Side::A => "a",
            Side::B => "b",
        }
    }
}

/// Where the search stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum StopAt {
    /// At the strict proxy threshold (the median distance of a +-1 % jitter): ADR-0007's setting.
    T1,
    /// At the loose one (+-3 %).
    T3,
    /// Never: the budget or the time cap ends the search.
    Never,
}

/// Everything that shapes the design (stored in the manifest).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Seeds the target list and the trial design.
    pub seed: u64,
    /// Number of targets (G0 asks for 20).
    pub n_targets: usize,
    /// Sessions, and trials per target.
    pub trials_per_target: usize,
    /// Obviously-same (null) trials per session.
    pub n_null: usize,
    /// Obviously-different trials per session.
    pub n_diff: usize,
    /// Search workers (G0 asks for 8).
    pub workers: usize,
    /// Wall-clock cap per target in seconds (G0 asks for 60).
    pub time_cap_s: f64,
    /// Evaluation cap per search.
    pub budget: usize,
    /// Where each search stops.
    pub stop_at: StopAt,
    /// First CMA-ES population.
    pub lambda: usize,
    /// Initial step size around the heuristic start.
    pub sigma_start: f64,
    /// Search seed base: target *i* searches with `search_seed + i`.
    pub search_seed: u64,
    /// The scorer's render settings (2 s, A3 held for 1.5 s, 48 kHz).
    pub render: RenderCfg,
}

/// One target and the solver's answer to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// 0-based.
    pub index: usize,
    /// The target's normalised parameters.
    pub target: Vec<f64>,
    /// The solver's best parameters.
    pub solution: Vec<f64>,
    /// Strict proxy threshold of distance A for this target.
    pub t1: f64,
    /// Loose proxy threshold.
    pub t3: f64,
    /// Distance A of the starting point (the heuristic's guess), if the search had one.
    pub start_distance: Option<f64>,
    /// Distance A of the solution.
    pub distance: f64,
    /// Total evaluations.
    pub evals: usize,
    /// CMA-ES restarts.
    pub restarts: usize,
    /// Wall-clock seconds of the search.
    pub wall_s: f64,
    /// The search reached T1 (the strict proxy).
    pub reached_t1: bool,
    /// The search reached T3.
    pub reached_t3: bool,
    /// Evaluations when T1 was first reached.
    pub evals_to_t1: Option<usize>,
    /// Wall-clock seconds when T1 was first reached.
    pub wall_to_t1_s: Option<f64>,
    /// The solution has the target's waveform.
    pub wave_ok: bool,
    /// Mean absolute error of the continuous parameters, in normalised units.
    pub param_err: f64,
    /// Clip id of the target.
    pub target_clip: String,
    /// Clip id of the solution.
    pub solution_clip: String,
}

/// What a clip is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The target's sound.
    Target,
    /// The solver's sound.
    Solution,
}

/// One clip file under `clips/`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipInfo {
    /// Random file stem.
    pub id: String,
    /// The item it belongs to.
    pub item: usize,
    /// Target or solution.
    pub role: Role,
    /// Integrated loudness of the raw render (after the fade), LUFS; `None` when silent.
    pub raw_lufs: Option<f64>,
    /// The linear gain applied to the raw render (loudness match times the shared peak factor).
    pub gain: f64,
    /// Peak of the final clip, dBFS.
    pub peak_dbfs: f64,
    /// Click and discontinuity findings (never a reason to drop a clip).
    pub flags: Vec<Flag>,
}

/// The kind of a trial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialKind {
    /// Target against solution.
    Real,
    /// The same clip as A and B: nothing to hear, any answer is a guess.
    Null,
    /// Two different targets: easy, checks attention.
    Different,
}

/// One trial.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trial {
    /// 1-based session.
    pub session: u32,
    /// 1-based position in the session.
    pub n: usize,
    /// Random id (the answer files refer to it).
    pub id: String,
    /// What kind of trial.
    pub kind: TrialKind,
    /// The item (for `Different`, the first one).
    pub item: usize,
    /// The second item of a `Different` trial.
    pub item_b: Option<usize>,
    /// Clip id played as A.
    pub a: String,
    /// Clip id played as B.
    pub b: String,
    /// Which of A and B X is a copy of.
    pub x_is: Side,
}

/// Everything about a design: the key to the answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`SCHEMA_MANIFEST`].
    pub schema: String,
    /// The design settings.
    pub config: Config,
    /// Logical threads of the machine that designed it.
    pub host_threads: usize,
    /// `uptime` load when the design started and ended.
    pub load: [String; 2],
    /// Wall-clock seconds of the whole design (all searches).
    pub design_wall_s: f64,
    /// Targets and solutions.
    pub items: Vec<Item>,
    /// Clip files.
    pub clips: Vec<ClipInfo>,
    /// Every trial of every session.
    pub trials: Vec<Trial>,
}

impl Manifest {
    /// The clip with this id.
    #[must_use]
    pub fn clip(&self, id: &str) -> Option<&ClipInfo> {
        self.clips.iter().find(|c| c.id == id)
    }

    /// The trials of one session, in play order.
    #[must_use]
    pub fn session_trials(&self, session: u32) -> Vec<&Trial> {
        self.trials
            .iter()
            .filter(|t| t.session == session)
            .collect()
    }

    /// Number of sessions.
    #[must_use]
    pub fn sessions(&self) -> u32 {
        self.config.trials_per_target as u32
    }
}
