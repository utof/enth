//! **Owner: lane SK3** (the Create skeleton, freeze point F2s). Pre-declared by lane S0s (R3, freeze point F0s).
//!
//! The pure create state (docs/r3.md, "The Create view and the session"; docs/session.md, "Create"). Like
//! [`game`](crate::game) and [`learn`](crate::learn) it does no I/O, spawns no threads and reads no clocks: **time
//! comes from the shell's posts**, and background work runs as jobs the shell executes on its own thread and hands
//! back. The audio thread never touches any of it.
//!
//! - **`Run` and `Cancel` go out to the shell** ([`CreateRequest`], drained with [`Session::drain_create_requests`]
//!   and acted on after unlocking): a create job (the file to read and the [`CreateSettings`]: K, the wall-clock cap,
//!   the render cap, the seed) goes to the shell's `enth-create` thread and its per-job pool (`enth-plugin`'s
//!   `create.rs`), never to the game worker. The shell decodes the file, runs the intake and the search, and posts
//!   back ([`CreatePost`], through [`Session::create_post`]): the intake's report, then after every round the
//!   progress and the K candidates (`enth_solver::create`'s [`Candidate`]: patch, distance, provenance, trace; each
//!   tile moves proposed, searching, done or pruned), then the end (why it stopped) or a failure. A post for a job the
//!   session no longer runs is refused as stale and changes nothing.
//! - **The read model** ([`CreateView`], from [`CreateEdit::create`]) is what the Create view draws: the file, the
//!   phase, the intake's summary and warnings, the progress, the K candidates (with their ranks), the one taken, the
//!   one auditioned, the shell's notes and the last error. It is rebuilt on every call; it holds no patch (the
//!   session keeps them, and every intent names a candidate by its branch).
//! - **"Use this" is one undo step** ([`CreateEdit::create_use`]): the candidate's refined patch (or its proposal,
//!   Synplant-style) loads through the session's preset load, `enth_patch::ops_between` of the patch and the
//!   candidate, delivered like a host load (one plan that forgets the remembered host slot values) with **no host
//!   writes**: patches load through the session, never through host automation (AGENTS.md, "Plugin shell"). Its
//!   name is the file's stem. Refused while a lesson is live, as a preset load is.
//! - **Audition** ([`CreateEdit::create_audition`]) asks the shell to render a candidate with the job's performance
//!   at the host's rate and play it once ([`CreateRequest::Audition`]); [`CreateRequest::StopAudition`] stops it.
//! - **Nothing is saved.** A create job does not survive a project reload (the coordinator's decision, kept by the
//!   F1s judge): the patch you take is in the project, the job is not. So this module adds no field to the `enth`
//!   state blob, `enth.plugin-state/1` is untouched, and host loads leave the create state alone.
//!
//! After F2s the session's create types change only additively (docs/phase-3.md, "Freezes"): the enums and structs
//! the shell builds are `non_exhaustive`, with constructors.

use std::path::PathBuf;

use enth_analysis::intake::{IntakeReport, Warning};
use enth_patch::Patch;
pub use enth_solver::create::{Candidate, CandidateState, CreateStop, Provenance};

use crate::model::{Session, SessionError};

/// The longest wall-clock cap a job may ask for (s): "every result arrives in under 5 minutes" (GR3, clause 3).
pub const MAX_SECONDS: f64 = 300.0;
/// The most candidates (K) a job may ask for.
pub const MAX_CANDIDATES: usize = 16;

/// A create job's id: every post carries it; a post for a job the session no longer runs is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CreateJobId(pub u64);

/// What a job is asked for. [`Default`] is the plugin's: 8 candidates in at most 60 s, the solver's render cap, seed 1.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateSettings {
    /// K, the candidates refined at once (1 to [`MAX_CANDIDATES`]).
    pub candidates: usize,
    /// The wall-clock cap (s), from the file's decoding to the last candidate (above 0, at most [`MAX_SECONDS`]).
    pub seconds: f64,
    /// The render cap; `None` is the solver's default (`CreateCfg::default().budget`). Only a cap: the schedule never
    /// depends on it.
    pub budget: Option<usize>,
    /// The search's seed.
    pub seed: u64,
}

impl Default for CreateSettings {
    fn default() -> Self {
        CreateSettings {
            candidates: 8,
            seconds: 60.0,
            budget: None,
            seed: 1,
        }
    }
}

