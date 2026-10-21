//! The job file `cargo xtask gr3-run` writes (`enth.gr3-job/1`): the run's settings and its targets, already picked
//! from `content/gr3/synth.toml` and checked against the freeze by xtask. Strict on input: an unknown field is an
//! error, so a newer xtask's job is never half-read.

use std::path::PathBuf;

use enth_solver::create::{CreateCfg, SCORE_S};
use serde::{Deserialize, Serialize};

/// The job's schema id.
pub const SCHEMA: &str = "enth.gr3-job/1";

/// Which arm (docs/r3.md, "GR3 and its hygiene").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arm {
    /// Search alone: `create`'s own portfolio.
    A,
    /// Model + search: A's portfolio plus the model's proposals.
    B,
    /// The model's proposals, unrefined.
    C,
    /// A given the true template.
    D,
}

impl Arm {
    /// "A" to "D".
    pub fn name(self) -> &'static str {
        match self {
            Arm::A => "A",
            Arm::B => "B",
            Arm::C => "C",
            Arm::D => "D",
        }
    }
}

/// How a run is capped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// By wall-clock time (`seconds`), under the quiet lock: the timed run.
    Wall,
    /// By renders (`budget`, or each target's own budget for a twin): reproducible on any machine.
    Renders,
}

/// The create settings a run uses (`CreateCfg` less the budget, plus the scored window).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CfgJob {
    /// K.
    pub k: usize,
    /// m0: each branch's first milestone, in its renders.
    pub first_rung: usize,
    /// The halving factor.
    pub eta: usize,
    /// The branches pruning keeps at least.
    pub finalists: usize,
    /// k-NN neighbours proposed.
    pub neighbours: usize,
    /// The spread between neighbours (nepers).
    pub min_spread: f64,
    /// The first CMA-ES population (0: Hansen's default).
    pub lambda: usize,
    /// The job's seed.
    pub seed: u64,
    /// The scored window (s), from the clip's first sample.
    pub score_s: f64,
}

impl Default for CfgJob {
    fn default() -> Self {
        let c = CreateCfg::default();
        CfgJob {
            k: c.k,
            first_rung: c.first_rung,
            eta: c.eta,
            finalists: c.finalists,
            neighbours: c.neighbours,
            min_spread: c.min_spread,
            lambda: c.lambda,
            seed: c.seed,
            score_s: SCORE_S,
        }
    }
}

impl CfgJob {
    /// The `CreateCfg` for a job with this budget.
    pub fn create_cfg(&self, budget: usize) -> CreateCfg {
        CreateCfg {
            k: self.k,
            budget,
            seed: self.seed,
            neighbours: self.neighbours,
            min_spread: self.min_spread,
            lambda: self.lambda,
            stop_at: None,
            first_rung: self.first_rung,
            eta: self.eta,
            finalists: self.finalists,
        }
    }
}

/// A target's kind in `synth.toml`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A generator-v1 code.
    Generator,
    /// A frozen preset file and a performance.
    Preset,
}

/// A preset target's performance (`synth.toml`'s `perf`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerfJob {
    /// MIDI key.
    pub key: u8,
    /// MIDI velocity, 1–127.
    pub velocity: u8,
    /// How long the note is held (ms).
    pub hold_ms: u32,
    /// The render's length (ms).
    pub length_ms: u32,
}

/// One target, as xtask picked it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetJob {
    /// Its id in `synth.toml`.
    pub id: String,
    /// `test` or `dev`.
    pub split: String,
    /// Generator or preset.
    pub kind: Kind,
    /// The generator level (generator targets).
    #[serde(default)]
    pub level: Option<u8>,
    /// The generator's phrase, as the manifest lists it (generator targets; checked against the regenerated one).
    #[serde(default)]
    pub phrase: Option<u16>,
    /// In the pre-declared 40-target core.
    #[serde(default)]
    pub core: bool,
    /// The code (generator targets).
    #[serde(default)]
    pub code: Option<String>,
    /// The target fingerprint the manifest lists (generator targets).
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// The patch file (preset targets), absolute.
    #[serde(default)]
    pub file: Option<PathBuf>,
    /// The patch file's SHA-256 the manifest lists (preset targets).
    #[serde(default)]
    pub sha256: Option<String>,
    /// The performance (preset targets).
    #[serde(default)]
    pub perf: Option<PerfJob>,
    /// This target's own budget (a render-equivalent twin's renders); overrides the job's in render mode.
    #[serde(default)]
    pub budget: Option<usize>,
}