impl CreateSettings {
    /// Whether the settings are in range.
    ///
    /// # Errors
    /// [`CreateError::BadSettings`] naming the first setting out of range.
    pub fn check(&self) -> Result<(), CreateError> {
        if !(1..=MAX_CANDIDATES).contains(&self.candidates) {
            return Err(CreateError::BadSettings("candidates must be 1 to 16"));
        }
        if !(self.seconds.is_finite() && self.seconds > 0.0 && self.seconds <= MAX_SECONDS) {
            return Err(CreateError::BadSettings(
                "seconds must be above 0 and at most 300",
            ));
        }
        if self.budget == Some(0) {
            return Err(CreateError::BadSettings(
                "the render budget must be at least 1",
            ));
        }
        Ok(())
    }
}

/// Where the sound comes from: a file the shell reads (the session never opens it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateSource {
    /// The file.
    pub path: PathBuf,
}

impl CreateSource {
    /// A file.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        CreateSource { path: path.into() }
    }

    /// The file's name, for the view (the whole path when it has none).
    #[must_use]
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    /// The file's stem ("kick" for `kick.wav`): the name a taken candidate gets. `None` when it has none.
    #[must_use]
    pub fn stem(&self) -> Option<String> {
        self.path
            .file_stem()
            .map(|s| s.to_string_lossy().trim().to_owned())
            .filter(|s| !s.is_empty())
    }
}

/// A job for the shell's `enth-create` thread.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateRun {
    /// Its id (the posts carry it back).
    pub id: CreateJobId,
    /// The file.
    pub source: CreateSource,
    /// What it is asked for.
    pub settings: CreateSettings,
}

/// Which patch of a candidate: what search made of it, or the start it grew from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Which {
    /// The branch's best patch.
    Refined,
    /// The start it grew from (the heuristic's, a template, a k-NN neighbour).
    Proposal,
}

/// The create outbox the shell drains ([`Session::drain_create_requests`]) and acts on after unlocking.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CreateRequest {
    /// Run a job on the `enth-create` thread.
    Run(CreateRun),
    /// Cancel a job: the shell's pool stops within one batch, drops the batch out (never tells it), and posts
    /// [`CreatePost::Done`] with what the rounds before it found.
    Cancel(CreateJobId),
    /// Render `patch` (a candidate of `id`) with the job's performance at the host's rate and play it once.
    Audition {
        /// The job whose performance it plays.
        id: CreateJobId,
        /// The candidate.
        branch: usize,
        /// Which of its patches.
        which: Which,
        /// The patch.
        patch: Box<Patch>,
    },
    /// Stop an audition.
    StopAudition,
}

/// What the intake found, for the drop zone: the file, how it decoded, the intake's report whole, the shell's notes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CreateIntake {
    /// How the file decoded ("MP3, 128 kbps, 44100 Hz, 2 channels").
    pub decoded: String,
    /// The intake's report: the performance it inferred, the degradation, the warnings.
    pub report: IntakeReport,
    /// What the user should know (a file decimated 2:1, no k-NN bank found).
    pub notes: Vec<String>,
}

impl CreateIntake {
    /// The intake's post.
    #[must_use]
    pub fn new(decoded: impl Into<String>, report: IntakeReport, notes: Vec<String>) -> Self {
        CreateIntake {
            decoded: decoded.into(),
            report,
            notes,
        }
    }

    /// The warnings as short sentences.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        self.report.warnings.iter().map(warning_text).collect()
    }
}

/// A warning of the intake as a short sentence.
#[must_use]
pub fn warning_text(w: &Warning) -> String {
    match w {
        Warning::Phrase { onsets } => format!("{onsets} onsets: this looks like a phrase; one note is matched"),
        Warning::Notes { keys } => format!("{} different notes: the first is matched", keys.len()),
        Warning::Chord { tones_hz } => format!("{} pitches at once: a chord, or an inharmonic sound", tones_hz.len()),
        Warning::Unpitched => "no stable pitch: the note is a default".to_owned(),
        Warning::Capped { source_s } => format!("the sound runs {source_s:.1} s: its first seconds are matched"),
        Warning::ChannelsCancel { loss_db } => {
            format!("the channels cancel in the mono mix ({loss_db:.1} dB lost)")
        }
        _ => "a warning this build does not know".to_owned(),
    }
}

/// The job between rounds, as the shell measured it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CreateProgress {
    /// Rounds evaluated.
    pub rounds: usize,
    /// Renders used.
    pub renders: usize,
    /// The render cap.
    pub budget: usize,
    /// Seconds since the job started (the shell's clock).
    pub seconds: f64,
    /// The wall-clock cap (s).
    pub cap_s: f64,
}

impl CreateProgress {
    /// The progress after a round.
    #[must_use]
    pub fn new(rounds: usize, renders: usize, budget: usize, seconds: f64, cap_s: f64) -> Self {
        CreateProgress {
            rounds,
            renders,
            budget,
            seconds,
            cap_s,
        }
    }

    /// How far the job is, 0 to 1: the larger of renders against the cap and time against the cap.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        let renders = if self.budget == 0 {
            0.0
        } else {
            self.renders as f64 / self.budget as f64
        };
        let time = if self.cap_s > 0.0 {
            self.seconds / self.cap_s
        } else {
            0.0
        };
        renders.max(time).clamp(0.0, 1.0) as f32
    }
}

/// What the shell posts back for a job ([`Session::create_post`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CreatePost {
    /// The file decoded and the intake ran.
    Intake(Box<CreateIntake>),
    /// A round ended: the progress and every branch as a candidate (its best patch so far).
    Round {
        /// The progress.
        progress: CreateProgress,
        /// The K branches, in branch order.
        candidates: Vec<Candidate>,
    },
    /// The job ended: the budget, the wall-clock cap or a cancel (`cancelled`), exhaustion or the goal.
    Done {
        /// The progress at the end.
        progress: CreateProgress,
        /// Why it ended (`cancelled` for the shell's cap as for a cancel; `cap` in `notes` says which).
        stop: CreateStop,
        /// The K branches, in branch order.
        candidates: Vec<Candidate>,
        /// What the user should know.
        notes: Vec<String>,
    },
    /// The job could not run (a file that does not decode, a silent file, a rate the analysis refuses).
    Failed {
        /// A stable snake-case code.
        code: String,
        /// What happened.
        message: String,
        /// What to do about it.
        hint: String,
    },
}

/// Where a job stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CreatePhase {
    /// Asked for: the shell decodes the file and runs the intake.
    Starting,
    /// The search runs.
    Searching,
    /// A cancel was asked for; the shell's last post is on its way.
    Cancelling,
    /// It ended.
    Done(CreateStop),
    /// It could not run ([`CreateView::error`]).
    Failed,
}

impl CreatePhase {
    /// Whether the job still runs in the shell.
    #[must_use]
    pub fn running(self) -> bool {
        matches!(
            self,
            CreatePhase::Starting | CreatePhase::Searching | CreatePhase::Cancelling
        )
    }
}

/// One candidate tile.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CandidateView {
    /// Its branch (stable while the job runs).
    pub branch: usize,
    /// Its place from best (0) to worst, by distance (ties to the lower branch).
    pub rank: usize,
    /// Where its start came from.
    pub provenance: Provenance,
    /// Where it stands.
    pub state: CandidateState,
    /// Its distance (the objective's, lower is better); `None` before it is evaluated.
    pub distance: Option<f64>,
    /// Its start's own distance.
    pub proposal_distance: Option<f64>,
    /// Renders its search used.
    pub renders: usize,
}

impl CandidateView {
    /// A short label: "heuristic", "template `pluck_1osc`", "k-NN 3", "model 2".
    #[must_use]
    pub fn label(&self) -> String {
        provenance_label(&self.provenance)
    }
}

/// A short label for a start's provenance.
#[must_use]
pub fn provenance_label(p: &Provenance) -> String {
    match p {
        Provenance::Heuristic => "heuristic".to_owned(),
        Provenance::Template { name } => format!("template {name}"),
        Provenance::Knn { rank, .. } => format!("k-NN {}", rank + 1),
        Provenance::Model { hypothesis, .. } => format!("model {}", hypothesis + 1),
        _ => "unknown".to_owned(),
    }
}

/// An error the shell posted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateFailure {
    /// A stable snake-case code.
    pub code: String,
    /// What happened.
    pub message: String,
    /// What to do about it.
    pub hint: String,
}

/// The Create view's read model ([`CreateEdit::create`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CreateView {
    /// The job.
    pub id: CreateJobId,
    /// Where it stands.
    pub phase: CreatePhase,
    /// The file's name.
    pub file: String,
    /// What it was asked for.
    pub settings: CreateSettings,
    /// What the intake found, once the shell posted it.
    pub intake: Option<CreateIntake>,
    /// The latest progress.
    pub progress: Option<CreateProgress>,
    /// The K candidates, in branch order.
    pub candidates: Vec<CandidateView>,
    /// The candidate last taken with "Use this".
    pub used: Option<(usize, Which)>,
    /// The candidate last auditioned.
    pub auditioning: Option<(usize, Which)>,
    /// Why it failed.
    pub error: Option<CreateFailure>,
    /// What the shell said at the end (the cap, a cancel, a missing bank).
    pub notes: Vec<String>,
}