/// A run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// [`SCHEMA`].
    pub schema: String,
    /// A unique id: every record of the run carries it.
    pub run_id: String,
    /// The runs file the records are appended to (`content/gr3/runs.jsonl` for test runs).
    pub out: PathBuf,
    /// Where each target's checkpoint patches go (a JSON line per target), if anywhere: not committed.
    #[serde(default)]
    pub patches_out: Option<PathBuf>,
    /// The arm.
    pub arm: Arm,
    /// Wall-clock or render-capped.
    pub mode: Mode,
    /// The wall-clock cap (s), wall mode.
    #[serde(default)]
    pub seconds: Option<f64>,
    /// The render budget, render mode (a target's own `budget` overrides it).
    #[serde(default)]
    pub budget: Option<usize>,
    /// The checkpoint marks: seconds (wall mode) or renders (render mode); the cap is always added.
    #[serde(default)]
    pub marks: Vec<f64>,
    /// The pool's threads.
    pub workers: usize,
    /// The quiet lock, taken exclusively per target in wall mode (`/tmp/enth-quiet.lock`).
    #[serde(default)]
    pub lock: Option<PathBuf>,
    /// The create settings.
    pub cfg: CfgJob,
    /// The k-NN bank file, if any (R3a runs without one).
    #[serde(default)]
    pub bank: Option<PathBuf>,
    /// Use only the bank's first N entries.
    #[serde(default)]
    pub bank_entries: Option<usize>,
    /// A proposals file for arms B and C (a dry run until R3b's model exists).
    #[serde(default)]
    pub proposals: Option<PathBuf>,
    /// `test` or `dev`.
    pub split: String,
    /// What xtask knows and the tool copies into the run's first line: the build, the set's hashes, the command,
    /// the machine's idle check, a note.
    #[serde(default)]
    pub header: serde_json::Value,
    /// The targets, in run order.
    pub targets: Vec<TargetJob>,
}

/// The default checkpoint grid in wall mode (s): enough for anytime curves at 10, 30, 60 and 300 s and for time to
/// pass, at the resolution of the grid.
pub const WALL_MARKS: [f64; 14] = [
    2.0, 5.0, 10.0, 15.0, 20.0, 30.0, 45.0, 60.0, 90.0, 120.0, 180.0, 240.0, 300.0, 600.0,
];

/// The default checkpoint grid in render mode (renders).
pub const RENDER_MARKS: [f64; 10] = [
    100.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0, 32000.0, 64000.0,
];

/// A wall-mode job's render budget: a cap it never reaches, so only the clock stops it.
pub const UNREACHED: usize = 100_000_000;

impl Job {
    /// Refuse a job that cannot run.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!("the job's schema is {}, not {SCHEMA}", self.schema));
        }
        if self.workers == 0 {
            return Err("workers must be at least 1".to_owned());
        }
        match self.mode {
            Mode::Wall => {
                if !self.seconds.is_some_and(|s| s.is_finite() && s > 0.0) {
                    return Err("a wall-clock run needs seconds > 0".to_owned());
                }
            }
            Mode::Renders => {
                if self.budget.is_none() && self.targets.iter().any(|t| t.budget.is_none()) {
                    return Err("a render-capped run needs a budget (or one per target)".to_owned());
                }
            }
        }
        if !(self.cfg.score_s.is_finite() && self.cfg.score_s > 0.0) {
            return Err("score_s must be positive".to_owned());
        }
        if self.targets.is_empty() {
            return Err("the job has no targets".to_owned());
        }
        Ok(())
    }

    /// A target's render budget (wall mode: one it never reaches).
    pub fn budget_of(&self, t: &TargetJob) -> usize {
        match self.mode {
            Mode::Wall => UNREACHED,
            Mode::Renders => t.budget.or(self.budget).unwrap_or(UNREACHED).max(1),
        }
    }

    /// The checkpoint marks for a target: the job's (or the default grid) up to the cap, and the cap.
    pub fn marks_of(&self, t: &TargetJob) -> Vec<f64> {
        let cap = match self.mode {
            Mode::Wall => self.seconds.unwrap_or(f64::INFINITY),
            Mode::Renders => self.budget_of(t) as f64,
        };
        let grid: Vec<f64> = if self.marks.is_empty() {
            match self.mode {
                Mode::Wall => WALL_MARKS.to_vec(),
                Mode::Renders => RENDER_MARKS.to_vec(),
            }
        } else {
            self.marks.clone()
        };
        let mut out: Vec<f64> = grid.into_iter().filter(|m| *m > 0.0 && *m < cap).collect();
        out.push(cap);
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }
}