impl CreateView {
    /// The best candidate's branch.
    #[must_use]
    pub fn best(&self) -> Option<usize> {
        self.candidates.iter().find(|c| c.rank == 0).map(|c| c.branch)
    }
}

/// Why the session refused a create intent or post.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CreateError {
    /// A job is running: cancel it first.
    Running,
    /// No job (start one first).
    NoJob,
    /// The job has no candidate on that branch (yet).
    NoCandidate {
        /// The branch asked for.
        branch: usize,
    },
    /// A post for a job the session no longer runs.
    Stale(CreateJobId),
    /// A setting is out of range.
    BadSettings(&'static str),
    /// The source names no file.
    NoFile,
    /// The load was refused (a lesson is live, or the patch does not apply).
    Session(SessionError),
}

impl CreateError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            CreateError::Running => "create_running",
            CreateError::NoJob => "no_create_job",
            CreateError::NoCandidate { .. } => "no_candidate",
            CreateError::Stale(_) => "stale_post",
            CreateError::BadSettings(_) => "bad_create_settings",
            CreateError::NoFile => "no_file",
            CreateError::Session(e) => e.code(),
        }
    }

    /// What to do about it.
    #[must_use]
    pub fn hint(&self) -> String {
        match self {
            CreateError::Running => "a create job is running: cancel it first, or wait for it".to_owned(),
            CreateError::NoJob => "drop a sound file and press Start first".to_owned(),
            CreateError::NoCandidate { .. } => "wait for the candidates of the first round".to_owned(),
            CreateError::Stale(_) => "a result for a job that is gone: nothing to do".to_owned(),
            CreateError::BadSettings(what) => format!("fix the setting: {what}"),
            CreateError::NoFile => "choose a sound file (WAV, FLAC, Ogg Vorbis or MP3)".to_owned(),
            CreateError::Session(e) => e.hint(),
        }
    }
}

impl std::fmt::Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateError::Running => f.write_str("a create job is running"),
            CreateError::NoJob => f.write_str("no create job"),
            CreateError::NoCandidate { branch } => write!(f, "no candidate {branch}"),
            CreateError::Stale(id) => write!(f, "a post for create job {} which is gone", id.0),
            CreateError::BadSettings(what) => write!(f, "bad create setting: {what}"),
            CreateError::NoFile => f.write_str("no file"),
            CreateError::Session(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CreateError {}

/// The editor's create intents and read model. [`Session`] implements it; a kittest fake can.
pub trait CreateEdit {
    /// The job, if there is one.
    fn create(&self) -> Option<CreateView>;
    /// Start a job on `source` with `settings`. Refused while a job runs ([`CreateError::Running`]); a finished
    /// job's candidates are forgotten.
    ///
    /// # Errors
    /// [`CreateError::Running`], [`CreateError::BadSettings`], [`CreateError::NoFile`].
    fn create_start(
        &mut self,
        source: CreateSource,
        settings: CreateSettings,
    ) -> Result<CreateJobId, CreateError>;
    /// Cancel the running job: the shell stops within one batch and posts what it found.
    ///
    /// # Errors
    /// [`CreateError::NoJob`] when no job runs.
    fn create_cancel(&mut self) -> Result<(), CreateError>;
    /// "Use this": load the candidate's patch as one undo step, with no host writes.
    ///
    /// # Errors
    /// [`CreateError::NoJob`], [`CreateError::NoCandidate`], or [`CreateError::Session`] when the load is refused
    /// (`lesson_live` while a lesson is live).
    fn create_use(&mut self, branch: usize, which: Which) -> Result<(), CreateError>;
    /// Play the candidate's patch once, rendered by the shell with the job's performance.
    ///
    /// # Errors
    /// [`CreateError::NoJob`], [`CreateError::NoCandidate`].
    fn create_audition(&mut self, branch: usize, which: Which) -> Result<(), CreateError>;
    /// Stop an audition.
    fn create_stop_audition(&mut self);
    /// Close the job: a running one is cancelled, and its later posts are stale.
    fn create_clear(&mut self);
}

#[derive(Debug)]
struct JobLive {
    id: CreateJobId,
    source: CreateSource,
    settings: CreateSettings,
    phase: CreatePhase,
    intake: Option<CreateIntake>,
    progress: Option<CreateProgress>,
    candidates: Vec<Candidate>,
    used: Option<(usize, Which)>,
    auditioning: Option<(usize, Which)>,
    error: Option<CreateFailure>,
    notes: Vec<String>,
}

/// The create state the session keeps between calls (never saved).
#[derive(Debug, Default)]
pub(crate) struct CreateLive {
    next_id: u64,
    job: Option<JobLive>,
    requests: Vec<CreateRequest>,
}

impl Session {
    /// The create outbox: [`CreateRequest`]s for the shell to act on after unlocking (wherever `drain_requests` is
    /// drained).
    pub fn drain_create_requests(&mut self) -> std::vec::Drain<'_, CreateRequest> {
        self.create.requests.drain(..)
    }

    /// Hand back a post of the shell's for job `id` (under the lock, on the GUI or host main thread).
    ///
    /// # Errors
    /// [`CreateError::Stale`] for a job the session no longer runs (nothing changes).
    pub fn create_post(&mut self, id: CreateJobId, post: CreatePost) -> Result<(), CreateError> {
        let Some(job) = self.create.job.as_mut().filter(|j| j.id == id) else {
            return Err(CreateError::Stale(id));
        };
        if !job.phase.running() {
            return Err(CreateError::Stale(id));
        }
        match post {
            CreatePost::Intake(intake) => {
                job.notes.extend(intake.notes.iter().cloned());
                job.intake = Some(*intake);
                if job.phase == CreatePhase::Starting {
                    job.phase = CreatePhase::Searching;
                }
            }
            CreatePost::Round {
                progress,
                candidates,
            } => {
                job.progress = Some(progress);
                job.candidates = candidates;
                if job.phase == CreatePhase::Starting {
                    job.phase = CreatePhase::Searching;
                }
            }
            CreatePost::Done {
                progress,
                stop,
                candidates,
                notes,
            } => {
                job.progress = Some(progress);
                if !candidates.is_empty() || job.candidates.is_empty() {
                    job.candidates = candidates;
                }
                job.notes.extend(notes);
                job.phase = CreatePhase::Done(stop);
            }
            CreatePost::Failed {
                code,
                message,
                hint,
            } => {
                job.error = Some(CreateFailure {
                    code,
                    message,
                    hint,
                });
                job.phase = CreatePhase::Failed;
            }
        }
        Ok(())
    }

    fn create_candidate(&self, branch: usize, which: Which) -> Result<(&JobLive, Patch), CreateError> {
        let job = self.create.job.as_ref().ok_or(CreateError::NoJob)?;
        let c = job
            .candidates
            .iter()
            .find(|c| c.branch == branch)
            .ok_or(CreateError::NoCandidate { branch })?;
        let patch = match which {
            Which::Refined => c.patch.clone(),
            Which::Proposal => c.proposal.clone(),
        };
        Ok((job, patch))
    }
}

/// The candidates' ranks: from best (0) to worst by distance, ties to the lower branch.
fn ranks(candidates: &[Candidate]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&a, &b| {
        candidates[a]
            .distance
            .total_cmp(&candidates[b].distance)
            .then(candidates[a].branch.cmp(&candidates[b].branch))
    });
    let mut rank = vec![0; candidates.len()];
    for (r, i) in order.into_iter().enumerate() {
        rank[i] = r;
    }
    rank
}

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

impl CreateEdit for Session {
    fn create(&self) -> Option<CreateView> {
        let job = self.create.job.as_ref()?;
        let rank = ranks(&job.candidates);
        Some(CreateView {
            id: job.id,
            phase: job.phase,
            file: job.source.name(),
            settings: job.settings.clone(),
            intake: job.intake.clone(),
            progress: job.progress.clone(),
            candidates: job
                .candidates
                .iter()
                .zip(rank)
                .map(|(c, rank)| CandidateView {
                    branch: c.branch,
                    rank,
                    provenance: c.provenance.clone(),
                    state: c.state,
                    distance: finite(c.distance),
                    proposal_distance: c.proposal_distance.and_then(finite),
                    renders: c.renders,
                })
                .collect(),
            used: job.used,
            auditioning: job.auditioning,
            error: job.error.clone(),
            notes: job.notes.clone(),
        })
    }

    fn create_start(
        &mut self,
        source: CreateSource,
        settings: CreateSettings,
    ) -> Result<CreateJobId, CreateError> {
        if self.create.job.as_ref().is_some_and(|j| j.phase.running()) {
            return Err(CreateError::Running);
        }
        settings.check()?;
        if source.path.as_os_str().is_empty() {
            return Err(CreateError::NoFile);
        }
        self.create.next_id += 1;
        let id = CreateJobId(self.create.next_id);
        if self.create.job.as_ref().is_some_and(|j| j.auditioning.is_some()) {
            self.create.requests.push(CreateRequest::StopAudition);
        }
        self.create.requests.push(CreateRequest::Run(CreateRun {
            id,
            source: source.clone(),
            settings: settings.clone(),
        }));
        self.create.job = Some(JobLive {
            id,
            source,
            settings,
            phase: CreatePhase::Starting,
            intake: None,
            progress: None,
            candidates: Vec::new(),
            used: None,
            auditioning: None,
            error: None,
            notes: Vec::new(),
        });
        Ok(id)
    }

    fn create_cancel(&mut self) -> Result<(), CreateError> {
        let job = self
            .create
            .job
            .as_mut()
            .filter(|j| j.phase.running())
            .ok_or(CreateError::NoJob)?;
        if job.phase != CreatePhase::Cancelling {
            job.phase = CreatePhase::Cancelling;
            let id = job.id;
            self.create.requests.push(CreateRequest::Cancel(id));
        }
        Ok(())
    }

    fn create_use(&mut self, branch: usize, which: Which) -> Result<(), CreateError> {
        let (job, mut patch) = self.create_candidate(branch, which)?;
        if let Some(stem) = job.source.stem() {
            patch.meta.name = stem;
        }
        self.load_preset(&patch, None)
            .map_err(CreateError::Session)?;
        if let Some(job) = self.create.job.as_mut() {
            job.used = Some((branch, which));
        }
        Ok(())
    }

    fn create_audition(&mut self, branch: usize, which: Which) -> Result<(), CreateError> {
        let (job, patch) = self.create_candidate(branch, which)?;
        let id = job.id;
        self.create.requests.push(CreateRequest::Audition {
            id,
            branch,
            which,
            patch: Box::new(patch),
        });
        if let Some(job) = self.create.job.as_mut() {
            job.auditioning = Some((branch, which));
        }
        Ok(())
    }

    fn create_stop_audition(&mut self) {
        if let Some(job) = self.create.job.as_mut() {
            job.auditioning = None;
        }
        self.create.requests.push(CreateRequest::StopAudition);
    }

    fn create_clear(&mut self) {
        if let Some(job) = self.create.job.take() {
            if job.phase.running() && job.phase != CreatePhase::Cancelling {
                self.create.requests.push(CreateRequest::Cancel(job.id));
            }
            if job.auditioning.is_some() {
                self.create.requests.push(CreateRequest::StopAudition);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn the_state_and_its_messages_move_between_threads() {
        assert_send::<CreateRequest>();
        assert_send::<CreatePost>();
        assert_send::<CreateView>();
    }

    #[test]
    fn settings_out_of_range_are_refused() {
        assert_eq!(CreateSettings::default().check(), Ok(()));
        for bad in [
            CreateSettings {
                candidates: 0,
                ..CreateSettings::default()
            },
            CreateSettings {
                candidates: 17,
                ..CreateSettings::default()
            },
            CreateSettings {
                seconds: 0.0,
                ..CreateSettings::default()
            },
            CreateSettings {
                seconds: 301.0,
                ..CreateSettings::default()
            },
            CreateSettings {
                seconds: f64::NAN,
                ..CreateSettings::default()
            },
            CreateSettings {
                budget: Some(0),
                ..CreateSettings::default()
            },
        ] {
            assert_eq!(bad.check().map_err(|e| e.code()), Err("bad_create_settings"));
        }
    }

    #[test]
    fn a_source_names_its_file_and_stem() {
        let s = CreateSource::file("/tmp/sounds/kick 01.wav");
        assert_eq!(s.name(), "kick 01.wav");
        assert_eq!(s.stem().as_deref(), Some("kick 01"));
        assert_eq!(CreateSource::file("").stem(), None);
    }

    #[test]
    fn progress_is_the_larger_of_renders_and_time() {
        let p = CreateProgress::new(3, 1000, 4000, 30.0, 60.0);
        assert!((p.fraction() - 0.5).abs() < 1e-6);
        let p = CreateProgress::new(3, 4000, 4000, 1.0, 60.0);
        assert!((p.fraction() - 1.0).abs() < 1e-6);
    }
}
